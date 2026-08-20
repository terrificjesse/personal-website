//! Reading postings and company signals out of SQLite into the [`super::models`] types.
//!
//! # Why this is a translation layer and not `#[derive(FromRow)]`
//!
//! [`Posting`] deliberately groups the flat columns into value types — `Option<PayRange>`
//! rather than four loose `Option`s, `Location` rather than five columns — because that is
//! what makes "absent is not zero" a compile-time property rather than a convention. The
//! database cannot express that grouping, so something has to translate, and doing it here
//! once is better than every caller reassembling it.
//!
//! [`PostingRow::into_posting`] is where a half-populated pay tuple would turn into a whole
//! `PayRange` if anyone got careless. It refuses instead: no currency or no period means no
//! `PayRange`, matching the CHECK constraints in migration `0012` rather than trusting them.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::collections::HashMap;

use super::models::{
    ClassYearRange, CompanySignals, ExpiryReason, Location, PayPeriod, PayRange, Posting, Season,
};

/// The flat shape of one `internship_postings` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PostingRow {
    pub id: String,
    pub dedup_key: String,
    pub company_key: String,
    pub company_name: String,
    pub title: String,
    pub canonical_url: String,
    pub term_season: Option<String>,
    pub term_year: Option<i64>,
    pub location_raw: Option<String>,
    pub location_city: Option<String>,
    pub location_region: Option<String>,
    pub location_country: Option<String>,
    pub is_remote: Option<bool>,
    pub pay_min: Option<f64>,
    pub pay_max: Option<f64>,
    pub pay_currency: Option<String>,
    pub pay_period: Option<String>,
    pub pay_raw: Option<String>,
    pub class_year_min: Option<i64>,
    pub class_year_max: Option<i64>,
    pub class_year_raw: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
    pub posted_at_is_estimated: bool,
    pub deadline: Option<DateTime<Utc>>,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub expired_at: Option<DateTime<Utc>>,
    pub expiry_reason: Option<String>,
}

/// The columns [`PostingRow`] expects, in one place so the several statements below cannot
/// drift apart from each other or from the struct.
pub const POSTING_COLUMNS: &str = "
    p.id, p.dedup_key, p.company_key, p.company_name, p.title, p.canonical_url,
    p.term_season, p.term_year,
    p.location_raw, p.location_city, p.location_region, p.location_country, p.is_remote,
    p.pay_min, p.pay_max, p.pay_currency, p.pay_period, p.pay_raw,
    p.class_year_min, p.class_year_max, p.class_year_raw,
    p.posted_at, p.posted_at_is_estimated, p.deadline,
    p.first_seen_at, p.last_seen_at, p.expired_at, p.expiry_reason";

impl PostingRow {
    /// Rebuild the typed [`Posting`].
    ///
    /// Unrecognized enum text degrades to `None` rather than erroring: the CHECK constraints
    /// already make a bad value nearly impossible to store, and a posting that is 95% good is
    /// worth showing. The one thing this must never do is *invent* a value — an unreadable
    /// season becomes "unknown season", never a guess.
    pub fn into_posting(self) -> Posting {
        // All-or-nothing, mirroring the table constraint. An amount without its currency and
        // period is not a comparable quantity, so it is not a PayRange.
        let pay = match (self.pay_min, self.pay_currency, self.pay_period.as_deref()) {
            (Some(min), Some(currency), Some(period)) => {
                parse_pay_period(period).map(|period| PayRange {
                    min,
                    max: self.pay_max,
                    currency,
                    period,
                })
            }
            _ => None,
        };

        Posting {
            id: self.id,
            dedup_key: self.dedup_key,
            company_key: self.company_key,
            company_name: self.company_name,
            title: self.title,
            canonical_url: self.canonical_url,
            term_season: self.term_season.as_deref().and_then(parse_season),
            term_year: self.term_year,
            location: Location {
                raw: self.location_raw,
                city: self.location_city,
                region: self.location_region,
                country: self.location_country,
                is_remote: self.is_remote,
            },
            pay,
            pay_raw: self.pay_raw,
            class_years: ClassYearRange {
                min: self.class_year_min,
                max: self.class_year_max,
                raw: self.class_year_raw,
            },
            posted_at: self.posted_at,
            posted_at_is_estimated: self.posted_at_is_estimated,
            deadline: self.deadline,
            first_seen_at: self.first_seen_at,
            last_seen_at: self.last_seen_at,
            expired_at: self.expired_at,
            expiry_reason: self.expiry_reason.as_deref().and_then(parse_expiry_reason),
        }
    }
}

fn parse_season(value: &str) -> Option<Season> {
    match value {
        "summer" => Some(Season::Summer),
        "fall" => Some(Season::Fall),
        "winter" => Some(Season::Winter),
        "spring" => Some(Season::Spring),
        _ => None,
    }
}

fn parse_pay_period(value: &str) -> Option<PayPeriod> {
    match value {
        "hour" => Some(PayPeriod::Hour),
        "month" => Some(PayPeriod::Month),
        "year" => Some(PayPeriod::Year),
        _ => None,
    }
}

fn parse_expiry_reason(value: &str) -> Option<ExpiryReason> {
    match value {
        "source_marked_closed" => Some(ExpiryReason::SourceMarkedClosed),
        "deadline_passed" => Some(ExpiryReason::DeadlinePassed),
        "vanished_from_sources" => Some(ExpiryReason::VanishedFromSources),
        "manual" => Some(ExpiryReason::Manual),
        _ => None,
    }
}

/// Every live posting, optionally narrowed to one source.
///
/// **The source filter lives here rather than in `rank`**, and that is not an accident of
/// layering. A deduped posting can be carried by several sources at once, so "which source is
/// this from" is a property of `posting_sightings`, not of the posting — there is no field on
/// [`Posting`] for it to filter on, and adding one would be inventing a single answer to a
/// question that genuinely has several. `EXISTS` against the sightings table asks the right
/// question: *is this posting carried by that source*, which is what a user picking "only
/// Greenhouse" actually means.
///
/// Expired postings are excluded in SQL so the common path uses the partial index. `rank`
/// also drops non-live postings, which is deliberate belt-and-braces: a forgotten
/// `WHERE expired_at IS NULL` then shows nothing rather than surfacing a closed posting.
pub async fn load_live_postings(pool: &SqlitePool, source: Option<&str>) -> Result<Vec<Posting>> {
    let mut sql = format!(
        "SELECT {POSTING_COLUMNS} FROM internship_postings p WHERE p.expired_at IS NULL"
    );
    if source.is_some() {
        sql.push_str(
            " AND EXISTS (
                 SELECT 1 FROM posting_sightings s
                 WHERE s.posting_id = p.id AND s.source = ?
               )",
        );
    }

    let mut query = sqlx::query_as::<_, PostingRow>(&sql);
    if let Some(source) = source {
        query = query.bind(source);
    }

    Ok(query
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(PostingRow::into_posting)
        .collect())
}

/// Every company's derived signals, keyed by `company_key` for `rank_postings`.
///
/// A company with no row here is simply absent from the map, which `rank` reads as unknown
/// prestige — the same meaning as a row whose `prestige` is NULL. Both must stay distinct
/// from a prestige of zero.
pub async fn load_company_signals(pool: &SqlitePool) -> Result<HashMap<String, CompanySignals>> {
    let rows = sqlx::query_as::<_, CompanySignalsRow>(
        "SELECT company_key, company_name, distinct_sources, live_postings,
                total_postings_seen, pay_observations, median_pay_hourly_usd, prestige
         FROM company_signals",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.company_key.clone(),
                CompanySignals {
                    company_key: row.company_key,
                    company_name: row.company_name,
                    distinct_sources: row.distinct_sources,
                    live_postings: row.live_postings,
                    total_postings_seen: row.total_postings_seen,
                    pay_observations: row.pay_observations,
                    median_pay_hourly_usd: row.median_pay_hourly_usd,
                    prestige: row.prestige,
                },
            )
        })
        .collect())
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct CompanySignalsRow {
    company_key: String,
    company_name: String,
    distinct_sources: i64,
    live_postings: i64,
    total_postings_seen: i64,
    pay_observations: i64,
    median_pay_hourly_usd: Option<f64>,
    prestige: Option<f64>,
}

/// Distinct source names that currently carry at least one live posting.
///
/// Populates the UI's source dropdown from real data rather than a hardcoded list, following
/// the precedent set by the recipes page — a hardcoded list silently rots as sources are
/// added or retired.
pub async fn live_source_names(pool: &SqlitePool) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT s.source
         FROM posting_sightings s
         JOIN internship_postings p ON p.id = s.posting_id
         WHERE p.expired_at IS NULL
         ORDER BY s.source",
    )
    .fetch_all(pool)
    .await?)
}
