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

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use super::models::SourceOutcome;

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
    pub seen_external_ids: Vec<String>,
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
    pub fn may_expire(self) -> bool {
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

    match previous_fetched {
        Some(previous) if fetched == 0 && previous > 0 => {
            ExpiryEligibility::SuspiciousZero {
                previous_fetched: previous,
            }
        }
        _ => ExpiryEligibility::Eligible,
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

    let eligibility = eligibility(result.outcome, result.fetched, previous_fetched);

    let outcome_text = match result.outcome {
        SourceOutcome::Success => "success",
        SourceOutcome::Partial => "partial",
        SourceOutcome::Failed => "failed",
        SourceOutcome::Skipped => "skipped",
    };

    let mut tx = pool.begin().await?;

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

    if eligibility.may_expire() {
        // Everything this source carries starts the run as a miss; the sightings actually
        // observed are then reset to zero. Written in this order — blanket increment, then
        // targeted reset — so a sighting cannot be skipped by an id that failed to match.
        //
        // The reset is the important half. A sighting seen this run must return to 0 rather
        // than merely stop climbing, or a posting that flickers in and out across many runs
        // eventually crosses the threshold while never having been absent twice running.
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
    }

    tx.commit().await?;

    match eligibility {
        ExpiryEligibility::Eligible => {}
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
}
