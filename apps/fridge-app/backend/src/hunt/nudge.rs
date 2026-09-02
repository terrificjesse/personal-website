//! Follow-up nudges (Phase 11e): an application nobody has answered, for long enough that it
//! is worth chasing.
//!
//! # A third producer, not a second pipeline
//!
//! This writes `hunt_events` rows exactly like the posting producer and the email producer, and
//! everything downstream — the poll, the ack, the notification, the popup — is untouched. If a
//! future alert needed its own table or its own poll, something would have gone wrong with the
//! shape 8e was built around.
//!
//! # What it does NOT write
//!
//! **Nothing in `application_events`.** A nudge is not a status transition: nothing about the
//! application changed, and nobody did anything to it. The `Actor::Sweep` variant sitting
//! unconstructed in that module looks like it belongs here and does not — it is reserved for
//! dead-application detection, and "dead" is derived rather than stored, so it may stay
//! unconstructed for a while yet. An event here would put a fabricated transition in the log
//! that the fold invariant would then cheerfully confirm.
//!
//! # The key
//!
//! `subject_id` is `"{application_id}:{threshold_days}"`, because `hunt_events` has
//! `UNIQUE (kind, subject_id)` and a nudge at 14 days and one at 30 are different events.
//! Keyed on the application alone you get one nudge ever; keyed on anything that varies per
//! sweep you get one per sweep, which is how a channel gets muted — taking the OA alerts with
//! it. Both failures are silent.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use sqlx::SqlitePool;

use crate::hunt::events::{self, EventKind, NewHuntEvent};
use crate::internships::application_events::HAS_RESPONDED;

/// Days of silence after which an application is worth chasing, when `HUNT_NUDGE_DAYS` is unset.
///
/// Two thresholds rather than one: a fortnight is "they have probably not looked at it", a
/// month is "this is over unless you do something". They are different messages and the second
/// should not be swallowed by having already sent the first.
const DEFAULT_THRESHOLDS: &[i64] = &[14, 30];

/// How often the sweep runs when `HUNT_NUDGE_INTERVAL_SECS` is unset.
///
/// Six hours, not minutes: the thing being measured moves in days, and the dedup key means a
/// faster sweep would only re-find rows it has already emitted for.
const DEFAULT_INTERVAL_SECS: u64 = 6 * 60 * 60;

/// One application that has gone unanswered past a threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stale {
    pub application_id: String,
    pub user_id: String,
    pub company_name: String,
    pub title: String,
    pub url: String,
    pub days: i64,
    pub threshold: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NudgeReport {
    /// Applications past a threshold, whether or not they had already been nudged for it.
    pub found: u64,
    /// Rows actually written. The difference is the dedup key doing its job.
    pub raised: u64,
}

/// Applications with no response, not closed, applied at least `threshold` days ago.
///
/// "No response" is [`HAS_RESPONDED`], negated — the definition lives there so this and the
/// analytics endpoint cannot drift apart.
pub async fn stale(pool: &SqlitePool, threshold: i64, now: DateTime<Utc>) -> Result<Vec<Stale>> {
    let cutoff = (now - Duration::days(threshold)).to_rfc3339();

    let sql = format!(
        "SELECT a.id, a.user_id, a.company_name, a.title, a.url, a.applied_at
           FROM internship_applications a
          WHERE a.applied_at <= ?1
            -- Never chase something that is already over. A rejection is a response and is
            -- caught below too; this also covers an application created as terminal, whose
            -- only event is its creation.
            AND a.status NOT IN ('offer', 'rejected')
            AND NOT {HAS_RESPONDED}
          ORDER BY a.applied_at ASC"
    );

    let rows: Vec<(String, String, String, String, String, String)> =
        sqlx::query_as(&sql).bind(&cutoff).fetch_all(pool).await?;

    Ok(rows
        .into_iter()
        .map(|(id, user_id, company_name, title, url, applied_at)| {
            let days = DateTime::parse_from_rfc3339(&applied_at)
                .map(|applied| (now - applied.with_timezone(&Utc)).num_days())
                .unwrap_or(threshold);
            Stale {
                application_id: id,
                user_id,
                company_name,
                title,
                url,
                days,
                threshold,
            }
        })
        .collect())
}

/// What the notification says. Rendered here, like every other producer's.
pub fn nudge_event(stale: &Stale) -> NewHuntEvent {
    NewHuntEvent {
        kind: EventKind::Nudge,
        // Always set: a nudge is about one person's application and must never be visible to
        // every signed-in user the way a posting alert is.
        user_id: Some(stale.user_id.clone()),
        subject_id: format!("{}:{}", stale.application_id, stale.threshold),
        title: format!("No reply from {} · {} days", stale.company_name, stale.days),
        body: truncate(&stale.title, 140),
        url: Some(stale.url.clone()),
        payload: json!({
            "application_id": stale.application_id,
            "company_name": stale.company_name,
            "title": stale.title,
            "days_since_applied": stale.days,
            "threshold_days": stale.threshold,
        }),
    }
}

/// One pass over every configured threshold.
pub async fn sweep(pool: &SqlitePool, now: DateTime<Utc>) -> Result<NudgeReport> {
    let mut report = NudgeReport::default();

    for threshold in thresholds() {
        for candidate in stale(pool, threshold, now).await? {
            report.found += 1;
            if events::emit(pool, &nudge_event(&candidate), now).await? {
                report.raised += 1;
            }
        }
    }

    Ok(report)
}

/// The background worker. Same shape as the blog watcher and the inbox sync: cadence from an
/// env var, `0` disables, spawned rather than awaited, and **never called from a request
/// handler**.
pub fn spawn(pool: SqlitePool) {
    let interval = match std::env::var("HUNT_NUDGE_INTERVAL_SECS") {
        Err(_) => Some(std::time::Duration::from_secs(DEFAULT_INTERVAL_SECS)),
        Ok(value) => match value.trim().parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(std::time::Duration::from_secs(secs)),
            Err(_) => {
                eprintln!(
                    "hunt: HUNT_NUDGE_INTERVAL_SECS={value:?} is not a number — nudges disabled; \
                     use 0 to disable deliberately"
                );
                None
            }
        },
    };

    let Some(interval) = interval else {
        println!("hunt: follow-up nudges disabled");
        return;
    };

    println!(
        "hunt: sweeping for unanswered applications every {}s at {:?} days",
        interval.as_secs(),
        thresholds()
    );

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            match sweep(&pool, Utc::now()).await {
                Ok(report) if report.raised > 0 => {
                    println!(
                        "hunt: {} unanswered applications, {} nudges raised",
                        report.found, report.raised
                    );
                }
                Ok(_) => {}
                // A failed sweep is a worse day, not lost data — same posture as the other
                // producers. It is logged and the next tick tries again.
                Err(err) => eprintln!("hunt: nudge sweep failed: {err:?}"),
            }
        }
    });
}

/// Thresholds from `HUNT_NUDGE_DAYS`, e.g. `"14,30"`. Unparseable entries are dropped with a
/// line rather than taking the whole list down.
fn thresholds() -> Vec<i64> {
    let Ok(raw) = std::env::var("HUNT_NUDGE_DAYS") else {
        return DEFAULT_THRESHOLDS.to_vec();
    };

    let parsed: Vec<i64> = raw
        .split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            match entry.parse::<i64>() {
                Ok(days) if days > 0 => Some(days),
                _ => {
                    eprintln!("hunt: HUNT_NUDGE_DAYS entry {entry:?} is not a positive number");
                    None
                }
            }
        })
        .collect();

    if parsed.is_empty() {
        DEFAULT_THRESHOLDS.to_vec()
    } else {
        parsed
    }
}

/// Cuts on a character boundary, like the posting producer's.
fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let kept: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internships::application_events::{self, Actor, NewApplicationEvent};
    use crate::internships::models::ApplicationStatus;
    use uuid::Uuid;

    async fn pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("nudge-{}.db", Uuid::new_v4()));
        crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("migrations")
    }

    async fn user(pool: &SqlitePool) -> String {
        let id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO users (id, email, created_at) VALUES (?1, ?2, ?3)")
            .bind(&id)
            .bind(format!("{id}@example.com"))
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await
            .unwrap();
        id
    }

    /// An application applied for `days_ago` days ago, with its creation event.
    async fn application(
        pool: &SqlitePool,
        user_id: &str,
        status: &str,
        days_ago: i64,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let applied = (Utc::now() - Duration::days(days_ago)).to_rfc3339();
        sqlx::query(
            "INSERT INTO internship_applications
                (id, user_id, company_name, title, url, snapshot_json, snapshot_at,
                 status, applied_at, status_changed_at, created_at, updated_at)
             VALUES (?1, ?2, 'Jump Trading', 'SWE Intern', 'https://example.com/j',
                     '{}', ?3, ?4, ?3, ?3, ?3, ?3)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(&applied)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();

        record(pool, &id, None, ApplicationStatus::Applied, days_ago).await;
        id
    }

    async fn record(
        pool: &SqlitePool,
        application_id: &str,
        from: Option<ApplicationStatus>,
        to: ApplicationStatus,
        days_ago: i64,
    ) {
        let mut tx = crate::db::begin_write(pool).await.unwrap();
        application_events::record(
            &mut tx,
            NewApplicationEvent {
                application_id,
                from_status: from,
                to_status: to,
                actor: Actor::Manual,
                cause: None,
                at: Utc::now() - Duration::days(days_ago),
                note: None,
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }

    async fn kinds(pool: &SqlitePool) -> Vec<(String, String)> {
        sqlx::query_as("SELECT kind, subject_id FROM hunt_events ORDER BY subject_id")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    /// The behaviour the dedup key exists for, in both directions.
    #[tokio::test]
    async fn a_silent_application_is_nudged_once_and_not_again() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let app = application(&pool, &user_id, "applied", 20).await;

        let first = sweep(&pool, Utc::now()).await.unwrap();
        assert_eq!(first.raised, 1, "one threshold has passed (14), not the other (30)");

        let second = sweep(&pool, Utc::now()).await.unwrap();
        assert_eq!(second.raised, 0, "the same silence must not re-notify");
        assert_eq!(second.found, 1, "it is still stale — it has simply been said once");

        assert_eq!(kinds(&pool).await, vec![("nudge".to_string(), format!("{app}:14"))]);
    }

    /// A second threshold is a second event, which is the whole reason the key carries it.
    #[tokio::test]
    async fn a_longer_silence_earns_a_second_distinct_nudge() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let app = application(&pool, &user_id, "applied", 45).await;

        let report = sweep(&pool, Utc::now()).await.unwrap();
        assert_eq!(report.raised, 2);
        assert_eq!(
            kinds(&pool).await,
            vec![
                ("nudge".to_string(), format!("{app}:14")),
                ("nudge".to_string(), format!("{app}:30")),
            ]
        );
    }

    #[tokio::test]
    async fn an_application_that_got_a_reply_is_never_nudged() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let app = application(&pool, &user_id, "oa", 40).await;
        record(&pool, &app, Some(ApplicationStatus::Applied), ApplicationStatus::Oa, 35).await;

        assert_eq!(sweep(&pool, Utc::now()).await.unwrap(), NudgeReport::default());
    }

    /// A backfilled transition has a NULL `from_status` exactly like a creation event, and it
    /// still means somebody answered. Testing the null instead of the ordering would score
    /// every reconstructed response as silence and chase applications that are alive.
    #[tokio::test]
    async fn a_response_whose_provenance_is_unknown_still_counts_as_a_response() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let app = application(&pool, &user_id, "interview", 40).await;
        record(&pool, &app, None, ApplicationStatus::Interview, 30).await;

        assert_eq!(sweep(&pool, Utc::now()).await.unwrap().raised, 0);
    }

    #[tokio::test]
    async fn a_closed_application_is_never_chased() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        for status in ["offer", "rejected"] {
            application(&pool, &user_id, status, 60).await;
        }

        assert_eq!(sweep(&pool, Utc::now()).await.unwrap().raised, 0);
    }

    #[tokio::test]
    async fn a_recent_application_is_left_alone() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        application(&pool, &user_id, "applied", 3).await;

        assert_eq!(sweep(&pool, Utc::now()).await.unwrap().found, 0);
    }

    /// A nudge is private. A posting alert with a NULL user_id is visible to everyone signed
    /// in, and this must never be written that way.
    #[tokio::test]
    async fn a_nudge_names_its_owner() {
        let stale = Stale {
            application_id: "app-1".into(),
            user_id: "user-1".into(),
            company_name: "Roblox".into(),
            title: "Software Engineer Intern".into(),
            url: "https://example.com/j".into(),
            days: 21,
            threshold: 14,
        };
        let event = nudge_event(&stale);

        assert_eq!(event.user_id.as_deref(), Some("user-1"));
        assert_eq!(event.subject_id, "app-1:14");
        assert_eq!(event.kind, EventKind::Nudge);
        assert!(event.title.contains("Roblox") && event.title.contains("21"));
    }

}
