//! Tests for the two orphan-cleanup migrations: `0027`, which deletes postings nothing points
//! at and nothing can ever expire, and `0030`, which deletes the `posting` alerts that
//! migration 0025 left pointing at rows it had deleted.
//!
//! Two jobs, as for `rekey`: assert the **shape** the DELETE must keep — a guard dropped in a
//! later edit is a posting deleted out from under an application or an alert — and run the real
//! file against a fixture holding one row of every kind it must and must not touch.

#[cfg(test)]
mod tests {
    use sqlx::{Connection, SqliteConnection};

    const MIGRATION: &str = include_str!("../../migrations/0027_delete_orphaned_postings.sql");

    /// The start of the first uncapped run. Anything created after it is out of scope, because
    /// a posting created moments ago whose sightings failed to write is precisely the row the
    /// sweep's zero-sighting guard exists to protect.
    const CUTOFF: &str = "2026-09-02T20:41:48";

    /// The DELETE statement itself, comments stripped — so a guard mentioned only in the
    /// header cannot satisfy these assertions.
    fn delete_statement() -> String {
        let body: String = MIGRATION
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join(" ");
        let start = body
            .find("DELETE FROM internship_postings")
            .expect("0027 must delete from internship_postings");
        body[start..].to_string()
    }

    #[test]
    fn the_delete_is_guarded_against_every_reference_a_posting_can_have() {
        // Two declared foreign keys and one soft reference. `hunt_events.subject_id` is the one
        // `PRAGMA foreign_keys` would not have caught, which is exactly why it is named here.
        let delete = delete_statement();
        for guard in ["posting_sightings", "internship_applications", "hunt_events"] {
            let clause = format!("NOT EXISTS (SELECT 1 FROM {guard}");
            assert!(
                delete.contains(&clause),
                "0027 must exclude postings referenced by {guard}; the DELETE reads:\n{delete}"
            );
        }
        assert!(
            delete.contains(CUTOFF),
            "0027 must keep its creation-date cutoff, or it becomes a standing rule that \
             deletes new postings whose sightings failed to record"
        );
    }

    async fn fixture() -> SqliteConnection {
        let path = std::env::temp_dir().join(format!("orphans-{}.db", uuid::Uuid::new_v4()));
        let mut conn = SqliteConnection::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .unwrap();

        sqlx::raw_sql(
            "CREATE TABLE internship_postings (
                 id TEXT PRIMARY KEY, dedup_key TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL);
             CREATE TABLE posting_sightings (id TEXT PRIMARY KEY, posting_id TEXT NOT NULL);
             CREATE TABLE internship_applications (
                 id TEXT PRIMARY KEY, user_id TEXT NOT NULL, posting_id TEXT);
             CREATE TABLE hunt_events (
                 id TEXT PRIMARY KEY, kind TEXT NOT NULL, subject_id TEXT NOT NULL);",
        )
        .execute(&mut conn)
        .await
        .unwrap();

        // One row of every kind the migration has to decide about.
        sqlx::raw_sql(
            "INSERT INTO internship_postings (id, dedup_key, created_at) VALUES
                 ('orphan',   'co:a|x|summer-any', '2026-08-21T14:34:04'),
                 ('watched',  'co:b|x|summer-any', '2026-08-21T14:34:04'),
                 ('applied',  'co:c|x|summer-any', '2026-08-21T14:34:04'),
                 ('alerted',  'co:d|x|summer-any', '2026-08-21T14:34:04'),
                 ('newborn',  'co:e|x|summer-any', '2026-09-03T09:00:00');
             INSERT INTO posting_sightings (id, posting_id) VALUES ('s1', 'watched');
             INSERT INTO internship_applications (id, user_id, posting_id)
                 VALUES ('a1', 'u1', 'applied');
             INSERT INTO hunt_events (id, kind, subject_id)
                 VALUES ('h1', 'posting', 'alerted');",
        )
        .execute(&mut conn)
        .await
        .unwrap();

        conn
    }

    async fn survivors(conn: &mut SqliteConnection) -> Vec<String> {
        sqlx::query_scalar("SELECT id FROM internship_postings ORDER BY id")
            .fetch_all(conn)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn only_the_posting_nothing_points_at_is_deleted() {
        let mut conn = fixture().await;
        sqlx::raw_sql(MIGRATION).execute(&mut conn).await.unwrap();

        assert_eq!(
            survivors(&mut conn).await,
            vec!["alerted", "applied", "newborn", "watched"],
            "a posting with a sighting, an application, an alert, or a recent creation date \
             must survive; only the one nothing points at goes"
        );
    }

    #[tokio::test]
    async fn a_newborn_posting_with_no_sightings_yet_is_never_touched() {
        // The row `expiry::sweep`'s zero-sighting guard was written to protect. It has no
        // sightings and looks identical to an orphan; only its creation date distinguishes it,
        // which is the whole reason the cutoff is in the predicate.
        let mut conn = fixture().await;
        sqlx::raw_sql(MIGRATION).execute(&mut conn).await.unwrap();

        let alive: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM internship_postings WHERE id = 'newborn'")
                .fetch_one(&mut conn)
                .await
                .unwrap();
        assert_eq!(alive, 1);
    }

    // ---- 0030, the alerts 0025 orphaned ----

    const ALERTS: &str = include_str!("../../migrations/0030_delete_orphaned_posting_alerts.sql");

    #[test]
    fn the_alert_cleanup_is_scoped_to_the_one_kind_that_holds_a_posting_id() {
        // `nudge` and `deadline` are keyed on applications and `email` on a Gmail message id,
        // so a predicate that forgot to scope by kind would find no matching posting for ANY of
        // them and delete every alert in the database.
        assert!(
            ALERTS.contains("kind = 'posting'"),
            "0030 must only touch the kind whose subject_id is a posting id"
        );
        assert!(
            ALERTS.contains("NOT EXISTS"),
            "0030 must delete only alerts whose posting is genuinely gone"
        );
    }

    #[tokio::test]
    async fn only_the_alert_whose_posting_vanished_is_deleted() {
        let path = std::env::temp_dir().join(format!("alerts-{}.db", uuid::Uuid::new_v4()));
        let mut conn = SqliteConnection::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .unwrap();

        sqlx::raw_sql(
            "CREATE TABLE internship_postings (id TEXT PRIMARY KEY);
             CREATE TABLE hunt_events (id TEXT PRIMARY KEY, kind TEXT NOT NULL, subject_id TEXT NOT NULL);
             INSERT INTO internship_postings (id) VALUES ('live');
             INSERT INTO hunt_events (id, kind, subject_id) VALUES
                 ('keep-live',     'posting',  'live'),
                 ('drop-orphan',   'posting',  'deleted-by-0025'),
                 ('keep-nudge',    'nudge',    'some-application'),
                 ('keep-deadline', 'deadline', 'some-application'),
                 ('keep-email',    'email',    'gmail-message-id');",
        )
        .execute(&mut conn)
        .await
        .unwrap();

        sqlx::raw_sql(ALERTS).execute(&mut conn).await.unwrap();

        let survivors: Vec<String> = sqlx::query_scalar("SELECT id FROM hunt_events ORDER BY id")
            .fetch_all(&mut conn)
            .await
            .unwrap();
        assert_eq!(
            survivors,
            vec!["keep-deadline", "keep-email", "keep-live", "keep-nudge"],
            "only the posting alert whose posting is gone may go; the other kinds hold ids that \
             are not postings at all and would every one of them look orphaned"
        );
    }

    #[tokio::test]
    async fn applying_it_twice_changes_nothing_the_second_time() {
        let mut conn = fixture().await;
        sqlx::raw_sql(MIGRATION).execute(&mut conn).await.unwrap();
        let after_first = survivors(&mut conn).await;

        sqlx::raw_sql(MIGRATION).execute(&mut conn).await.unwrap();
        assert_eq!(survivors(&mut conn).await, after_first);
    }
}
