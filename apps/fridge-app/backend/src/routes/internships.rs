//! The internship tab's HTTP surface. Phase 7.
//!
//! This file currently carries the **applied tracker**. The ranked-list and run-health
//! endpoints land alongside it once the collection runner is wired.
//!
//! # The rule this file exists to protect
//!
//! An application must survive its posting. Migration `0012` snapshots company/title/URL/pay
//! onto the application row at apply time precisely so the tracker never depends on the
//! posting still being there — see that migration's header comment on
//! `internship_applications`. The invariant, stated so it can be tested:
//!
//! > **The tracker renders correctly from `internship_applications` alone, with zero joins.**
//!
//! The `LEFT JOIN` below is enrichment layered on top, and it is a `LEFT` join for a reason
//! that is easy to get wrong twice over:
//!
//! 1. An `INNER JOIN` drops the application entirely once the posting is gone — trap 1
//!    arriving by the back door after the snapshot was supposed to have closed it.
//! 2. A hard-deleted posting can leave `posting_id` either NULL **or dangling**, and this
//!    code has to handle both.
//!
//!    Correcting the note in migration `0007`: **sqlx turns `PRAGMA foreign_keys` ON per
//!    connection**, so through the application the `REFERENCES` clauses really are enforced
//!    and `ON DELETE SET NULL` does fire. That was proved the hard way — an insert-ordering
//!    bug in `internships::collector` failed with `FOREIGN KEY constraint failed`, which is
//!    impossible if they are off. But the `sqlite3` CLI does *not* enable them, so a delete
//!    performed by hand leaves the column pointing at an id that no longer resolves. Both
//!    states were reproduced against a real database.
//!
//!    (This note lived in migration `0012` briefly and was moved here: **editing an applied
//!    migration changes its checksum and sqlx then refuses to start**, with
//!    `migration 12 was previously applied but has been modified`. A migration is immutable
//!    once it has run anywhere — corrections go in the code or in a new migration.)
//!
//! Together those mean the liveness column cannot be written as `p.expired_at IS NULL`:
//! when the join misses, every column of `p` is NULL, and `NULL IS NULL` is **true** — so a
//! posting that no longer exists would report as *live*. The `CASE` in
//! [`SELECT_APPLICATION_COLUMNS`] tests `p.id IS NULL` first for exactly that reason.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{Datelike, DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use axum::extract::Query;
use serde::{Deserialize, Serialize};

use crate::internships::rank::{
    ClassYearFilter, InternshipFilters, LocationFilter, OnUnknown, PayRangeFilter, RankedPosting,
    SortBy, TermFilter, rank_postings,
};
use crate::internships::store;

use crate::internships::models::{
    Application, Season, ApplicationStatus, CreateApplicationRequest, MAX_APPLICATION_NOTES_LENGTH,
    UpdateApplicationRequest,
};
use crate::routes::auth::{CurrentUser, RequireAdmin};

/// The application columns plus the one derived field.
///
/// `posting_is_live` is three-valued on purpose — see [`Application::posting_is_live`]. The
/// `CASE` is load-bearing: without the `p.id IS NULL` arm, a dangling or absent posting_id
/// yields `NULL IS NULL` = true and every vanished posting reports as open.
const SELECT_APPLICATION_COLUMNS: &str = "
    a.id, a.posting_id, a.company_name, a.title, a.url, a.location_raw,
    a.pay_min, a.pay_max, a.pay_currency, a.pay_period,
    a.term_season, a.term_year, a.source, a.snapshot_at,
    a.status, a.applied_at, a.status_changed_at, a.notes,
    CASE WHEN p.id IS NULL THEN NULL ELSE (p.expired_at IS NULL) END AS posting_is_live";

/// Validates and trims a notes field. `None` and whitespace-only both mean "no notes".
fn clean_notes(notes: Option<&str>) -> Result<Option<String>, StatusCode> {
    let Some(notes) = notes else {
        return Ok(None);
    };
    if notes.len() > MAX_APPLICATION_NOTES_LENGTH {
        return Err(StatusCode::BAD_REQUEST);
    }
    let trimmed = notes.trim();
    Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
}

/// Every application this user has tracked, newest first.
pub async fn list_applications(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<Application>>, StatusCode> {
    let applications = sqlx::query_as::<_, Application>(&format!(
        "SELECT {SELECT_APPLICATION_COLUMNS}
         FROM internship_applications a
         LEFT JOIN internship_postings p ON p.id = a.posting_id
         WHERE a.user_id = ?
         ORDER BY a.applied_at DESC, a.id DESC"
    ))
    .bind(&user.id)
    .fetch_all(&pool)
    .await
    .map_err(|err| {
        eprintln!("internships: listing applications failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(applications))
}

/// Record that the user applied to a posting.
///
/// The snapshot is taken **here, once**, from the posting as it stands at this moment. Nothing
/// later rewrites it: not the collector, not a re-sync, not the expiry sweep. If the company
/// edits the listing afterwards, the tracker keeps showing what was actually applied to, and
/// the live posting stays available through the join for as long as it exists.
pub async fn create_application(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateApplicationRequest>,
) -> Result<(StatusCode, Json<Application>), StatusCode> {
    let status = match req.status.as_deref() {
        None => ApplicationStatus::Applied,
        Some(raw) => ApplicationStatus::parse(raw).ok_or(StatusCode::BAD_REQUEST)?,
    };
    let notes = clean_notes(req.notes.as_deref())?;

    // Read the posting first: it is both the existence check and the snapshot source.
    // Expired postings are deliberately still applicable — you can apply to something the
    // sweep has since closed, and recording that is more useful than refusing it.
    let posting = sqlx::query_as::<_, PostingSnapshot>(
        "SELECT id, company_name, title, canonical_url, location_raw,
                pay_min, pay_max, pay_currency, pay_period, term_season, term_year
         FROM internship_postings WHERE id = ?",
    )
    .bind(&req.posting_id)
    .fetch_optional(&pool)
    .await
    .map_err(|err| {
        eprintln!("internships: reading posting for snapshot failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::NOT_FOUND)?;

    // The source is whichever sighting we saw first — a deduped posting can have several,
    // so this is genuinely "where I found it", not "the only place it exists".
    let source: Option<String> = sqlx::query_scalar(
        "SELECT source FROM posting_sightings
         WHERE posting_id = ? ORDER BY first_seen_at ASC, source ASC LIMIT 1",
    )
    .bind(&posting.id)
    .fetch_optional(&pool)
    .await
    .map_err(|err| {
        eprintln!("internships: reading sighting source failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .flatten();

    // The whole posting record, so a field nobody thought to snapshot is still recoverable.
    let snapshot_json: String = sqlx::query_scalar(
        "SELECT json_object(
            'id', id, 'dedup_key', dedup_key, 'company_key', company_key,
            'company_name', company_name, 'title', title, 'canonical_url', canonical_url,
            'term_season', term_season, 'term_year', term_year,
            'location_raw', location_raw, 'location_city', location_city,
            'location_region', location_region, 'location_country', location_country,
            'is_remote', is_remote,
            'pay_min', pay_min, 'pay_max', pay_max, 'pay_currency', pay_currency,
            'pay_period', pay_period, 'pay_raw', pay_raw,
            'class_year_min', class_year_min, 'class_year_max', class_year_max,
            'class_year_raw', class_year_raw,
            'posted_at', posted_at, 'deadline', deadline,
            'first_seen_at', first_seen_at, 'last_seen_at', last_seen_at,
            'expired_at', expired_at, 'expiry_reason', expiry_reason
         ) FROM internship_postings WHERE id = ?",
    )
    .bind(&posting.id)
    .fetch_one(&pool)
    .await
    .map_err(|err| {
        eprintln!("internships: building snapshot json failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let id = Uuid::new_v4().to_string();
    let now = Utc::now();

    let result = sqlx::query(
        "INSERT INTO internship_applications
            (id, user_id, posting_id,
             company_name, title, url, location_raw,
             pay_min, pay_max, pay_currency, pay_period,
             term_season, term_year, source, snapshot_json, snapshot_at,
             status, applied_at, status_changed_at, notes, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                 ?17, ?16, ?16, ?18, ?16, ?16)",
    )
    .bind(&id)
    .bind(&user.id)
    .bind(&posting.id)
    .bind(&posting.company_name)
    .bind(&posting.title)
    .bind(&posting.canonical_url)
    .bind(&posting.location_raw)
    .bind(posting.pay_min)
    .bind(posting.pay_max)
    .bind(&posting.pay_currency)
    .bind(&posting.pay_period)
    .bind(&posting.term_season)
    .bind(posting.term_year)
    .bind(&source)
    .bind(&snapshot_json)
    .bind(now)
    .bind(status.as_str())
    .bind(&notes)
    .execute(&pool)
    .await;

    if let Err(err) = result {
        // The UNIQUE (user_id, posting_id) index is what makes "already applied" a database
        // guarantee rather than a check-then-insert race. 409 rather than 400: the request is
        // well-formed and conflicts with existing state.
        if let Some(db_err) = err.as_database_error()
            && db_err.is_unique_violation()
        {
            return Err(StatusCode::CONFLICT);
        }
        eprintln!("internships: inserting application failed: {err:?}");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let created = fetch_application(&pool, &user.id, &id).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// Move an application along, or edit its notes.
pub async fn update_application(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateApplicationRequest>,
) -> Result<Json<Application>, StatusCode> {
    // Scoped to the caller, and a miss is 404 rather than 403 — answering "forbidden" would
    // confirm that somebody else's application exists under that id.
    let existing = fetch_application(&pool, &user.id, &id).await?;

    if req.status.is_none() && req.notes.is_none() {
        return Ok(Json(existing));
    }

    let now = Utc::now();

    if let Some(raw) = req.status.as_deref() {
        let status = ApplicationStatus::parse(raw).ok_or(StatusCode::BAD_REQUEST)?;

        // `status_changed_at` moves only when the status genuinely differs. Bumping it on a
        // no-op write would destroy the one thing it is for — "how long have I been sitting
        // at this stage" — every time the notes were edited.
        let changed = status.as_str() != existing.status;
        sqlx::query(
            "UPDATE internship_applications
             SET status = ?1,
                 status_changed_at = CASE WHEN ?2 THEN ?3 ELSE status_changed_at END,
                 updated_at = ?3
             WHERE id = ?4 AND user_id = ?5",
        )
        .bind(status.as_str())
        .bind(changed)
        .bind(now)
        .bind(&id)
        .bind(&user.id)
        .execute(&pool)
        .await
        .map_err(|err| {
            eprintln!("internships: updating application status failed: {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    if let Some(raw) = req.notes.as_deref() {
        let notes = clean_notes(Some(raw))?;
        sqlx::query(
            "UPDATE internship_applications SET notes = ?1, updated_at = ?2
             WHERE id = ?3 AND user_id = ?4",
        )
        .bind(&notes)
        .bind(now)
        .bind(&id)
        .bind(&user.id)
        .execute(&pool)
        .await
        .map_err(|err| {
            eprintln!("internships: updating application notes failed: {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    Ok(Json(fetch_application(&pool, &user.id, &id).await?))
}

/// Stop tracking an application.
///
/// This is a real delete, unlike posting expiry — the row is the user's own record and they
/// are asking for it gone. Scoped by `user_id` so one account cannot delete another's.
pub async fn delete_application(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let deleted = sqlx::query("DELETE FROM internship_applications WHERE id = ? AND user_id = ?")
        .bind(&id)
        .bind(&user.id)
        .execute(&pool)
        .await
        .map_err(|err| {
            eprintln!("internships: deleting application failed: {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .rows_affected();

    if deleted == 0 {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// One application belonging to this user, or 404.
async fn fetch_application(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
) -> Result<Application, StatusCode> {
    sqlx::query_as::<_, Application>(&format!(
        "SELECT {SELECT_APPLICATION_COLUMNS}
         FROM internship_applications a
         LEFT JOIN internship_postings p ON p.id = a.posting_id
         WHERE a.id = ? AND a.user_id = ?"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| {
        eprintln!("internships: fetching application failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::NOT_FOUND)
}

/// Just the posting fields the snapshot copies. A narrow struct rather than the full
/// `Posting`, so it is obvious at the call site exactly what gets frozen onto the row.
#[derive(Debug, Clone, sqlx::FromRow)]
struct PostingSnapshot {
    id: String,
    company_name: String,
    title: String,
    canonical_url: String,
    location_raw: Option<String>,
    pay_min: Option<f64>,
    pay_max: Option<f64>,
    pay_currency: Option<String>,
    pay_period: Option<String>,
    term_season: Option<String>,
    term_year: Option<i64>,
}

// ------------------------------------------------------------------------------------------
// Filter input: interpreting what the user typed
// ------------------------------------------------------------------------------------------

/// The month an academic year is taken to begin. August, so a filter used in the autumn talks
/// about the year the student is actually in rather than the one they just finished.
const ACADEMIC_YEAR_START_MONTH: u32 = 8;

/// The academic year a date falls in, named by the calendar year it *started*.
///
/// August through December belong to the year that just began; January through July belong to
/// the one that started the previous August. Without this, every filter used between January
/// and July would be a year out — a junior in March 2027 has been a junior since August 2026.
fn academic_year_start(now: DateTime<Utc>) -> i64 {
    let year = i64::from(now.year());
    if now.month() >= ACADEMIC_YEAR_START_MONTH {
        year
    } else {
        year - 1
    }
}

/// Turn a year of study into the graduation year it implies.
///
/// A user filtering internships thinks in "I'm a sophomore", while postings — on the rare
/// occasions they say anything — state graduation years. This is the translation, and it is
/// the *only* place it happens.
///
/// Assumes a four-year program finishing in the spring, which is the common case and is
/// stated here rather than buried: a sophomore in the academic year starting `Y` graduates in
/// `Y + 3`, because they have this year plus two more.
fn graduation_year_for_study_year(word: &str, now: DateTime<Utc>) -> Option<i64> {
    let start = academic_year_start(now);
    let years_remaining = match word {
        "freshman" | "first-year" | "firstyear" => 4,
        "sophomore" | "second-year" | "secondyear" => 3,
        "junior" | "third-year" | "thirdyear" => 2,
        "senior" | "fourth-year" | "fourthyear" => 1,
        _ => return None,
    };
    Some(start + years_remaining)
}

/// Parse the `class_year` query parameter, which accepts either form:
///
/// - a literal graduation year — `class_year=2029`
/// - a year of study — `class_year=sophomore`
///
/// Both are offered because both are natural. Somebody who knows their graduation year should
/// not have to convert it into a word, and somebody thinking "I'm a sophomore" should not have
/// to do arithmetic that depends on what month it is.
///
/// Returns `None` for anything else, so the route can answer 400 rather than silently
/// filtering on a year the user never asked for.
fn parse_class_year(value: &str, now: DateTime<Utc>) -> Option<i64> {
    let value = value.trim().to_lowercase();

    // A bare number is a graduation year. Bounded so a typo'd `class_year=20299` is a 400
    // rather than a filter that quietly matches nothing.
    if let Ok(year) = value.parse::<i64>() {
        let current = academic_year_start(now);
        return (current - 10..=current + 15).contains(&year).then_some(year);
    }

    graduation_year_for_study_year(&value, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap()
    }

    #[test]
    fn a_sophomore_in_the_autumn_graduates_three_years_later() {
        // August 2026: the 2026-27 academic year has begun. A sophomore has this year plus
        // two more.
        assert_eq!(parse_class_year("sophomore", at(2026, 8, 20)), Some(2029));
    }

    #[test]
    fn a_senior_graduates_at_the_end_of_the_current_academic_year() {
        assert_eq!(parse_class_year("senior", at(2026, 8, 20)), Some(2027));
    }

    #[test]
    fn the_same_student_gets_the_same_answer_in_the_spring() {
        // The bug this pins: in March 2027 a sophomore is still in the 2026-27 academic year
        // and still graduates 2029. Reading the calendar year instead would say 2030 and
        // shift every filter by one, silently.
        assert_eq!(parse_class_year("sophomore", at(2026, 10, 1)), Some(2029));
        assert_eq!(parse_class_year("sophomore", at(2027, 3, 15)), Some(2029));
        assert_eq!(parse_class_year("sophomore", at(2027, 7, 31)), Some(2029));
    }

    #[test]
    fn the_academic_year_rolls_over_exactly_on_the_first_of_august() {
        // An explicit threshold, tested *on* the boundary rather than either side of it --
        // the repo has already lost a whole rating band to a `>` that should have been `>=`.
        assert_eq!(academic_year_start(at(2026, 7, 31)), 2025);
        assert_eq!(academic_year_start(at(2026, 8, 1)), 2026);
    }

    #[test]
    fn every_study_year_is_one_year_apart() {
        let now = at(2026, 8, 20);
        assert_eq!(parse_class_year("freshman", now), Some(2030));
        assert_eq!(parse_class_year("sophomore", now), Some(2029));
        assert_eq!(parse_class_year("junior", now), Some(2028));
        assert_eq!(parse_class_year("senior", now), Some(2027));
    }

    #[test]
    fn a_literal_graduation_year_passes_through_untouched() {
        assert_eq!(parse_class_year("2029", at(2026, 8, 20)), Some(2029));
        assert_eq!(parse_class_year(" 2029 ", at(2026, 8, 20)), Some(2029));
    }

    #[test]
    fn study_years_are_case_and_spelling_tolerant_within_reason() {
        let now = at(2026, 8, 20);
        assert_eq!(parse_class_year("Sophomore", now), Some(2029));
        assert_eq!(parse_class_year("SECOND-YEAR", now), Some(2029));
    }

    #[test]
    fn nonsense_is_rejected_rather_than_defaulted() {
        // Each of these must be a 400 at the route. Defaulting any of them to "this year"
        // would filter on a class year the user never asked for and never see it.
        let now = at(2026, 8, 20);
        assert_eq!(parse_class_year("", now), None);
        assert_eq!(parse_class_year("sophmore", now), None);
        assert_eq!(parse_class_year("grad student", now), None);
        assert_eq!(parse_class_year("20299", now), None, "typo'd year");
        assert_eq!(parse_class_year("1998", now), None, "implausibly past");
    }
}

// ------------------------------------------------------------------------------------------
// Run-health panel
// ------------------------------------------------------------------------------------------

/// One source's outcome within one collection run.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SourceRunSummary {
    pub run_id: String,
    pub source: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub outcome: String,
    pub fetched_count: i64,
    pub accepted_count: i64,
    pub filtered_count: i64,
    pub rejected_count: i64,
    /// Whether this run was allowed to advance disappearance counters. Surfaced rather than
    /// hidden because "this source succeeded but still expired nothing" is a state a human
    /// needs to be able to see and understand, not a silent internal detail.
    pub counts_for_expiry: bool,
    pub error: Option<String>,
}

/// A per-source rollup across all runs — the actual content of the health panel.
///
/// The panel exists because **a source can break quietly**: it stops being attempted, or it
/// starts returning zero, and nothing anywhere says so. A list of recent runs alone does not
/// surface that, because a source that has stopped running simply *isn't in the list*, and
/// absence is exactly what a human eye slides over. So this is computed per known source,
/// including sources that appear in no recent run at all.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SourceHealth {
    pub source: String,
    /// The outcome of this source's most recent run, whenever that was.
    pub last_outcome: String,
    pub last_run_at: DateTime<Utc>,
    /// When it last completed a **full** enumeration. `None` means it has never had one —
    /// which for a source that has been running a while is the loudest signal on this panel.
    pub last_success_at: Option<DateTime<Utc>>,
    /// How many postings it accepted on its most recent run.
    pub last_accepted: i64,
    /// Consecutive non-successful runs, most recent first. `0` means the last run succeeded.
    pub consecutive_failures: i64,
    /// Live postings currently attributable to this source.
    pub live_postings: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunHealthResponse {
    pub runs: Vec<CollectionRunSummary>,
    pub sources: Vec<SourceHealth>,
    /// Present only while a run is actually in flight. See [`current_progress`].
    pub in_progress: Option<RunProgress>,
}

/// A collection run that has started and not yet finished.
///
/// This exists because a running scrape was previously indistinguishable from a broken one:
/// the tab was empty, the health panel was empty, and nothing said whether anything was
/// happening. `sources_done` climbs as each source lands, which is only meaningful because the
/// coordinator persists per-source rather than batching — see `internships::collector`.
#[derive(Debug, Clone, Serialize)]
pub struct RunProgress {
    pub run_id: String,
    pub started_at: DateTime<Utc>,
    pub trigger: String,
    /// Sources that have finished and been recorded so far.
    pub sources_done: i64,
    /// How many there are in total, from the registry.
    pub sources_total: usize,
    /// Postings accepted so far in this run — visible progress rather than a spinner.
    pub postings_so_far: i64,
}

/// The in-flight run, if there is one.
///
/// A run is in flight when its `collection_runs` row has no `finished_at`. Note this is also
/// true of a run whose process died mid-way, which is deliberate: a run that never finished is
/// a real thing to surface, and the alternative — treating it as complete — would hide it.
async fn current_progress(pool: &SqlitePool) -> Result<Option<RunProgress>, StatusCode> {
    let row = sqlx::query_as::<_, InFlightRow>(
        "SELECT id, started_at, trigger FROM collection_runs
         WHERE finished_at IS NULL ORDER BY started_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(internal("reading the in-flight run"))?;

    let Some(row) = row else {
        return Ok(None);
    };

    let sources_done: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM source_runs WHERE run_id = ?")
            .bind(&row.id)
            .fetch_one(pool)
            .await
            .map_err(internal("counting finished sources"))?;

    let postings_so_far: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(accepted_count), 0) FROM source_runs WHERE run_id = ?")
            .bind(&row.id)
            .fetch_one(pool)
            .await
            .map_err(internal("counting accepted postings"))?;

    Ok(Some(RunProgress {
        run_id: row.id,
        started_at: row.started_at,
        trigger: row.trigger,
        sources_done,
        sources_total: crate::internships::sources::registry().len(),
        postings_so_far,
    }))
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct InFlightRow {
    id: String,
    started_at: DateTime<Utc>,
    trigger: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionRunSummary {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub trigger: String,
    /// The process running this died before it finished. Reconciled at the next startup —
    /// see `collector::reconcile_interrupted_runs`. Worth showing: a source that keeps being
    /// interrupted is a real signal, and it explains a gap in the data.
    pub interrupted: bool,
    pub sources: Vec<SourceRunSummary>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct CollectionRunRow {
    id: String,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    trigger: String,
    interrupted: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunHealthQuery {
    /// How many recent runs to include. Clamped rather than rejected — an oversized `limit`
    /// is a UI bug, not a reason to fail the request.
    pub limit: Option<u32>,
}

const DEFAULT_RUN_LIMIT: u32 = 10;
const MAX_RUN_LIMIT: u32 = 100;

/// Recent collection runs, plus the per-source rollup that makes a quietly broken source
/// visible.
pub async fn run_health(
    State(pool): State<SqlitePool>,
    CurrentUser(_user): CurrentUser,
    Query(params): Query<RunHealthQuery>,
) -> Result<Json<RunHealthResponse>, StatusCode> {
    let limit = params.limit.unwrap_or(DEFAULT_RUN_LIMIT).min(MAX_RUN_LIMIT);

    let run_rows = sqlx::query_as::<_, CollectionRunRow>(
        "SELECT id, started_at, finished_at, trigger, interrupted
         FROM collection_runs ORDER BY started_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&pool)
    .await
    .map_err(internal("listing collection runs"))?;

    let mut runs = Vec::with_capacity(run_rows.len());
    for run in run_rows {
        let sources = sqlx::query_as::<_, SourceRunSummary>(
            "SELECT run_id, source, started_at, finished_at, outcome,
                    fetched_count, accepted_count, filtered_count, rejected_count,
                    counts_for_expiry, error
             FROM source_runs WHERE run_id = ? ORDER BY source",
        )
        .bind(&run.id)
        .fetch_all(&pool)
        .await
        .map_err(internal("listing source runs"))?;

        runs.push(CollectionRunSummary {
            id: run.id,
            started_at: run.started_at,
            finished_at: run.finished_at,
            trigger: run.trigger,
            interrupted: run.interrupted,
            sources,
        });
    }

    // Per-source rollup over ALL history, not just the runs above — a source that stopped
    // being attempted a month ago must still appear here, or the panel hides precisely the
    // failure it was built to reveal.
    let sources = sqlx::query_as::<_, SourceHealth>(
        "WITH latest AS (
             SELECT source, MAX(started_at) AS started_at FROM source_runs GROUP BY source
         )
         SELECT
             l.source AS source,
             r.outcome AS last_outcome,
             r.started_at AS last_run_at,
             r.accepted_count AS last_accepted,
             (SELECT MAX(started_at) FROM source_runs s
               WHERE s.source = l.source AND s.outcome = 'success') AS last_success_at,
             (SELECT COUNT(*) FROM source_runs s
               WHERE s.source = l.source
                 AND s.outcome <> 'success'
                 AND s.started_at > COALESCE(
                       (SELECT MAX(started_at) FROM source_runs s2
                         WHERE s2.source = l.source AND s2.outcome = 'success'),
                       '')) AS consecutive_failures,
             (SELECT COUNT(DISTINCT p.id) FROM posting_sightings ps
                JOIN internship_postings p ON p.id = ps.posting_id
               WHERE ps.source = l.source AND p.expired_at IS NULL) AS live_postings
         FROM latest l
         JOIN source_runs r ON r.source = l.source AND r.started_at = l.started_at
         ORDER BY l.source",
    )
    .fetch_all(&pool)
    .await
    .map_err(internal("computing source health"))?;

    let in_progress = current_progress(&pool).await?;
    Ok(Json(RunHealthResponse {
        runs,
        sources,
        in_progress,
    }))
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RejectSummary {
    pub id: String,
    pub source: String,
    pub kind: String,
    pub reason: String,
    pub field: Option<String>,
    pub detail: Option<String>,
    pub url: Option<String>,
    pub external_id: Option<String>,
    pub raw_json: String,
    pub created_at: DateTime<Utc>,
}

/// The rows one source run did not accept.
///
/// This endpoint is the reason `posting_rejects` keeps `raw_json`: a reject count tells you
/// something went wrong and nothing at all about what. Note `kind` — `filtered` in bulk is
/// healthy, `rejected` is a defect worth chasing.
pub async fn list_rejects(
    State(pool): State<SqlitePool>,
    CurrentUser(_user): CurrentUser,
    Path(source_run_id): Path<String>,
) -> Result<Json<Vec<RejectSummary>>, StatusCode> {
    let rejects = sqlx::query_as::<_, RejectSummary>(
        "SELECT id, source, kind, reason, field, detail, url, external_id, raw_json, created_at
         FROM posting_rejects WHERE source_run_id = ?
         ORDER BY kind DESC, reason, created_at
         LIMIT 500",
    )
    .bind(&source_run_id)
    .fetch_all(&pool)
    .await
    .map_err(internal("listing rejects"))?;

    Ok(Json(rejects))
}

/// Logs a database error and turns it into a 500, so handlers don't each repeat the closure.
fn internal(context: &'static str) -> impl Fn(sqlx::Error) -> StatusCode {
    move |err| {
        eprintln!("internships: {context} failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

// ------------------------------------------------------------------------------------------
// The ranked list
// ------------------------------------------------------------------------------------------

/// Query parameters for `GET /internships`.
///
/// Every field here is a **hard filter** or the sort axis. None of them nudges a score — that
/// separation is enforced structurally in `rank`, where `score_posting` cannot see this struct
/// at all.
///
/// # The `*_unknown` parameters, and why they exist at all
///
/// Most postings do not state pay, and almost none state class-year eligibility
/// (`docs/INTERNSHIP_SCRAPING.md` § B: "Sponsorship and class-year eligibility are effectively
/// unavailable"). So for every filter there is a third case that is neither pass nor fail:
/// **the posting does not say**. Hiding that choice would mean picking silently between two
/// filters that return wildly different result sets, so it is exposed instead.
///
/// All four default to `keep`, meaning *a filter never hides a posting merely for being
/// silent*. That default is deliberate in both directions:
///
/// - It is the honest reading. A posting that states no class-year restriction has not told
///   you that you are ineligible, and `ClassYearRange::admits` already treats an unstated
///   range as admitting everyone.
/// - `drop` as the default would make the tab look broken. With pay absent from well over half
///   the corpus, a pay filter defaulting to `drop` would empty the list and look like a bug
///   rather than a policy.
///
/// `pay_unknown=drop` is the one users will reach for most — "only postings that actually say
/// what they pay" is a reasonable thing to want, and it is one parameter away.
#[derive(Debug, Clone, Deserialize)]
pub struct ListPostingsQuery {
    /// `composite` (default), `pay`, `posted`, `deadline`, `prestige`.
    pub sort: Option<String>,

    pub term_season: Option<String>,
    pub term_year: Option<i64>,
    pub term_unknown: Option<String>,

    pub remote: Option<bool>,
    pub location: Option<String>,
    pub location_unknown: Option<String>,

    /// A graduation year (`2029`) or a year of study (`sophomore`).
    pub class_year: Option<String>,
    pub class_year_unknown: Option<String>,

    /// Inclusive bounds, in hourly USD — the same unit `rank` reasons in throughout.
    pub pay_min: Option<f64>,
    pub pay_max: Option<f64>,
    pub pay_unknown: Option<String>,

    pub company: Option<String>,
    /// Applied in SQL against `posting_sightings`, not in `rank` — a deduped posting can be
    /// carried by several sources, so "which source" is a property of the sighting.
    pub source: Option<String>,
}

/// Parses an `OnUnknown` policy, defaulting to `Keep`.
///
/// An unrecognized value is an error rather than a fallback: silently reading
/// `pay_unknown=dorp` as `keep` would return a result set the user did not ask for and give
/// them no way to notice. Same reasoning as the blog's `?sort=oldset` answering 400.
fn parse_on_unknown(value: Option<&str>) -> Result<OnUnknown, StatusCode> {
    match value {
        None => Ok(OnUnknown::Keep),
        Some(raw) => match raw.trim().to_lowercase().as_str() {
            "keep" => Ok(OnUnknown::Keep),
            "drop" => Ok(OnUnknown::Drop),
            _ => Err(StatusCode::BAD_REQUEST),
        },
    }
}

fn parse_season_param(value: &str) -> Option<Season> {
    match value.trim().to_lowercase().as_str() {
        "summer" => Some(Season::Summer),
        "fall" | "autumn" => Some(Season::Fall),
        "winter" => Some(Season::Winter),
        "spring" => Some(Season::Spring),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ListPostingsResponse {
    /// How many live postings existed before filtering. With `returned`, this is what lets the
    /// UI say "12 of 1,881" rather than leaving an empty list ambiguous between "nothing
    /// matched" and "nothing collected yet".
    pub total_live: usize,
    pub returned: usize,
    pub sort: String,
    pub postings: Vec<RankedPosting>,
    /// Set while a collection is running, so an empty or partial list can say why.
    pub collection: Option<RunProgress>,
}

/// The ranked, filtered list of open postings.
pub async fn list_postings(
    State(pool): State<SqlitePool>,
    CurrentUser(_user): CurrentUser,
    Query(params): Query<ListPostingsQuery>,
) -> Result<Json<ListPostingsResponse>, StatusCode> {
    let now = Utc::now();

    let sort = match params.sort.as_deref() {
        None => SortBy::Composite,
        Some(raw) => SortBy::parse(raw).ok_or(StatusCode::BAD_REQUEST)?,
    };

    let term = match (params.term_season.as_deref(), params.term_year) {
        (None, None) => None,
        (season_raw, year) => {
            let season = match season_raw {
                None => None,
                Some(raw) => Some(parse_season_param(raw).ok_or(StatusCode::BAD_REQUEST)?),
            };
            Some(TermFilter {
                season,
                year,
                on_unknown: parse_on_unknown(params.term_unknown.as_deref())?,
            })
        }
    };

    let location = match (params.remote, params.location.as_deref()) {
        (None, None) => None,
        (remote, contains) => Some(LocationFilter {
            remote,
            contains: contains.map(str::to_string),
            on_unknown: parse_on_unknown(params.location_unknown.as_deref())?,
        }),
    };

    let class_year = match params.class_year.as_deref() {
        None => None,
        Some(raw) => Some(ClassYearFilter {
            grad_year: parse_class_year(raw, now).ok_or(StatusCode::BAD_REQUEST)?,
            on_unknown: parse_on_unknown(params.class_year_unknown.as_deref())?,
        }),
    };

    let pay = match (params.pay_min, params.pay_max) {
        (None, None) => None,
        (min, max) => {
            // A window that cannot contain anything is a client bug, and answering it with an
            // empty list would look like "nothing pays that" rather than "you asked for
            // nothing". Both bounds are inclusive, so equal bounds are a legal point query.
            if let (Some(min), Some(max)) = (min, max)
                && min > max
            {
                return Err(StatusCode::BAD_REQUEST);
            }
            if min.is_some_and(|value| value < 0.0) || max.is_some_and(|value| value < 0.0) {
                return Err(StatusCode::BAD_REQUEST);
            }
            Some(PayRangeFilter {
                min_hourly_usd: min,
                max_hourly_usd: max,
                on_unknown: parse_on_unknown(params.pay_unknown.as_deref())?,
            })
        }
    };

    let filters = InternshipFilters {
        term,
        location,
        class_year,
        pay,
        company_key: params.company.as_deref().map(str::to_string),
    };

    let postings = store::load_live_postings(&pool, params.source.as_deref())
        .await
        .map_err(|err| {
            eprintln!("internships: loading postings failed: {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let total_live = postings.len();

    let signals = store::load_company_signals(&pool).await.map_err(|err| {
        eprintln!("internships: loading company signals failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let ranked = rank_postings(&postings, &signals, &filters, sort, now);

    Ok(Json(ListPostingsResponse {
        total_live,
        returned: ranked.len(),
        sort: sort.as_str().to_string(),
        postings: ranked,
        collection: current_progress(&pool).await?,
    }))
}

/// Source names that currently carry a live posting, for the UI's dropdown.
///
/// Derived from real data rather than a hardcoded list, following `recipes/page.tsx` — a
/// hardcoded list silently rots as sources are added or retired.
pub async fn list_sources(
    State(pool): State<SqlitePool>,
    CurrentUser(_user): CurrentUser,
) -> Result<Json<Vec<String>>, StatusCode> {
    store::live_source_names(&pool).await.map(Json).map_err(|err| {
        eprintln!("internships: listing sources failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

/// Trigger a collection by hand.
///
/// Admin-only, matching `POST /blog/sync`: it reaches out to every configured source, so it is
/// not something an ordinary signed-in user should be able to set off repeatedly. The scheduled
/// runner is the normal path — this exists for "I just changed a source, show me".
///
/// Runs inline and returns the report, so the caller sees what happened rather than a 202 and
/// a shrug. That makes it slow by design; the scheduler is what you want for routine use.
pub async fn collect_now(
    State(pool): State<SqlitePool>,
    RequireAdmin(_user): RequireAdmin,
) -> Result<Json<CollectionSummary>, StatusCode> {
    let report = crate::internships::collector::collect(&pool, "manual")
        .await
        .map_err(|err| {
            eprintln!("internships: manual collection failed: {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(CollectionSummary {
        run_id: report.run_id,
        sources_run: report.sources_run,
        sources_succeeded: report.sources_succeeded,
        fetched: report.fetched,
        accepted: report.accepted,
        filtered: report.filtered,
        rejected: report.rejected,
        postings_created: report.postings_created,
        postings_updated: report.postings_updated,
        alerts_created: report.alerts_created,
        marked_closed: report.marked_closed,
        swept_deadline: report.swept_deadline,
        swept_vanished: report.swept_vanished,
    }))
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionSummary {
    pub run_id: String,
    pub sources_run: usize,
    pub sources_succeeded: usize,
    pub fetched: i64,
    pub accepted: i64,
    pub filtered: i64,
    pub rejected: i64,
    pub postings_created: i64,
    pub postings_updated: i64,
    /// Desktop-notification events this run raised. See `internships::alerts`.
    pub alerts_created: i64,
    pub marked_closed: u64,
    pub swept_deadline: u64,
    pub swept_vanished: u64,
}
