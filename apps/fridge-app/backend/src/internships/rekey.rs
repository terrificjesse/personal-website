//! Tests for migration `0025`, which re-keys the postings 12a's parsers identify and merges the
//! duplicates that creates.
//!
//! The migration is a generated list of literal ids, so these do two different jobs: assert the
//! **shape** every statement must keep (a regenerated file that loses a guard would abort a
//! boot), and run the real file against a fixture built from the **real ids of the one group
//! that matters** — the posting the only application to reach OA points at.

#[cfg(test)]
mod tests {
    use sqlx::{Connection, SqliteConnection};

    const MIGRATION: &str = include_str!("../../migrations/0025_rekey_ats_postings.sql");

    /// The posting the Roblox application pointed at before the merge, and the row it must be
    /// repointed to. Named literally, because in a year the only way to know this mattered is
    /// for the test to say so.
    const ROBLOX_LOSER: &str = "e482889e-8709-4935-a0de-1572a3e49311";
    const ROBLOX_SURVIVOR: &str = "61216a67-1b91-4a0b-adc0-f85a97fa1076";

    fn statements() -> Vec<&'static str> {
        MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.lines().all(|l| l.trim_start().starts_with("--")))
            .collect()
    }

    /// `UNIQUE (user_id, posting_id)` would abort the migration, and an aborted migration takes
    /// the whole boot with it.
    #[test]
    fn every_application_repoint_is_guarded_against_a_duplicate() {
        for statement in statements() {
            if statement.contains("UPDATE internship_applications SET posting_id") {
                assert!(
                    statement.contains("NOT EXISTS"),
                    "unguarded repoint would violate UNIQUE (user_id, posting_id):\n{statement}"
                );
            }
        }
    }

    /// A losing row may only go once nothing points at it. If a repoint was skipped, the loser
    /// survives as a duplicate — the safe direction.
    #[test]
    fn no_posting_is_deleted_while_an_application_points_at_it() {
        for statement in statements() {
            if statement.starts_with("DELETE FROM internship_postings") {
                assert!(
                    statement.contains("NOT EXISTS")
                        && statement.contains("internship_applications"),
                    "unguarded delete would orphan an application:\n{statement}"
                );
            }
        }
    }

    /// `dedup_key` is UNIQUE, so a row cannot claim a key another row has not released yet.
    /// Every changing row moves to `rekey-0025:<id>` first; the file must keep that order.
    #[test]
    fn every_key_is_released_before_any_key_is_claimed() {
        // Matched on the SET clause specifically: `dedup_key = 'ats:…'` also appears in a
        // release statement's WHERE, where it is the key being given up rather than taken.
        let first_claim = MIGRATION
            .find("SET dedup_key = 'ats:")
            .expect("the migration claims at least one ATS key");
        let last_release = MIGRATION
            .rfind("SET dedup_key = 'rekey-0025:")
            .expect("the migration releases at least one key");
        assert!(
            last_release < first_claim,
            "a key is claimed before every release has run — this aborts on UNIQUE"
        );
    }

    /// The two refused groups must not be touched at all: rewriting either row to the shared
    /// key collides, and rewriting only one merges them by the back door.
    #[test]
    fn the_refused_groups_are_absent_entirely() {
        for refused in ["tower-research", "8044334", "JR101733"] {
            assert!(
                !MIGRATION.contains(refused),
                "{refused} belongs to a group this migration refuses to merge"
            );
        }
    }

    /// The whole point, on the real ids.
    #[tokio::test]
    async fn the_roblox_application_follows_its_posting_to_the_survivor() {
        let path = std::env::temp_dir().join(format!("rekey-{}.db", uuid::Uuid::new_v4()));
        let mut conn = SqliteConnection::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .unwrap();

        // The smallest fixture that can show the repoint: the two postings the migration names
        // and the application pointing at the one it deletes.
        sqlx::raw_sql(
            "CREATE TABLE internship_postings (id TEXT PRIMARY KEY, dedup_key TEXT NOT NULL UNIQUE);
             CREATE TABLE internship_applications (
                 id TEXT PRIMARY KEY, user_id TEXT NOT NULL, posting_id TEXT,
                 UNIQUE (user_id, posting_id));
             CREATE TABLE posting_sightings (id TEXT PRIMARY KEY, posting_id TEXT NOT NULL);",
        )
        .execute(&mut conn)
        .await
        .unwrap();

        sqlx::query("INSERT INTO internship_postings (id, dedup_key) VALUES (?1, 'co:roblox|software-engineer|summer-2027'), (?2, 'ats:greenhouse:gh_jid:8072713')")
            .bind(ROBLOX_LOSER).bind(ROBLOX_SURVIVOR)
            .execute(&mut conn).await.unwrap();
        sqlx::query("INSERT INTO internship_applications (id, user_id, posting_id) VALUES ('a1', 'u1', ?1)")
            .bind(ROBLOX_LOSER)
            .execute(&mut conn).await.unwrap();
        sqlx::query("INSERT INTO posting_sightings (id, posting_id) VALUES ('s1', ?1)")
            .bind(ROBLOX_LOSER)
            .execute(&mut conn).await.unwrap();

        sqlx::raw_sql(MIGRATION).execute(&mut conn).await.unwrap();

        let points_at: String =
            sqlx::query_scalar("SELECT posting_id FROM internship_applications WHERE id = 'a1'")
                .fetch_one(&mut conn).await.unwrap();
        assert_eq!(points_at, ROBLOX_SURVIVOR, "the application must follow the merge");

        let loser_gone: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM internship_postings WHERE id = ?1")
                .bind(ROBLOX_LOSER)
                .fetch_one(&mut conn).await.unwrap();
        assert_eq!(loser_gone, 0);

        let sighting_moved: String =
            sqlx::query_scalar("SELECT posting_id FROM posting_sightings WHERE id = 's1'")
                .fetch_one(&mut conn).await.unwrap();
        assert_eq!(sighting_moved, ROBLOX_SURVIVOR, "its history follows too");
    }
}
