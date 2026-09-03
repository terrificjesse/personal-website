//! Warning before a due date passes (Phase 11f).
//!
//! The fourth producer on `hunt_events`, and the one whose whole justification is that missing
//! it costs the opportunity. It reads `application_deadlines` — rows extracted from mail by
//! `inbox::due_dates` — and raises an alert some hours ahead of each one.
//!
//! # What it does not do
//!
//! It never advances a status, never writes `application_events`, and never touches a mailbox.
//! A deadline is a fact parsed out of untrusted text by patterns that will sometimes be wrong;
//! the most it may do is tell you about it.
//!
//! # A deadline already past raises nothing
//!
//! If the backend was down for three days and a due date went by, this stays quiet. A "you
//! missed it" notification is noise arriving exactly when it can no longer help, and the
//! channel's value is that everything in it is actionable.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use sqlx::SqlitePool;

use crate::hunt::events::{self, EventKind, NewHuntEvent};

/// Hours before a due date at which to warn, when `HUNT_DEADLINE_LEAD_HOURS` is unset.
///
/// Three days is "plan for this"; one day is "do it now". They say different things, and the
/// key below is what stops the second being swallowed by the first having already fired.
const DEFAULT_LEADS: &[i64] = &[72, 24];

/// Deadlines move in hours, so this sweeps far more often than the follow-up one.
const DEFAULT_INTERVAL_SECS: u64 = 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub deadline_id: String,
    pub user_id: String,
    pub application_id: Option<String>,
    /// The application's company when it matched, else the subject of the email it came from.
    /// Rule 8: an unmatched deadline is still a deadline.
    pub label: String,
    pub url: Option<String>,
    pub due_at: DateTime<Utc>,
    pub source_text: String,
    pub lead_hours: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeadlineReport {
    pub found: u64,
    pub raised: u64,
}

/// Deadlines inside `lead_hours` of falling due, and not yet past.
/// One row of the join below, named because a seven-field tuple is not a type anyone can read.
#[derive(sqlx::FromRow)]
struct DueRow {
    id: String,
    user_id: String,
    application_id: Option<String>,
    company_name: Option<String>,
    url: Option<String>,
    due_at: String,
    /// The email's subject when it still exists, else the phrase the date was parsed from.
    label: String,
}

pub async fn approaching(pool: &SqlitePool, lead_hours: i64, now: DateTime<Utc>) -> Result<Vec<Due>> {
    let rows: Vec<DueRow> =
        sqlx::query_as(
            "SELECT d.id, d.user_id, d.application_id, a.company_name, a.url, d.due_at,
                    COALESCE(m.subject, d.source_text) AS label
               FROM application_deadlines d
               -- LEFT, both of them: rule 8 says an unmatched deadline is still a deadline,
               -- and a hand-deleted message must not take its deadline out of the warning.
               LEFT JOIN internship_applications a ON a.id = d.application_id
               LEFT JOIN email_messages m ON m.id = d.message_id
              WHERE d.due_at > ?1 AND d.due_at <= ?2
              ORDER BY d.due_at ASC",
        )
        .bind(now.to_rfc3339())
        .bind((now + Duration::hours(lead_hours)).to_rfc3339())
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let due_at = DateTime::parse_from_rfc3339(&row.due_at)
                .ok()?
                .with_timezone(&Utc);
            Some(Due {
                deadline_id: row.id,
                user_id: row.user_id,
                // The company when the matcher found one, else whatever the email was called.
                label: row.company_name.unwrap_or_else(|| row.label.clone()),
                application_id: row.application_id,
                url: row.url,
                due_at,
                source_text: row.label,
                lead_hours,
            })
        })
        .collect())
}

/// What the notification says.
pub fn deadline_event(due: &Due, now: DateTime<Utc>) -> NewHuntEvent {
    let hours_left = (due.due_at - now).num_hours().max(0);
    NewHuntEvent {
        kind: EventKind::Deadline,
        // Always set. A deadline is about one person's application.
        user_id: Some(due.user_id.clone()),
        // One alert per deadline PER LEAD TIME, made structural by `UNIQUE (kind, subject_id)`:
        // the 24-hour warning is a different event from the 72-hour one, and neither may repeat.
        subject_id: format!("{}:{}", due.deadline_id, due.lead_hours),
        title: format!("{} · due in {hours_left}h", due.label),
        body: format!(
            "From “{}” — check the email before relying on this.",
            truncate(&due.source_text, 90)
        ),
        url: due.url.clone(),
        payload: json!({
            "deadline_id": due.deadline_id,
            "application_id": due.application_id,
            "due_at": due.due_at.to_rfc3339(),
            "lead_hours": due.lead_hours,
            "source_text": due.source_text,
        }),
    }
}

/// One pass over every configured lead time.
pub async fn sweep(pool: &SqlitePool, now: DateTime<Utc>) -> Result<DeadlineReport> {
    let mut report = DeadlineReport::default();

    for lead in leads() {
        for due in approaching(pool, lead, now).await? {
            report.found += 1;
            if events::emit(pool, &deadline_event(&due, now), now).await? {
                report.raised += 1;
            }
        }
    }

    Ok(report)
}

/// The background worker. Cadence from an env var, `0` disables, spawned rather than awaited,
/// never called from a request handler.
pub fn spawn(pool: SqlitePool) {
    let interval = match std::env::var("HUNT_DEADLINE_INTERVAL_SECS") {
        Err(_) => Some(std::time::Duration::from_secs(DEFAULT_INTERVAL_SECS)),
        Ok(value) => match value.trim().parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(std::time::Duration::from_secs(secs)),
            Err(_) => {
                eprintln!(
                    "hunt: HUNT_DEADLINE_INTERVAL_SECS={value:?} is not a number — deadline \
                     warnings disabled; use 0 to disable deliberately"
                );
                None
            }
        },
    };

    let Some(interval) = interval else {
        println!("hunt: deadline warnings disabled");
        return;
    };

    println!(
        "hunt: warning {:?} hours before a deadline, checked every {}s",
        leads(),
        interval.as_secs()
    );

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            match sweep(&pool, Utc::now()).await {
                Ok(report) if report.raised > 0 => {
                    println!("hunt: {} deadline warnings raised", report.raised);
                }
                Ok(_) => {}
                Err(err) => eprintln!("hunt: deadline sweep failed: {err:?}"),
            }
        }
    });
}

fn leads() -> Vec<i64> {
    let Ok(raw) = std::env::var("HUNT_DEADLINE_LEAD_HOURS") else {
        return DEFAULT_LEADS.to_vec();
    };
    let parsed: Vec<i64> = raw
        .split(',')
        .filter_map(|entry| match entry.trim().parse::<i64>() {
            Ok(hours) if hours > 0 => Some(hours),
            _ => None,
        })
        .collect();
    if parsed.is_empty() { DEFAULT_LEADS.to_vec() } else { parsed }
}

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
    use uuid::Uuid;

    async fn pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("deadline-{}.db", Uuid::new_v4()));
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

    /// A deadline `hours` from now, extracted from a message, optionally matched to nothing.
    async fn deadline(pool: &SqlitePool, user_id: &str, hours: i64) -> String {
        let message_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO email_messages (id, user_id, gmail_message_id, subject, created_at)
             VALUES (?1, ?2, ?3, 'Your Roblox Assessments Invitation', ?4)",
        )
        .bind(&message_id)
        .bind(user_id)
        .bind(Uuid::new_v4().to_string())
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .unwrap();

        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO application_deadlines
                 (id, message_id, user_id, application_id, due_at, source_text, created_at)
             VALUES (?1, ?2, ?3, NULL, ?4, 'within 5 days', ?5)",
        )
        .bind(&id)
        .bind(&message_id)
        .bind(user_id)
        .bind((Utc::now() + Duration::hours(hours)).to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn alerts(pool: &SqlitePool) -> Vec<String> {
        sqlx::query_scalar("SELECT subject_id FROM hunt_events WHERE kind = 'deadline' ORDER BY subject_id")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    /// Rule 8, at the alert layer: the matcher missing does not make the deadline stop existing.
    #[tokio::test]
    async fn an_unmatched_deadline_is_still_warned_about() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let id = deadline(&pool, &user_id, 30).await;

        let report = sweep(&pool, Utc::now()).await.unwrap();

        assert_eq!(report.raised, 1, "the 72h lead applies at 30h out; the 24h one does not");
        assert_eq!(alerts(&pool).await, vec![format!("{id}:72")]);
    }

    /// The second warning says something the first did not, and the key is what lets it.
    #[tokio::test]
    async fn a_closer_deadline_earns_the_second_lead_without_repeating_the_first() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let id = deadline(&pool, &user_id, 10).await;

        assert_eq!(sweep(&pool, Utc::now()).await.unwrap().raised, 2);
        assert_eq!(
            alerts(&pool).await,
            vec![format!("{id}:24"), format!("{id}:72")]
        );

        // And neither repeats on the next pass, which is the half that keeps the channel usable.
        let again = sweep(&pool, Utc::now()).await.unwrap();
        assert_eq!(again.raised, 0);
        assert_eq!(again.found, 2, "still approaching — it has simply been said");
    }

    #[tokio::test]
    async fn a_deadline_already_past_raises_nothing() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        deadline(&pool, &user_id, -5).await;

        assert_eq!(sweep(&pool, Utc::now()).await.unwrap(), DeadlineReport::default());
        assert!(alerts(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn a_distant_deadline_is_left_alone() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        deadline(&pool, &user_id, 24 * 30).await;

        assert_eq!(sweep(&pool, Utc::now()).await.unwrap().found, 0);
    }

    #[tokio::test]
    async fn the_warning_names_its_owner_and_carries_its_evidence() {
        let now = Utc::now();
        let due = Due {
            deadline_id: "d1".into(),
            user_id: "u1".into(),
            application_id: None,
            label: "Roblox".into(),
            url: None,
            due_at: now + Duration::hours(20),
            source_text: "complete within 5 days".into(),
            lead_hours: 24,
        };
        let event = deadline_event(&due, now);

        assert_eq!(event.kind, EventKind::Deadline);
        assert_eq!(event.user_id.as_deref(), Some("u1"));
        assert_eq!(event.subject_id, "d1:24");
        assert!(event.title.contains("Roblox") && event.title.contains("20h"));
        // The words that produced the date travel with the alert: this is parsed out of
        // untrusted text and the reader has to be able to check it.
        assert!(event.body.contains("complete within 5 days"));
    }
}
