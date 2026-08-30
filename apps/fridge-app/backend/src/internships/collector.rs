//! Running a collection: fetch, QC, dedup, persist, settle, sweep.
//!
//! This is the coordinator. Adapters fetch and return values; `normalize` judges rows;
//! `expiry` owns the disappearance rule; nothing else writes to the database. Everything that
//! could lose data lives here, deliberately, in one readable place.
//!
//! # The order of operations matters
//!
//! 1. Open a `collection_runs` row.
//! 2. `sources::collect_streaming` — per-source isolation, one task each, failures recorded
//!    not raised, **each source handed over as it finishes** rather than after the slowest.
//! 3. Per source: QC every raw posting, write the rejects, upsert what survived, then
//!    [`expiry::settle_source_run`] — which is the **only** thing that may advance
//!    disappearance counters, and only if the run earned it.
//! 4. Recompute `company_signals`.
//! 5. `expiry::sweep`, which reads counters and deadlines and never looks at `source_runs`.
//!
//! # Two rules here that are easy to get subtly wrong
//!
//! **`seen_external_ids` is every id the source returned, not every id that survived QC.**
//! The miss counter answers "does the source still list this?", which is a question about the
//! *source*, not about our opinion of the row. If a posting we already track comes back and QC
//! rejects it because of a parser bug on our side, it has still been *seen* — counting it as
//! missing would let one of our own defects expire real postings. Filtering is our judgement;
//! disappearance is the source's statement.
//!
//! **An explicit closure flag does not need a complete enumeration; an absence does.**
//! `closed_external_ids` (Simplify's `active: false`, per `docs/INTERNSHIP_SCRAPING.md` § D.1,
//! the strongest closure signal available) is a *positive statement* about a specific posting:
//! the record we read says it is closed. That is valid evidence even from a `Partial` run,
//! because we read the record. Absence is the opposite — it is only evidence if we saw
//! everything — which is why it goes through `counts_for_expiry` and this does not.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::hunt;

use super::alerts;
use super::dedup::dedup_key;
use super::expiry::{self, SourceRunResult};
use super::http::PoliteClient;
use super::models::{NormalizedPosting, QcOutcome, RawPosting};
use super::normalize;
use super::prestige;
use super::sources::{self, SourceContext};

/// How often a collection runs, when nothing overrides it. Six hours, not the blog watcher's
/// five seconds: this one crosses the network to other people's servers, and every source in
/// `docs/INTERNSHIP_SCRAPING.md` is a job board that changes on the order of hours.
pub const DEFAULT_COLLECT_INTERVAL_SECS: u64 = 21_600;

/// How often the expiry sweep runs. More often than collection, because a *deadline* passing
/// needs no new data — it happens on the clock.
pub const DEFAULT_EXPIRY_INTERVAL_SECS: u64 = 3_600;

/// What one collection did, for the log and the manual-trigger response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CollectionReport {
    pub run_id: String,
    pub sources_run: usize,
    pub sources_succeeded: usize,
    pub fetched: i64,
    pub accepted: i64,
    pub filtered: i64,
    pub rejected: i64,
    pub postings_created: i64,
    pub postings_updated: i64,
    /// `hunt_events` rows this run wrote — see [`super::alerts`]. Reported rather than left
    /// to be counted by hand in `sqlite3`, because "a new tier-1/2 posting raises exactly one
    /// notification, and re-running raises none" is the checkpoint this producer has to meet,
    /// and a number in the manual-collect response is how you check it.
    pub alerts_created: i64,
    pub marked_closed: u64,
    pub swept_deadline: u64,
    pub swept_vanished: u64,
}

/// Run one collection across every registered source.
///
/// Never returns `Err` for a source-level problem — that is the isolation rule. An `Err` here
/// means the *database* or the HTTP client could not be set up at all, which is a real failure
/// of the run rather than of a source.
pub async fn collect(pool: &SqlitePool, trigger: &str) -> Result<CollectionReport> {
    let client = PoliteClient::new()?;
    collect_with(pool, trigger, sources::registry(), Arc::new(configured_context(client))).await
}

/// Build the source context from the environment.
///
/// # `INTERNSHIP_MAX_BOARDS_PER_RUN` is an operational safety valve with a real cost
///
/// The vendored directory holds ~2,084 board slugs. At the polite one-second-per-host floor,
/// an unbounded run is on the order of half an hour of continuous requests to other people's
/// servers. Capping it is often the right call — but **a capped run has not enumerated the
/// source**, so its adapter reports `Partial`, and per the expiry rule a `Partial` run can
/// never expire anything. Set this and postings stop being swept for disappearance; leave it
/// unset and every run is a full sweep. Neither is free, so it is a deliberate setting rather
/// than a default.
///
/// `INTERNSHIP_DISABLED_SOURCES` is a comma-separated list. A disabled source still writes a
/// `Skipped` run record — a source that silently vanishes from the health panel looks exactly
/// like a source nobody noticed breaking.
fn configured_context(client: PoliteClient) -> SourceContext {
    let mut ctx = SourceContext::new(client);

    if let Ok(raw) = std::env::var("INTERNSHIP_DISABLED_SOURCES") {
        ctx.disabled_sources = raw
            .split(',')
            .map(|name| name.trim().to_lowercase())
            .filter(|name| !name.is_empty())
            .collect();
        if !ctx.disabled_sources.is_empty() {
            println!(
                "internships: sources disabled by configuration: {}",
                ctx.disabled_sources.join(", ")
            );
        }
    }

    match std::env::var("INTERNSHIP_MAX_BOARDS_PER_RUN") {
        Err(_) => {}
        Ok(value) => match value.trim().parse::<usize>() {
            Ok(0) | Err(_) => eprintln!(
                "internships: INTERNSHIP_MAX_BOARDS_PER_RUN={value:?} is not a positive \
                 number — ignoring, so every board is polled"
            ),
            Ok(cap) => {
                println!(
                    "internships: capped at {cap} boards per run — capped sources report \
                     Partial and will never expire postings"
                );
                ctx.max_boards_per_run = cap;
            }
        },
    }

    ctx
}

/// [`collect`] with the sources and context injected.
///
/// Split out purely so this path can be tested. Everything expensive to get wrong lives below
/// — the QC accounting, the merge, the sighting bookkeeping, what counts as "seen" — and none
/// of it is reachable by a test that has to cross the network first. The Phase 5 lesson in
/// `apps/fridge-app/CLAUDE.md` is blunt about this: its four worst bugs were all in paths that
/// had never executed, while `cargo test` reported everything green.
pub async fn collect_with(
    pool: &SqlitePool,
    trigger: &str,
    source_list: Vec<Arc<dyn sources::Source>>,
    ctx: Arc<SourceContext>,
) -> Result<CollectionReport> {
    let started = Utc::now();
    let run_id = Uuid::new_v4().to_string();

    sqlx::query("INSERT INTO collection_runs (id, started_at, trigger) VALUES (?1, ?2, ?3)")
        .bind(&run_id)
        .bind(started.to_rfc3339())
        .bind(trigger)
        .execute(pool)
        .await?;

    // Streamed rather than batched: each source is persisted the moment it finishes, so the
    // tab fills up progressively instead of staying empty until the slowest source returns.
    // On an uncapped run that is the difference between "nothing for half an hour" and "most
    // of the corpus within seconds", because Simplify answers quickly and supplies the bulk.
    // It also means `source_runs` rows appear incrementally, which is what lets the UI show
    // real progress while a run is in flight.
    let sources_run = source_list.len();
    let mut receiver = sources::collect_streaming(source_list, Arc::clone(&ctx));

    // Loaded once for the whole run and shared by the two things that need it: the alert
    // predicate below, and `recompute_company_signals` at the end. It reads a file, and a run
    // that inserts a few hundred postings would otherwise read it a few hundred times.
    let tiers = prestige::CompanyTiers::load();

    let mut report = CollectionReport {
        run_id: run_id.clone(),
        sources_run,
        ..CollectionReport::default()
    };

    while let Some((_, output)) = receiver.recv().await {
        let source = output.result.source.clone();
        let now = Utc::now();

        // Every id this source returned, before QC has an opinion. See the module doc.
        let seen_external_ids: Vec<String> = output
            .postings
            .iter()
            .map(|posting| posting.external_id.clone())
            .collect();

        let source_run_id = Uuid::new_v4().to_string();
        let mut counts = QcCounts::default();

        // Open the `source_runs` row *before* QC, because every reject references it and
        // `posting_rejects.source_run_id` is a real enforced foreign key — sqlx turns
        // `PRAGMA foreign_keys` on per connection, whatever migration 0007's comment says.
        // Writing rejects first meant every single one failed, which is the precise failure
        // this table exists to prevent: the row dropped *and* invisible.
        //
        // The outcome is already known here — it comes from the fetch, and QC only supplies
        // the counts — so this is the final outcome with zero counts, which
        // `settle_source_run` then updates in place via its ON CONFLICT (run_id, source).
        if let Err(err) = open_source_run(pool, &run_id, &source_run_id, &source,
                                          output.result.outcome, now).await
        {
            eprintln!("internships: {source}: could not open the run record: {err:?}");
        }

        for raw in &output.postings {
            match normalize::normalize(raw, now) {
                QcOutcome::Accepted(normalized) => {
                    counts.accepted += 1;
                    match upsert_posting(pool, &normalized, &run_id, now).await {
                        Ok(upserted) if upserted.created => {
                            report.postings_created += 1;
                            report.alerts_created +=
                                emit_posting_alert(pool, &tiers, &normalized, &upserted.id, now)
                                    .await;
                        }
                        Ok(_) => report.postings_updated += 1,
                        Err(err) => {
                            // One unwritable row must not abort the source, let alone the run.
                            // It is recorded as a reject so it is visible rather than lost.
                            eprintln!(
                                "internships: {source}: could not store {}: {err:?}",
                                raw.external_id
                            );
                            counts.accepted -= 1;
                            counts.rejected += 1;
                            record_reject(
                                pool,
                                &source_run_id,
                                raw,
                                RejectRecord {
                                    kind: "rejected",
                                    reason: "storage_failed",
                                    field: None,
                                    detail: Some(&err.to_string()),
                                },
                                now,
                            )
                            .await
                            .unwrap_or_else(log_reject_failure);
                        }
                    }
                }
                QcOutcome::Filtered { reason, detail } => {
                    counts.filtered += 1;
                    record_reject(
                        pool,
                        &source_run_id,
                        raw,
                        RejectRecord {
                            kind: "filtered",
                            reason: &reason,
                            field: None,
                            detail: detail.as_deref(),
                        },
                        now,
                    )
                    .await
                    .unwrap_or_else(log_reject_failure);
                }
                QcOutcome::Rejected {
                    reason,
                    field,
                    detail,
                } => {
                    counts.rejected += 1;
                    record_reject(
                        pool,
                        &source_run_id,
                        raw,
                        RejectRecord {
                            kind: "rejected",
                            reason: &reason,
                            field: field.as_deref(),
                            detail: detail.as_deref(),
                        },
                        now,
                    )
                    .await
                    .unwrap_or_else(log_reject_failure);
                }
            }
        }

        // Positive closure statements, applied regardless of outcome. See the module doc for
        // why this does not go through `counts_for_expiry`.
        if !output.closed_external_ids.is_empty() {
            match mark_closed(pool, &source, &output.closed_external_ids, now).await {
                Ok(closed) => report.marked_closed += closed,
                Err(err) => eprintln!("internships: {source}: marking closed failed: {err:?}"),
            }
        }

        let result = SourceRunResult {
            fetched: output.result.fetched,
            accepted: counts.accepted,
            filtered: counts.filtered,
            rejected: counts.rejected,
            seen_external_ids,
            ..output.result
        };

        report.fetched += result.fetched;
        report.accepted += counts.accepted;
        report.filtered += counts.filtered;
        report.rejected += counts.rejected;
        if result.outcome == super::models::SourceOutcome::Success {
            report.sources_succeeded += 1;
        }

        if let Err(err) =
            expiry::settle_source_run(pool, &run_id, &source_run_id, &result, now).await
        {
            eprintln!("internships: {source}: recording the run failed: {err:?}");
        }
    }

    sqlx::query("UPDATE collection_runs SET finished_at = ?1 WHERE id = ?2")
        .bind(Utc::now().to_rfc3339())
        .bind(&run_id)
        .execute(pool)
        .await?;

    if let Err(err) = recompute_company_signals(pool, &tiers, Utc::now()).await {
        eprintln!("internships: recomputing company signals failed: {err:?}");
    }

    match expiry::sweep(pool, Utc::now(), expiry::miss_threshold()).await {
        Ok(sweep) => {
            report.swept_deadline = sweep.deadline_passed;
            report.swept_vanished = sweep.vanished;
        }
        Err(err) => eprintln!("internships: expiry sweep failed: {err:?}"),
    }

    Ok(report)
}

/// A reject we could not record is the one failure mode this whole table exists to prevent:
/// the row is gone *and* invisible. It cannot be allowed to fail quietly, so it is logged
/// loudly rather than swallowed — the counts on `source_runs` will still show the row was not
/// accepted, so the discrepancy is at least discoverable.
fn log_reject_failure(err: anyhow::Error) {
    eprintln!("internships: FAILED TO RECORD A REJECTED ROW — it is now invisible: {err:?}");
}

#[derive(Debug, Default, Clone, Copy)]
struct QcCounts {
    accepted: i64,
    filtered: i64,
    rejected: i64,
}

/// What one upsert did: which row it landed on, and whether that row is new.
///
/// The id is returned rather than discarded because the alert producer needs something stable
/// to key its event on, and this function has already looked it up.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Upserted {
    id: String,
    created: bool,
}

/// Insert or merge one posting, and record the sighting.
///
/// # Merging rule: a source that knows something beats a source that does not
///
/// Sparse fields use `COALESCE(excluded.x, x)`, so a later source with no salary cannot erase
/// a salary an earlier one supplied. Pay is the scarcest field in the corpus and the
/// highest-weighted ranking input, so losing it to a merge with a less informative source is
/// the expensive mistake. The cost is that a figure genuinely withdrawn upstream lingers —
/// accepted knowingly, and the reason it is written here rather than assumed.
///
/// `first_seen_at` is never overwritten: it is our own observation and does not change.
async fn upsert_posting(
    pool: &SqlitePool,
    posting: &NormalizedPosting,
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<Upserted> {
    let key = dedup_key(posting);
    let id = Uuid::new_v4().to_string();

    // A source that states no posting date gets our first sighting, explicitly flagged — never
    // silently presented as though the source said it.
    let (posted_at, estimated) = match posting.posted_at {
        Some(stated) => (stated, false),
        None => (now, true),
    };

    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM internship_postings WHERE dedup_key = ?")
            .bind(&key)
            .fetch_optional(pool)
            .await?;

    sqlx::query(
        "INSERT INTO internship_postings (
             id, dedup_key, company_key, company_name, title, canonical_url,
             term_season, term_year,
             location_raw, location_city, location_region, location_country, is_remote,
             pay_min, pay_max, pay_currency, pay_period, pay_raw,
             class_year_min, class_year_max, class_year_raw,
             posted_at, posted_at_is_estimated, deadline,
             first_seen_at, last_seen_at, created_at, updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,
                   ?22,?23,?24,?25,?25,?25,?25)
         ON CONFLICT (dedup_key) DO UPDATE SET
             company_name = excluded.company_name,
             title = excluded.title,
             canonical_url = excluded.canonical_url,
             term_season = COALESCE(excluded.term_season, term_season),
             term_year = COALESCE(excluded.term_year, term_year),
             location_raw = COALESCE(excluded.location_raw, location_raw),
             location_city = COALESCE(excluded.location_city, location_city),
             location_region = COALESCE(excluded.location_region, location_region),
             location_country = COALESCE(excluded.location_country, location_country),
             is_remote = COALESCE(excluded.is_remote, is_remote),
             pay_min = COALESCE(excluded.pay_min, pay_min),
             pay_max = COALESCE(excluded.pay_max, pay_max),
             pay_currency = COALESCE(excluded.pay_currency, pay_currency),
             pay_period = COALESCE(excluded.pay_period, pay_period),
             pay_raw = COALESCE(excluded.pay_raw, pay_raw),
             class_year_min = COALESCE(excluded.class_year_min, class_year_min),
             class_year_max = COALESCE(excluded.class_year_max, class_year_max),
             class_year_raw = COALESCE(excluded.class_year_raw, class_year_raw),
             deadline = COALESCE(excluded.deadline, deadline),
             -- A stated date always beats an estimate, in either direction.
             posted_at = CASE
                 WHEN excluded.posted_at_is_estimated = 0 THEN excluded.posted_at
                 ELSE posted_at END,
             posted_at_is_estimated = CASE
                 WHEN excluded.posted_at_is_estimated = 0 THEN 0
                 ELSE posted_at_is_estimated END,
             last_seen_at = excluded.last_seen_at,
             -- Seeing it again resurrects it: a reposted job is open again, and the tombstone
             -- must clear together with its reason or the CHECK constraint rejects the row.
             expired_at = NULL,
             expiry_reason = NULL,
             updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(&key)
    .bind(&posting.company_key)
    .bind(&posting.company_name)
    .bind(&posting.title)
    .bind(&posting.url)
    .bind(posting.term_season.map(|s| format!("{s:?}").to_lowercase()))
    .bind(posting.term_year)
    .bind(&posting.location.raw)
    .bind(&posting.location.city)
    .bind(&posting.location.region)
    .bind(&posting.location.country)
    .bind(posting.location.is_remote)
    .bind(posting.pay.as_ref().map(|p| p.min))
    .bind(posting.pay.as_ref().and_then(|p| p.max))
    .bind(posting.pay.as_ref().map(|p| p.currency.clone()))
    .bind(
        posting
            .pay
            .as_ref()
            .map(|p| format!("{:?}", p.period).to_lowercase()),
    )
    .bind(&posting.pay_raw)
    .bind(posting.class_years.min)
    .bind(posting.class_years.max)
    .bind(&posting.class_years.raw)
    .bind(posted_at.to_rfc3339())
    .bind(i64::from(estimated))
    .bind(posting.deadline.map(|d| d.to_rfc3339()))
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;

    let posting_id: String =
        sqlx::query_scalar("SELECT id FROM internship_postings WHERE dedup_key = ?")
            .bind(&key)
            .fetch_one(pool)
            .await?;

    // The sighting: this source, this listing. Its `consecutive_misses` resets to 0 here so a
    // posting that reappears after a gap starts counting again from clean, rather than
    // carrying stale misses toward a threshold it no longer deserves.
    sqlx::query(
        "INSERT INTO posting_sightings
             (id, posting_id, source, external_id, url, first_seen_at, last_seen_at,
              last_seen_run_id, consecutive_misses)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, 0)
         ON CONFLICT (source, external_id) DO UPDATE SET
             posting_id = excluded.posting_id,
             url = excluded.url,
             last_seen_at = excluded.last_seen_at,
             last_seen_run_id = excluded.last_seen_run_id,
             consecutive_misses = 0",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&posting_id)
    .bind(&posting.source)
    .bind(&posting.external_id)
    .bind(&posting.url)
    .bind(now.to_rfc3339())
    .bind(run_id)
    .execute(pool)
    .await?;

    Ok(Upserted {
        id: posting_id,
        created: existing.is_none(),
    })
}

/// Alert on a newly collected posting, if [`super::alerts`] judges it worth one. Returns how
/// many events were written — 0 or 1.
///
/// **A failed alert never fails the posting.** The posting is already stored by the time this
/// runs; an undelivered notification is a worse day, not lost data, and the isolation rule
/// that governs sources applies just as much to a producer bolted onto them. It is also not
/// recorded as a reject: a reject means the posting did not land, and this one did.
///
/// `emit` returning `false` is normal rather than exceptional — it means an event already
/// exists for this posting, which is what stops a second run raising a second notification.
async fn emit_posting_alert(
    pool: &SqlitePool,
    tiers: &prestige::CompanyTiers,
    posting: &NormalizedPosting,
    posting_id: &str,
    now: DateTime<Utc>,
) -> i64 {
    let Some(event) = alerts::posting_event(tiers, posting, posting_id) else {
        return 0;
    };

    match hunt::events::emit(pool, &event, now).await {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(err) => {
            eprintln!(
                "internships: could not raise an alert for {} — {}: {err:?}",
                posting.company_name, posting.title
            );
            0
        }
    }
}

/// Expire postings a source has explicitly declared closed.
///
/// Scoped to sightings from *that* source, and only expires the posting when **no other
/// source still lists it** — one aggregator's stale `active: false` must not close a job the
/// authoritative ATS is still serving. § D.4 records exactly this disagreement happening.
async fn mark_closed(
    pool: &SqlitePool,
    source: &str,
    external_ids: &[String],
    now: DateTime<Utc>,
) -> Result<u64> {
    let ids: HashSet<&str> = external_ids.iter().map(String::as_str).collect();
    let mut closed = 0;

    for external_id in ids {
        let affected = sqlx::query(
            "UPDATE internship_postings
             SET expired_at = ?1, expiry_reason = 'source_marked_closed', updated_at = ?1
             WHERE expired_at IS NULL
               AND id IN (
                   SELECT posting_id FROM posting_sightings
                   WHERE source = ?2 AND external_id = ?3
               )
               AND NOT EXISTS (
                   SELECT 1 FROM posting_sightings other
                   WHERE other.posting_id = internship_postings.id
                     AND other.source <> ?2
                     AND other.consecutive_misses = 0
               )",
        )
        .bind(now.to_rfc3339())
        .bind(source)
        .bind(external_id)
        .execute(pool)
        .await?
        .rows_affected();
        closed += affected;
    }

    Ok(closed)
}

/// Open the `source_runs` row so rejects have something to reference.
///
/// `settle_source_run` later updates this same row (its ON CONFLICT is on `(run_id, source)`),
/// which is why the id generated here is the one rejects are written against.
async fn open_source_run(
    pool: &SqlitePool,
    run_id: &str,
    source_run_id: &str,
    source: &str,
    outcome: super::models::SourceOutcome,
    now: DateTime<Utc>,
) -> Result<()> {
    let outcome_text = match outcome {
        super::models::SourceOutcome::Success => "success",
        super::models::SourceOutcome::Partial => "partial",
        super::models::SourceOutcome::Failed => "failed",
        super::models::SourceOutcome::Skipped => "skipped",
    };
    sqlx::query(
        "INSERT INTO source_runs (id, run_id, source, started_at, outcome, counts_for_expiry)
         VALUES (?1, ?2, ?3, ?4, ?5, 0)
         ON CONFLICT (run_id, source) DO NOTHING",
    )
    .bind(source_run_id)
    .bind(run_id)
    .bind(source)
    .bind(now.to_rfc3339())
    .bind(outcome_text)
    .execute(pool)
    .await?;
    Ok(())
}

/// Why a row was not accepted. Bundled rather than passed as five loose strings — at that
/// width the call sites become positional puzzles, and swapping `reason` with `field` would
/// compile silently.
struct RejectRecord<'a> {
    kind: &'a str,
    reason: &'a str,
    field: Option<&'a str>,
    detail: Option<&'a str>,
}

async fn record_reject(
    pool: &SqlitePool,
    source_run_id: &str,
    raw: &RawPosting,
    record: RejectRecord<'_>,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO posting_rejects
             (id, source_run_id, source, kind, reason, field, detail, external_id, url,
              raw_json, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(source_run_id)
    .bind(&raw.source)
    .bind(record.kind)
    .bind(record.reason)
    .bind(record.field)
    .bind(record.detail)
    .bind(&raw.external_id)
    .bind(&raw.url)
    .bind(&raw.raw_json)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Rebuild `company_signals` from what collection has actually observed.
///
/// Derived, per the owner's choice of derived signals over a hand-maintained tier list. The
/// inputs are stored beside the output so a score can be reproduced by hand — the one thing a
/// tier list gives you for free and a derived signal otherwise does not.
///
/// **`prestige` is left NULL below the evidence threshold**, and NULL means *unknown* to the
/// ranking, never *worst*. A company we have barely seen is not a company we know to be bad.
async fn recompute_company_signals(
    pool: &SqlitePool,
    tiers: &prestige::CompanyTiers,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO company_signals (
             company_key, company_name, distinct_sources, live_postings, total_postings_seen,
             pay_observations, median_pay_hourly_usd, first_seen_at, prestige, computed_at
         )
         SELECT
             p.company_key,
             MIN(p.company_name),
             (SELECT COUNT(DISTINCT s.source) FROM posting_sightings s
               JOIN internship_postings p2 ON p2.id = s.posting_id
              WHERE p2.company_key = p.company_key),
             SUM(CASE WHEN p.expired_at IS NULL THEN 1 ELSE 0 END),
             COUNT(*),
             SUM(CASE WHEN p.pay_min IS NOT NULL AND p.pay_period = 'hour'
                       AND p.pay_currency = 'USD' THEN 1 ELSE 0 END),
             NULL,
             MIN(p.first_seen_at),
             NULL,
             ?1
         FROM internship_postings p
         GROUP BY p.company_key
         ON CONFLICT (company_key) DO UPDATE SET
             company_name = excluded.company_name,
             distinct_sources = excluded.distinct_sources,
             live_postings = excluded.live_postings,
             total_postings_seen = excluded.total_postings_seen,
             pay_observations = excluded.pay_observations,
             computed_at = excluded.computed_at",
    )
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;

    // Median hourly pay per company, over the postings that state one in USD/hour.
    //
    // SQLite has no median aggregate. The obvious `LIMIT 1 OFFSET (SELECT COUNT(*)/2 …)`
    // shape does not work here: the offset subquery has to correlate to `company_signals`
    // from *two* levels of nesting, and SQLite answers that with
    // `no such column: company_signals.company_key`. A window function keeps the correlation
    // one level deep. Verified against a real database before being written here.
    sqlx::query(
        "WITH ranked AS (
             SELECT company_key, pay_min,
                    ROW_NUMBER() OVER (PARTITION BY company_key ORDER BY pay_min) AS rn,
                    COUNT(*) OVER (PARTITION BY company_key) AS n
               FROM internship_postings
              WHERE pay_min IS NOT NULL AND pay_period = 'hour' AND pay_currency = 'USD'
         )
         UPDATE company_signals SET median_pay_hourly_usd = (
             SELECT pay_min FROM ranked
              WHERE ranked.company_key = company_signals.company_key
                AND ranked.rn = (ranked.n + 1) / 2
         ) WHERE pay_observations > 0",
    )
    .execute(pool)
    .await?;

    // Prestige. Computed in Rust rather than SQL because it consults the curated tier file —
    // see `internships::prestige` for why the signal is half stated and half derived.
    //
    // This replaced a one-line SQL expression scoring companies by how many sources carried
    // them. On a real 455-company corpus that gave **60 companies a score, all of them exactly
    // 1.0**, because nothing was carried by more than two sources. It read like a ranking and
    // behaved like a coin flip.
    let max_sources: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(distinct_sources), 0) FROM company_signals")
            .fetch_one(pool)
            .await?;

    let rows: Vec<(String, i64, i64, Option<f64>)> = sqlx::query_as(
        "SELECT company_key, live_postings, distinct_sources, median_pay_hourly_usd
           FROM company_signals",
    )
    .fetch_all(pool)
    .await?;

    for (company_key, live_postings, distinct_sources, median_pay_hourly_usd) in rows {
        let value = prestige::score(
            tiers,
            &company_key,
            prestige::DerivedInputs {
                live_postings,
                distinct_sources,
                max_distinct_sources: max_sources,
                median_pay_hourly_usd,
            },
        );
        sqlx::query("UPDATE company_signals SET prestige = ?1 WHERE company_key = ?2")
            .bind(value)
            .bind(&company_key)
            .execute(pool)
            .await?;
    }

    Ok(())
}

/// Close out runs abandoned by a process that died.
///
/// A collection run cannot outlive its process, so at startup **every** run still marked
/// unfinished is dead by definition. This is reconciliation, not a timeout — there is no
/// "how old is too old" constant, because the fact that we are booting is the proof.
///
/// Without it, one interrupted run poisons everything downstream: it reports as permanently
/// in flight, the UI hides the "Collect now" button behind its progress banner, and
/// [`should_collect_on_startup`] reads it as a recent run and declines to collect. All three
/// were observed together on 2026-08-21.
pub async fn reconcile_interrupted_runs(pool: &SqlitePool) -> Result<u64> {
    let interrupted = sqlx::query(
        "UPDATE collection_runs
            SET interrupted = 1, finished_at = COALESCE(finished_at, ?1)
          WHERE finished_at IS NULL",
    )
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?
    .rows_affected();

    if interrupted > 0 {
        println!(
            "internships: closed {interrupted} collection run(s) abandoned by a previous \
             process — they are marked interrupted, not completed"
        );
    }
    Ok(interrupted)
}

/// Whether the last collection is old enough to justify running one at startup.
///
/// True when nothing has ever run, and true when the newest run is older than the configured
/// cadence. False on a quick restart, which is what stops a development loop from becoming a
/// fetch storm against third-party job boards.
///
/// A run that *failed* still counts as a run for this purpose: retrying immediately on every
/// restart is precisely the retry storm the scraping rules forbid, and the interval is the
/// right place to wait.
async fn should_collect_on_startup(pool: &SqlitePool, interval: Duration) -> bool {
    let last: Option<String> =
        // `interrupted = 0` is load-bearing: a run that died before doing any work is not
        // evidence that the data is fresh, and counting it means every restart after a crash
        // declines to collect — the state this project was actually found in.
        match sqlx::query_scalar("SELECT MAX(started_at) FROM collection_runs WHERE interrupted = 0")
            .fetch_one(pool)
            .await
        {
            Ok(value) => value,
            Err(err) => {
                eprintln!("internships: could not read the last run time: {err:?}");
                return false;
            }
        };

    let Some(last) = last else {
        return true; // never collected
    };
    let Ok(last) = DateTime::parse_from_rfc3339(&last) else {
        return true; // unreadable timestamp: treat as absent rather than as fresh
    };

    let age = Utc::now().signed_duration_since(last.with_timezone(&Utc));
    age.to_std().map(|age| age >= interval).unwrap_or(false)
}

/// `INTERNSHIP_COLLECT_INTERVAL_SECS` / `INTERNSHIP_EXPIRY_INTERVAL_SECS`.
///
/// Same shape as `BLOG_SYNC_INTERVAL_SECS`: `0` disables deliberately, and anything
/// unparseable disables *and says so* — a typo'd interval that silently behaved like the
/// default would be a setting that looks applied and isn't.
fn interval_from_env(name: &str, default_secs: u64) -> Option<Duration> {
    match std::env::var(name) {
        Err(_) => Some(Duration::from_secs(default_secs)),
        Ok(value) => match value.trim().parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(Duration::from_secs(secs)),
            Err(_) => {
                eprintln!(
                    "internships: {name}={value:?} is not a number — disabled; use 0 to \
                     disable deliberately"
                );
                None
            }
        },
    }
}

/// Start the background collector and the expiry sweeper.
///
/// Two tasks rather than one because they answer to different clocks: collection is bounded by
/// how often other people's job boards change, while a *deadline* passing needs no new data at
/// all and should be noticed within the hour.
/// Start the internship subsystem: startup housekeeping, then scheduling.
///
/// Call once from `main`, awaited — the same shape as `auth::purge_expired_sessions`, and for
/// the same reason: it is housekeeping that must happen before anything else reads the tables.
pub async fn start(pool: SqlitePool) {
    start_with(
        pool,
        interval_from_env("INTERNSHIP_COLLECT_INTERVAL_SECS", DEFAULT_COLLECT_INTERVAL_SECS),
        interval_from_env("INTERNSHIP_EXPIRY_INTERVAL_SECS", DEFAULT_EXPIRY_INTERVAL_SECS),
    )
    .await;
}

/// [`start`] with the schedule injected, so the ordering below can be tested without touching
/// process environment variables (racy against every other test in the binary) and without
/// spawning anything that would reach the network.
pub(crate) async fn start_with(
    pool: SqlitePool,
    collect_interval: Option<Duration>,
    expiry_interval: Option<Duration>,
) {
    // **Unconditional, and outside every schedule check.** This used to live inside the
    // collector task, which meant `INTERNSHIP_COLLECT_INTERVAL_SECS=0` skipped it entirely:
    // runs abandoned by a killed process were never closed, the UI reported "Collecting…"
    // forever, and `should_collect_on_startup` read the phantom as recent and declined to
    // collect. That is the original lockout, reachable again through the one setting
    // documented for keeping the collector quiet.
    //
    // A run cannot outlive its process, so this is true whatever the schedule says.
    if let Err(err) = reconcile_interrupted_runs(&pool).await {
        eprintln!("internships: could not reconcile abandoned runs: {err:?}");
    }

    spawn_scheduled(pool, collect_interval, expiry_interval);
}

fn spawn_scheduled(
    pool: SqlitePool,
    collect_interval: Option<Duration>,
    expiry_interval: Option<Duration>,
) {
    match collect_interval {
        None => println!("internships: scheduled collection disabled — POST /internships/collect only"),
        Some(interval) => {
            println!(
                "internships: collecting every {}s (checking on startup whether one is due)",
                interval.as_secs()
            );
            let pool = pool.clone();
            tokio::spawn(async move {
                // Catch up on boot if the data is stale or absent, then settle into the
                // interval.
                //
                // The first tick of a tokio interval fires immediately, and simply consuming
                // it (the blog watcher's pattern) is wrong here: with a six-hour cadence, a
                // fresh database then shows an empty tab and an empty run-health panel for six
                // hours with nothing anywhere saying a run is even scheduled — which looks
                // exactly like a broken collector. That is this phase's own failure mode
                // reproduced at the application level.
                //
                // Equally, collecting unconditionally on every boot is wrong the other way:
                // during development the backend restarts constantly, and each restart would
                // be a fresh sweep of other people's job boards. So the question asked is
                // "is what we have older than the cadence?", which is false on a quick restart
                // and true on a cold start or after a long gap.
                if should_collect_on_startup(&pool, interval).await {
                    println!("internships: no recent collection — running one now");
                    match collect(&pool, "startup").await {
                        Ok(report) => println!(
                            "internships: startup run — {} kept, {} filtered, {} unparsed",
                            report.accepted, report.filtered, report.rejected
                        ),
                        Err(err) => eprintln!("internships: startup collection failed: {err:?}"),
                    }
                } else {
                    println!(
                        "internships: recent collection found — next run in {}s",
                        interval.as_secs()
                    );
                }

                let mut ticker = tokio::time::interval(interval);
                ticker.tick().await;
                loop {
                    ticker.tick().await;
                    match collect(&pool, "scheduled").await {
                        Ok(report) => println!(
                            "internships: run {} — {}/{} sources ok, {} fetched, {} kept, \
                             {} filtered, {} unparsed, {} closed, swept {}+{}",
                            &report.run_id[..8],
                            report.sources_succeeded,
                            report.sources_run,
                            report.fetched,
                            report.accepted,
                            report.filtered,
                            report.rejected,
                            report.marked_closed,
                            report.swept_deadline,
                            report.swept_vanished,
                        ),
                        // Logged and swallowed: a failing run must not kill the scheduler, or
                        // one transient error silently ends collection for the whole process.
                        Err(err) => eprintln!("internships: collection failed: {err:?}"),
                    }
                }
            });
        }
    }

    match expiry_interval {
        None => println!("internships: scheduled expiry sweep disabled"),
        Some(interval) => {
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(interval);
                ticker.tick().await;
                loop {
                    ticker.tick().await;
                    match expiry::sweep(&pool, Utc::now(), expiry::miss_threshold()).await {
                        Ok(report) if report.is_empty() => {}
                        Ok(report) => println!(
                            "internships: swept {} past-deadline, {} vanished",
                            report.deadline_passed, report.vanished
                        ),
                        Err(err) => eprintln!("internships: expiry sweep failed: {err:?}"),
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_interval_uses_the_default() {
        assert_eq!(
            interval_from_env("INTERNSHIP_TEST_UNSET_VAR_XYZ", 42),
            Some(Duration::from_secs(42))
        );
    }

    // These two are `const` blocks rather than `#[test]`s on purpose: both compare
    // compile-time constants, so a violation should stop the build rather than wait for
    // someone to run the suite. Clippy flags the `#[test]` form as a constant assertion, and
    // it is right to.

    /// The blog watcher polls a local directory every 5s. This one crosses the network to
    /// other people's servers; a short interval is a politeness failure, not a tuning choice.
    const _: () = assert!(DEFAULT_COLLECT_INTERVAL_SECS >= 3_600);
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::internships::sources::{BoxFuture, Source, SourceFetch};

    /// A source that returns exactly what the test tells it to, without touching the network.
    struct FakeSource {
        name: String,
        fetch: std::sync::Mutex<Option<SourceFetch>>,
    }

    impl FakeSource {
        /// Named `arc` rather than `new` because it returns `Arc<dyn Source>`, not `Self` —
        /// the registry stores trait objects, so that is the useful shape here.
        fn arc(name: &str, fetch: SourceFetch) -> Arc<dyn Source> {
            Arc::new(FakeSource {
                name: name.to_string(),
                fetch: std::sync::Mutex::new(Some(fetch)),
            })
        }
    }

    impl Source for FakeSource {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "fake source, for tests"
        }
        fn fetch<'a>(&'a self, _ctx: &'a SourceContext) -> BoxFuture<'a, SourceFetch> {
            let taken = self.fetch.lock().unwrap().take();
            Box::pin(async move { taken.unwrap_or_else(|| SourceFetch::failed("already taken")) })
        }
    }

    fn raw(source: &str, external_id: &str, url: &str, title: &str, company: &str) -> RawPosting {
        RawPosting {
            source: source.to_string(),
            external_id: external_id.to_string(),
            url: url.to_string(),
            company: company.to_string(),
            title: title.to_string(),
            location_raw: Some("San Francisco, CA".to_string()),
            pay_raw: None,
            term_raw: Some("Summer 2027".to_string()),
            class_year_raw: None,
            posted_at_raw: None,
            deadline_raw: None,
            description: Some("Write software as a summer intern.".to_string()),
            remote_hint: None,
            raw_json: "{}".to_string(),
        }
    }

    async fn test_pool() -> SqlitePool {
        // A file rather than `:memory:`: the pool opens several connections, and each one
        // would get its own private in-memory database.
        let path = std::env::temp_dir().join(format!("collector-{}.db", Uuid::new_v4()));
        let url = format!("sqlite://{}?mode=rwc", path.display());
        crate::db::init_pool(&url).await.expect("migrations")
    }

    async fn ctx() -> Arc<SourceContext> {
        let client = PoliteClient::with_host_delay(Duration::ZERO).expect("client");
        Arc::new(SourceContext::new(client))
    }

    async fn misses(pool: &SqlitePool, source: &str, external_id: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT consecutive_misses FROM posting_sightings WHERE source = ? AND external_id = ?",
        )
        .bind(source)
        .bind(external_id)
        .fetch_one(pool)
        .await
        .expect("sighting")
    }

    #[tokio::test]
    async fn an_abandoned_run_is_closed_and_marked_interrupted() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO collection_runs (id, started_at, trigger) VALUES ('r1', ?1, 'startup')")
            .bind(Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(reconcile_interrupted_runs(&pool).await.unwrap(), 1);

        let (finished, interrupted): (Option<String>, i64) =
            sqlx::query_as("SELECT finished_at, interrupted FROM collection_runs WHERE id = 'r1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(finished.is_some(), "an abandoned run must stop reading as in-flight");
        assert_eq!(interrupted, 1, "but must not read as completed either");
    }

    #[tokio::test]
    async fn startup_reconciles_abandoned_runs_even_when_scheduling_is_disabled() {
        // The regression this pins: reconciliation used to sit *inside* the collector task, so
        // `INTERNSHIP_COLLECT_INTERVAL_SECS=0` skipped it. A run abandoned by a killed process
        // then stayed "in flight" forever — the UI showed "Collecting…" against a database
        // where nothing was running, and `should_collect_on_startup` read the phantom as a
        // recent run and declined to collect. Both intervals are `None` here, which is exactly
        // that configuration, and also means no task is spawned and nothing reaches the
        // network.
        let pool = test_pool().await;
        sqlx::query("INSERT INTO collection_runs (id, started_at, trigger) VALUES ('r1', ?1, 'startup')")
            .bind(Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();

        start_with(pool.clone(), None, None).await;

        let (finished, interrupted): (Option<String>, i64) =
            sqlx::query_as("SELECT finished_at, interrupted FROM collection_runs WHERE id = 'r1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            finished.is_some(),
            "an abandoned run must be closed even with the scheduler off"
        );
        assert_eq!(interrupted, 1);

        // And the phantom must no longer suppress a real collection.
        assert!(should_collect_on_startup(&pool, Duration::from_secs(21_600)).await);
    }

    #[tokio::test]
    async fn an_interrupted_run_does_not_suppress_the_next_startup_collection() {
        // The bug this exists to prevent: a crashed run made every later restart decide the
        // data was fresh and skip collecting, so the tab stayed empty indefinitely.
        let pool = test_pool().await;
        sqlx::query("INSERT INTO collection_runs (id, started_at, trigger) VALUES ('r1', ?1, 'startup')")
            .bind(Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        reconcile_interrupted_runs(&pool).await.unwrap();

        assert!(
            should_collect_on_startup(&pool, Duration::from_secs(21_600)).await,
            "an interrupted run is not evidence that the data is fresh"
        );
    }

    #[tokio::test]
    async fn a_cold_database_collects_on_startup() {
        // Otherwise a fresh install shows an empty tab and an empty health panel for a full
        // interval, with nothing anywhere indicating a run is even scheduled.
        let pool = test_pool().await;
        assert!(should_collect_on_startup(&pool, Duration::from_secs(21_600)).await);
    }

    #[tokio::test]
    async fn a_restart_right_after_a_run_does_not_collect_again() {
        // The development loop restarts the backend constantly. Collecting on every boot
        // would turn that into a fetch storm against third-party job boards.
        let pool = test_pool().await;
        sqlx::query("INSERT INTO collection_runs (id, started_at, trigger) VALUES (?1, ?2, ?3)")
            .bind("r1")
            .bind(Utc::now().to_rfc3339())
            .bind("scheduled")
            .execute(&pool)
            .await
            .unwrap();
        assert!(!should_collect_on_startup(&pool, Duration::from_secs(21_600)).await);
    }

    #[tokio::test]
    async fn a_restart_after_a_long_gap_does_collect() {
        // The other direction, so the test above cannot pass by the gate simply never firing.
        let pool = test_pool().await;
        sqlx::query("INSERT INTO collection_runs (id, started_at, trigger) VALUES (?1, ?2, ?3)")
            .bind("r1")
            .bind((Utc::now() - chrono::Duration::days(2)).to_rfc3339())
            .bind("scheduled")
            .execute(&pool)
            .await
            .unwrap();
        assert!(should_collect_on_startup(&pool, Duration::from_secs(21_600)).await);
    }

    #[tokio::test]
    async fn a_failed_run_still_counts_as_a_run_for_startup_purposes() {
        // Retrying immediately on every restart is the retry storm the scraping rules forbid.
        // The interval is the right place to wait.
        let pool = test_pool().await;
        sqlx::query("INSERT INTO collection_runs (id, started_at, trigger) VALUES (?1, ?2, ?3)")
            .bind("r1")
            .bind(Utc::now().to_rfc3339())
            .bind("scheduled")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO source_runs (id, run_id, source, started_at, outcome, error)
             VALUES ('sr1', 'r1', 'greenhouse', ?1, 'failed', 'HTTP 403')",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
        assert!(!should_collect_on_startup(&pool, Duration::from_secs(21_600)).await);
    }

    #[tokio::test]
    async fn a_successful_run_stores_postings_and_sightings() {
        let pool = test_pool().await;
        let sources = vec![FakeSource::arc(
            "greenhouse",
            SourceFetch::success(vec![raw(
                "greenhouse",
                "gh-1",
                "https://job-boards.greenhouse.io/acme/jobs/1",
                "Software Engineer Intern",
                "Acme",
            )]),
        )];

        let report = collect_with(&pool, "manual", sources, ctx().await)
            .await
            .expect("collect");

        assert_eq!(report.accepted, 1, "the posting should have survived QC");
        assert_eq!(report.postings_created, 1);
        assert_eq!(misses(&pool, "greenhouse", "gh-1").await, 0);
    }

    /// Every `hunt_events` row, oldest first.
    async fn events(pool: &SqlitePool) -> Vec<(String, String, String, Option<String>)> {
        sqlx::query_as("SELECT kind, subject_id, title, user_id FROM hunt_events ORDER BY created_at")
            .fetch_all(pool)
            .await
            .expect("events")
    }

    #[tokio::test]
    async fn a_new_posting_from_a_tier_one_company_raises_exactly_one_alert() {
        let pool = test_pool().await;
        let sources = vec![FakeSource::arc(
            "greenhouse",
            SourceFetch::success(vec![raw(
                "greenhouse",
                "gh-1",
                "https://job-boards.greenhouse.io/google/jobs/1",
                "Software Engineer Intern",
                "Google",
            )]),
        )];

        let report = collect_with(&pool, "manual", sources, ctx().await)
            .await
            .expect("collect");

        assert_eq!(report.postings_created, 1);
        assert_eq!(report.alerts_created, 1);

        let events = events(&pool).await;
        assert_eq!(events.len(), 1, "one new posting, one alert");
        assert_eq!(events[0].0, "posting");
        assert!(events[0].2.contains("Google"), "got {:?}", events[0].2);
        assert_eq!(
            events[0].3, None,
            "a posting belongs to the shared corpus, not to a user"
        );

        // The event points at the posting it is about, so the popup can link to it.
        let posting_id: String =
            sqlx::query_scalar("SELECT id FROM internship_postings LIMIT 1")
                .fetch_one(&pool)
                .await
                .expect("posting");
        assert_eq!(events[0].1, posting_id);
    }

    #[tokio::test]
    async fn a_tier_three_or_unlisted_company_raises_none() {
        // The predicate must not degrade to "is the company listed at all", and it must not
        // read a NULL prestige as a low one — that would alert on nearly every posting
        // collected, which is how a notification channel gets muted wholesale.
        let pool = test_pool().await;
        let sources = vec![FakeSource::arc(
            "greenhouse",
            SourceFetch::success(vec![
                raw(
                    "greenhouse",
                    "gh-1",
                    "https://job-boards.greenhouse.io/intel/jobs/1",
                    "Software Engineer Intern",
                    "Intel",
                ),
                raw(
                    "greenhouse",
                    "gh-2",
                    "https://job-boards.greenhouse.io/acme/jobs/2",
                    "Software Engineer Intern",
                    "Acme",
                ),
            ]),
        )];

        let report = collect_with(&pool, "manual", sources, ctx().await)
            .await
            .expect("collect");

        assert_eq!(report.postings_created, 2, "both postings should be stored");
        assert_eq!(report.alerts_created, 0);
        assert!(events(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn re_running_collection_over_the_same_posting_does_not_alert_twice() {
        // The MV3 failure this whole design exists to prevent, one layer down: collection runs
        // every six hours over a corpus that mostly does not change, so a producer that keys
        // on "did we see it this run" would re-notify for the same job four times a day.
        let pool = test_pool().await;

        for run in 0..3 {
            let sources = vec![FakeSource::arc(
                "greenhouse",
                SourceFetch::success(vec![raw(
                    "greenhouse",
                    "gh-1",
                    "https://job-boards.greenhouse.io/google/jobs/1",
                    "Software Engineer Intern",
                    "Google",
                )]),
            )];
            let report = collect_with(&pool, "scheduled", sources, ctx().await)
                .await
                .expect("collect");
            assert_eq!(
                report.alerts_created,
                i64::from(run == 0),
                "only the first run should alert (run {run})"
            );
        }

        assert_eq!(events(&pool).await.len(), 1);
    }

    #[tokio::test]
    async fn an_acked_alert_is_not_resurrected_by_a_later_collection() {
        // "Restart Firefox after acking and it does not come back" has a server-side half:
        // the next collection run must not clear or replace the ack. The posting is seen
        // again on every run, so this is the ordinary path, not an edge case.
        let pool = test_pool().await;

        let listing = || {
            vec![FakeSource::arc(
                "greenhouse",
                SourceFetch::success(vec![raw(
                    "greenhouse",
                    "gh-1",
                    "https://job-boards.greenhouse.io/google/jobs/1",
                    "Software Engineer Intern",
                    "Google",
                )]),
            )]
        };

        collect_with(&pool, "manual", listing(), ctx().await)
            .await
            .expect("first collect");

        let event_id: String = sqlx::query_scalar("SELECT id FROM hunt_events")
            .fetch_one(&pool)
            .await
            .expect("event");

        sqlx::query("INSERT INTO users (id, email, password_hash, created_at) VALUES (?1,?2,?3,?4)")
            .bind("u1")
            .bind("hunter@example.com")
            .bind("x")
            .bind(Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .expect("user");

        let acked = crate::hunt::events::ack(&pool, &event_id, "u1", Utc::now())
            .await
            .expect("ack");
        assert_eq!(acked, crate::hunt::events::AckOutcome::Acked);

        collect_with(&pool, "scheduled", listing(), ctx().await)
            .await
            .expect("second collect");

        let still_acked: Option<String> =
            sqlx::query_scalar("SELECT acked_at FROM hunt_events WHERE id = ?")
                .bind(&event_id)
                .fetch_one(&pool)
                .await
                .expect("row");
        assert!(still_acked.is_some(), "the ack must survive a later run");
        assert_eq!(events(&pool).await.len(), 1, "and no second event");
    }

    #[tokio::test]
    async fn a_failed_source_does_not_expire_the_postings_it_previously_supplied() {
        // THE test for this phase. One blocked fetch must not close everything a source has
        // ever carried.
        let pool = test_pool().await;
        let posting = raw(
            "linkedin",
            "li-1",
            "https://job-boards.greenhouse.io/acme/jobs/1",
            "Software Engineer Intern",
            "Acme",
        );

        collect_with(
            &pool,
            "manual",
            vec![FakeSource::arc(
                "linkedin",
                SourceFetch::success(vec![posting]),
            )],
            ctx().await,
        )
        .await
        .expect("first run");
        assert_eq!(misses(&pool, "linkedin", "li-1").await, 0);

        // Now the source breaks, three runs running.
        for _ in 0..3 {
            collect_with(
                &pool,
                "manual",
                vec![FakeSource::arc("linkedin", SourceFetch::failed("HTTP 403"))],
                ctx().await,
            )
            .await
            .expect("failed run");
        }

        assert_eq!(
            misses(&pool, "linkedin", "li-1").await,
            0,
            "a failed run must leave disappearance counters untouched"
        );
        let live: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM internship_postings WHERE expired_at IS NULL")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(live, 1, "the posting must still be live");
    }

    #[tokio::test]
    async fn a_genuine_disappearance_from_successful_runs_does_expire() {
        // The other direction — proving the test above isn't passing vacuously because
        // nothing ever expires.
        let pool = test_pool().await;
        collect_with(
            &pool,
            "manual",
            vec![FakeSource::arc(
                "greenhouse",
                SourceFetch::success(vec![raw(
                    "greenhouse",
                    "gh-1",
                    "https://job-boards.greenhouse.io/acme/jobs/1",
                    "Software Engineer Intern",
                    "Acme",
                )]),
            )],
            ctx().await,
        )
        .await
        .expect("first run");

        // The board keeps answering; the job is simply no longer on it. Note the board is not
        // empty — an empty successful run trips the suspicious-zero breaker on purpose.
        for _ in 0..expiry::DEFAULT_MISS_THRESHOLD {
            collect_with(
                &pool,
                "manual",
                vec![FakeSource::arc(
                    "greenhouse",
                    SourceFetch::success(vec![raw(
                        "greenhouse",
                        "gh-2",
                        "https://job-boards.greenhouse.io/acme/jobs/2",
                        "Software Engineer Intern",
                        "Acme",
                    )]),
                )],
                ctx().await,
            )
            .await
            .expect("later run");
        }

        let reason: Option<String> = sqlx::query_scalar(
            "SELECT expiry_reason FROM internship_postings WHERE dedup_key LIKE '%jobs:1' OR
             id IN (SELECT posting_id FROM posting_sightings WHERE external_id = 'gh-1')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(reason.as_deref(), Some("vanished_from_sources"));
    }

    #[tokio::test]
    async fn a_row_we_failed_to_parse_still_counts_as_seen() {
        // Our parser having an opinion is not the source withdrawing the listing. If a QC
        // rejection counted as a miss, one of our own bugs would expire real postings.
        let pool = test_pool().await;
        let good = raw(
            "greenhouse",
            "gh-1",
            "https://job-boards.greenhouse.io/acme/jobs/1",
            "Software Engineer Intern",
            "Acme",
        );
        collect_with(
            &pool,
            "manual",
            vec![FakeSource::arc(
                "greenhouse",
                SourceFetch::success(vec![good.clone()]),
            )],
            ctx().await,
        )
        .await
        .expect("first run");

        // Same listing comes back, but with no company — QC rejects it.
        let mut broken = good.clone();
        broken.company = String::new();
        for _ in 0..expiry::DEFAULT_MISS_THRESHOLD + 1 {
            collect_with(
                &pool,
                "manual",
                vec![FakeSource::arc(
                    "greenhouse",
                    SourceFetch::success(vec![broken.clone()]),
                )],
                ctx().await,
            )
            .await
            .expect("later run");
        }

        assert_eq!(
            misses(&pool, "greenhouse", "gh-1").await,
            0,
            "a rejected row was still seen by the source"
        );
    }

    #[tokio::test]
    async fn one_job_carried_by_two_sources_is_one_posting_with_two_sightings() {
        let pool = test_pool().await;
        let url = "https://job-boards.greenhouse.io/acme/jobs/1";
        let sources = vec![
            FakeSource::arc(
                "greenhouse",
                SourceFetch::success(vec![raw(
                    "greenhouse",
                    "gh-1",
                    url,
                    "Software Engineer Intern",
                    "Acme",
                )]),
            ),
            FakeSource::arc(
                "simplify",
                // Same job, different URL shape and a differently-worded title.
                SourceFetch::success(vec![raw(
                    "simplify",
                    "sy-9",
                    "https://boards.greenhouse.io/acme/jobs/1?utm_source=simplify",
                    "Software Engineering Intern, Summer 2027",
                    "Acme Corporation",
                )]),
            ),
        ];

        collect_with(&pool, "manual", sources, ctx().await)
            .await
            .expect("collect");

        let postings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM internship_postings")
            .fetch_one(&pool)
            .await
            .unwrap();
        let sightings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posting_sightings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(postings, 1, "the ATS triple should have merged these");
        assert_eq!(sightings, 2, "but both sightings must be recorded");
    }

    #[tokio::test]
    async fn one_source_failing_does_not_reduce_what_the_others_produced() {
        // Per-source isolation, end to end rather than at the adapter boundary.
        let pool = test_pool().await;
        let sources = vec![
            FakeSource::arc("linkedin", SourceFetch::failed("HTTP 403")),
            FakeSource::arc(
                "greenhouse",
                SourceFetch::success(vec![raw(
                    "greenhouse",
                    "gh-1",
                    "https://job-boards.greenhouse.io/acme/jobs/1",
                    "Software Engineer Intern",
                    "Acme",
                )]),
            ),
        ];

        let report = collect_with(&pool, "manual", sources, ctx().await)
            .await
            .expect("collect");

        assert_eq!(report.accepted, 1, "the healthy source still landed");
        let failed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM source_runs WHERE outcome = 'failed' AND error IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(failed, 1, "and the failure is recorded where a human sees it");
    }

    #[tokio::test]
    async fn every_fetched_row_is_accounted_for() {
        // fetched = accepted + filtered + rejected. If this drifts, rows are being dropped
        // silently, which is the failure this whole phase is built to prevent.
        let pool = test_pool().await;
        let mut not_software = raw(
            "greenhouse",
            "gh-2",
            "https://job-boards.greenhouse.io/acme/jobs/2",
            "Marketing Intern",
            "Acme",
        );
        not_software.description = Some("Run campaigns.".to_string());
        let mut no_company = raw(
            "greenhouse",
            "gh-3",
            "https://job-boards.greenhouse.io/acme/jobs/3",
            "Software Engineer Intern",
            "",
        );
        no_company.description = None;

        let sources = vec![FakeSource::arc(
            "greenhouse",
            SourceFetch::success(vec![
                raw(
                    "greenhouse",
                    "gh-1",
                    "https://job-boards.greenhouse.io/acme/jobs/1",
                    "Software Engineer Intern",
                    "Acme",
                ),
                not_software,
                no_company,
            ]),
        )];

        let report = collect_with(&pool, "manual", sources, ctx().await)
            .await
            .expect("collect");

        assert_eq!(
            report.fetched,
            report.accepted + report.filtered + report.rejected,
            "fetched={} accepted={} filtered={} rejected={}",
            report.fetched,
            report.accepted,
            report.filtered,
            report.rejected
        );

        // And each non-accepted row kept its payload, so it can be diagnosed rather than
        // merely counted.
        let with_payload: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM posting_rejects WHERE raw_json IS NOT NULL AND raw_json <> ''",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(with_payload, report.filtered + report.rejected);
    }

    #[tokio::test]
    async fn an_explicit_closure_flag_closes_a_posting_no_one_else_lists() {
        let pool = test_pool().await;
        let posting = raw(
            "simplify",
            "sy-1",
            "https://job-boards.greenhouse.io/acme/jobs/1",
            "Software Engineer Intern",
            "Acme",
        );
        collect_with(
            &pool,
            "manual",
            vec![FakeSource::arc(
                "simplify",
                SourceFetch::success(vec![posting.clone()]),
            )],
            ctx().await,
        )
        .await
        .expect("first run");

        collect_with(
            &pool,
            "manual",
            vec![FakeSource::arc(
                "simplify",
                SourceFetch::success(vec![posting])
                    .with_closed_ids(vec!["sy-1".to_string()]),
            )],
            ctx().await,
        )
        .await
        .expect("closing run");

        let reason: Option<String> =
            sqlx::query_scalar("SELECT expiry_reason FROM internship_postings")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reason.as_deref(), Some("source_marked_closed"));
    }

    #[tokio::test]
    async fn an_aggregators_stale_closure_cannot_close_a_job_the_ats_still_lists() {
        // Straight from § D.4: Simplify said `active: true` for a job Greenhouse had already
        // 404'd, and the doc's rule is that the ATS is the system of record. This is the same
        // disagreement in the other direction — the aggregator says closed, the ATS is still
        // serving it, and the ATS wins.
        let pool = test_pool().await;
        let url = "https://job-boards.greenhouse.io/acme/jobs/1";

        collect_with(
            &pool,
            "manual",
            vec![
                FakeSource::arc(
                    "greenhouse",
                    SourceFetch::success(vec![raw(
                        "greenhouse",
                        "gh-1",
                        url,
                        "Software Engineer Intern",
                        "Acme",
                    )]),
                ),
                FakeSource::arc(
                    "simplify",
                    SourceFetch::success(vec![raw(
                        "simplify",
                        "sy-1",
                        url,
                        "Software Engineer Intern",
                        "Acme",
                    )]),
                ),
            ],
            ctx().await,
        )
        .await
        .expect("first run");

        collect_with(
            &pool,
            "manual",
            vec![
                FakeSource::arc(
                    "greenhouse",
                    SourceFetch::success(vec![raw(
                        "greenhouse",
                        "gh-1",
                        url,
                        "Software Engineer Intern",
                        "Acme",
                    )]),
                ),
                FakeSource::arc(
                    "simplify",
                    SourceFetch::success(vec![]).with_closed_ids(vec!["sy-1".to_string()]),
                ),
            ],
            ctx().await,
        )
        .await
        .expect("second run");

        let expired: Option<String> =
            sqlx::query_scalar("SELECT expired_at FROM internship_postings")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            expired, None,
            "greenhouse still lists it, so simplify's flag must not close it"
        );
    }

    #[tokio::test]
    async fn a_source_with_no_pay_cannot_erase_pay_another_source_supplied() {
        let pool = test_pool().await;
        let url = "https://job-boards.greenhouse.io/acme/jobs/1";
        let mut with_pay = raw("ashby", "ab-1", url, "Software Engineer Intern", "Acme");
        with_pay.pay_raw = Some("USD 45.00 - 55.00 per hour".to_string());

        collect_with(
            &pool,
            "manual",
            vec![FakeSource::arc("ashby", SourceFetch::success(vec![with_pay]))],
            ctx().await,
        )
        .await
        .expect("first run");

        let before: Option<f64> = sqlx::query_scalar("SELECT pay_min FROM internship_postings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(before, Some(45.0), "the pay should have parsed");

        // A second source carries the same job and knows nothing about pay.
        collect_with(
            &pool,
            "manual",
            vec![FakeSource::arc(
                "simplify",
                SourceFetch::success(vec![raw(
                    "simplify",
                    "sy-1",
                    url,
                    "Software Engineer Intern",
                    "Acme",
                )]),
            )],
            ctx().await,
        )
        .await
        .expect("second run");

        let after: Option<f64> = sqlx::query_scalar("SELECT pay_min FROM internship_postings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            after,
            Some(45.0),
            "merging with a less informative source must not erase a known salary"
        );
    }
}
