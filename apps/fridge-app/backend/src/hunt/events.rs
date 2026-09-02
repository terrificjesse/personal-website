//! Reading and writing `hunt_events`. Producer-agnostic on purpose.
//!
//! Nothing here decides what is worth alerting about — see the module doc on
//! [`super`]. This file knows how an event is stored, who may see it, and what "already
//! delivered" means.
//!
//! # `acked_at` is a delivery receipt, not a user dismissal
//!
//! The extension acks an event once it has actually raised a desktop notification for it.
//! "Ack" is used here the way messaging uses it — the client confirming it took delivery —
//! which is what makes the checkpoint hold: ack, restart Firefox, and the alert does not come
//! back, because the record of having shown it never lived in the browser.
//!
//! The popup consequently lists *recent* events rather than undelivered ones
//! ([`EventQuery::include_acked`]); by the time you open it, everything you were notified
//! about is acked.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Which producer wrote an event.
///
/// 8e writes only [`EventKind::Posting`]. [`EventKind::Email`] exists now because 8d writes it
/// against this same table, and because the extension filters alerts by kind — the open
/// question of whether cold outreach should interrupt you is then a client-side predicate
/// rather than a schema change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Posting,
    Email,
    /// A follow-up: an application that has had no response for long enough to be worth
    /// chasing. Phase 11e. A third *producer* on this table, not a second pipeline.
    Nudge,
}

impl EventKind {
    /// The stored spelling. Matches the CHECK constraint, which migration `0022` widened from
    /// migration `0014`'s original two kinds.
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Posting => "posting",
            EventKind::Email => "email",
            EventKind::Nudge => "nudge",
        }
    }
}

/// An event a producer wants written.
///
/// `title` and `body` are **rendered**, not structured: the extension shows them verbatim in
/// one notification path shared by both producers. `payload` carries the facts behind them
/// for the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewHuntEvent {
    pub kind: EventKind,
    /// `None` = visible to every signed-in user, which is what a posting from the shared
    /// corpus is. A producer handling someone's private data must always set this.
    pub user_id: Option<String>,
    /// The real-world thing this is about, and the idempotency key. See migration `0014`.
    pub subject_id: String,
    pub title: String,
    pub body: String,
    pub url: Option<String>,
    pub payload: serde_json::Value,
}

/// An event as the API returns it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HuntEvent {
    pub id: String,
    pub kind: EventKind,
    pub subject_id: String,
    pub title: String,
    pub body: String,
    pub url: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    /// `None` means no client has raised a notification for this yet.
    pub acked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct HuntEventRow {
    id: String,
    kind: EventKind,
    subject_id: String,
    title: String,
    body: String,
    url: Option<String>,
    payload_json: String,
    created_at: DateTime<Utc>,
    acked_at: Option<DateTime<Utc>>,
}

impl From<HuntEventRow> for HuntEvent {
    fn from(row: HuntEventRow) -> Self {
        HuntEvent {
            id: row.id,
            kind: row.kind,
            subject_id: row.subject_id,
            title: row.title,
            body: row.body,
            url: row.url,
            // A payload we cannot parse costs the popup some detail. Dropping the event over
            // it would cost the notification entirely, which is the whole point of the row.
            payload: serde_json::from_str(&row.payload_json).unwrap_or(serde_json::Value::Null),
            created_at: row.created_at,
            acked_at: row.acked_at,
        }
    }
}

/// Who may see an event, as a SQL fragment. Bound parameter: the viewer's user id.
///
/// Written once and used by every read here so the two halves cannot drift — a private event
/// leaking is a read-site omission, and this is the read site.
const VISIBLE_TO_VIEWER: &str = "(user_id IS NULL OR user_id = ?)";

const SELECT_EVENT_COLUMNS: &str =
    "id, kind, subject_id, title, body, url, payload_json, created_at, acked_at";

/// Write an event, unless one already exists for the same subject.
///
/// Returns whether a row was actually inserted. The `ON CONFLICT DO NOTHING` is the
/// load-bearing part: it makes "one alert per posting" a property of the schema rather than
/// of the caller remembering to ask whether it had alerted already.
pub async fn emit(pool: &SqlitePool, event: &NewHuntEvent, now: DateTime<Utc>) -> Result<bool> {
    let inserted = sqlx::query(
        "INSERT INTO hunt_events
             (id, kind, user_id, subject_id, title, body, url, payload_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (kind, subject_id) DO NOTHING",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(event.kind.as_str())
    .bind(&event.user_id)
    .bind(&event.subject_id)
    .bind(&event.title)
    .bind(&event.body)
    .bind(&event.url)
    .bind(event.payload.to_string())
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?
    .rows_affected();

    Ok(inserted > 0)
}

/// What a caller is asking to see.
#[derive(Debug, Clone)]
pub struct EventQuery<'a> {
    pub viewer: &'a str,
    /// Only events created strictly after this.
    ///
    /// The background poller deliberately does **not** send it: a watermark the client holds
    /// is exactly the state an MV3 background page loses, and an event that arrived while the
    /// browser was closed would then never be returned again. `acked_at` is the record; this
    /// is a convenience for the popup.
    pub since: Option<DateTime<Utc>>,
    /// The popup passes `true` for a recent-alerts list. The poller leaves it `false` and so
    /// sees only what has not been delivered.
    pub include_acked: bool,
    pub limit: i64,
}

/// Events visible to this viewer, newest first.
pub async fn list(pool: &SqlitePool, query: &EventQuery<'_>) -> Result<Vec<HuntEvent>> {
    let mut sql = format!("SELECT {SELECT_EVENT_COLUMNS} FROM hunt_events WHERE {VISIBLE_TO_VIEWER}");
    if !query.include_acked {
        sql.push_str(" AND acked_at IS NULL");
    }
    if query.since.is_some() {
        sql.push_str(" AND created_at > ?");
    }
    sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ?");

    let mut statement = sqlx::query_as::<_, HuntEventRow>(&sql).bind(query.viewer);
    if let Some(since) = query.since {
        statement = statement.bind(since.to_rfc3339());
    }

    let rows = statement.bind(query.limit).fetch_all(pool).await?;
    Ok(rows.into_iter().map(HuntEvent::from).collect())
}

/// How many events this viewer has that no client has taken delivery of.
///
/// Sent alongside every list so the popup can say "and 40 more" without a second round trip,
/// and so a `limit` that truncates the list is visible rather than silently lossy.
pub async fn unacked_total(pool: &SqlitePool, viewer: &str) -> Result<i64> {
    let total = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM hunt_events WHERE acked_at IS NULL AND {VISIBLE_TO_VIEWER}"
    ))
    .bind(viewer)
    .fetch_one(pool)
    .await?;
    Ok(total)
}

/// What acking an event did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckOutcome {
    Acked,
    /// Already delivered. Not an error: the extension retrying an ack it is unsure landed is
    /// the correct thing for it to do, and must not look like a failure.
    AlreadyAcked,
    /// No such event, or not visible to this viewer. The two are deliberately one outcome —
    /// distinguishing them tells a caller whether an id exists that they cannot see.
    NotFound,
}

/// Record that a client has raised a notification for this event.
///
/// Idempotent, and the first receipt wins: a second ack does not move the timestamp.
pub async fn ack(
    pool: &SqlitePool,
    id: &str,
    viewer: &str,
    now: DateTime<Utc>,
) -> Result<AckOutcome> {
    let updated = sqlx::query(&format!(
        "UPDATE hunt_events SET acked_at = ?1
          WHERE id = ?2 AND acked_at IS NULL AND {VISIBLE_TO_VIEWER}"
    ))
    .bind(now.to_rfc3339())
    .bind(id)
    .bind(viewer)
    .execute(pool)
    .await?
    .rows_affected();

    if updated > 0 {
        return Ok(AckOutcome::Acked);
    }

    // Zero rows updated is ambiguous — already acked, or not ours to ack. Ask.
    let exists: Option<String> = sqlx::query_scalar(&format!(
        "SELECT id FROM hunt_events WHERE id = ? AND {VISIBLE_TO_VIEWER}"
    ))
    .bind(id)
    .bind(viewer)
    .fetch_optional(pool)
    .await?;

    Ok(match exists {
        Some(_) => AckOutcome::AlreadyAcked,
        None => AckOutcome::NotFound,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        // A file rather than `:memory:`, for the reason `collector`'s tests record: the pool
        // opens several connections and each would get its own private in-memory database.
        let path = std::env::temp_dir().join(format!("hunt-events-{}.db", Uuid::new_v4()));
        let url = format!("sqlite://{}?mode=rwc", path.display());
        crate::db::init_pool(&url).await.expect("migrations")
    }

    async fn user(pool: &SqlitePool, id: &str) {
        sqlx::query("INSERT INTO users (id, email, password_hash, created_at) VALUES (?1,?2,?3,?4)")
            .bind(id)
            .bind(format!("{id}@example.com"))
            .bind("x")
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await
            .expect("user");
    }

    fn event(kind: EventKind, subject: &str, user_id: Option<&str>) -> NewHuntEvent {
        NewHuntEvent {
            kind,
            user_id: user_id.map(str::to_string),
            subject_id: subject.to_string(),
            title: format!("about {subject}"),
            body: "body".to_string(),
            url: Some("https://example.com".to_string()),
            payload: serde_json::json!({ "subject": subject }),
        }
    }

    fn poll(viewer: &str) -> EventQuery<'_> {
        EventQuery {
            viewer,
            since: None,
            include_acked: false,
            limit: 50,
        }
    }

    #[tokio::test]
    async fn the_same_subject_cannot_produce_two_events() {
        let pool = test_pool().await;
        let now = Utc::now();

        assert!(emit(&pool, &event(EventKind::Posting, "p1", None), now)
            .await
            .expect("first"));
        assert!(
            !emit(&pool, &event(EventKind::Posting, "p1", None), now)
                .await
                .expect("second"),
            "a second event for the same subject must not be written"
        );

        user(&pool, "u1").await;
        assert_eq!(list(&pool, &poll("u1")).await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn the_same_subject_id_under_a_different_kind_is_a_different_event() {
        // `subject_id` is polymorphic — a posting id and a Gmail message id share one column.
        // Keying dedup on it alone would let one producer swallow the other's alert.
        let pool = test_pool().await;
        let now = Utc::now();
        user(&pool, "u1").await;

        assert!(emit(&pool, &event(EventKind::Posting, "x", None), now)
            .await
            .expect("posting"));
        assert!(emit(&pool, &event(EventKind::Email, "x", Some("u1")), now)
            .await
            .expect("email"));

        assert_eq!(list(&pool, &poll("u1")).await.expect("list").len(), 2);
    }

    #[tokio::test]
    async fn a_private_event_is_invisible_to_another_user() {
        // 8d's email events carry someone's mail. The read path is the only thing standing
        // between that and every other account on the site.
        let pool = test_pool().await;
        let now = Utc::now();
        user(&pool, "u1").await;
        user(&pool, "u2").await;

        emit(&pool, &event(EventKind::Email, "m1", Some("u1")), now)
            .await
            .expect("emit");
        emit(&pool, &event(EventKind::Posting, "p1", None), now)
            .await
            .expect("emit");

        let mine = list(&pool, &poll("u1")).await.expect("list");
        assert_eq!(mine.len(), 2, "my private event plus the shared posting");

        let theirs = list(&pool, &poll("u2")).await.expect("list");
        assert_eq!(theirs.len(), 1, "only the shared posting");
        assert_eq!(theirs[0].kind, EventKind::Posting);

        assert_eq!(unacked_total(&pool, "u2").await.expect("count"), 1);
    }

    #[tokio::test]
    async fn acking_hides_an_event_from_the_poll_but_not_from_the_popup() {
        let pool = test_pool().await;
        let now = Utc::now();
        user(&pool, "u1").await;

        emit(&pool, &event(EventKind::Posting, "p1", None), now)
            .await
            .expect("emit");
        let id = list(&pool, &poll("u1")).await.expect("list")[0].id.clone();

        assert_eq!(
            ack(&pool, &id, "u1", now).await.expect("ack"),
            AckOutcome::Acked
        );

        assert!(
            list(&pool, &poll("u1")).await.expect("list").is_empty(),
            "the background poll must not see a delivered event again"
        );
        assert_eq!(unacked_total(&pool, "u1").await.expect("count"), 0);

        let recent = list(
            &pool,
            &EventQuery {
                include_acked: true,
                ..poll("u1")
            },
        )
        .await
        .expect("list");
        assert_eq!(recent.len(), 1, "the popup still lists it");
        assert!(recent[0].acked_at.is_some());
    }

    #[tokio::test]
    async fn a_repeated_ack_succeeds_and_keeps_the_first_receipt() {
        // The extension retrying an ack it isn't sure landed is correct behaviour. If that
        // read as a failure it would stop acking and re-notify instead.
        let pool = test_pool().await;
        let first = Utc::now();
        user(&pool, "u1").await;

        emit(&pool, &event(EventKind::Posting, "p1", None), first)
            .await
            .expect("emit");
        let id = list(&pool, &poll("u1")).await.expect("list")[0].id.clone();

        ack(&pool, &id, "u1", first).await.expect("ack");
        let later = first + chrono::Duration::hours(1);
        assert_eq!(
            ack(&pool, &id, "u1", later).await.expect("ack"),
            AckOutcome::AlreadyAcked
        );

        let stored: DateTime<Utc> = sqlx::query_scalar("SELECT acked_at FROM hunt_events")
            .fetch_one(&pool)
            .await
            .expect("row");
        assert_eq!(stored, first, "the first receipt stands");
    }

    #[tokio::test]
    async fn a_user_cannot_ack_someone_elses_private_event() {
        let pool = test_pool().await;
        let now = Utc::now();
        user(&pool, "u1").await;
        user(&pool, "u2").await;

        emit(&pool, &event(EventKind::Email, "m1", Some("u1")), now)
            .await
            .expect("emit");
        let id = list(&pool, &poll("u1")).await.expect("list")[0].id.clone();

        assert_eq!(
            ack(&pool, &id, "u2", now).await.expect("ack"),
            AckOutcome::NotFound,
            "an event you cannot see is one you cannot ack"
        );
        assert_eq!(unacked_total(&pool, "u1").await.expect("count"), 1);
    }

    #[tokio::test]
    async fn since_narrows_by_creation_time() {
        let pool = test_pool().await;
        user(&pool, "u1").await;

        let old = Utc::now() - chrono::Duration::days(2);
        let recent = Utc::now();
        emit(&pool, &event(EventKind::Posting, "old", None), old)
            .await
            .expect("emit");
        emit(&pool, &event(EventKind::Posting, "new", None), recent)
            .await
            .expect("emit");

        let since = Utc::now() - chrono::Duration::days(1);
        let events = list(
            &pool,
            &EventQuery {
                since: Some(since),
                ..poll("u1")
            },
        )
        .await
        .expect("list");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].subject_id, "new");
    }

    #[tokio::test]
    async fn a_truncated_page_still_reports_the_true_unacked_count() {
        // Otherwise a `limit` that cuts the list off is indistinguishable from a complete
        // one, and the popup quietly under-reports how much is waiting.
        let pool = test_pool().await;
        let now = Utc::now();
        user(&pool, "u1").await;

        for n in 0..5 {
            emit(&pool, &event(EventKind::Posting, &format!("p{n}"), None), now)
                .await
                .expect("emit");
        }

        let page = list(&pool, &EventQuery { limit: 2, ..poll("u1") })
            .await
            .expect("list");
        assert_eq!(page.len(), 2);
        assert_eq!(unacked_total(&pool, "u1").await.expect("count"), 5);
    }

    #[tokio::test]
    async fn an_unreadable_payload_does_not_lose_the_event() {
        // The payload is detail for the popup; the title and body are the alert. A parse
        // failure must cost the former, never the latter.
        let pool = test_pool().await;
        user(&pool, "u1").await;

        emit(&pool, &event(EventKind::Posting, "p1", None), Utc::now())
            .await
            .expect("emit");
        sqlx::query("UPDATE hunt_events SET payload_json = 'not json'")
            .execute(&pool)
            .await
            .expect("corrupt");

        let events = list(&pool, &poll("u1")).await.expect("list");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload, serde_json::Value::Null);
        assert_eq!(events[0].title, "about p1");
    }

}

#[cfg(test)]
mod hunt_event_rebuild_tests {
    //! Migration `0022` rebuilds this table to widen one CHECK constraint, and a rebuild is the
    //! one migration shape that can lose data while reporting success.
    //!
    //! These build the PRE-0022 table by hand, run the migration's own SQL against it, and
    //! check what came out the other side — so they test the file that will actually run, not
    //! a description of it.

    use sqlx::{Connection, SqliteConnection, Row};
    use uuid::Uuid;

    const MIGRATION: &str = include_str!("../../migrations/0022_widen_hunt_event_kinds.sql");

    /// Migration `0014`'s table, verbatim in shape, plus the one table it references.
    ///
    /// `users` has to exist: the rebuild's INSERT is checked against
    /// `user_id REFERENCES users (id)`, and sqlx turns `PRAGMA foreign_keys` on per connection.
    const ORIGINAL: &str = "
        CREATE TABLE users (id TEXT PRIMARY KEY NOT NULL);
        CREATE TABLE hunt_events (
            id TEXT PRIMARY KEY NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('posting', 'email')),
            user_id TEXT,
            subject_id TEXT NOT NULL,
            title TEXT NOT NULL,
            body TEXT NOT NULL,
            url TEXT,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            acked_at TEXT,
            UNIQUE (kind, subject_id)
        );
        CREATE INDEX idx_hunt_events_unacked ON hunt_events (created_at) WHERE acked_at IS NULL;
        CREATE INDEX idx_hunt_events_created ON hunt_events (created_at);
    ";

    async fn rebuilt_from_seeded() -> SqliteConnection {
        let path = std::env::temp_dir().join(format!("rebuild-{}.db", Uuid::new_v4()));
        let mut conn = SqliteConnection::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .unwrap();
        sqlx::raw_sql(ORIGINAL).execute(&mut conn).await.unwrap();

        // Three events, two of them already delivered.
        for (id, kind, subject, acked) in [
            ("e1", "posting", "posting-1", Some("2026-09-01T00:00:00Z")),
            ("e2", "posting", "posting-2", Some("2026-09-01T00:05:00Z")),
            ("e3", "email", "gmail-1", None),
        ] {
            sqlx::query(
                "INSERT INTO hunt_events
                     (id, kind, user_id, subject_id, title, body, url, payload_json,
                      created_at, acked_at)
                 VALUES (?1, ?2, NULL, ?3, 't', 'b', NULL, '{}', '2026-09-01T00:00:00Z', ?4)",
            )
            .bind(id).bind(kind).bind(subject).bind(acked)
            .execute(&mut conn).await.unwrap();
        }

        sqlx::raw_sql(MIGRATION).execute(&mut conn).await.unwrap();
        conn
    }

    /// The failure this migration could cause, and the one it must not.
    ///
    /// Losing `acked_at` re-raises every historical alert the next time the extension polls —
    /// an MV3 background page remembers nothing across restarts, so the server's receipt is
    /// the only thing standing between a rebuild and 60 notifications at once.
    #[tokio::test]
    async fn every_row_and_every_delivery_receipt_survives_the_rebuild() {
        let mut conn = rebuilt_from_seeded().await;

        let row = sqlx::query("SELECT COUNT(*) AS rows, COUNT(acked_at) AS acked FROM hunt_events")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(row.get::<i64, _>("rows"), 3, "rows lost in the rebuild");
        assert_eq!(row.get::<i64, _>("acked"), 2, "delivery receipts lost in the rebuild");

        // Columns carried across by name, not by position.
        let kind: String = sqlx::query_scalar("SELECT kind FROM hunt_events WHERE id = 'e3'")
            .fetch_one(&mut conn).await.unwrap();
        assert_eq!(kind, "email");
    }

    #[tokio::test]
    async fn the_new_kind_is_accepted_and_an_unknown_one_is_still_refused() {
        let mut conn = rebuilt_from_seeded().await;

        let insert = |kind: &'static str, subject: &'static str| {
            sqlx::query(
                "INSERT INTO hunt_events
                     (id, kind, user_id, subject_id, title, body, url, payload_json, created_at)
                 VALUES (?1, ?2, NULL, ?3, 't', 'b', NULL, '{}', '2026-09-02T00:00:00Z')",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(kind)
            .bind(subject)
        };

        insert("nudge", "app-1:14").execute(&mut conn).await.expect("nudge must be accepted");
        assert!(
            insert("bogus", "whatever").execute(&mut conn).await.is_err(),
            "the CHECK was widened, not removed — a typo'd kind must still fail loudly"
        );
    }

    /// A rebuild that drops the UNIQUE turns structural dedup back into a convention, and the
    /// symptom is a duplicate notification months later.
    #[tokio::test]
    async fn the_unique_constraint_and_both_indexes_come_back() {
        let mut conn = rebuilt_from_seeded().await;

        let duplicate = sqlx::query(
            "INSERT INTO hunt_events
                 (id, kind, user_id, subject_id, title, body, url, payload_json, created_at)
             VALUES ('dup', 'posting', NULL, 'posting-1', 't', 'b', NULL, '{}', 'now')",
        )
        .execute(&mut conn)
        .await;
        assert!(duplicate.is_err(), "UNIQUE (kind, subject_id) did not survive");

        let indexes: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'hunt_events'
              AND name LIKE 'idx_%' ORDER BY name",
        )
        .fetch_all(&mut conn)
        .await
        .unwrap();
        assert_eq!(indexes, vec!["idx_hunt_events_created", "idx_hunt_events_unacked"]);
    }
}

