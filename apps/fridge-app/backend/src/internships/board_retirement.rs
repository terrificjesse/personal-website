//! Which board slugs have been gone long enough to retire from the directory.
//!
//! # The finding this exists to keep
//!
//! Every collection run prints something like
//!
//! ```text
//! internships: 6 Greenhouse board(s) 404'd and should be retired: 10xgenomics, ...
//! ```
//!
//! and until migration `0032` nothing anywhere consumed it. The database could not tell a
//! deleted board from an empty one — both were a `completed` scope with no ids — so the only
//! record of a dead board was a line of stdout. `source_run_scopes.gone` is where the finding
//! now lands, and this module is what reads it back.
//!
//! # Why this is a query and not an action
//!
//! Retiring a slug edits `data/internships/board-slugs.json`, which is compiled into the binary
//! and is what the adapters poll. Removing one is not free: `sources::BoardDirectory`'s own doc
//! warns that a pruned slug makes every posting on it stop advancing, and under scopes it also
//! means that board never produces a completed scope again. So the cost of retiring a *live*
//! board by mistake is rows that linger invisibly.
//!
//! **One 404 is not evidence.** On 2026-09-03 this project inferred a rate from a single
//! observation and propagated it through five documents before one query retracted it
//! (`92ef2f4`); doing the same thing here would spend the mistake on a data file. A 404 can be
//! a deploy, a rename in flight, a CDN with an opinion. What makes it evidence is repetition:
//! [`RETIREMENT_RUNS`] consecutive verdicts, all of them gone, with no completed-and-populated
//! run in between.
//!
//! And because `0032` starts the clock at zero, the honest answer for the first few runs is
//! "not yet, and here is how far off it is" — which is what [`report`] prints rather than
//! answering from a history too short to support it.

use anyhow::{Result, bail};
use sqlx::SqlitePool;

/// Consecutive gone verdicts before a slug is a retirement candidate.
///
/// Three, matching `expiry::DEFAULT_MISS_THRESHOLD` in spirit but not in mechanism — this counts
/// verdicts about the board's *existence*, that one counts a posting's absence. It must be
/// greater than one: at one, this is the single-observation inference the module doc refuses.
pub const RETIREMENT_RUNS: i64 = 3;

const USAGE: &str = "\
usage: boards retire [--runs N]

Lists board slugs whose last N verdicts were all 404. Reports only; retiring a slug is an edit
to data/internships/board-slugs.json and stays a human's decision.
";

/// One slug's standing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub source: String,
    pub scope: String,
    /// Verdicts examined — always exactly the window size, since a scope with fewer is not a
    /// candidate at all.
    pub considered: i64,
    pub oldest_considered_at: String,
}

/// How much evidence exists at all, so a short history reads as short rather than as clean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub source: String,
    /// Runs that recorded per-scope verdicts *since `0032`* — the ones that could have set
    /// `gone`. Runs before it are excluded: their scopes all read `gone = 0` because the column
    /// did not exist, not because the boards were alive.
    pub runs_with_gone_data: i64,
}

/// Slugs whose most recent `runs` verdicts were every one of them a 404.
///
/// A scope with fewer than `runs` verdicts is never a candidate: "all of the two runs we have"
/// is the short-history version of the same inference the module doc refuses.
pub async fn candidates(pool: &SqlitePool, runs: i64) -> Result<Vec<Candidate>> {
    if runs < 2 {
        bail!("a retirement window of {runs} run(s) is a single-observation inference; use 2+");
    }

    let rows: Vec<(String, String, i64, String)> = sqlx::query_as(
        // The window is per (source, scope) and ordered by run time, so "the last N verdicts"
        // means the last N times this board was actually reached — not the last N runs, which
        // would let a board the budget skipped look like a board that answered.
        "WITH verdicts AS (
             SELECT r.source        AS source,
                    s.scope         AS scope,
                    s.gone          AS gone,
                    r.started_at    AS started_at,
                    row_number() OVER (
                        PARTITION BY r.source, s.scope ORDER BY r.started_at DESC
                    ) AS recency
             FROM source_run_scopes s
             JOIN source_runs r ON r.id = s.source_run_id
         )
         SELECT source, scope, count(*) AS considered, min(started_at) AS oldest
         FROM verdicts
         WHERE recency <= ?1
         GROUP BY source, scope
         HAVING count(*) = ?1 AND sum(gone) = ?1
         ORDER BY source, scope",
    )
    .bind(runs)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(source, scope, considered, oldest)| Candidate {
            source,
            scope,
            considered,
            oldest_considered_at: oldest,
        })
        .collect())
}

/// How many runs per source could have recorded a gone verdict at all.
///
/// Reported beside the candidates so an empty list is readable. "No candidates" after two runs
/// means the window is not full; after twenty it means the directory is clean, and those are
/// different facts that would otherwise print identically.
pub async fn windows(pool: &SqlitePool) -> Result<Vec<Window>> {
    // A run "could have recorded gone" if any of its scopes did, or if it ran after the first
    // run that did. Approximated by the simpler and more conservative test: runs at or after
    // the earliest run holding a gone flag. Before any board has 404'd since 0032 this is 0,
    // which is exactly the "we do not know yet" the caller needs to hear.
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT r.source, count(DISTINCT r.id)
         FROM source_runs r
         JOIN source_run_scopes s ON s.source_run_id = r.id
         WHERE r.started_at >= COALESCE(
             (SELECT min(r2.started_at)
              FROM source_runs r2
              JOIN source_run_scopes s2 ON s2.source_run_id = r2.id
              WHERE s2.gone = 1),
             '9999')
         GROUP BY r.source
         ORDER BY r.source",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(source, runs_with_gone_data)| Window {
            source,
            runs_with_gone_data,
        })
        .collect())
}

pub async fn main(pool: &SqlitePool, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("retire") => {}
        _ => {
            print!("{USAGE}");
            return Ok(());
        }
    }

    let runs = match &args[1..] {
        [] => RETIREMENT_RUNS,
        [flag, value] if flag == "--runs" => value
            .parse()
            .map_err(|_| anyhow::anyhow!("--runs takes a number, got {value:?}"))?,
        _ => {
            print!("{USAGE}");
            bail!("unrecognized arguments to `boards retire`");
        }
    };

    let windows = windows(pool).await?;
    let candidates = candidates(pool, runs).await?;

    if windows.is_empty() {
        println!(
            "boards retire: no run has recorded a 404 since migration 0032, so there is no \
             evidence to judge yet. Runs before 0032 read gone=0 because the column did not \
             exist, not because their boards were alive."
        );
        return Ok(());
    }

    for window in &windows {
        println!(
            "boards retire: {} — {} run(s) able to record a 404; {} needed for a verdict",
            window.source, window.runs_with_gone_data, runs
        );
    }

    if candidates.is_empty() {
        println!("boards retire: no slug has 404'd on {runs} consecutive verdicts.");
        return Ok(());
    }

    println!(
        "boards retire: {} candidate(s) — remove from data/internships/board-slugs.json by hand, \
         and say in the commit which runs they were absent on.",
        candidates.len()
    );
    for candidate in &candidates {
        println!(
            "  {}/{}: gone on the last {} verdicts, oldest {}",
            candidate.source, candidate.scope, candidate.considered, candidate.oldest_considered_at
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internships::expiry::settle_source_run;
    use crate::internships::models::{ScopeRun, SourceOutcome};
    use crate::internships::expiry::SourceRunResult;
    use chrono::{Duration, Utc};

    async fn run(
        pool: &SqlitePool,
        ordinal: i64,
        scopes: Vec<ScopeRun>,
        outcome: SourceOutcome,
    ) -> String {
        let run_id = format!("run-{ordinal}");
        let at = Utc::now() + Duration::minutes(ordinal);
        sqlx::query("INSERT INTO collection_runs (id, started_at, trigger) VALUES (?1, ?2, 'manual')")
            .bind(&run_id)
            .bind(at.to_rfc3339())
            .execute(pool)
            .await
            .expect("collection run");

        let result = SourceRunResult {
            source: "greenhouse".to_string(),
            outcome,
            fetched: 1,
            accepted: 0,
            filtered: 0,
            rejected: 0,
            seen_external_ids: Vec::new(),
            scopes,
            error: None,
        };
        settle_source_run(pool, &run_id, &format!("sr-{ordinal}"), &result, at)
            .await
            .expect("settle");
        run_id
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_slug_gone_on_every_run_in_the_window_is_a_candidate(pool: SqlitePool) {
        for ordinal in 1..=3 {
            run(
                &pool,
                ordinal,
                vec![
                    ScopeRun::gone("deadboard"),
                    ScopeRun::completed("liveboard", vec!["1".to_string()]),
                ],
                SourceOutcome::Success,
            )
            .await;
        }

        let found = candidates(&pool, 3).await.expect("query");
        assert_eq!(found.len(), 1, "only the dead board qualifies: {found:?}");
        assert_eq!(found[0].scope, "deadboard");
        assert_eq!(found[0].considered, 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_short_history_yields_nothing_rather_than_a_verdict(pool: SqlitePool) {
        // Two gone verdicts out of a three-run window is exactly the inference this module
        // exists to refuse. It must read as "not yet", not as "all of them".
        for ordinal in 1..=2 {
            run(
                &pool,
                ordinal,
                vec![ScopeRun::gone("deadboard")],
                SourceOutcome::Success,
            )
            .await;
        }

        assert!(candidates(&pool, 3).await.expect("query").is_empty());
        assert_eq!(candidates(&pool, 2).await.expect("query").len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn one_answer_in_the_window_disqualifies_the_slug(pool: SqlitePool) {
        // The board came back. A 404 that is a deploy, a rename in flight, or a CDN with an
        // opinion looks exactly like a dead board until it answers again — which is the whole
        // reason the window is longer than one.
        run(&pool, 1, vec![ScopeRun::gone("flaky")], SourceOutcome::Success).await;
        run(
            &pool,
            2,
            vec![ScopeRun::completed("flaky", vec!["1".to_string()])],
            SourceOutcome::Success,
        )
        .await;
        run(&pool, 3, vec![ScopeRun::gone("flaky")], SourceOutcome::Success).await;

        assert!(
            candidates(&pool, 3).await.expect("query").is_empty(),
            "two gone verdicts either side of an answer are not three consecutive ones"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_read_is_not_a_gone_verdict(pool: SqlitePool) {
        // The distinction the whole column exists for. Unreachable proves nothing; 404 proves
        // the board offers nothing.
        for ordinal in 1..=3 {
            run(
                &pool,
                ordinal,
                vec![ScopeRun::failed("unreachable", "HTTP 500")],
                SourceOutcome::Partial,
            )
            .await;
        }

        assert!(candidates(&pool, 3).await.expect("query").is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_window_of_one_is_refused_rather_than_answered(pool: SqlitePool) {
        run(&pool, 1, vec![ScopeRun::gone("deadboard")], SourceOutcome::Success).await;
        assert!(candidates(&pool, 1).await.is_err());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn no_gone_verdict_anywhere_reports_no_window_rather_than_a_clean_directory(
        pool: SqlitePool,
    ) {
        // "No candidates" after two runs and "no candidates" after twenty are different facts.
        run(
            &pool,
            1,
            vec![ScopeRun::completed("liveboard", vec!["1".to_string()])],
            SourceOutcome::Success,
        )
        .await;

        assert!(windows(&pool).await.expect("query").is_empty());
    }
}
