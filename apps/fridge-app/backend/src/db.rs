use sqlx::{
    Sqlite, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions},
};
use std::{str::FromStr, time::Duration};

/// How long a writer waits for another writer before giving up with `SQLITE_BUSY`.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Open the pool and run migrations.
///
/// # Why these three pragmas are set explicitly
///
/// They were previously left to whatever `SqlitePoolOptions::connect(url)` produced, and what
/// it produces is **not** what the rest of this codebase assumes. Measured 2026-09-02, in a
/// test that now pins it: `journal_mode` came back `delete`, and eight concurrent writes
/// failed immediately with `database is locked` rather than waiting.
///
/// That was survivable while every write was a single autocommit statement. Phase 10 made the
/// status writers transactional — a transaction spans two statements and holds the write lock
/// across both — so the window a competing writer can land in got wider, and the failure
/// stopped being theoretical. It also used to be invisible: `routes::inbox::decide` discarded
/// the `Result` of its application UPDATE, so a lock collision marked the proposal reviewed
/// and silently did not move the tracker.
///
/// - **WAL** so readers never block writers and a second writer waits its turn instead of
///   colliding. The internship collector, the blog watcher and the inbox worker all write on
///   their own schedules while requests are being served; that is three background writers
///   plus the request path.
/// - **A busy timeout**, because "wait five seconds" is the correct behaviour for a
///   single-user app and "fail instantly" is not.
/// - **`foreign_keys`**, which sqlx does enable per connection — pinned here because the
///   schema genuinely depends on it (`docs/HUNT.md` § `application_events` explains what
///   breaks without it) and a default nobody states is a default nobody notices changing.
pub async fn init_pool(database_url: &str) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(BUSY_TIMEOUT)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

/// Begin a transaction that is going to write, as `BEGIN IMMEDIATE`.
///
/// **Use this, not `pool.begin()`, for anything that will write.** `begin()` issues a
/// *deferred* transaction: it takes a read snapshot on the first SELECT and tries to upgrade to
/// a write lock on the first UPDATE. When two deferred transactions both hold a snapshot and
/// both try to upgrade, one fails **immediately** with `SQLITE_BUSY_SNAPSHOT` (code 517) — the
/// busy handler is deliberately not consulted, because waiting could only deadlock. So the
/// busy timeout above does not help a deferred transaction at all, which is the opposite of
/// what it looks like it does.
///
/// `BEGIN IMMEDIATE` takes the write lock up front, before the first read, so a competing
/// writer *can* wait for it and the busy timeout does its job.
///
/// Measured, not reasoned: eight concurrent proposal decisions failed five times with
/// `database is locked` under `begin()` in WAL mode, and zero times under this.
pub async fn begin_write(pool: &SqlitePool) -> Result<Transaction<'static, Sqlite>, sqlx::Error> {
    pool.begin_with("BEGIN IMMEDIATE").await
}

/// The compile option `.cargo/config.toml` sets. See `docs/PUBLIC_API_STRESS_TEST.md`, S1.
pub const MEMSTATUS_OFF: &str = "DEFAULT_MEMSTATUS=0";

/// Whether the linked SQLite was built without its process-wide memory-statistics lock.
///
/// A property of the library rather than of any database, so any connection answers it.
/// `main` checks it at startup because a build that lost the flag compiles cleanly, and the
/// deploy does not run the test that would catch it.
pub async fn sqlite_memstatus_disabled(pool: &SqlitePool) -> sqlx::Result<bool> {
    let options: Vec<String> = sqlx::query_scalar("PRAGMA compile_options")
        .fetch_all(pool)
        .await?;
    Ok(options.iter().any(|option| option == MEMSTATUS_OFF))
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    /// SQLite keeps memory-usage statistics under **one process-wide mutex**, taken on every
    /// `sqlite3_malloc` and `sqlite3_free`, and shared by every connection in the pool.
    ///
    /// Stress-tested 2026-10-21: sqlx copies every column of every row out with
    /// `sqlite3_value_dup`, so one public listing request is ~90,000 allocations and as many
    /// frees, all through that lock. Under concurrency the listing *lost* throughput — 38
    /// requests a second for one client, ~21 for any number above four — while burning eight
    /// cores, and 99% of the lock waits in a profile were inside SQLite's allocator.
    /// `-DSQLITE_DEFAULT_MEMSTATUS=0` removes the lock; measured, it took the listing from
    /// ~21 to ~290 requests a second. SQLite's own "Recommended Compile-time Options" lists it
    /// for exactly this reason.
    ///
    /// The flag is a build setting (`.cargo/config.toml` at the repo root), so nothing in the
    /// code would notice it going missing — a build run from a directory that does not see
    /// that file would compile, pass every other test, and ship the collapse. This is the
    /// check that notices. `compile_options` is a property of the linked library, not of any
    /// one database, so an in-memory connection is enough.
    #[tokio::test]
    async fn sqlite_is_built_without_its_global_memory_statistics_lock() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory database");

        let options: Vec<String> = sqlx::query_scalar("PRAGMA compile_options")
            .fetch_all(&pool)
            .await
            .expect("compile_options");

        assert!(
            options.iter().any(|option| option == super::MEMSTATUS_OFF),
            "SQLite was built with memory statistics on, which serializes every allocation \
             across the whole process. Is LIBSQLITE3_FLAGS reaching libsqlite3-sys? \
             compile_options: {options:?}"
        );
    }
}
