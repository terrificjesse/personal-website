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
