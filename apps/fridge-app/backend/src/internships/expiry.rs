//! The expiry sweep, and the one write site that decides whether a posting has "vanished".
//!
//! # Trap 2, and why it is split across two functions
//!
//! A posting disappears from a source for two very different reasons: it closed, or the fetch
//! broke. Blocked, rate-limited, reshaped, half-paginated — all of them look identical to
//! "the job is gone" if you only compare this run's ids against last run's. One blocked
//! LinkedIn fetch would then silently expire everything LinkedIn ever supplied.
//!
//! So the decision is split, and the split is the safety property:
//!
//! - [`settle_source_run`] is the **only** place `consecutive_misses` is ever advanced, and it
//!   advances nothing unless the run earned the right. A failed, partial, skipped, or
//!   suspicious-zero run leaves every counter exactly where it was.
//! - [`sweep`] reads counters and deadlines and **never looks at `source_runs` at all**.
//!
//! That asymmetry is deliberate. The sweep cannot get the successful-run rule wrong, because
//! the sweep does not implement the successful-run rule. There is no `AND outcome = 'success'`
//! for a future edit to drop.
//!
//! # Scopes, and where the finer grain leaks
//!
//! `settle_source_run` grants the right to advance counters per SOURCE, and for a source that
//! is one endpoint that is the right unit. Greenhouse is 485 endpoints under one name, and
//! "`Success` only if every board was enumerated" then means one unreachable board disqualifies
//! the other 484. On the 2026-09-02 uncapped run that is precisely what happened.
//!
//! **Corrected 2026-09-03.** This doc previously said Greenhouse "could essentially never expire
//! anything", inferring a rate from one observation. Measured over its full history it succeeds
//! on **8 of 16 runs** — half. So the cost was never "no expiry"; it was that *half* of
//! Greenhouse's runs contributed nothing, and the three-consecutive-eligible-runs threshold
//! therefore took about twice as long to reach as the collection cadence suggests. Scoping
//! recovers those runs, which is a real gain and a smaller one than the original claim.
//!
//! A source may now report [`ScopeRun`]s — per-board verdicts — and counters advance for the
//! scopes that were completely enumerated. The rule is unchanged, only its grain: absence is
//! evidence exactly when it is absence from a complete enumeration.
//!
//! ## What it under-expires, which is most of it
//!
//! - A sighting whose scope failed is untouched, run after run. Same as today, one board at a
//!   time instead of the whole source.
//! - Sightings recorded before migration 0026 carry `scope IS NULL`, and on a scoped source's
//!   *partial* run those do not advance. A sighting is tagged the next time it is **seen** —
//!   so this clears for everything still listed, and never for a sighting whose job is already
//!   gone. That one cannot be seen, therefore is never tagged, therefore never advances on a
//!   partial run; and Greenhouse is partial on about half its runs — 9 of 17 as of 2026-09-03.
//!   ("Nearly always" is what this line said until 12r. It is the same single-observation
//!   inference the correction fifteen lines above this one exists to retract, and it survived
//!   that correction because the correction was made by grepping for the *number*, not for
//!   the claim. Grep for the claim.)
//!
//!   Measured rather than assumed (2026-09-02, `docs/PLAN.md` § 12j): of 42 legacy sightings on
//!   100 completely-enumerated boards, 37 were tagged and **5 were already dead and stayed
//!   untagged**. An earlier draft of this doc claimed the untagged population "self-clears over
//!   a run or two". It does not. This is not a regression — before 0026 a partial run advanced
//!   nothing at all, so those 5 were equally stuck — but it means scoped expiry is
//!   **forward-looking only**: it expires what disappears after it starts watching, and does
//!   nothing for what had already gone before its sighting was tagged.
//!
//!   **Lever and Ashby joined the scoped sources in 12r with no backfill migration, and that
//!   was measured rather than assumed.** Greenhouse needed `0028` because it is `Partial` on
//!   half its runs, so its untagged rows were frozen. These two are not: Lever succeeds on 17
//!   of 21 runs and Ashby on 15 of 19, and on a `Success` run the scoped path advances untagged
//!   sightings exactly as the unscoped path did — `source_fully_enumerated()` is the branch
//!   that says so. Their untagged population on 2026-09-03 was 89 Lever and 151 Ashby, and it
//!   splits cleanly: 205 sit at 0 misses (live, and tagged by the next `Success` run), and 34
//!   sit at or past the 3-miss threshold, of which 31 belong to postings that are **already
//!   expired** and 3 are held alive by a live sighting on another source, which no scope tag
//!   can override — the sweep expires a posting only when *every* sighting is at threshold. So
//!   a backfill would have changed the fate of zero rows. Re-measure before assuming this still
//!   holds; if either source's success rate falls, the Greenhouse argument starts applying.
//! - A slug dropped from [`BoardDirectory`](super::sources::BoardDirectory) produces no
//!   completed scope, so its sightings stop advancing entirely. This is a strict improvement:
//!   that type's doc warns that pruning a slug makes its postings expire together,
//!   indistinguishable from the board genuinely closing. Under scopes the hazard changes from
//!   silent mass expiry into rows that linger, which is the direction to be wrong in.
//!
//! ## The one place it over-expires, stated plainly
//!
//! A sighting is tagged with the scope it was last seen in. Suppose a job's only sighting is
//! tagged board A, the job actually moves to board B, and B then fails on
//! `miss_threshold` consecutive runs while A completes. A really is a complete enumeration and
//! really does not contain the job, so its counter climbs, and the posting expires while being
//! live on B. Today it would survive, because B's failure would make the whole source partial.
//!
//! That is a real regression in one narrow case, and worth being exact about rather than
//! explaining away:
//!
//! - It is not a new *kind* of error. The source-level rule already concludes "gone" from
//!   absence in an enumeration that may not cover where the job now lives — a Greenhouse board
//!   whose slug was never harvested has always been invisible in exactly this way. Scoping
//!   makes that existing assumption reachable in one more situation; it does not introduce it.
//! - It is bounded three ways: [`sweep`] expires a posting only when **every** sighting is at
//!   threshold, so a Simplify or Lever sighting protects it outright; expiry is a soft delete,
//!   so an application referencing the posting still resolves; and reappearance anywhere resets
//!   the counter to 0.
//! - A job listed on two boards at once is safe by construction — one sighting row, re-tagged
//!   to whichever completed board last reported it, and reset because that board reported it.
//!
//! The trade is 484 boards' worth of expiry that could not happen at all, against one narrow
//! path to a soft delete that reverses itself the moment the job is seen again. Taken
//! knowingly.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use super::models::{ScopeRun, SourceOutcome};

/// How many consecutive expiry-eligible runs a sighting must be absent from before it counts
/// as gone. Overridable with `INTERNSHIP_MISS_THRESHOLD`.
pub const DEFAULT_MISS_THRESHOLD: i64 = 3;

/// What one source run actually produced, handed to [`settle_source_run`].
#[derive(Debug, Clone)]
pub struct SourceRunResult {
    pub source: String,
    pub outcome: SourceOutcome,
    /// The `external_id`s this run actually saw. Only meaningful when `outcome` is
    /// [`SourceOutcome::Success`]; ignored otherwise, because a partial or failed run's idea
    /// of "everything I saw" is not a statement about what exists.
    ///
    /// Read only when `scopes` is empty. A scoped source's ids live in
    /// [`ScopeRun::external_ids`], per scope, because "what I saw" is only usable paired with
    /// "what I finished looking at".
    pub seen_external_ids: Vec<String>,
    /// Per-scope verdicts, empty for every source that is a single endpoint. See [`ScopeRun`]
    /// and this module's doc.
    pub scopes: Vec<ScopeRun>,
    pub fetched: i64,
    pub accepted: i64,
    pub filtered: i64,
    pub rejected: i64,
    pub error: Option<String>,
}

/// Why a run was or wasn't allowed to advance disappearance counters. Recorded in the log and
/// surfaced in the run-health panel, so "this source stopped expiring things" is visible
/// rather than mysterious.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpiryEligibility {
    /// Full enumeration, non-suspicious. Absence from this run is evidence.
    Eligible,
    /// Not a complete enumeration of the whole source, but some of its scopes *were*
    /// completely enumerated — 484 Greenhouse boards read while one failed. Absence from those
    /// scopes is evidence; absence from anything else is not.
    ///
    /// Deliberately distinct from [`Eligible`](ExpiryEligibility::Eligible) rather than folded
    /// into it. Both advance counters, so both set `counts_for_expiry`, but only one of them
    /// means "this source is fully enumerated", and the health panel has to be able to tell a
    /// clean run from a 484-of-485 one.
    EligibleForScopes { completed: usize, attempted: usize },
    /// Not a complete enumeration — nothing can be concluded from absence.
    NotSuccessful,
    /// Completed, but returned nothing where it previously returned plenty. Far more likely a
    /// reshaped response or a silent block than a mass closure, so it is treated as evidence
    /// of nothing. This is a circuit breaker, not a correctness guarantee: a source genuinely
    /// closing its last posting stays live until it returns a non-empty run, which is the
    /// safe direction to be wrong in.
    SuspiciousZero { previous_fetched: i64 },
}

impl ExpiryEligibility {
    /// Whether this run advanced any disappearance counters at all — which is what
    /// `source_runs.counts_for_expiry` records.
    ///
    /// **The meaning of that column widened in migration 0026**, from "this source was fully
    /// enumerated" to "this run was trusted for at least one scope". For every unscoped source
    /// the two are identical. For Greenhouse they are not, and the difference is recorded in
    /// `source_run_scopes` rather than lost.
    pub fn may_expire(self) -> bool {
        matches!(
            self,
            ExpiryEligibility::Eligible | ExpiryEligibility::EligibleForScopes { .. }
        )
    }

    /// Whether the source as a whole was completely enumerated. Narrower than
    /// [`may_expire`](ExpiryEligibility::may_expire), and the right question when deciding
    /// whether an unscoped sighting may advance.
    pub fn source_fully_enumerated(self) -> bool {
        matches!(self, ExpiryEligibility::Eligible)
    }
}

/// Decide whether a completed source run has earned the right to expire postings.
///
/// Split out from the database work so it can be unit-tested without a pool — the rule is the
/// part worth pinning, and it is pure.
///
/// `previous_fetched` is what this source's last **expiry-eligible** run fetched, or `None`
/// if it has never had one (first run for a new source — nothing to be suspicious about yet).
pub fn eligibility(
    outcome: SourceOutcome,
    fetched: i64,
    previous_fetched: Option<i64>,
) -> ExpiryEligibility {
    if outcome != SourceOutcome::Success {
        return ExpiryEligibility::NotSuccessful;
    }

    match suspicious_zero(fetched, previous_fetched) {
        Some(previous_fetched) => ExpiryEligibility::SuspiciousZero { previous_fetched },
        None => ExpiryEligibility::Eligible,
    }
}

/// The circuit breaker on its own: `Some(previous)` when this run returned nothing and the last
/// trusted one returned plenty.
///
/// Split out of [`eligibility`] because [`scoped_eligibility`] needs the same test on a path
/// that never reaches `eligibility`'s success branch, and a second copy of it is a second place
/// for the threshold to drift.
fn suspicious_zero(fetched: i64, previous_fetched: Option<i64>) -> Option<i64> {
    match previous_fetched {
        Some(previous) if fetched == 0 && previous > 0 => Some(previous),
        _ => None,
    }
}

/// [`eligibility`], plus the per-scope verdicts a multi-scope source reports.
///
/// This is what [`settle_source_run`] actually calls; `eligibility` is left alone as the
/// source-level rule, unchanged and separately pinned, so that "a partial run may never expire
/// postings **at source granularity**" stays a property with its own tests rather than becoming
/// a branch inside a bigger function.
///
/// The upgrade path is narrow on purpose: only [`ExpiryEligibility::NotSuccessful`] can become
/// [`ExpiryEligibility::EligibleForScopes`], and only when at least one scope completed. A
/// suspicious zero stays a suspicious zero — a source that returned nothing at all while
/// claiming its boards were fine is the exact shape of a reshaped response, and it must not
/// escape the breaker by having reported scopes.
pub fn scoped_eligibility(
    outcome: SourceOutcome,
    fetched: i64,
    previous_fetched: Option<i64>,
    scopes: &[ScopeRun],
) -> ExpiryEligibility {
    let base = eligibility(outcome, fetched, previous_fetched);
    if base != ExpiryEligibility::NotSuccessful {
        return base;
    }

    let completed = scopes.iter().filter(|scope| scope.is_completed()).count();
    if completed == 0 {
        return base;
    }

    if let Some(previous_fetched) = suspicious_zero(fetched, previous_fetched) {
        return ExpiryEligibility::SuspiciousZero { previous_fetched };
    }

    ExpiryEligibility::EligibleForScopes {
        completed,
        attempted: scopes.len(),
    }
}

/// Record a finished source run and, **only if it earned the right**, advance disappearance
/// counters for that source.
///
/// This is the single write site for `posting_sightings.consecutive_misses`. Both halves land
/// in one transaction so a crash cannot leave a run recorded as expiry-eligible without its
/// counters moved, or vice versa.
pub async fn settle_source_run(
    pool: &SqlitePool,
    run_id: &str,
    source_run_id: &str,
    result: &SourceRunResult,
    now: DateTime<Utc>,
) -> Result<ExpiryEligibility> {
    // What this source last fetched on a run that was itself trusted. Comparing against an
    // untrusted run would let one bad run poison the next one's judgement.
    let previous_fetched: Option<i64> = sqlx::query_scalar(
        "SELECT fetched_count FROM source_runs
         WHERE source = ?1 AND counts_for_expiry = 1
         ORDER BY started_at DESC LIMIT 1",
    )
    .bind(&result.source)
    .fetch_optional(pool)
    .await?;

    let eligibility =
        scoped_eligibility(result.outcome, result.fetched, previous_fetched, &result.scopes);

    let outcome_text = match result.outcome {
        SourceOutcome::Success => "success",
        SourceOutcome::Partial => "partial",
        SourceOutcome::Failed => "failed",
        SourceOutcome::Skipped => "skipped",
    };

    // `begin_write`, not `begin()`. This transaction's first statement is a write today, so a
    // deferred BEGIN would take the write lock there and the busy handler would do its job —
    // measured, in `concurrent_source_settlements_do_not_collide`, which passes either way.
    // What it must not depend on is *staying* that way: adding a SELECT above the INSERT would
    // silently turn this into a read-then-write transaction, and those fail instantly under a
    // competing writer with no wait. See `db::begin_write`.
    let mut tx = crate::db::begin_write(pool).await?;

    sqlx::query(
        "INSERT INTO source_runs
             (id, run_id, source, started_at, finished_at, outcome,
              fetched_count, accepted_count, filtered_count, rejected_count,
              counts_for_expiry, error)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT (run_id, source) DO UPDATE SET
             finished_at = excluded.finished_at,
             outcome = excluded.outcome,
             fetched_count = excluded.fetched_count,
             accepted_count = excluded.accepted_count,
             filtered_count = excluded.filtered_count,
             rejected_count = excluded.rejected_count,
             counts_for_expiry = excluded.counts_for_expiry,
             error = excluded.error",
    )
    .bind(source_run_id)
    .bind(run_id)
    .bind(&result.source)
    .bind(now.to_rfc3339())
    .bind(outcome_text)
    .bind(result.fetched)
    .bind(result.accepted)
    .bind(result.filtered)
    .bind(result.rejected)
    .bind(i64::from(eligibility.may_expire()))
    .bind(result.error.as_deref())
    .execute(&mut *tx)
    .await?;

    // The id `source_run_scopes` must reference. Read **after** the INSERT, never before: this
    // transaction is already a writer by now, so this is not the read-then-write shape
    // `db::begin_write` warns about above. It is asked rather than assumed because the ON
    // CONFLICT is on `(run_id, source)` and leaves `id` untouched, so the row that ends up
    // holding this run is not necessarily the one whose id was passed in.
    let scope_run_id: String =
        sqlx::query_scalar("SELECT id FROM source_runs WHERE run_id = ?1 AND source = ?2")
            .bind(run_id)
            .bind(&result.source)
            .fetch_one(&mut *tx)
            .await?;

    // Recorded before the counters move, and in the same transaction, so the increment below
    // can read back the completed set and a crash can never leave counters advanced with no
    // record of which scopes earned it.
    for scope in &result.scopes {
        sqlx::query(
            "INSERT INTO source_run_scopes
                 (source_run_id, scope, outcome, fetched_count, error, gone)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (source_run_id, scope) DO UPDATE SET
                 outcome = excluded.outcome,
                 fetched_count = excluded.fetched_count,
                 error = excluded.error,
                 gone = excluded.gone",
        )
        .bind(&scope_run_id)
        .bind(&scope.scope)
        .bind(scope.outcome.as_str())
        .bind(scope.fetched)
        .bind(scope.error.as_deref())
        .bind(i64::from(scope.gone))
        .execute(&mut *tx)
        .await?;
    }

    if eligibility.may_expire() {
        // Everything in scope for this run starts as a miss; the sightings actually observed
        // are then reset to zero. Written in this order — blanket increment, then targeted
        // reset — so a sighting cannot be skipped by an id that failed to match.
        //
        // The reset is the important half. A sighting seen this run must return to 0 rather
        // than merely stop climbing, or a posting that flickers in and out across many runs
        // eventually crosses the threshold while never having been absent twice running.
        if result.scopes.is_empty() {
            // The unscoped path: every source that genuinely is a single endpoint — Simplify,
            // vanshb03, weworkremotely and the best-effort three. Byte for byte what this did
            // before scopes existed, and kept as its own branch rather than as a degenerate
            // case of the scoped one so that "nothing changed for these sources" is visible
            // instead of argued.
            sqlx::query(
                "UPDATE posting_sightings
                 SET consecutive_misses = consecutive_misses + 1
                 WHERE source = ?1",
            )
            .bind(&result.source)
            .execute(&mut *tx)
            .await?;

            for external_id in &result.seen_external_ids {
                sqlx::query(
                    "UPDATE posting_sightings
                     SET consecutive_misses = 0, last_seen_at = ?3, last_seen_run_id = ?4
                     WHERE source = ?1 AND external_id = ?2",
                )
                .bind(&result.source)
                .bind(external_id)
                .bind(now.to_rfc3339())
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
            }
        } else {
            // The scoped path. Two kinds of sighting may advance:
            //
            //   - one tagged with a scope this run completely enumerated, and
            //   - an untagged one, but only when the source as a whole was completely
            //     enumerated. Untagged means "seen before 0026, or seen by an unscoped
            //     adapter", and on a partial run there is no scope to check it against, so
            //     the only honest answer is to leave it alone.
            let unscoped_may_advance = i64::from(eligibility.source_fully_enumerated());
            sqlx::query(
                "UPDATE posting_sightings
                 SET consecutive_misses = consecutive_misses + 1
                 WHERE source = ?1
                   AND ( (?2 = 1 AND scope IS NULL)
                      OR scope IN (SELECT scope FROM source_run_scopes
                                    WHERE source_run_id = ?3 AND outcome = 'completed') )",
            )
            .bind(&result.source)
            .bind(unscoped_may_advance)
            .bind(&scope_run_id)
            .execute(&mut *tx)
            .await?;

            // Reset and re-tag together: `scope` is "where we last saw it" and
            // `last_seen_at` is "when", and they are one fact. A job that moved from one board
            // to another is re-tagged here, which is what keeps the next run's increment
            // asking the right board about it. A job listed on two completed boards lands on
            // whichever comes later in board order — deterministic, and it does not matter
            // which, since either one reporting it resets the counter.
            for scope in result.scopes.iter().filter(|scope| scope.is_completed()) {
                for external_id in &scope.external_ids {
                    sqlx::query(
                        "UPDATE posting_sightings
                         SET consecutive_misses = 0, last_seen_at = ?3, last_seen_run_id = ?4,
                             scope = ?5
                         WHERE source = ?1 AND external_id = ?2",
                    )
                    .bind(&result.source)
                    .bind(external_id)
                    .bind(now.to_rfc3339())
                    .bind(run_id)
                    .bind(&scope.scope)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }
    }

    tx.commit().await?;

    match eligibility {
        ExpiryEligibility::Eligible => {}
        ExpiryEligibility::EligibleForScopes {
            completed,
            attempted,
        } => {
            // Not an error and not silence. A source sitting at 400-of-485 for weeks is a
            // directory going stale, and it is only visible if someone can watch the number.
            println!(
                "internships: {} finished {outcome_text} — disappearance counters advanced for \
                 {completed} of {attempted} scope(s)",
                result.source
            );
        }
        ExpiryEligibility::NotSuccessful => {
            println!(
                "internships: {} finished {outcome_text} — disappearance counters untouched",
                result.source
            );
        }
        ExpiryEligibility::SuspiciousZero { previous_fetched } => {
            eprintln!(
                "internships: {} returned 0 postings after a run of {previous_fetched} — \
                 treating as a broken fetch, not a mass closure; counters untouched",
                result.source
            );
        }
    }

    Ok(eligibility)
}

/// What one sweep did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub deadline_passed: u64,
    pub vanished: u64,
}

impl SweepReport {
    pub fn is_empty(&self) -> bool {
        *self == SweepReport::default()
    }
}

/// Expire postings that have closed.
///
/// Two rules, and **neither one reads `source_runs`** — see the module doc. Expiry is a soft
/// delete throughout: this sets `expired_at`/`expiry_reason` and never removes a row, so an
/// application that references a posting keeps resolving.
pub async fn sweep(pool: &SqlitePool, now: DateTime<Utc>, miss_threshold: i64) -> Result<SweepReport> {
    let now_text = now.to_rfc3339();

    // 1. A deadline the source stated, now in the past. NULL deadline is the common case and
    //    means "none stated" — it must never be read as "closes now".
    let deadline_passed = sqlx::query(
        "UPDATE internship_postings
         SET expired_at = ?1, expiry_reason = 'deadline_passed', updated_at = ?1
         WHERE expired_at IS NULL AND deadline IS NOT NULL AND deadline < ?1",
    )
    .bind(&now_text)
    .execute(pool)
    .await?
    .rows_affected();

    // 2. Gone from *every* source that ever carried it.
    //
    // The EXISTS clause is load-bearing and not defensive noise: `NOT EXISTS (a sighting
    // below threshold)` is **vacuously true for a posting with no sightings at all**, so
    // without it a posting whose sightings failed to record would expire on the first sweep
    // after being created. Pinned by `a_posting_with_no_sightings_is_never_swept`.
    let vanished = sqlx::query(
        "UPDATE internship_postings
         SET expired_at = ?1, expiry_reason = 'vanished_from_sources', updated_at = ?1
         WHERE expired_at IS NULL
           AND EXISTS (
                 SELECT 1 FROM posting_sightings s WHERE s.posting_id = internship_postings.id
               )
           AND NOT EXISTS (
                 SELECT 1 FROM posting_sightings s
                 WHERE s.posting_id = internship_postings.id
                   AND s.consecutive_misses < ?2
               )",
    )
    .bind(&now_text)
    .bind(miss_threshold)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(SweepReport {
        deadline_passed,
        vanished,
    })
}

/// `INTERNSHIP_MISS_THRESHOLD`, defaulting to [`DEFAULT_MISS_THRESHOLD`].
pub fn miss_threshold() -> i64 {
    std::env::var("INTERNSHIP_MISS_THRESHOLD")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|threshold| *threshold >= 1)
        .unwrap_or(DEFAULT_MISS_THRESHOLD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    // ---- the eligibility rule, which is the whole of trap 2 ----

    #[test]
    fn a_full_successful_run_may_expire_postings() {
        assert_eq!(
            eligibility(SourceOutcome::Success, 400, Some(400)),
            ExpiryEligibility::Eligible
        );
    }

    #[test]
    fn a_failed_run_may_never_expire_postings() {
        // The named data-loss bug of this phase: one blocked fetch must not expire
        // everything that source ever supplied.
        assert_eq!(
            eligibility(SourceOutcome::Failed, 0, Some(400)),
            ExpiryEligibility::NotSuccessful
        );
        assert!(!eligibility(SourceOutcome::Failed, 0, Some(400)).may_expire());
    }

    #[test]
    fn a_partial_run_may_never_expire_postings() {
        // A paginated fetch that died on page 3 of 10 returns real postings and proves
        // nothing whatsoever about the ones it never reached.
        assert_eq!(
            eligibility(SourceOutcome::Partial, 120, Some(400)),
            ExpiryEligibility::NotSuccessful
        );
    }

    #[test]
    fn a_skipped_run_may_never_expire_postings() {
        assert_eq!(
            eligibility(SourceOutcome::Skipped, 0, Some(400)),
            ExpiryEligibility::NotSuccessful
        );
    }

    #[test]
    fn a_successful_run_that_suddenly_returns_nothing_is_not_trusted() {
        assert_eq!(
            eligibility(SourceOutcome::Success, 0, Some(400)),
            ExpiryEligibility::SuspiciousZero {
                previous_fetched: 400
            }
        );
        assert!(!eligibility(SourceOutcome::Success, 0, Some(400)).may_expire());
    }

    #[test]
    fn a_sources_very_first_run_is_trusted_even_when_empty() {
        // Nothing to be suspicious about yet: a brand-new source that legitimately has no
        // open postings must not be permanently barred from ever expiring anything.
        assert_eq!(
            eligibility(SourceOutcome::Success, 0, None),
            ExpiryEligibility::Eligible
        );
    }

    #[test]
    fn a_source_that_was_already_empty_and_still_is_stays_trusted() {
        // 0 following 0 is not a cliff, so the circuit breaker must not latch on.
        assert_eq!(
            eligibility(SourceOutcome::Success, 0, Some(0)),
            ExpiryEligibility::Eligible
        );
    }

    #[test]
    fn one_posting_left_is_not_suspicious() {
        // The boundary the threshold actually turns on: `fetched == 0`, not "fetched dropped
        // a lot". A source going 400 -> 1 is a real collapse we still trust, because
        // guessing at how large a drop is too large is a heuristic nobody asked for.
        assert_eq!(
            eligibility(SourceOutcome::Success, 1, Some(400)),
            ExpiryEligibility::Eligible
        );
    }

    // ---- the threshold env var ----

    #[test]
    fn a_miss_threshold_below_one_is_refused() {
        // 0 would expire a posting the first run it wasn't seen, which defeats the point of
        // counting consecutive misses at all.
        assert_eq!(
            "0".trim().parse::<i64>().ok().filter(|t| *t >= 1),
            None,
            "0 must not be accepted as a threshold"
        );
    }

    // ---- the eligibility rule at scope granularity ----
    //
    // The tests above pin the source-level rule and are deliberately untouched: `eligibility`
    // still takes the same three arguments and still answers the same way. Scopes are a second
    // question asked afterwards, not a fourth parameter to the first one.

    fn done(scope: &str) -> ScopeRun {
        ScopeRun::completed(scope, Vec::new())
    }

    fn broke(scope: &str) -> ScopeRun {
        ScopeRun::failed(scope, "HTTP 500")
    }

    #[test]
    fn a_partial_run_may_expire_within_the_scopes_it_did_finish() {
        // The whole point of 0026. 484 boards enumerated and one dead one used to mean nobody
        // expired anything.
        let scopes: Vec<ScopeRun> = (0..484)
            .map(|i| done(&format!("board{i}")))
            .chain(std::iter::once(broke("designmehair")))
            .collect();
        assert_eq!(
            scoped_eligibility(SourceOutcome::Partial, 1_900, Some(1_900), &scopes),
            ExpiryEligibility::EligibleForScopes {
                completed: 484,
                attempted: 485
            }
        );
    }

    #[test]
    fn a_partial_run_whose_every_scope_failed_still_expires_nothing() {
        let scopes = vec![broke("a"), broke("b")];
        assert_eq!(
            scoped_eligibility(SourceOutcome::Partial, 0, Some(400), &scopes),
            ExpiryEligibility::NotSuccessful
        );
    }

    #[test]
    fn a_partial_run_reporting_no_scopes_is_judged_exactly_as_before() {
        // Every single-endpoint source takes this path, and it must reach the same verdict the
        // source-level rule does.
        for outcome in [
            SourceOutcome::Success,
            SourceOutcome::Partial,
            SourceOutcome::Failed,
            SourceOutcome::Skipped,
        ] {
            for (fetched, previous) in [(400, Some(400)), (0, Some(400)), (0, None), (1, Some(0))] {
                assert_eq!(
                    scoped_eligibility(outcome, fetched, previous, &[]),
                    eligibility(outcome, fetched, previous),
                    "{outcome:?} {fetched} {previous:?}"
                );
            }
        }
    }

    #[test]
    fn scopes_cannot_smuggle_a_suspicious_zero_past_the_breaker() {
        // A source that reports 485 healthy boards and returns nothing at all is a reshaped
        // response, not 485 simultaneous mass closures. Reporting scopes must not be a way out
        // of the circuit breaker.
        let scopes = vec![done("a"), done("b")];
        assert_eq!(
            scoped_eligibility(SourceOutcome::Partial, 0, Some(1_900), &scopes),
            ExpiryEligibility::SuspiciousZero {
                previous_fetched: 1_900
            }
        );
    }

    #[test]
    fn a_fully_enumerated_scoped_run_is_plainly_eligible() {
        // Not `EligibleForScopes`: the distinction is what the health panel reads to tell a
        // clean run from a 484-of-485 one.
        let scopes = vec![done("a"), done("b")];
        assert_eq!(
            scoped_eligibility(SourceOutcome::Success, 100, Some(100), &scopes),
            ExpiryEligibility::Eligible
        );
    }

    #[test]
    fn advancing_some_scopes_counts_for_expiry_but_is_not_a_full_enumeration() {
        let partial = ExpiryEligibility::EligibleForScopes {
            completed: 484,
            attempted: 485,
        };
        assert!(partial.may_expire(), "484 boards' worth of expiry did happen");
        assert!(
            !partial.source_fully_enumerated(),
            "and an untagged sighting must not ride along on it"
        );
        assert!(ExpiryEligibility::Eligible.source_fully_enumerated());
    }

    // ---- settle, against a real database ----

    async fn scoped_pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("expiry-scoped-{}.db", Uuid::new_v4()));
        crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("migrations")
    }

    async fn open_run(pool: &SqlitePool, now: DateTime<Utc>) -> String {
        let run_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO collection_runs (id, started_at, trigger) VALUES (?1, ?2, 'manual')",
        )
        .bind(&run_id)
        .bind(now.to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
        run_id
    }

    /// A posting plus one sighting of it, optionally already tagged with a scope and already
    /// carrying misses.
    async fn seed(
        pool: &SqlitePool,
        source: &str,
        external_id: &str,
        scope: Option<&str>,
        misses: i64,
    ) -> String {
        let posting_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO internship_postings
                 (id, dedup_key, company_key, company_name, title, canonical_url,
                  first_seen_at, last_seen_at, created_at, updated_at)
             VALUES (?1, ?1, 'acme', 'Acme', 'SWE Intern', 'https://example.test', ?2, ?2, ?2, ?2)",
        )
        .bind(&posting_id)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO posting_sightings
                 (id, posting_id, source, external_id, url, first_seen_at, last_seen_at,
                  consecutive_misses, scope)
             VALUES (?1, ?2, ?3, ?4, 'https://example.test', ?5, ?5, ?6, ?7)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&posting_id)
        .bind(source)
        .bind(external_id)
        .bind(&now)
        .bind(misses)
        .bind(scope)
        .execute(pool)
        .await
        .unwrap();

        posting_id
    }

    async fn misses_of(pool: &SqlitePool, source: &str, external_id: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT consecutive_misses FROM posting_sightings
              WHERE source = ?1 AND external_id = ?2",
        )
        .bind(source)
        .bind(external_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn scope_of(pool: &SqlitePool, source: &str, external_id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT scope FROM posting_sightings WHERE source = ?1 AND external_id = ?2")
            .bind(source)
            .bind(external_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    fn greenhouse_run(scopes: Vec<ScopeRun>, outcome: SourceOutcome, fetched: i64) -> SourceRunResult {
        SourceRunResult {
            source: "greenhouse".to_string(),
            outcome,
            seen_external_ids: Vec::new(),
            scopes,
            fetched,
            accepted: fetched,
            filtered: 0,
            rejected: 0,
            error: None,
        }
    }

    #[tokio::test]
    async fn one_failed_board_does_not_stop_the_others_from_advancing() {
        // The 2026-09-02 finding, in miniature: boards A and B were read completely, board C
        // was not. A and B may conclude closure; C may conclude nothing.
        let pool = scoped_pool().await;
        let now = Utc::now();
        let run_id = open_run(&pool, now).await;

        seed(&pool, "greenhouse", "gone-from-a", Some("boardA"), 0).await;
        seed(&pool, "greenhouse", "gone-from-b", Some("boardB"), 0).await;
        seed(&pool, "greenhouse", "on-broken-c", Some("boardC"), 0).await;
        seed(&pool, "greenhouse", "still-on-a", Some("boardA"), 0).await;

        let result = greenhouse_run(
            vec![
                ScopeRun::completed("boardA", vec!["still-on-a".to_string()]),
                ScopeRun::completed("boardB", Vec::new()),
                ScopeRun::failed("boardC", "HTTP 500"),
            ],
            SourceOutcome::Partial,
            1,
        );

        let verdict = settle_source_run(&pool, &run_id, &Uuid::new_v4().to_string(), &result, now)
            .await
            .unwrap();
        assert_eq!(
            verdict,
            ExpiryEligibility::EligibleForScopes {
                completed: 2,
                attempted: 3
            }
        );

        assert_eq!(misses_of(&pool, "greenhouse", "gone-from-a").await, 1);
        assert_eq!(misses_of(&pool, "greenhouse", "gone-from-b").await, 1);
        assert_eq!(misses_of(&pool, "greenhouse", "still-on-a").await, 0);
        assert_eq!(
            misses_of(&pool, "greenhouse", "on-broken-c").await,
            0,
            "a board that could not be read proves nothing about what is on it"
        );
    }

    #[tokio::test]
    async fn a_gone_scope_is_a_completed_scope_for_every_expiry_purpose() {
        // The invariant migration 0032 preserves, and the reason `gone` is a flag beside the
        // outcome rather than a third `ScopeOutcome`. Every expiry decision keys off
        // `outcome = 'completed'`; a third enum value would have quietly removed 404'd boards
        // from expiry, which is the exact opposite of what 0026 was built to do.
        //
        // So: two empty boards, one recorded as gone and one merely empty, must move their
        // sightings identically. The only difference the flag may make is to what a human can
        // read back afterwards.
        let pool = scoped_pool().await;
        let now = Utc::now();
        let run_id = open_run(&pool, now).await;

        seed(&pool, "greenhouse", "on-an-empty-board", Some("empty"), 1).await;
        seed(&pool, "greenhouse", "on-a-dead-board", Some("dead"), 1).await;

        let result = greenhouse_run(
            vec![ScopeRun::completed("empty", Vec::new()), ScopeRun::gone("dead")],
            SourceOutcome::Partial,
            0,
        );

        let verdict = settle_source_run(&pool, &run_id, &Uuid::new_v4().to_string(), &result, now)
            .await
            .unwrap();
        assert_eq!(
            verdict,
            ExpiryEligibility::EligibleForScopes {
                completed: 2,
                attempted: 2
            },
            "a gone scope counts as a completed one when the run's eligibility is decided"
        );

        assert_eq!(misses_of(&pool, "greenhouse", "on-an-empty-board").await, 2);
        assert_eq!(
            misses_of(&pool, "greenhouse", "on-a-dead-board").await,
            2,
            "gone must advance exactly as empty-but-alive does"
        );

        // And the distinction the flag exists for survives into the row, which is the half
        // that did not exist before 0032.
        let gone: Vec<(String, i64)> =
            sqlx::query_as("SELECT scope, gone FROM source_run_scopes ORDER BY scope")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            gone,
            vec![("dead".to_string(), 1), ("empty".to_string(), 0)],
            "the database can now tell a deleted board from an empty one"
        );
    }

    #[tokio::test]
    async fn a_posting_on_a_failed_board_is_left_exactly_where_it_was() {
        // Sharper than "unchanged from zero": this one is one run short of expiring, and a
        // board that failed must not be what tips it over.
        let pool = scoped_pool().await;
        let now = Utc::now();
        let run_id = open_run(&pool, now).await;

        seed(&pool, "greenhouse", "nearly-gone", Some("boardC"), 2).await;

        let result = greenhouse_run(
            vec![
                ScopeRun::completed("boardA", Vec::new()),
                ScopeRun::failed("boardC", "connection reset"),
            ],
            SourceOutcome::Partial,
            0,
        );
        settle_source_run(&pool, &run_id, &Uuid::new_v4().to_string(), &result, now)
            .await
            .unwrap();

        assert_eq!(misses_of(&pool, "greenhouse", "nearly-gone").await, 2);
        assert_eq!(
            sweep(&pool, now, DEFAULT_MISS_THRESHOLD).await.unwrap(),
            SweepReport::default()
        );
    }

    #[tokio::test]
    async fn an_unscoped_source_settles_exactly_as_it_did_before_scopes() {
        // Simplify, Lever, Ashby, WeWorkRemotely. Blanket increment, targeted reset, and the
        // scope column left alone — the branch this exercises is the pre-0026 code verbatim.
        let pool = scoped_pool().await;
        let now = Utc::now();
        let run_id = open_run(&pool, now).await;

        seed(&pool, "simplify", "still-listed", None, 0).await;
        seed(&pool, "simplify", "vanished", None, 1).await;

        let result = SourceRunResult {
            source: "simplify".to_string(),
            outcome: SourceOutcome::Success,
            seen_external_ids: vec!["still-listed".to_string()],
            scopes: Vec::new(),
            fetched: 1,
            accepted: 1,
            filtered: 0,
            rejected: 0,
            error: None,
        };
        let verdict = settle_source_run(&pool, &run_id, &Uuid::new_v4().to_string(), &result, now)
            .await
            .unwrap();

        assert_eq!(verdict, ExpiryEligibility::Eligible);
        assert_eq!(misses_of(&pool, "simplify", "still-listed").await, 0);
        assert_eq!(misses_of(&pool, "simplify", "vanished").await, 2);
        assert_eq!(scope_of(&pool, "simplify", "still-listed").await, None);
        assert_eq!(scope_of(&pool, "simplify", "vanished").await, None);
    }

    #[tokio::test]
    async fn an_untagged_sighting_does_not_advance_on_a_partly_enumerated_run() {
        // Every Greenhouse sighting that predates 0026 is untagged. On a partial run there is
        // no scope to check it against, so the only honest answer is to leave it alone; it
        // gets tagged the next time it is actually seen.
        let pool = scoped_pool().await;
        let now = Utc::now();
        let run_id = open_run(&pool, now).await;

        seed(&pool, "greenhouse", "legacy", None, 0).await;

        let result = greenhouse_run(
            vec![
                ScopeRun::completed("boardA", Vec::new()),
                ScopeRun::failed("boardB", "HTTP 500"),
            ],
            SourceOutcome::Partial,
            0,
        );
        settle_source_run(&pool, &run_id, &Uuid::new_v4().to_string(), &result, now)
            .await
            .unwrap();

        assert_eq!(misses_of(&pool, "greenhouse", "legacy").await, 0);
    }

    #[tokio::test]
    async fn an_untagged_sighting_advances_when_the_whole_source_was_enumerated() {
        // The other half: a fully successful run is exactly as trustworthy as it was before
        // scopes existed, so an untagged sighting advances on it just as it always did.
        let pool = scoped_pool().await;
        let now = Utc::now();
        let run_id = open_run(&pool, now).await;

        seed(&pool, "greenhouse", "legacy", None, 0).await;

        let result = greenhouse_run(
            vec![ScopeRun::completed("boardA", vec!["other".to_string()])],
            SourceOutcome::Success,
            1,
        );
        let verdict = settle_source_run(&pool, &run_id, &Uuid::new_v4().to_string(), &result, now)
            .await
            .unwrap();

        assert_eq!(verdict, ExpiryEligibility::Eligible);
        assert_eq!(misses_of(&pool, "greenhouse", "legacy").await, 1);
    }

    #[tokio::test]
    async fn a_tagged_and_an_untagged_sighting_advance_together_on_a_full_run() {
        // Migration 0028 backfills `scope` onto sightings that predate 0026, which moves a row
        // from the "untagged, advances because the source was fully enumerated" branch to the
        // "tagged, advances because its board completed" one. On a fully successful run every
        // board is in the completed set, so the two branches must select the same rows — if
        // they did not, backfilling would have quietly narrowed what a clean run can expire.
        let pool = scoped_pool().await;
        let now = Utc::now();
        let run_id = open_run(&pool, now).await;

        seed(&pool, "greenhouse", "untagged-gone", None, 0).await;
        seed(&pool, "greenhouse", "tagged-gone", Some("boardA"), 0).await;

        let result = greenhouse_run(
            vec![
                ScopeRun::completed("boardA", Vec::new()),
                ScopeRun::completed("boardB", vec!["something-else".to_string()]),
            ],
            SourceOutcome::Success,
            1,
        );
        settle_source_run(&pool, &run_id, &Uuid::new_v4().to_string(), &result, now)
            .await
            .unwrap();

        assert_eq!(misses_of(&pool, "greenhouse", "untagged-gone").await, 1);
        assert_eq!(
            misses_of(&pool, "greenhouse", "tagged-gone").await,
            1,
            "a backfilled tag must not cost a row the advance it would have had untagged"
        );
    }

    #[tokio::test]
    async fn a_sighting_is_retagged_with_the_board_that_reported_it() {
        // A job that moves between boards. The tag is "where we last saw it", and keeping it
        // current is what makes the next run's increment ask the right board about it.
        let pool = scoped_pool().await;
        let now = Utc::now();
        let run_id = open_run(&pool, now).await;

        seed(&pool, "greenhouse", "movable", Some("boardA"), 2).await;

        let result = greenhouse_run(
            vec![
                ScopeRun::completed("boardA", Vec::new()),
                ScopeRun::completed("boardB", vec!["movable".to_string()]),
            ],
            SourceOutcome::Success,
            1,
        );
        settle_source_run(&pool, &run_id, &Uuid::new_v4().to_string(), &result, now)
            .await
            .unwrap();

        assert_eq!(misses_of(&pool, "greenhouse", "movable").await, 0);
        assert_eq!(
            scope_of(&pool, "greenhouse", "movable").await.as_deref(),
            Some("boardB")
        );
    }

    #[tokio::test]
    async fn every_board_verdict_is_recorded_with_its_reason() {
        // The source-level error string shows three failures and a count. When 40 boards fail,
        // these rows are the only place the 4th one is legible.
        let pool = scoped_pool().await;
        let now = Utc::now();
        let run_id = open_run(&pool, now).await;
        let source_run_id = Uuid::new_v4().to_string();

        let result = greenhouse_run(
            vec![
                ScopeRun::completed("boardA", vec!["x".to_string()]),
                ScopeRun::failed("boardB", "HTTP 503"),
            ],
            SourceOutcome::Partial,
            1,
        );
        settle_source_run(&pool, &run_id, &source_run_id, &result, now)
            .await
            .unwrap();

        let rows: Vec<(String, String, i64, Option<String>)> = sqlx::query_as(
            "SELECT scope, outcome, fetched_count, error FROM source_run_scopes
              WHERE source_run_id = ?1 ORDER BY scope",
        )
        .bind(&source_run_id)
        .fetch_all(&pool)
        .await
        .unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ("boardA".into(), "completed".into(), 1, None));
        assert_eq!(
            rows[1],
            ("boardB".into(), "failed".into(), 0, Some("HTTP 503".into()))
        );

        // And the run itself reads as "this expired something", which before 0026 it could not.
        let counts: bool =
            sqlx::query_scalar("SELECT counts_for_expiry FROM source_runs WHERE id = ?1")
                .bind(&source_run_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(counts);
    }

    // ---- the sweep, which scopes do not touch ----

    #[tokio::test]
    async fn a_posting_with_no_sightings_is_never_swept() {
        // The test `sweep`'s comment has claimed for two phases was pinning this rule. It was
        // not: it did not exist until 0026. `NOT EXISTS (a sighting below threshold)` is
        // vacuously true for a posting with no sightings at all, so without the EXISTS guard a
        // posting whose sightings failed to record expires on the very next sweep.
        let pool = scoped_pool().await;
        let now = Utc::now();

        let orphan = seed(&pool, "greenhouse", "temp", None, 0).await;
        sqlx::query("DELETE FROM posting_sightings WHERE posting_id = ?1")
            .bind(&orphan)
            .execute(&pool)
            .await
            .unwrap();

        // A control, so this cannot pass by the sweep simply doing nothing.
        seed(&pool, "greenhouse", "really-gone", Some("boardA"), 9).await;

        let report = sweep(&pool, now, DEFAULT_MISS_THRESHOLD).await.unwrap();
        assert_eq!(report.vanished, 1, "the control must expire");

        let expired: Option<String> =
            sqlx::query_scalar("SELECT expired_at FROM internship_postings WHERE id = ?1")
                .bind(&orphan)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(expired, None, "a posting with no sightings must survive");
    }

    // ---- concurrency: the sweep is a background writer sharing a pool with the request path ----

    /// Eight sources settling at once, which is what a collection run does at the end.
    ///
    /// The sweep writes on its own schedule while requests are being served, so it is the
    /// most likely collision partner for anything on the request path. This asserts the whole
    /// call succeeds under contention rather than that any particular locking scheme is used.
    #[tokio::test]
    async fn concurrent_source_settlements_do_not_collide() {
        let path = std::env::temp_dir().join(format!("expiry-concurrency-{}.db", Uuid::new_v4()));
        let pool = crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("migrations");

        let run_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        // `source_runs.run_id` is a real enforced foreign key, so the parent has to exist.
        sqlx::query(
            "INSERT INTO collection_runs (id, started_at, trigger) VALUES (?1, ?2, 'manual')",
        )
        .bind(&run_id)
        .bind(now.to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        let mut handles = Vec::new();
        for index in 0..8 {
            let pool = pool.clone();
            let run_id = run_id.clone();
            handles.push(tokio::spawn(async move {
                let result = SourceRunResult {
                    source: format!("source-{index}"),
                    outcome: SourceOutcome::Success,
                    seen_external_ids: Vec::new(),
                    scopes: Vec::new(),
                    fetched: 10,
                    accepted: 10,
                    filtered: 0,
                    rejected: 0,
                    error: None,
                };
                settle_source_run(&pool, &run_id, &Uuid::new_v4().to_string(), &result, now).await
            }));
        }

        for handle in handles {
            handle
                .await
                .unwrap()
                .expect("a settlement failed under contention");
        }

        let settled: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_runs WHERE run_id = ?")
            .bind(&run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(settled, 8);
    }
}
