//! Read-only analytics over the append-only application event log.
//!
//! This route is a cohort report: `from` and `to` select applications by the UTC instant in
//! `internship_applications.applied_at`; an event may happen months later without moving the
//! application to a later cohort. Conversion is likewise history-based: it means the maximum
//! stage present after the application's creation event, not the mutable status cache's final
//! value. Applications survive a pruned posting because the only posting join is a `LEFT JOIN`
//! and snapshot columns on the application remain authoritative.
//!
//! # Wire decisions not explicit in the Phase 11 reference
//!
//! The query window is required and half-open (`from <= applied_at < to`), which makes adjacent
//! reports non-overlapping. Breakdown entries are `{ "key": ..., "totals": ... }`; a NULL or
//! blank source is `unknown`; counts, rather than redundant rates, are returned. For an empty
//! response-time sample, `median` and `p90` are JSON null with `n: 0`. A non-empty sample uses
//! the average of the middle pair for an even-sized median and nearest-rank p90. These choices
//! belong in `docs/HUNT.md` when the documentation lane reconciles this commit.
//!
//! This layer deliberately has no writer. It loads the company-tier file once per request,
//! groups tiers in Rust using the application's company snapshot, and uses
//! [`crate::internships::application_events::HAS_RESPONDED`] verbatim so the dashboard and
//! nudge producer cannot disagree. That predicate excludes the earliest event rather than
//! treating every NULL `from_status` as creation: a provenance-unknown backfill transition
//! also has a NULL `from_status`, and it is still a real response.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{
    internships::{
        application_events::HAS_RESPONDED, models::ApplicationStatus, normalize::company_key,
        prestige::CompanyTiers,
    },
    routes::auth::CurrentUser,
};

const DEFAULT_DEAD_AFTER_DAYS: u32 = 45;

#[derive(Debug, Deserialize)]
pub struct AnalyticsQuery {
    from: String,
    to: String,
    dead_after_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalyticsWindow {
    from: String,
    to: String,
    dead_after_days: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Totals {
    applications: u64,
    responded: u64,
    no_response_live: u64,
    no_response_dead: u64,
    reached_oa: u64,
    reached_interview: u64,
    offers: u64,
    rejected: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResponseTimeDays {
    median: Option<f64>,
    p90: Option<f64>,
    n: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Breakdown {
    key: String,
    totals: Totals,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalyticsResponse {
    window: AnalyticsWindow,
    totals: Totals,
    time_to_first_response_days: ResponseTimeDays,
    by_source: Vec<Breakdown>,
    by_tier: Vec<Breakdown>,
    by_month: Vec<Breakdown>,
}

#[derive(Debug, sqlx::FromRow)]
struct AnalyticsRow {
    application_id: String,
    company_name: String,
    source: Option<String>,
    applied_at: DateTime<Utc>,
    _posting_id: Option<String>,
    has_responded: i64,
    event_id: Option<String>,
    event_at: Option<DateTime<Utc>>,
    event_to_status: Option<String>,
}

#[derive(Debug)]
struct EventFact {
    at: DateTime<Utc>,
    to_status: ApplicationStatus,
}

#[derive(Debug)]
struct ApplicationFacts {
    id: String,
    company_name: String,
    source: Option<String>,
    applied_at: DateTime<Utc>,
    has_responded: bool,
    events: Vec<EventFact>,
}

#[derive(Debug, Clone, Copy)]
struct ApplicationMetrics {
    responded: bool,
    no_response_live: bool,
    no_response_dead: bool,
    reached_oa: bool,
    reached_interview: bool,
    offers: bool,
    rejected: bool,
    response_days: Option<f64>,
}

pub async fn analytics(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<AnalyticsQuery>,
) -> Result<Json<AnalyticsResponse>, StatusCode> {
    let from = parse_bound(&query.from)?;
    let to = parse_bound(&query.to)?;
    if from >= to {
        return Err(StatusCode::BAD_REQUEST);
    }

    let dead_after_days = query.dead_after_days.unwrap_or(DEFAULT_DEAD_AFTER_DAYS);
    let tiers = CompanyTiers::load();
    build_analytics(
        &pool,
        &user.id,
        from,
        to,
        dead_after_days,
        Utc::now(),
        &tiers,
    )
    .await
    .map(Json)
    .map_err(|error| {
        eprintln!("hunt analytics failed: {error:#}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

fn parse_bound(value: &str) -> Result<DateTime<Utc>, StatusCode> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| StatusCode::BAD_REQUEST)
}

async fn build_analytics(
    pool: &SqlitePool,
    user_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    dead_after_days: u32,
    now: DateTime<Utc>,
    tiers: &CompanyTiers,
) -> Result<AnalyticsResponse> {
    // Do not push the window into a TEXT comparison. RFC 3339 permits equivalent UTC instants
    // with different offsets and spellings, so comparing parsed instants in Rust is exact.
    let sql = format!(
        "SELECT a.id AS application_id,
                a.company_name,
                a.source,
                a.applied_at,
                p.id AS _posting_id,
                CASE WHEN {HAS_RESPONDED} THEN 1 ELSE 0 END AS has_responded,
                history.id AS event_id,
                history.at AS event_at,
                history.to_status AS event_to_status
           FROM internship_applications a
           LEFT JOIN internship_postings p ON p.id = a.posting_id
           LEFT JOIN application_events history ON history.application_id = a.id
          WHERE a.user_id = ?1
          ORDER BY a.id ASC, history.at ASC, history.created_at ASC, history.id ASC"
    );
    let rows = sqlx::query_as::<_, AnalyticsRow>(&sql)
        .bind(user_id)
        .fetch_all(pool)
        .await
        .context("load analytics cohort and event histories")?;

    let applications = group_rows(rows)?;
    let mut totals = Totals::default();
    let mut response_days = Vec::new();
    let mut by_source = BTreeMap::new();
    let mut by_tier = BTreeMap::new();
    let mut by_month = BTreeMap::new();

    for application in applications
        .iter()
        .filter(|application| application.applied_at >= from && application.applied_at < to)
    {
        let metrics = metrics_for(application, dead_after_days, now)?;
        totals.add(metrics);
        if let Some(days) = metrics.response_days {
            response_days.push(days);
        }

        let source = application
            .source
            .as_deref()
            .map(str::trim)
            .filter(|source| !source.is_empty())
            .unwrap_or("unknown")
            .to_string();
        let tier = tiers
            .tier(&company_key(&application.company_name))
            .map_or_else(|| "unknown".to_string(), |tier| tier.to_string());
        let month = application.applied_at.format("%Y-%m").to_string();

        by_source
            .entry(source)
            .or_insert_with(Totals::default)
            .add(metrics);
        by_tier
            .entry(tier)
            .or_insert_with(Totals::default)
            .add(metrics);
        by_month
            .entry(month)
            .or_insert_with(Totals::default)
            .add(metrics);
    }

    Ok(AnalyticsResponse {
        window: AnalyticsWindow {
            from: from.to_rfc3339(),
            to: to.to_rfc3339(),
            dead_after_days,
        },
        totals,
        time_to_first_response_days: summarize_response_days(response_days),
        by_source: breakdowns(by_source),
        by_tier: breakdowns(by_tier),
        by_month: breakdowns(by_month),
    })
}

fn group_rows(rows: Vec<AnalyticsRow>) -> Result<Vec<ApplicationFacts>> {
    let mut applications: Vec<ApplicationFacts> = Vec::new();

    for row in rows {
        if applications
            .last()
            .map(|application| application.id.as_str())
            != Some(row.application_id.as_str())
        {
            applications.push(ApplicationFacts {
                id: row.application_id.clone(),
                company_name: row.company_name,
                source: row.source,
                applied_at: row.applied_at,
                has_responded: row.has_responded != 0,
                events: Vec::new(),
            });
        }

        match (row.event_id, row.event_at, row.event_to_status) {
            (None, None, None) => {}
            (Some(_), Some(at), Some(to_status)) => {
                let to_status = ApplicationStatus::parse(&to_status)
                    .ok_or_else(|| anyhow!("invalid to_status {to_status:?} in event log"))?;
                applications
                    .last_mut()
                    .expect("application was inserted above")
                    .events
                    .push(EventFact { at, to_status });
            }
            _ => bail!("incomplete application event row"),
        }
    }

    Ok(applications)
}

fn metrics_for(
    application: &ApplicationFacts,
    dead_after_days: u32,
    now: DateTime<Utc>,
) -> Result<ApplicationMetrics> {
    // HAS_RESPONDED defines creation structurally as the earliest ordered event. Locate the
    // first qualifying event only to calculate elapsed time; do not re-decide the boolean here.
    let events_after_creation = application.events.get(1..).unwrap_or_default();
    let first_response = application
        .has_responded
        .then(|| {
            events_after_creation
                .iter()
                .find(|event| event.to_status != ApplicationStatus::Applied)
                .ok_or_else(|| {
                    anyhow!(
                        "HAS_RESPONDED was true for {} but no response event was loaded",
                        application.id
                    )
                })
        })
        .transpose()?;
    let responded = application.has_responded;
    let is_dead = now - application.applied_at > Duration::days(i64::from(dead_after_days));
    let reached_oa = events_after_creation.iter().any(|event| {
        matches!(
            event.to_status,
            ApplicationStatus::Oa | ApplicationStatus::Interview | ApplicationStatus::Offer
        )
    });
    let reached_interview = events_after_creation.iter().any(|event| {
        matches!(
            event.to_status,
            ApplicationStatus::Interview | ApplicationStatus::Offer
        )
    });
    let offers = events_after_creation
        .iter()
        .any(|event| event.to_status == ApplicationStatus::Offer);
    let rejected = events_after_creation
        .iter()
        .any(|event| event.to_status == ApplicationStatus::Rejected);
    let response_days = first_response.map(|response| {
        let creation_at = application.events[0].at;
        (response.at - creation_at).num_milliseconds() as f64 / 86_400_000.0
    });

    Ok(ApplicationMetrics {
        responded,
        no_response_live: !responded && !is_dead,
        no_response_dead: !responded && is_dead,
        reached_oa,
        reached_interview,
        offers,
        rejected,
        response_days,
    })
}

impl Totals {
    fn add(&mut self, metrics: ApplicationMetrics) {
        self.applications += 1;
        self.responded += u64::from(metrics.responded);
        self.no_response_live += u64::from(metrics.no_response_live);
        self.no_response_dead += u64::from(metrics.no_response_dead);
        self.reached_oa += u64::from(metrics.reached_oa);
        self.reached_interview += u64::from(metrics.reached_interview);
        self.offers += u64::from(metrics.offers);
        self.rejected += u64::from(metrics.rejected);
    }
}

fn summarize_response_days(mut values: Vec<f64>) -> ResponseTimeDays {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n == 0 {
        return ResponseTimeDays {
            median: None,
            p90: None,
            n: 0,
        };
    }

    let median = if n.is_multiple_of(2) {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    } else {
        values[n / 2]
    };
    let p90_index = ((n as f64 * 0.9).ceil() as usize).saturating_sub(1);

    ResponseTimeDays {
        median: Some(median),
        p90: Some(values[p90_index]),
        n: n as u64,
    }
}

fn breakdowns(groups: BTreeMap<String, Totals>) -> Vec<Breakdown> {
    groups
        .into_iter()
        .map(|(key, totals)| Breakdown { key, totals })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        sqlx::query(
            "INSERT INTO users (id, email, created_at)
             VALUES ('analytics-user', 'analytics@example.com', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("user");
        pool
    }

    struct ApplicationFixture<'a> {
        id: &'a str,
        company: &'a str,
        source: Option<&'a str>,
        status: ApplicationStatus,
        applied_at: DateTime<Utc>,
    }

    async fn insert_application(pool: &SqlitePool, fixture: ApplicationFixture<'_>) {
        sqlx::query(
            "INSERT INTO internship_applications
                (id, user_id, posting_id, company_name, title, url, source, snapshot_json,
                 snapshot_at, status, applied_at, status_changed_at, created_at, updated_at)
             VALUES (?1, 'analytics-user', NULL, ?2, 'Engineer', 'https://example.com/job', ?3,
                     '{}', ?4, ?5, ?4, ?4, ?4, ?4)",
        )
        .bind(fixture.id)
        .bind(fixture.company)
        .bind(fixture.source)
        .bind(fixture.applied_at.to_rfc3339())
        .bind(fixture.status.as_str())
        .execute(pool)
        .await
        .expect("application");

        insert_event(
            pool,
            fixture.id,
            &format!("{}-created", fixture.id),
            fixture.applied_at,
            None,
            ApplicationStatus::Applied,
        )
        .await;
    }

    async fn insert_event(
        pool: &SqlitePool,
        application_id: &str,
        id: &str,
        at: DateTime<Utc>,
        from_status: Option<ApplicationStatus>,
        to_status: ApplicationStatus,
    ) {
        sqlx::query(
            "INSERT INTO application_events
                (id, application_id, at, created_at, from_status, to_status, actor)
             VALUES (?1, ?2, ?3, ?3, ?4, ?5, 'manual')",
        )
        .bind(id)
        .bind(application_id)
        .bind(at.to_rfc3339())
        .bind(from_status.map(ApplicationStatus::as_str))
        .bind(to_status.as_str())
        .execute(pool)
        .await
        .expect("event");
    }

    fn instant(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("test timestamp")
            .with_timezone(&Utc)
    }

    async fn report(
        pool: &SqlitePool,
        from: &str,
        to: &str,
        dead_after_days: u32,
        now: DateTime<Utc>,
    ) -> AnalyticsResponse {
        build_analytics(
            pool,
            "analytics-user",
            instant(from),
            instant(to),
            dead_after_days,
            now,
            &CompanyTiers::default(),
        )
        .await
        .expect("analytics")
    }

    fn all_totals(report: &AnalyticsResponse) -> impl Iterator<Item = Totals> + '_ {
        std::iter::once(report.totals).chain(
            report
                .by_source
                .iter()
                .chain(&report.by_tier)
                .chain(&report.by_month)
                .map(|bucket| bucket.totals),
        )
    }

    #[tokio::test]
    async fn maximum_stage_survives_a_later_rejection() {
        let pool = pool().await;
        let applied_at = instant("2026-06-01T00:00:00Z");
        insert_application(
            &pool,
            ApplicationFixture {
                id: "advanced-then-rejected",
                company: "Example Co",
                source: Some("simplify"),
                status: ApplicationStatus::Rejected,
                applied_at,
            },
        )
        .await;
        insert_event(
            &pool,
            "advanced-then-rejected",
            "oa",
            applied_at + Duration::days(2),
            // A backfilled fallback transition has unknown provenance, hence NULL here. It is
            // still a response because HAS_RESPONDED excludes only the earliest event.
            None,
            ApplicationStatus::Oa,
        )
        .await;
        insert_event(
            &pool,
            "advanced-then-rejected",
            "rejected",
            applied_at + Duration::days(3),
            Some(ApplicationStatus::Oa),
            ApplicationStatus::Rejected,
        )
        .await;

        let analytics = report(
            &pool,
            "2026-06-01T00:00:00Z",
            "2026-07-01T00:00:00Z",
            45,
            instant("2026-09-01T00:00:00Z"),
        )
        .await;

        assert_eq!(analytics.totals.applications, 1);
        assert_eq!(analytics.totals.responded, 1);
        assert_eq!(analytics.totals.reached_oa, 1);
        assert_eq!(analytics.totals.rejected, 1);
        for totals in all_totals(&analytics) {
            assert_eq!(totals.no_response_live, 0);
            assert_eq!(totals.no_response_dead, 0);
            assert!(totals.rejected <= totals.responded);
        }
    }

    #[tokio::test]
    async fn silence_is_live_or_dead_while_terminal_applications_are_neither() {
        let pool = pool().await;
        let now = instant("2026-09-01T00:00:00Z");
        for (id, age, status) in [
            ("young-silent", 10, ApplicationStatus::Applied),
            ("old-silent", 50, ApplicationStatus::Applied),
            ("old-rejected", 100, ApplicationStatus::Rejected),
        ] {
            let applied_at = now - Duration::days(age);
            insert_application(
                &pool,
                ApplicationFixture {
                    id,
                    company: "Example Co",
                    source: Some("simplify"),
                    status,
                    applied_at,
                },
            )
            .await;
            if status == ApplicationStatus::Rejected {
                insert_event(
                    &pool,
                    id,
                    "terminal-response",
                    applied_at + Duration::days(1),
                    Some(ApplicationStatus::Applied),
                    ApplicationStatus::Rejected,
                )
                .await;
            }
        }

        let analytics = report(
            &pool,
            "2026-01-01T00:00:00Z",
            "2027-01-01T00:00:00Z",
            45,
            now,
        )
        .await;

        assert_eq!(analytics.totals.applications, 3);
        assert_eq!(analytics.totals.responded, 1);
        assert_eq!(analytics.totals.no_response_live, 1);
        assert_eq!(analytics.totals.no_response_dead, 1);
        assert_eq!(analytics.totals.rejected, 1);
        assert_eq!(
            analytics.totals.responded
                + analytics.totals.no_response_live
                + analytics.totals.no_response_dead,
            analytics.totals.applications
        );
    }

    #[tokio::test]
    async fn a_later_interview_stays_in_the_application_month_cohort() {
        let pool = pool().await;
        let applied_at = instant("2026-06-20T12:00:00Z");
        insert_application(
            &pool,
            ApplicationFixture {
                id: "summer-cohort",
                company: "Example Co",
                source: Some("simplify"),
                status: ApplicationStatus::Interview,
                applied_at,
            },
        )
        .await;
        insert_event(
            &pool,
            "summer-cohort",
            "september-interview",
            instant("2026-09-03T12:00:00Z"),
            Some(ApplicationStatus::Applied),
            ApplicationStatus::Interview,
        )
        .await;

        let june = report(
            &pool,
            "2026-06-01T00:00:00Z",
            "2026-07-01T00:00:00Z",
            45,
            instant("2026-10-01T00:00:00Z"),
        )
        .await;
        assert_eq!(june.totals.applications, 1);
        assert_eq!(june.totals.reached_interview, 1);
        assert_eq!(june.by_month[0].key, "2026-06");
        assert_eq!(june.by_month[0].totals.applications, 1);

        let september = report(
            &pool,
            "2026-09-01T00:00:00Z",
            "2026-10-01T00:00:00Z",
            45,
            instant("2026-10-01T00:00:00Z"),
        )
        .await;
        assert_eq!(september.totals.applications, 0);
        assert!(september.by_month.is_empty());
    }

    #[tokio::test]
    async fn response_time_reports_n_median_and_nearest_rank_p90() {
        let pool = pool().await;
        let applied_at = instant("2026-06-01T00:00:00Z");
        for days in [1, 2, 3, 4, 200] {
            let id = format!("response-{days}");
            insert_application(
                &pool,
                ApplicationFixture {
                    id: &id,
                    company: "Example Co",
                    source: Some("simplify"),
                    status: ApplicationStatus::Oa,
                    applied_at,
                },
            )
            .await;
            insert_event(
                &pool,
                &id,
                &format!("{id}-oa"),
                applied_at + Duration::days(days),
                Some(ApplicationStatus::Applied),
                ApplicationStatus::Oa,
            )
            .await;
        }

        let analytics = report(
            &pool,
            "2026-01-01T00:00:00Z",
            "2027-01-01T00:00:00Z",
            45,
            instant("2027-01-01T00:00:00Z"),
        )
        .await;

        assert_eq!(analytics.time_to_first_response_days.n, 5);
        assert_eq!(analytics.time_to_first_response_days.median, Some(3.0));
        assert_eq!(analytics.time_to_first_response_days.p90, Some(200.0));
    }

    #[tokio::test]
    async fn an_unlisted_company_uses_unknown_instead_of_tier_three() {
        let pool = pool().await;
        insert_application(
            &pool,
            ApplicationFixture {
                id: "unlisted-company",
                company: "A Company Definitely Missing From The Tier File",
                source: None,
                status: ApplicationStatus::Applied,
                applied_at: instant("2026-06-01T00:00:00Z"),
            },
        )
        .await;

        let analytics = report(
            &pool,
            "2026-01-01T00:00:00Z",
            "2027-01-01T00:00:00Z",
            45,
            instant("2026-06-02T00:00:00Z"),
        )
        .await;

        assert_eq!(analytics.by_tier.len(), 1);
        assert_eq!(analytics.by_tier[0].key, "unknown");
        assert_ne!(analytics.by_tier[0].key, "3");
        assert_eq!(analytics.by_source[0].key, "unknown");
    }

    #[test]
    fn malformed_bounds_are_rejected_instead_of_defaulted() {
        assert_eq!(parse_bound("June 2026"), Err(StatusCode::BAD_REQUEST));
        assert_eq!(parse_bound(""), Err(StatusCode::BAD_REQUEST));
        assert!(parse_bound("2026-06-01T00:00:00-04:00").is_ok());
    }
}
