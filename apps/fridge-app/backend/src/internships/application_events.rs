//! The append-only history behind `internship_applications.status`.
//!
//! The mutable status column stays as the tracker's read cache. This module owns the log that
//! explains it, and every live writer joins [`record`] to the same `BEGIN IMMEDIATE`
//! transaction as its status change. Task 10d intentionally adds no live callers; task 10e
//! wires those writers after this table exists.
//!
//! # Backfill contract
//!
//! `application-events backfill` runs before any server background work. It holds one write
//! transaction for all three evidence sources, in this order:
//!
//! 1. accepted or auto-applied status proposals, including the compensating transition for a
//!    rejected auto-applied proposal;
//! 2. every application's creation at `applied_at`;
//! 3. the current status at `status_changed_at` only when the first two sources do not fold to
//!    it.
//!
//! Proposal rows can prove their actor and both sides of a transition. Creation and fallback
//! rows cannot, so they use `unknown` and a NULL `from_status`; calling them `manual` would
//! invent provenance. Backfilled `created_at` is write time, not event time, so it remains
//! visibly different from `at`.
//!
//! # Ordering and idempotency
//!
//! A status folds by `at ASC, created_at ASC, id ASC`, with the last `to_status` winning.
//! Causal events use `(application_id, cause_kind, cause_id, to_status)` as their structural
//! idempotency key. A proposal's forward transition and undo share a cause but have different
//! target statuses. Events without a cause deliberately do not deduplicate in SQLite; the
//! backfill is a one-shot command, and two manual edits must remain two events.

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use sqlx::{Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use super::models::ApplicationStatus;

/// Whether an event's destination proves that an application received a response.
///
/// Both analytics and the Phase 11 nudge producer use this predicate. Keeping it beside the
/// event model prevents the dashboard and notifications from disagreeing about the same
/// application. `Rejected` is a response; only another `applied` event is not.
pub fn is_response_status(status: ApplicationStatus) -> bool {
    status != ApplicationStatus::Applied
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    Email,
    Extension,
    Manual,
    Sweep,
    Unknown,
}

impl Actor {
    fn as_str(self) -> &'static str {
        match self {
            Actor::Email => "email",
            Actor::Extension => "extension",
            Actor::Manual => "manual",
            Actor::Sweep => "sweep",
            Actor::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause<'a> {
    StatusProposal(&'a str),
    EmailVerdict(&'a str),
    HuntEvent(&'a str),
}

impl<'a> Cause<'a> {
    fn parts(self) -> (&'static str, &'a str) {
        match self {
            Cause::StatusProposal(id) => ("status_proposal", id),
            Cause::EmailVerdict(id) => ("email_verdict", id),
            Cause::HuntEvent(id) => ("hunt_event", id),
        }
    }
}

/// Everything a writer has to state. No `Default` impl: every field is a decision, and a
/// defaulted `actor` is the one mistake this table cannot survive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewApplicationEvent<'a> {
    pub application_id: &'a str,
    pub from_status: Option<ApplicationStatus>,
    pub to_status: ApplicationStatus,
    pub actor: Actor,
    pub cause: Option<Cause<'a>>,
    pub at: DateTime<Utc>,
    pub note: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    Written,
    AlreadyRecorded,
}

/// Records a transition. `INSERT … ON CONFLICT DO NOTHING`; `AlreadyRecorded` is a normal
/// outcome, not an error — same contract as `hunt::events::emit`.
///
/// **Takes a transaction, not a pool, on purpose.** The status UPDATE and this INSERT must
/// land together or not at all: a committed status change with no event breaks the fold
/// invariant, and a committed event with no status change is a lie about the tracker.
///
/// Callers open that transaction with `db::begin_write` (`BEGIN IMMEDIATE`), never
/// `pool.begin()` — a deferred transaction that upgrades read→write fails instantly under a
/// competing writer instead of waiting. See `src/db.rs`.
pub async fn record(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    event: NewApplicationEvent<'_>,
) -> anyhow::Result<Recorded> {
    let (cause_kind, cause_id) = event
        .cause
        .map(Cause::parts)
        .map_or((None, None), |(kind, id)| (Some(kind), Some(id)));

    let inserted = sqlx::query(
        "INSERT INTO application_events
             (id, application_id, at, created_at, from_status, to_status,
              actor, cause_kind, cause_id, note)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (application_id, cause_kind, cause_id, to_status) DO NOTHING",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(event.application_id)
    .bind(event.at.to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .bind(event.from_status.map(ApplicationStatus::as_str))
    .bind(event.to_status.as_str())
    .bind(event.actor.as_str())
    .bind(cause_kind)
    .bind(cause_id)
    .bind(event.note)
    .execute(&mut **tx)
    .await?
    .rows_affected();

    Ok(if inserted == 1 {
        Recorded::Written
    } else {
        Recorded::AlreadyRecorded
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackfillReport {
    pub proposal_events: u64,
    pub creation_events: u64,
    pub fallback_events: u64,
    pub already_recorded: u64,
}

impl BackfillReport {
    pub fn total_written(self) -> u64 {
        self.proposal_events + self.creation_events + self.fallback_events
    }

    fn count(&mut self, source: Source, recorded: Recorded) {
        match recorded {
            Recorded::Written => match source {
                Source::Proposal => self.proposal_events += 1,
                Source::Creation => self.creation_events += 1,
                Source::Fallback => self.fallback_events += 1,
            },
            Recorded::AlreadyRecorded => self.already_recorded += 1,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Source {
    Proposal,
    Creation,
    Fallback,
}

#[derive(Debug, sqlx::FromRow)]
struct ProposalRow {
    id: String,
    application_id: String,
    from_status: String,
    to_status: String,
    applied_automatically: i64,
    reviewed_at: Option<DateTime<Utc>>,
    accepted: Option<i64>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct ApplicationRow {
    id: String,
    status: String,
    applied_at: DateTime<Utc>,
    status_changed_at: DateTime<Utc>,
}

const USAGE: &str = "\
application-events — rebuild the append-only application status history

  application-events backfill
      Reconstruct application_events in one transaction from proposals and tracker timestamps.

  application-events verify
      Assert that every non-empty event history folds to its application's cached status.
";

/// Dispatch for the `application-events` server-binary subcommand.
pub async fn main(pool: &SqlitePool, args: &[String]) -> Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        print!("{USAGE}");
        return Ok(());
    };

    match command {
        "backfill" if args.len() == 1 => {
            let report = backfill(pool).await?;
            println!(
                "application-events backfill: {} written ({} proposal, {} creation, {} fallback); {} already recorded",
                report.total_written(),
                report.proposal_events,
                report.creation_events,
                report.fallback_events,
                report.already_recorded,
            );
            Ok(())
        }
        "backfill" => bail!("application-events backfill takes no arguments"),
        "verify" if args.len() == 1 => {
            let report = verify_invariant(pool).await?;
            println!(
                "application-events invariant: {} covered, {} exempt, {} mismatches",
                report.covered,
                report.exempt,
                report.mismatches.len(),
            );
            for mismatch in &report.mismatches {
                eprintln!(
                    "application {}: status={}, fold(events)={}",
                    mismatch.application_id,
                    mismatch.status.as_str(),
                    mismatch.folded_status.as_str(),
                );
            }
            if report.mismatches.is_empty() {
                Ok(())
            } else {
                bail!(
                    "{} application statuses disagree with their event fold",
                    report.mismatches.len()
                )
            }
        }
        "verify" => bail!("application-events verify takes no arguments"),
        other => {
            print!("{USAGE}");
            bail!("unknown application-events command: {other}")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldMismatch {
    pub application_id: String,
    pub status: ApplicationStatus,
    pub folded_status: ApplicationStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvariantReport {
    /// Applications with at least one event, whether they agree or not.
    pub covered: u64,
    /// Applications with no events. The Phase 10 contract explicitly exempts these.
    pub exempt: u64,
    pub mismatches: Vec<FoldMismatch>,
}

#[derive(Debug, sqlx::FromRow)]
struct FoldRow {
    id: String,
    status: String,
    folded_status: Option<String>,
}

/// Check `status == fold(events)` for every application with at least one event.
///
/// The correlated subquery is deliberately the literal fold definition in reverse: newest
/// `at`, then newest `created_at`, then greatest `id`. `LIMIT 1` is therefore exactly the
/// final `to_status` from the ascending definition, including deterministic ties.
pub async fn verify_invariant(pool: &SqlitePool) -> Result<InvariantReport> {
    let rows = sqlx::query_as::<_, FoldRow>(
        "SELECT a.id, a.status,
                (SELECT e.to_status
                   FROM application_events e
                  WHERE e.application_id = a.id
                  ORDER BY e.at DESC, e.created_at DESC, e.id DESC
                  LIMIT 1) AS folded_status
           FROM internship_applications a
          ORDER BY a.id ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut report = InvariantReport::default();
    for row in rows {
        let status = parse_status(&row.status, "application status", &row.id)?;
        let Some(folded_raw) = row.folded_status else {
            report.exempt += 1;
            continue;
        };
        report.covered += 1;
        let folded_status = parse_status(&folded_raw, "event to_status", &row.id)?;
        if status != folded_status {
            report.mismatches.push(FoldMismatch {
                application_id: row.id,
                status,
                folded_status,
            });
        }
    }

    Ok(report)
}

/// Reconstruct all provable history atomically.
pub async fn backfill(pool: &SqlitePool) -> Result<BackfillReport> {
    let mut tx = crate::db::begin_write(pool).await?;
    let report = backfill_in(&mut tx).await?;
    tx.commit().await?;
    Ok(report)
}

async fn backfill_in(tx: &mut Transaction<'_, Sqlite>) -> Result<BackfillReport> {
    let mut report = BackfillReport::default();

    // Source 1: these rows prove both statuses and who caused the transition.
    let proposals = sqlx::query_as::<_, ProposalRow>(
        "SELECT id, application_id, from_status, to_status, applied_automatically,
                reviewed_at, accepted, created_at
           FROM status_proposals
          WHERE accepted = 1 OR applied_automatically = 1
          ORDER BY created_at ASC, id ASC",
    )
    .fetch_all(&mut **tx)
    .await?;

    for proposal in proposals {
        let from_status =
            parse_status(&proposal.from_status, "proposal from_status", &proposal.id)?;
        let to_status = parse_status(&proposal.to_status, "proposal to_status", &proposal.id)?;
        let actor = if proposal.applied_automatically == 1 {
            Actor::Email
        } else {
            Actor::Manual
        };

        let recorded = record(
            tx,
            NewApplicationEvent {
                application_id: &proposal.application_id,
                from_status: Some(from_status),
                to_status,
                actor,
                cause: Some(Cause::StatusProposal(&proposal.id)),
                at: proposal.created_at,
                note: None,
            },
        )
        .await?;
        report.count(Source::Proposal, recorded);

        // Rejecting an auto-applied proposal is a real reverse transition. A pending or
        // accepted auto proposal has no compensation to reconstruct.
        if proposal.applied_automatically == 1 && proposal.accepted == Some(0) {
            let reviewed_at = proposal.reviewed_at.with_context(|| {
                format!(
                    "rejected auto-applied proposal {} has no reviewed_at",
                    proposal.id
                )
            })?;
            let recorded = record(
                tx,
                NewApplicationEvent {
                    application_id: &proposal.application_id,
                    from_status: Some(to_status),
                    to_status: from_status,
                    actor: Actor::Manual,
                    cause: Some(Cause::StatusProposal(&proposal.id)),
                    at: reviewed_at,
                    note: None,
                },
            )
            .await?;
            report.count(Source::Proposal, recorded);
        }
    }

    // Source 2: every application proves only that it began at `applied` at `applied_at`.
    let applications = sqlx::query_as::<_, ApplicationRow>(
        "SELECT id, status, applied_at, status_changed_at
           FROM internship_applications
          ORDER BY applied_at ASC, id ASC",
    )
    .fetch_all(&mut **tx)
    .await?;

    for application in &applications {
        let recorded = record(
            tx,
            NewApplicationEvent {
                application_id: &application.id,
                from_status: None,
                to_status: ApplicationStatus::Applied,
                actor: Actor::Unknown,
                cause: None,
                at: application.applied_at,
                note: None,
            },
        )
        .await?;
        report.count(Source::Creation, recorded);
    }

    // Source 3: add only what sources 1 and 2 do not already explain.
    for application in applications {
        let current = parse_status(&application.status, "application status", &application.id)?;
        if folded_status(tx, &application.id).await? == Some(current) {
            continue;
        }

        let recorded = record(
            tx,
            NewApplicationEvent {
                application_id: &application.id,
                from_status: None,
                to_status: current,
                actor: Actor::Unknown,
                cause: None,
                at: application.status_changed_at,
                note: None,
            },
        )
        .await?;
        report.count(Source::Fallback, recorded);
    }

    Ok(report)
}

async fn folded_status(
    tx: &mut Transaction<'_, Sqlite>,
    application_id: &str,
) -> Result<Option<ApplicationStatus>> {
    let status: Option<String> = sqlx::query_scalar(
        "SELECT to_status
           FROM application_events
          WHERE application_id = ?
          ORDER BY at DESC, created_at DESC, id DESC
          LIMIT 1",
    )
    .bind(application_id)
    .fetch_optional(&mut **tx)
    .await?;

    status
        .map(|value| parse_status(&value, "event to_status", application_id))
        .transpose()
}

fn parse_status(value: &str, field: &str, id: &str) -> Result<ApplicationStatus> {
    ApplicationStatus::parse(value)
        .ok_or_else(|| anyhow!("{field} on {id} has invalid value {value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    async fn insert_user(pool: &SqlitePool) {
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, created_at)
             VALUES ('user-1', 'person@example.com', NULL, '2026-01-01T00:00:00Z')",
        )
        .execute(pool)
        .await
        .expect("user");
    }

    async fn insert_application(
        pool: &SqlitePool,
        id: &str,
        status: ApplicationStatus,
        changed_at: &str,
    ) {
        sqlx::query(
            "INSERT INTO internship_applications
                (id, user_id, posting_id, company_name, title, url, snapshot_json, snapshot_at,
                 status, applied_at, status_changed_at, created_at, updated_at)
             VALUES (?1, 'user-1', NULL, 'Example Co', 'Engineer', 'https://example.com/job',
                     '{}', '2026-01-01T00:00:00Z', ?2, '2026-01-01T00:00:00Z', ?3,
                     '2026-01-01T00:00:00Z', ?3)",
        )
        .bind(id)
        .bind(status.as_str())
        .bind(changed_at)
        .execute(pool)
        .await
        .expect("application");
    }

    struct ProposalFixture<'a> {
        id: &'a str,
        application_id: &'a str,
        from: ApplicationStatus,
        to: ApplicationStatus,
        auto: bool,
        accepted: Option<bool>,
        created_at: &'a str,
        reviewed_at: Option<&'a str>,
    }

    async fn insert_proposal(pool: &SqlitePool, proposal: ProposalFixture<'_>) {
        let message_id = format!("message-{}", proposal.id);
        let verdict_id = format!("verdict-{}", proposal.id);
        sqlx::query(
            "INSERT INTO email_messages
                (id, user_id, gmail_message_id, created_at)
             VALUES (?1, 'user-1', ?2, '2026-01-01T00:00:00Z')",
        )
        .bind(&message_id)
        .bind(format!("gmail-{}", proposal.id))
        .execute(pool)
        .await
        .expect("message");
        sqlx::query(
            "INSERT INTO email_verdicts
                (id, message_id, category, matched_application_id, classifier, created_at)
             VALUES (?1, ?2, 'oa', ?3, 'rules', '2026-01-01T00:00:00Z')",
        )
        .bind(&verdict_id)
        .bind(&message_id)
        .bind(proposal.application_id)
        .execute(pool)
        .await
        .expect("verdict");
        sqlx::query(
            "INSERT INTO status_proposals
                (id, application_id, verdict_id, from_status, to_status,
                 applied_automatically, reviewed_at, accepted, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(proposal.id)
        .bind(proposal.application_id)
        .bind(&verdict_id)
        .bind(proposal.from.as_str())
        .bind(proposal.to.as_str())
        .bind(proposal.auto)
        .bind(proposal.reviewed_at)
        .bind(proposal.accepted)
        .bind(proposal.created_at)
        .execute(pool)
        .await
        .expect("proposal");
    }

    #[tokio::test]
    async fn causal_record_is_idempotent_but_uncaused_events_are_distinct() {
        let pool = pool().await;
        insert_user(&pool).await;
        insert_application(
            &pool,
            "application-1",
            ApplicationStatus::Oa,
            "2026-01-02T00:00:00Z",
        )
        .await;

        let at = DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let causal = || NewApplicationEvent {
            application_id: "application-1",
            from_status: Some(ApplicationStatus::Applied),
            to_status: ApplicationStatus::Oa,
            actor: Actor::Email,
            cause: Some(Cause::StatusProposal("proposal-1")),
            at,
            note: None,
        };
        let uncaused = || NewApplicationEvent {
            application_id: "application-1",
            from_status: None,
            to_status: ApplicationStatus::Applied,
            actor: Actor::Unknown,
            cause: None,
            at,
            note: None,
        };

        let mut tx = crate::db::begin_write(&pool).await.expect("transaction");
        assert_eq!(record(&mut tx, causal()).await.unwrap(), Recorded::Written);
        assert_eq!(
            record(&mut tx, causal()).await.unwrap(),
            Recorded::AlreadyRecorded
        );
        assert_eq!(
            record(&mut tx, uncaused()).await.unwrap(),
            Recorded::Written
        );
        assert_eq!(
            record(&mut tx, uncaused()).await.unwrap(),
            Recorded::Written
        );
        tx.commit().await.expect("commit");

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM application_events WHERE application_id = 'application-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn backfill_uses_proposals_then_creation_then_only_needed_fallbacks() {
        let pool = pool().await;
        insert_user(&pool).await;
        insert_application(
            &pool,
            "manual",
            ApplicationStatus::Oa,
            "2026-01-02T00:00:00Z",
        )
        .await;
        insert_application(
            &pool,
            "auto-undo",
            ApplicationStatus::Applied,
            "2026-01-03T00:00:00Z",
        )
        .await;
        insert_application(
            &pool,
            "fallback",
            ApplicationStatus::Rejected,
            "2026-01-05T00:00:00Z",
        )
        .await;
        insert_application(
            &pool,
            "creation-only",
            ApplicationStatus::Applied,
            "2026-01-01T00:00:00Z",
        )
        .await;

        insert_proposal(
            &pool,
            ProposalFixture {
                id: "proposal-manual",
                application_id: "manual",
                from: ApplicationStatus::Applied,
                to: ApplicationStatus::Oa,
                auto: false,
                accepted: Some(true),
                created_at: "2026-01-02T00:00:00Z",
                reviewed_at: Some("2026-01-02T00:01:00Z"),
            },
        )
        .await;
        insert_proposal(
            &pool,
            ProposalFixture {
                id: "proposal-auto-undo",
                application_id: "auto-undo",
                from: ApplicationStatus::Applied,
                to: ApplicationStatus::Oa,
                auto: true,
                accepted: Some(false),
                created_at: "2026-01-02T00:00:00Z",
                reviewed_at: Some("2026-01-03T00:00:00Z"),
            },
        )
        .await;

        let report = backfill(&pool).await.expect("backfill");
        assert_eq!(
            report,
            BackfillReport {
                proposal_events: 3,
                creation_events: 4,
                fallback_events: 1,
                already_recorded: 0,
            }
        );

        let rows: Vec<(String, String, Option<String>, String, String)> = sqlx::query_as(
            "SELECT application_id, actor, from_status, to_status,
                    COALESCE(cause_kind, '')
               FROM application_events
              ORDER BY application_id, at, created_at, id",
        )
        .fetch_all(&pool)
        .await
        .expect("events");

        assert!(rows.contains(&(
            "manual".into(),
            "manual".into(),
            Some("applied".into()),
            "oa".into(),
            "status_proposal".into(),
        )));
        assert!(rows.contains(&(
            "auto-undo".into(),
            "email".into(),
            Some("applied".into()),
            "oa".into(),
            "status_proposal".into(),
        )));
        assert!(rows.contains(&(
            "auto-undo".into(),
            "manual".into(),
            Some("oa".into()),
            "applied".into(),
            "status_proposal".into(),
        )));
        assert!(rows.contains(&(
            "fallback".into(),
            "unknown".into(),
            None,
            "rejected".into(),
            "".into(),
        )));
        assert_eq!(rows.len(), 8);
    }

    #[tokio::test]
    async fn every_enum_spelling_is_accepted_by_the_schema() {
        let pool = pool().await;
        insert_user(&pool).await;
        insert_application(
            &pool,
            "application-1",
            ApplicationStatus::Applied,
            "2026-01-01T00:00:00Z",
        )
        .await;

        let actors = [
            Actor::Email,
            Actor::Extension,
            Actor::Manual,
            Actor::Sweep,
            Actor::Unknown,
        ];
        let causes = [
            Cause::StatusProposal("p"),
            Cause::EmailVerdict("v"),
            Cause::HuntEvent("h"),
        ];
        let at = Utc::now();
        let mut tx = crate::db::begin_write(&pool).await.expect("transaction");

        for actor in actors {
            record(
                &mut tx,
                NewApplicationEvent {
                    application_id: "application-1",
                    from_status: None,
                    to_status: ApplicationStatus::Applied,
                    actor,
                    cause: None,
                    at,
                    note: None,
                },
            )
            .await
            .expect("actor accepted");
        }
        for cause in causes {
            record(
                &mut tx,
                NewApplicationEvent {
                    application_id: "application-1",
                    from_status: None,
                    to_status: ApplicationStatus::Oa,
                    actor: Actor::Unknown,
                    cause: Some(cause),
                    at,
                    note: None,
                },
            )
            .await
            .expect("cause accepted");
        }
        tx.commit().await.expect("commit");
    }

    async fn insert_raw_event(
        pool: &SqlitePool,
        id: &str,
        application_id: &str,
        at: &str,
        created_at: &str,
        to_status: ApplicationStatus,
    ) {
        sqlx::query(
            "INSERT INTO application_events
                (id, application_id, at, created_at, from_status, to_status, actor,
                 cause_kind, cause_id, note)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, 'unknown', NULL, NULL, NULL)",
        )
        .bind(id)
        .bind(application_id)
        .bind(at)
        .bind(created_at)
        .bind(to_status.as_str())
        .execute(pool)
        .await
        .expect("event");
    }

    #[tokio::test]
    async fn invariant_uses_every_tie_breaker_reports_mismatches_and_exempts_empty_histories() {
        let pool = pool().await;
        insert_user(&pool).await;
        insert_application(
            &pool,
            "ordered",
            ApplicationStatus::Rejected,
            "2026-01-02T00:00:00Z",
        )
        .await;
        insert_application(
            &pool,
            "mismatch",
            ApplicationStatus::Applied,
            "2026-01-02T00:00:00Z",
        )
        .await;
        insert_application(
            &pool,
            "empty",
            ApplicationStatus::Applied,
            "2026-01-01T00:00:00Z",
        )
        .await;

        // `at` beats created_at: this was written later, but describes an earlier transition.
        insert_raw_event(
            &pool,
            "z-earlier-at",
            "ordered",
            "2026-01-01T00:00:00Z",
            "2026-01-09T00:00:00Z",
            ApplicationStatus::Offer,
        )
        .await;
        // At the greatest `at`, created_at breaks the first tie.
        insert_raw_event(
            &pool,
            "z-earlier-created",
            "ordered",
            "2026-01-02T00:00:00Z",
            "2026-01-01T00:00:00Z",
            ApplicationStatus::Oa,
        )
        .await;
        // With both timestamps tied, lexical id order is the final deterministic tie-breaker.
        insert_raw_event(
            &pool,
            "a-final-tie",
            "ordered",
            "2026-01-02T00:00:00Z",
            "2026-01-02T00:00:00Z",
            ApplicationStatus::Interview,
        )
        .await;
        insert_raw_event(
            &pool,
            "z-final-tie",
            "ordered",
            "2026-01-02T00:00:00Z",
            "2026-01-02T00:00:00Z",
            ApplicationStatus::Rejected,
        )
        .await;
        insert_raw_event(
            &pool,
            "mismatch-event",
            "mismatch",
            "2026-01-02T00:00:00Z",
            "2026-01-02T00:00:00Z",
            ApplicationStatus::Oa,
        )
        .await;

        let report = verify_invariant(&pool).await.expect("invariant report");
        assert_eq!(report.covered, 2);
        assert_eq!(report.exempt, 1);
        assert_eq!(
            report.mismatches,
            vec![FoldMismatch {
                application_id: "mismatch".into(),
                status: ApplicationStatus::Applied,
                folded_status: ApplicationStatus::Oa,
            }]
        );
    }
}
