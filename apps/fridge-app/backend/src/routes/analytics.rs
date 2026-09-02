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
//! groups tiers in Rust using the application's company snapshot, and shares the response
//! predicate in [`crate::internships::application_events::is_response_status`] with the nudge
//! producer.

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
        application_events::is_response_status, models::ApplicationStatus, normalize::company_key,
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
    event_id: Option<String>,
    event_at: Option<DateTime<Utc>>,
    event_from_status: Option<String>,
    event_to_status: Option<String>,
}

#[derive(Debug)]
struct EventFact {
    at: DateTime<Utc>,
    from_status: Option<ApplicationStatus>,
    to_status: ApplicationStatus,
}

#[derive(Debug)]
struct ApplicationFacts {
    id: String,
    company_name: String,
    source: Option<String>,
    applied_at: DateTime<Utc>,
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
    let rows = sqlx::query_as::<_, AnalyticsRow>(
        "SELECT a.id AS application_id,
                a.company_name,
                a.source,
                a.applied_at,
                p.id AS _posting_id,
                e.id AS event_id,
                e.at AS event_at,
                e.from_status AS event_from_status,
                e.to_status AS event_to_status
           FROM internship_applications a
           LEFT JOIN internship_postings p ON p.id = a.posting_id
           LEFT JOIN application_events e ON e.application_id = a.id
          WHERE a.user_id = ?1
          ORDER BY a.id ASC, e.at ASC, e.created_at ASC, e.id ASC",
    )
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
                events: Vec::new(),
            });
        }

        match (row.event_id, row.event_at, row.event_to_status) {
            (None, None, None) => {}
            (Some(_), Some(at), Some(to_status)) => {
                let to_status = ApplicationStatus::parse(&to_status)
                    .ok_or_else(|| anyhow!("invalid to_status {to_status:?} in event log"))?;
                let from_status = match row.event_from_status.as_deref() {
                    Some(value) => Some(
                        ApplicationStatus::parse(value)
                            .ok_or_else(|| anyhow!("invalid from_status {value:?} in event log"))?,
                    ),
                    None => None,
                };
                applications
                    .last_mut()
                    .expect("application was inserted above")
                    .events
                    .push(EventFact {
                        at,
                        from_status,
                        to_status,
                    });
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
    let creation = application.events.iter().position(|event| {
        event.from_status.is_none() && event.to_status == ApplicationStatus::Applied
    });

    let events_after_creation = match creation {
        Some(index) => &application.events[index + 1..],
        None if application.events.is_empty() => &application.events[..],
        None => bail!(
            "application {} has transition events but no creation event",
            application.id
        ),
    };

    let first_response = events_after_creation
        .iter()
        .find(|event| is_response_status(event.to_status));
    let responded = first_response.is_some();
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
        let creation_at = application.events[creation.expect("response requires creation")].at;
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
