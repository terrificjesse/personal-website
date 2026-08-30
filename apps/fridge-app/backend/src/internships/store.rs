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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use uuid::Uuid;

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    /// A row with only the NOT NULL columns filled, for mutating per test.
    fn row(id: &str) -> PostingRow {
        PostingRow {
            id: id.to_string(),
            dedup_key: format!("key-{id}"),
            company_key: "acme".into(),
            company_name: "Acme".into(),
            title: "Software Engineer Intern".into(),
            canonical_url: "https://example.com/1".into(),
            term_season: None,
            term_year: None,
            location_raw: None,
            location_city: None,
            location_region: None,
            location_country: None,
            is_remote: None,
            pay_min: None,
            pay_max: None,
            pay_currency: None,
            pay_period: None,
            pay_raw: None,
            class_year_min: None,
            class_year_max: None,
            class_year_raw: None,
            posted_at: None,
            posted_at_is_estimated: false,
            deadline: None,
            first_seen_at: at(2026, 8, 1),
            last_seen_at: at(2026, 8, 20),
            expired_at: None,
            expiry_reason: None,
        }
    }

    // --- the translation itself, tested directly ---
    //
    // `into_posting` is a pure function, which matters here: the interesting cases are rows
    // the CHECK constraints in migration 0012 would refuse, so they cannot be reached by
    // inserting and reading back. Constructing the row directly is the only way to prove the
    // translation layer defends itself rather than trusting the database to have done it.

    #[test]
    fn a_full_row_round_trips_into_the_typed_model() {
        let mut r = row("p1");
        r.term_season = Some("summer".into());
        r.term_year = Some(2027);
        r.location_raw = Some("San Francisco, CA".into());
        r.location_city = Some("San Francisco".into());
        r.is_remote = Some(false);
        r.pay_min = Some(45.0);
        r.pay_max = Some(55.0);
        r.pay_currency = Some("USD".into());
        r.pay_period = Some("hour".into());
        r.class_year_min = Some(2027);
        r.deadline = Some(at(2026, 9, 1));

        let p = r.into_posting();
        assert_eq!(p.term_season, Some(Season::Summer));
        assert_eq!(p.term_year, Some(2027));
        assert_eq!(p.location.city.as_deref(), Some("San Francisco"));
        assert_eq!(p.location.is_remote, Some(false));
        let pay = p.pay.clone().expect("a complete pay tuple must survive");
        assert_eq!((pay.min, pay.max, pay.period), (45.0, Some(55.0), PayPeriod::Hour));
        assert_eq!(pay.currency, "USD");
        assert_eq!(p.class_years.min, Some(2027));
        assert!(p.is_live());
    }

    #[test]
    fn pay_without_a_currency_becomes_no_pay_rather_than_half_a_figure() {
        // Migration 0012 has a CHECK forbidding this, so it should be unreachable — which is
        // exactly why it is worth pinning here. An amount with no currency is not a comparable
        // quantity, and the type system must not be handed one.
        let mut r = row("p1");
        r.pay_min = Some(45.0);
        r.pay_period = Some("hour".into());
        assert!(r.into_posting().pay.is_none());
    }

    #[test]
    fn pay_without_a_period_becomes_no_pay() {
        let mut r = row("p1");
        r.pay_min = Some(45.0);
        r.pay_currency = Some("USD".into());
        assert!(r.into_posting().pay.is_none());
    }

    #[test]
    fn an_unreadable_pay_period_degrades_to_unknown_rather_than_panicking() {
        // `'hourly'` instead of `'hour'`. The CHECK makes it unreachable today; if a future
        // migration widens the column, this must lose the pay quietly rather than crash a
        // request — and `pay_raw` still carries the original text.
        let mut r = row("p1");
        r.pay_min = Some(45.0);
        r.pay_currency = Some("USD".into());
        r.pay_period = Some("hourly".into());
        r.pay_raw = Some("USD 45.00 per hour".into());
        let p = r.into_posting();
        assert!(p.pay.is_none());
        assert_eq!(p.pay_raw.as_deref(), Some("USD 45.00 per hour"));
    }

    #[test]
    fn is_remote_keeps_all_three_states() {
        // The distinction the whole phase turns on: unknown is not onsite.
        for (stored, expected) in [(None, None), (Some(false), Some(false)), (Some(true), Some(true))] {
            let mut r = row("p1");
            r.is_remote = stored;
            assert_eq!(r.into_posting().location.is_remote, expected);
        }
    }

    #[test]
    fn an_unreadable_season_becomes_unknown_not_a_default() {
        let mut r = row("p1");
        r.term_season = Some("monsoon".into());
        assert_eq!(r.into_posting().term_season, None, "never guess a season");
    }

    #[test]
    fn every_expiry_reason_the_schema_allows_is_readable() {
        // A reason the translation cannot read silently becomes a live posting, because
        // `is_live` keys on `expired_at`. Drift between this match and the CHECK constraint in
        // migration 0012 is the hazard.
        for (text, expected) in [
            ("deadline_passed", ExpiryReason::DeadlinePassed),
            ("vanished_from_sources", ExpiryReason::VanishedFromSources),
            ("source_marked_closed", ExpiryReason::SourceMarkedClosed),
            ("manual", ExpiryReason::Manual),
        ] {
            let mut r = row("p1");
            r.expired_at = Some(at(2026, 8, 20));
            r.expiry_reason = Some(text.into());
            let p = r.into_posting();
            assert_eq!(p.expiry_reason, Some(expected), "{text} did not translate");
            assert!(!p.is_live());
        }
    }

    // --- the queries, against a real database ---

    async fn pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("store-{}.db", Uuid::new_v4()));
        crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("migrations")
    }

    async fn insert(pool: &SqlitePool, id: &str, expired: bool) {
        sqlx::query(
            "INSERT INTO internship_postings
                 (id, dedup_key, company_key, company_name, title, canonical_url,
                  first_seen_at, last_seen_at, created_at, updated_at, expired_at, expiry_reason)
             VALUES (?1, ?2, 'acme', 'Acme', 'SWE Intern', 'https://x/1',
                     ?3, ?3, ?3, ?3, ?4, ?5)",
        )
        .bind(id)
        .bind(format!("key-{id}"))
        .bind(at(2026, 8, 1).to_rfc3339())
        .bind(expired.then(|| at(2026, 8, 20).to_rfc3339()))
        .bind(expired.then_some("manual"))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn sight(pool: &SqlitePool, posting: &str, source: &str, external: &str) {
        sqlx::query(
            "INSERT INTO posting_sightings
                 (id, posting_id, source, external_id, url, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, 'https://x/1', ?5, ?5)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(posting)
        .bind(source)
        .bind(external)
        .bind(at(2026, 8, 1).to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn expired_postings_are_excluded_from_the_live_list() {
        let pool = pool().await;
        insert(&pool, "live", false).await;
        insert(&pool, "gone", true).await;
        let loaded = load_live_postings(&pool, None).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "live");
    }

    #[tokio::test]
    async fn the_source_filter_matches_through_sightings() {
        let pool = pool().await;
        insert(&pool, "gh-only", false).await;
        insert(&pool, "ashby-only", false).await;
        sight(&pool, "gh-only", "greenhouse", "g-1").await;
        sight(&pool, "ashby-only", "ashby", "a-1").await;

        let gh = load_live_postings(&pool, Some("greenhouse")).await.unwrap();
        assert_eq!(gh.len(), 1);
        assert_eq!(gh[0].id, "gh-only");
        assert_eq!(load_live_postings(&pool, Some("nobody")).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn a_posting_carried_by_two_sources_is_returned_once() {
        // `EXISTS` rather than a join, precisely so the row is not duplicated per sighting.
        let pool = pool().await;
        insert(&pool, "shared", false).await;
        sight(&pool, "shared", "greenhouse", "g-1").await;
        sight(&pool, "shared", "simplify", "s-1").await;
        assert_eq!(load_live_postings(&pool, Some("greenhouse")).await.unwrap().len(), 1);
        assert_eq!(load_live_postings(&pool, None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_null_prestige_stays_unknown_rather_than_becoming_zero() {
        // The rule the ranking depends on: a company we know nothing about is not a company we
        // know to be bad.
        let pool = pool().await;
        sqlx::query(
            "INSERT INTO company_signals (company_key, company_name, first_seen_at, computed_at)
             VALUES ('acme', 'Acme', ?1, ?1)",
        )
        .bind(at(2026, 8, 1).to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        let signals = load_company_signals(&pool).await.unwrap();
        let acme = signals.get("acme").expect("keyed by company_key");
        assert_eq!(acme.prestige, None, "NULL prestige must not read as 0.0");
        assert_eq!(acme.median_pay_hourly_usd, None);
    }

    #[tokio::test]
    async fn source_names_come_from_live_postings_only() {
        // A dropdown built from expired-only sources offers filters that return nothing.
        let pool = pool().await;
        insert(&pool, "live", false).await;
        insert(&pool, "gone", true).await;
        sight(&pool, "live", "greenhouse", "g-1").await;
        sight(&pool, "gone", "deadsource", "d-1").await;
        assert_eq!(live_source_names(&pool).await.unwrap(), vec!["greenhouse"]);
    }
}
