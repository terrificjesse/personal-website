//! Ingests blog posts from markdown files committed to the repo.
//!
//! A `.md` file under `content/blog/` becomes a row in `blog_posts` with `source = 'file'`.
//! Putting them in the same table as browser-authored posts is the whole design: `list_posts`
//! is then **one query** over one table, so `?sort=` and `?q=` cover both kinds identically
//! and no endpoint has to merge two stores or reconcile two orderings.
//!
//! The sync **mirrors** the directory rather than importing from it — a file that disappears
//! takes its row with it. Every write here is scoped to `source = 'file'`; a browser-authored
//! row is never created, updated, or deleted by this module.
//!
//! Two things that look like details but are load-bearing:
//!
//! - **A file's slug comes from its filename, not its title.** `docs/BLOG.md` requires that a
//!   published URL survive a title edit, and the filename is the only identity a file has that
//!   an edit to its contents doesn't change. Frontmatter `slug` overrides it for the case
//!   where you want a URL that differs from the filename.
//! - **`created_at` comes from frontmatter `date`, never the file's mtime.** mtime is reset by
//!   `git clone` and `git checkout`, so sorting by it would silently reshuffle the blog on a
//!   fresh checkout.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::{BLOG_SOURCE_FILE, MAX_BLOG_BODY_LENGTH, MAX_BLOG_TITLE_LENGTH, slugify};

/// Where the markdown lives, relative to the repo root.
const DEFAULT_CONTENT_SUBPATH: &str = "content/blog";

/// How often the watcher re-checks the directory when `BLOG_SYNC_INTERVAL_SECS` is unset.
/// Short enough that saving a file and switching to the browser feels immediate, long enough
/// that the idle cost is nothing.
const DEFAULT_SYNC_INTERVAL_SECS: u64 = 5;

/// What one sync did. Returned by `POST /blog/sync` and logged at startup.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct SyncReport {
    pub created: usize,
    pub updated: usize,
    /// Rows removed because their file is no longer on disk.
    pub deleted: usize,
    /// Files that could not be ingested — unparseable, or their slug is taken by a
    /// browser-authored post. Each one is also logged with its reason.
    pub skipped: usize,
}

/// The metadata block at the top of a post file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontMatter {
    pub title: String,
    pub date: DateTime<Utc>,
    pub published: bool,
    /// Overrides the filename-derived slug when present.
    pub slug: Option<String>,
}

/// The directory to read posts from.
///
/// `include_str!` — the pattern `foodkeeper.rs` and `themealdb.rs` use for their vendored data
/// — cannot work here: it needs a literal, fixed file list at compile time, and the whole
/// point is that you can add a file without touching the code. So this is a runtime read, and
/// that makes the working directory matter: the backend runs from
/// `apps/fridge-app/backend/`, three levels below the repo root. Resolving against
/// `CARGO_MANIFEST_DIR` rather than a relative path means `cargo run` works from anywhere with
/// no configuration, while `BLOG_CONTENT_DIR` still overrides it for a real deployment where
/// the source tree isn't next to the binary.
pub fn content_dir() -> PathBuf {
    if let Ok(configured) = std::env::var("BLOG_CONTENT_DIR") {
        return PathBuf::from(configured);
    }

    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(DEFAULT_CONTENT_SUBPATH)
}

/// Splits a post file into its frontmatter and its markdown body.
///
/// The format is the conventional one: a `---` line, `key: value` lines, a closing `---`, then
/// the body. Recognized keys are `title`, `date`, `published`, and `slug`.
///
/// **An unrecognized key is an error, not something to ignore.** A misspelled `pubished: true`
/// that parses "successfully" leaves the post a draft with nothing anywhere saying why, and
/// the author's only symptom is a post that never appears. Failing the file is louder and
/// costs one rename to fix.
pub fn parse_front_matter(text: &str) -> Result<(FrontMatter, String), String> {
    // Tolerate a UTF-8 BOM and leading blank lines before the opening fence — both are things
    // an editor can add without the author ever seeing them.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .trim_start_matches(['\n', '\r'])
        .strip_prefix("---")
        .ok_or("missing opening `---` frontmatter fence")?
        .trim_start_matches('\r')
        .strip_prefix('\n')
        .ok_or("the opening `---` must be alone on its own line")?;

    let mut title: Option<String> = None;
    let mut date: Option<DateTime<Utc>> = None;
    let mut published = false;
    let mut slug: Option<String> = None;
    let mut closed = false;
    let mut body_offset = rest.len();

    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        // Offset arithmetic on `split_inclusive` slices: each line's start is its pointer
        // offset from `rest`, so the body begins just past the closing fence.
        let line_start = line.as_ptr() as usize - rest.as_ptr() as usize;

        if trimmed.trim() == "---" {
            closed = true;
            body_offset = line_start + line.len();
            break;
        }

        // Blank lines and `#` comments inside the block are ignorable; a bare word is not.
        if trimmed.trim().is_empty() || trimmed.trim_start().starts_with('#') {
            continue;
        }

        let (key, value) = trimmed
            .split_once(':')
            .ok_or_else(|| format!("frontmatter line is not `key: value`: {trimmed:?}"))?;
        let key = key.trim();
        let value = unquote(value.trim());

        match key {
            "title" => title = Some(value.to_string()),
            "slug" => slug = Some(value.to_string()),
            "date" => date = Some(parse_date(value)?),
            "published" => {
                published = match value {
                    "true" | "yes" => true,
                    "false" | "no" => false,
                    other => {
                        return Err(format!("`published` must be true or false, got {other:?}"));
                    }
                }
            }
            other => {
                return Err(format!(
                    "unknown frontmatter key {other:?} (expected title, date, published, or slug)"
                ));
            }
        }
    }

    if !closed {
        return Err("missing closing `---` frontmatter fence".to_string());
    }

    let title = title.ok_or("frontmatter is missing required key `title`")?;
    let date = date.ok_or("frontmatter is missing required key `date`")?;

    if title.trim().is_empty() {
        return Err("`title` is empty".to_string());
    }

    // The body starts after the closing fence; drop the blank line authors put there.
    let body = rest[body_offset..]
        .trim_start_matches(['\n', '\r'])
        .to_string();

    Ok((
        FrontMatter {
            title: title.trim().to_string(),
            date,
            published,
            slug: slug.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        },
        body,
    ))
}

/// Strips one layer of matching quotes, so `title: "Hello: World"` keeps its colon.
fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return &value[1..value.len() - 1];
        }
    }
    value
}

/// Accepts a full RFC 3339 timestamp or a bare `YYYY-MM-DD`, which is what anyone actually
/// types. A bare date is taken as midnight UTC.
fn parse_date(value: &str) -> Result<DateTime<Utc>, String> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Ok(parsed.with_timezone(&Utc));
    }

    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| format!("`date` must be YYYY-MM-DD or an RFC 3339 timestamp, got {value:?}"))
        .map(|naive| {
            naive
                .and_hms_opt(0, 0, 0)
                .expect("midnight always exists")
                .and_utc()
        })
}

/// A cheap summary of the content directory: one entry per post file, as
/// `(filename, modified-time-nanos, byte-length)`, sorted.
///
/// `None` means the directory does not exist, which is deliberately distinct from `Some(vec![])`
/// (it exists and is empty) — creating the directory is itself a change worth syncing.
type DirFingerprint = Option<Vec<(String, u128, u64)>>;

/// Summarizes the content directory without reading a single file's contents.
///
/// This is what makes polling cheap enough to do every few seconds: one `read_dir` plus a
/// `stat` per file, no file reads, no database round-trip, and no allocation beyond the
/// filenames. Only when the result differs from the previous tick does anything else happen.
///
/// Using mtime is safe *here* even though `sync` deliberately refuses to use it for
/// `created_at`. There, a `git checkout` resetting mtimes would produce a wrong ordering;
/// here it produces at most one extra sync, which finds nothing changed and does nothing.
/// **False positives cost a no-op; false negatives would cost a missed post** — so the
/// comparison is tuned to err toward syncing.
fn fingerprint(dir: &Path) -> DirFingerprint {
    let entries = std::fs::read_dir(dir).ok()?;

    let mut summary: Vec<(String, u128, u64)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_post_file(path))
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?.to_string();
            let meta = std::fs::metadata(&path).ok()?;
            // An unreadable mtime falls back to 0 rather than dropping the file: a file whose
            // timestamp can't be read should still register as present.
            let modified = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_nanos());

            Some((name, modified, meta.len()))
        })
        .collect();

    summary.sort();
    Some(summary)
}

/// How often to re-check the content directory, from `BLOG_SYNC_INTERVAL_SECS`.
///
/// `None` disables the watcher entirely — either by setting the variable to `0`, or by giving
/// it something unparseable, which is reported rather than silently treated as the default.
fn sync_interval() -> Option<Duration> {
    parse_interval(std::env::var("BLOG_SYNC_INTERVAL_SECS").ok().as_deref())
}

/// The parsing half of `sync_interval`, split out so it can be tested — mutating process
/// environment variables from a test is racy against every other test in the binary.
fn parse_interval(configured: Option<&str>) -> Option<Duration> {
    let Some(configured) = configured else {
        return Some(Duration::from_secs(DEFAULT_SYNC_INTERVAL_SECS));
    };

    match configured.trim().parse::<u64>() {
        Ok(0) => None,
        Ok(seconds) => Some(Duration::from_secs(seconds)),
        Err(_) => {
            eprintln!(
                "blog sync: BLOG_SYNC_INTERVAL_SECS={configured:?} is not a number — \
                 auto-sync disabled, use 0 to disable deliberately"
            );
            None
        }
    }
}

/// Watches the content directory in the background and syncs when it changes.
///
/// Call **after** the startup sync: the initial fingerprint is taken here, so whatever startup
/// already ingested doesn't immediately re-trigger.
///
/// Nothing is logged on a quiet tick. That matters more than it looks — at a few seconds per
/// tick, a watcher that logged every poll would bury every other line the backend prints.
pub fn spawn_watcher(pool: SqlitePool) {
    let Some(interval) = sync_interval() else {
        println!("blog sync: auto-sync disabled — startup and POST /blog/sync only");
        return;
    };

    let dir = content_dir();
    println!(
        "blog sync: watching {} every {}s",
        dir.display(),
        interval.as_secs()
    );

    tokio::spawn(async move {
        let mut previous = fingerprint(&dir);
        let mut ticker = tokio::time::interval(interval);
        // The first tick of a tokio interval completes immediately; consume it so the first
        // real check happens one interval from now rather than instantly.
        ticker.tick().await;

        loop {
            ticker.tick().await;

            let current = fingerprint(&dir);
            if current == previous {
                continue;
            }
            previous = current;

            match sync(&pool).await {
                Ok(report) if report == SyncReport::default() => {}
                Ok(report) => println!(
                    "blog sync: {} created, {} updated, {} deleted, {} skipped",
                    report.created, report.updated, report.deleted, report.skipped
                ),
                // Logged and swallowed: a failing sync must not kill the watcher, or one
                // transient database error would silently end auto-sync for the whole run.
                Err(err) => eprintln!("blog sync failed: {err:?}"),
            }
        }
    });
}

/// Whether a path is one of the files `sync` turns into a post.
///
/// Shared by `read_dir_posts` and `fingerprint` on purpose: if the watcher's idea of which
/// files matter drifted from the reader's, you would get either changes that never trigger a
/// sync or a sync that runs forever on a file it then ignores.
fn is_post_file(path: &Path) -> bool {
    if path.extension().is_none_or(|ext| ext != "md") {
        return false;
    }

    // `README.md` documents the directory; it is not a post.
    !path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("README"))
}

/// One post read off disk, ready to be written to the database.
struct FilePost {
    slug: String,
    front_matter: FrontMatter,
    body: String,
}

/// Reads and validates every `.md` file in `dir`. Returns the posts that parsed, plus the
/// number that didn't — a bad file is skipped and logged, never fatal, so one malformed post
/// can't stop the other twenty from publishing.
fn read_dir_posts(dir: &Path) -> std::io::Result<(Vec<FilePost>, usize)> {
    let mut posts = Vec::new();
    let mut skipped = 0;

    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_post_file(path))
        .collect();
    // Sorted so the log output and the id-assignment order are stable between runs; the
    // filesystem makes no ordering promise.
    paths.sort();

    for path in paths {
        let display = path.display();

        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(stem) => stem,
            None => {
                eprintln!("blog sync: skipping {display} — filename is not valid UTF-8");
                skipped += 1;
                continue;
            }
        };

        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("blog sync: skipping {display} — could not read it: {err}");
                skipped += 1;
                continue;
            }
        };

        let (front_matter, body) = match parse_front_matter(&text) {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("blog sync: skipping {display} — {err}");
                skipped += 1;
                continue;
            }
        };

        let slug = match &front_matter.slug {
            Some(explicit) => slugify(explicit),
            None => slugify(stem),
        };
        if slug.is_empty() {
            eprintln!("blog sync: skipping {display} — filename produces an empty slug");
            skipped += 1;
            continue;
        }

        // The same limits `create_post` enforces on the API. A file bypasses that handler, so
        // without this a 200k-character post would reach a column the rest of the app assumes
        // is bounded.
        if front_matter.title.len() > MAX_BLOG_TITLE_LENGTH {
            eprintln!(
                "blog sync: skipping {display} — title exceeds {MAX_BLOG_TITLE_LENGTH} characters"
            );
            skipped += 1;
            continue;
        }
        if body.trim().is_empty() {
            eprintln!("blog sync: skipping {display} — body is empty");
            skipped += 1;
            continue;
        }
        if body.len() > MAX_BLOG_BODY_LENGTH {
            eprintln!(
                "blog sync: skipping {display} — body exceeds {MAX_BLOG_BODY_LENGTH} characters"
            );
            skipped += 1;
            continue;
        }

        posts.push(FilePost {
            slug,
            front_matter,
            body,
        });
    }

    Ok((posts, skipped))
}

/// Reconciles `blog_posts` with the markdown files on disk.
///
/// Runs at startup and from `POST /blog/sync`. Never fatal: a missing directory, an
/// unreadable file, or a database with no admin account yet all return a report rather than an
/// error, because the blog falling back to database-only posts should not stop the backend
/// from serving the fridge app.
pub async fn sync(pool: &SqlitePool) -> Result<SyncReport, sqlx::Error> {
    let dir = content_dir();

    let (file_posts, unparseable) = match read_dir_posts(&dir) {
        Ok(result) => result,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "blog sync: no content directory at {} — database posts only",
                dir.display()
            );
            return Ok(SyncReport::default());
        }
        Err(err) => {
            eprintln!("blog sync: could not read {}: {err}", dir.display());
            return Ok(SyncReport::default());
        }
    };

    // `blog_posts.author_id` is NOT NULL REFERENCES users(id) and a file carries no author.
    // The first-registered admin is the honest answer: the file is in their repo, committed by
    // them. On a fresh database there is no such row yet, and inserting would violate the
    // foreign key — so skip the whole sync and say why, rather than panicking during boot.
    let author_id: Option<String> =
        sqlx::query_scalar("SELECT id FROM users WHERE is_admin = 1 ORDER BY id LIMIT 1")
            .fetch_optional(pool)
            .await?;

    // Files that failed to parse are already counted, even on the paths below that do no
    // database work at all — they are a fact about the directory, not about the sync.
    let mut report = SyncReport {
        skipped: unparseable,
        ..SyncReport::default()
    };

    let Some(author_id) = author_id else {
        if !file_posts.is_empty() {
            println!(
                "blog sync: {} file(s) waiting in {}, but no admin account exists yet to own them \
                 — grant is_admin and re-sync",
                file_posts.len(),
                dir.display()
            );
        }
        return Ok(report);
    };

    let mut seen_slugs: HashSet<String> = HashSet::new();

    for post in &file_posts {
        // Two files resolving to the same slug: first wins, and the second is reported rather
        // than silently overwriting the first within a single run.
        if !seen_slugs.insert(post.slug.clone()) {
            eprintln!(
                "blog sync: skipping a second file claiming slug {:?} — rename one of them",
                post.slug
            );
            report.skipped += 1;
            continue;
        }

        let existing: Option<(String, String)> =
            sqlx::query_as("SELECT id, source FROM blog_posts WHERE slug = ?")
                .bind(&post.slug)
                .fetch_optional(pool)
                .await?;

        match existing {
            // The slug already belongs to a post written in the browser. Taking it would
            // repoint a URL that may already be published, at content nobody linked to — the
            // exact thing the never-rewrite-a-slug rule exists to prevent. Leave it alone.
            Some((_, source)) if source != BLOG_SOURCE_FILE => {
                eprintln!(
                    "blog sync: skipping slug {:?} — a {source}-sourced post already owns it; \
                     rename the file or the post",
                    post.slug
                );
                report.skipped += 1;
            }
            Some((id, _)) => {
                // `updated_at` moves only when something actually changed, so re-syncing an
                // untouched directory doesn't rewrite every timestamp.
                let changed = sqlx::query(
                    "UPDATE blog_posts \
                     SET title = ?, body = ?, published = ?, created_at = ?, updated_at = ? \
                     WHERE id = ? \
                       AND (title <> ? OR body <> ? OR published <> ? OR created_at <> ?)",
                )
                .bind(&post.front_matter.title)
                .bind(&post.body)
                .bind(post.front_matter.published)
                .bind(post.front_matter.date)
                .bind(Utc::now())
                .bind(&id)
                .bind(&post.front_matter.title)
                .bind(&post.body)
                .bind(post.front_matter.published)
                .bind(post.front_matter.date)
                .execute(pool)
                .await?;

                if changed.rows_affected() > 0 {
                    report.updated += 1;
                }
            }
            None => {
                let now = Utc::now();
                sqlx::query(
                    "INSERT INTO blog_posts \
                     (id, author_id, title, slug, body, published, created_at, updated_at, source) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(Uuid::new_v4().to_string())
                .bind(&author_id)
                .bind(&post.front_matter.title)
                .bind(&post.slug)
                .bind(&post.body)
                .bind(post.front_matter.published)
                .bind(post.front_matter.date)
                .bind(now)
                .bind(BLOG_SOURCE_FILE)
                .execute(pool)
                .await?;

                report.created += 1;
            }
        }
    }

    // Mirror, not import: a file removed from the repo removes its post. Scoped to
    // `source = 'file'`, so a browser-authored post is never caught by this.
    let existing_file_slugs: Vec<String> =
        sqlx::query_scalar("SELECT slug FROM blog_posts WHERE source = ?")
            .bind(BLOG_SOURCE_FILE)
            .fetch_all(pool)
            .await?;

    for slug in existing_file_slugs {
        if seen_slugs.contains(&slug) {
            continue;
        }

        sqlx::query("DELETE FROM blog_posts WHERE slug = ? AND source = ?")
            .bind(&slug)
            .bind(BLOG_SOURCE_FILE)
            .execute(pool)
            .await?;

        println!("blog sync: removed {slug:?} — its file is gone");
        report.deleted += 1;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str =
        "---\ntitle: Hello World\ndate: 2026-08-19\npublished: true\n---\n\nBody text.\n";

    #[test]
    fn parses_a_well_formed_file() {
        let (front, body) = parse_front_matter(GOOD).expect("this is the happy path");

        assert_eq!(front.title, "Hello World");
        assert_eq!(front.date, parse_date("2026-08-19").unwrap());
        assert!(front.published);
        assert_eq!(front.slug, None);
        assert_eq!(body, "Body text.\n");
    }

    #[test]
    fn published_defaults_to_false_when_absent() {
        let text = "---\ntitle: Draft\ndate: 2026-08-19\n---\nstuff\n";
        let (front, _) = parse_front_matter(text).unwrap();

        // Matches `CreateBlogPostRequest`: publishing is always deliberate, never a default.
        assert!(!front.published);
    }

    #[test]
    fn a_missing_fence_is_an_error() {
        assert!(parse_front_matter("title: Hello\n\nBody\n").is_err());
        assert!(parse_front_matter("---\ntitle: Hello\ndate: 2026-08-19\n\nBody\n").is_err());
    }

    #[test]
    fn a_missing_required_key_is_an_error() {
        assert!(parse_front_matter("---\ndate: 2026-08-19\n---\nBody\n").is_err());
        assert!(parse_front_matter("---\ntitle: Hello\n---\nBody\n").is_err());
    }

    /// The reason unknown keys are rejected: a typo would otherwise leave the post a draft
    /// forever with no diagnostic anywhere.
    #[test]
    fn an_unknown_key_is_an_error_rather_than_ignored() {
        let typo = "---\ntitle: Hello\ndate: 2026-08-19\npubished: true\n---\nBody\n";
        let err = parse_front_matter(typo).expect_err("a misspelled key must not parse");

        assert!(
            err.contains("pubished"),
            "the error should name the key: {err}"
        );
    }

    /// A `---` horizontal rule in the body must not be mistaken for the closing fence — the
    /// scan stops at the first one, so this only works because the body is never rescanned.
    #[test]
    fn a_horizontal_rule_in_the_body_survives() {
        let text = "---\ntitle: Hello\ndate: 2026-08-19\n---\n\nAbove.\n\n---\n\nBelow.\n";
        let (front, body) = parse_front_matter(text).unwrap();

        assert_eq!(front.title, "Hello");
        assert!(
            body.contains("---"),
            "the rule should still be in the body: {body:?}"
        );
        assert!(body.starts_with("Above."));
        assert!(body.trim_end().ends_with("Below."));
    }

    #[test]
    fn a_date_may_be_a_bare_day_or_a_full_timestamp() {
        let bare = parse_date("2026-08-19").unwrap();
        let full = parse_date("2026-08-19T00:00:00Z").unwrap();

        assert_eq!(bare, full);
        assert!(parse_date("19-08-2026").is_err());
        assert!(parse_date("not a date").is_err());
    }

    #[test]
    fn a_quoted_title_keeps_its_colon() {
        let text = "---\ntitle: \"Rust: a love story\"\ndate: 2026-08-19\n---\nBody\n";
        let (front, _) = parse_front_matter(text).unwrap();

        assert_eq!(front.title, "Rust: a love story");
    }

    #[test]
    fn an_explicit_slug_is_read_and_an_empty_one_is_not() {
        let with = "---\ntitle: T\ndate: 2026-08-19\nslug: custom-url\n---\nBody\n";
        assert_eq!(
            parse_front_matter(with).unwrap().0.slug,
            Some("custom-url".to_string())
        );

        let empty = "---\ntitle: T\ndate: 2026-08-19\nslug:\n---\nBody\n";
        assert_eq!(parse_front_matter(empty).unwrap().0.slug, None);
    }

    #[test]
    fn a_non_boolean_published_is_an_error() {
        let text = "---\ntitle: T\ndate: 2026-08-19\npublished: maybe\n---\nBody\n";
        assert!(parse_front_matter(text).is_err());
    }

    /// A scratch directory under the OS temp dir, unique per test.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fridge-blog-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir is creatable");
        dir
    }

    fn write_post(dir: &Path, name: &str, body: &str) {
        std::fs::write(
            dir.join(name),
            format!("---\ntitle: T\ndate: 2026-08-19\n---\n\n{body}\n"),
        )
        .expect("test file is writable");
    }

    #[test]
    fn a_missing_directory_is_distinct_from_an_empty_one() {
        let dir = temp_dir("missing");

        assert_eq!(
            fingerprint(&dir),
            Some(vec![]),
            "an empty directory is Some"
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(fingerprint(&dir), None, "a missing directory is None");

        // The distinction is the point: creating the directory has to register as a change,
        // or the first post added to a fresh checkout would never sync.
        std::fs::create_dir_all(&dir).unwrap();
        assert_ne!(fingerprint(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn adding_removing_and_editing_a_post_all_change_the_fingerprint() {
        let dir = temp_dir("changes");

        let empty = fingerprint(&dir);
        write_post(&dir, "one.md", "first");
        let added = fingerprint(&dir);
        assert_ne!(added, empty, "adding a file must be noticed");

        // Different length, so this is caught even where mtime granularity wouldn't.
        write_post(&dir, "one.md", "first, but meaningfully longer now");
        let edited = fingerprint(&dir);
        assert_ne!(edited, added, "editing a file must be noticed");

        std::fs::remove_file(dir.join("one.md")).unwrap();
        assert_eq!(fingerprint(&dir), empty, "removing it returns to the start");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The quiet-tick guarantee: with nothing touched, consecutive polls must compare equal,
    /// or the watcher would sync (and log) every few seconds forever.
    #[test]
    fn an_untouched_directory_fingerprints_identically() {
        let dir = temp_dir("stable");
        write_post(&dir, "one.md", "body");
        write_post(&dir, "two.md", "body");

        assert_eq!(fingerprint(&dir), fingerprint(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The watcher and the reader must agree on which files matter. A README edit triggering
    /// a sync would be harmless; a post the watcher ignores would be a missed publish.
    #[test]
    fn the_fingerprint_covers_exactly_the_files_sync_reads() {
        let dir = temp_dir("filter");
        write_post(&dir, "real-post.md", "body");
        std::fs::write(dir.join("README.md"), "# docs").unwrap();
        std::fs::write(dir.join("notes.txt"), "not markdown").unwrap();
        std::fs::write(dir.join("draft.md.swp"), "editor swap file").unwrap();

        let names: Vec<String> = fingerprint(&dir)
            .unwrap()
            .into_iter()
            .map(|(name, _, _)| name)
            .collect();
        assert_eq!(names, vec!["real-post.md".to_string()]);

        // ...and that is the same set `read_dir_posts` will actually ingest.
        let (posts, _) = read_dir_posts(&dir).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].slug, "real-post");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_interval_defaults_and_zero_disables() {
        assert_eq!(
            parse_interval(None),
            Some(Duration::from_secs(DEFAULT_SYNC_INTERVAL_SECS))
        );
        assert_eq!(parse_interval(Some("30")), Some(Duration::from_secs(30)));
        assert_eq!(parse_interval(Some(" 30 ")), Some(Duration::from_secs(30)));
        assert_eq!(parse_interval(Some("0")), None, "0 disables the watcher");

        // Garbage disables rather than silently falling back to the default: a typo'd
        // interval that quietly behaves like the default is a setting that looks applied and
        // isn't.
        assert_eq!(parse_interval(Some("five")), None);
        assert_eq!(parse_interval(Some("")), None);
    }

    #[test]
    fn readme_is_not_a_post_but_other_markdown_is() {
        assert!(is_post_file(Path::new("/x/hello.md")));
        assert!(!is_post_file(Path::new("/x/README.md")));
        assert!(!is_post_file(Path::new("/x/readme.md")), "case-insensitive");
        assert!(!is_post_file(Path::new("/x/notes.txt")));
    }

    /// CRLF line endings are what a file edited on Windows arrives with; the parser must not
    /// treat the trailing `\r` as part of the value.
    #[test]
    fn crlf_line_endings_parse() {
        let text = "---\r\ntitle: Hello\r\ndate: 2026-08-19\r\n---\r\nBody\r\n";
        let (front, body) = parse_front_matter(text).unwrap();

        assert_eq!(front.title, "Hello");
        assert_eq!(body.trim_end(), "Body");
    }
}
