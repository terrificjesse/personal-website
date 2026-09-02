//! The CV details the extension fills into ATS forms (Phase 8f).
//!
//! One row per user, every field optional. Read by the extension over `GET /hunt/profile`,
//! edited on the site.
//!
//! # `None` and `Some("")` are different answers and must stay different
//!
//! This is the "absent is not zero" rule from the internship tab, one subsystem over and with
//! sharper teeth. A field the user has never filled in is `None`, and the autofill must skip
//! it. If it round-trips as `Some("")` instead, the mapper types a blank into the form — and a
//! blank in a required field looks filled to the user and empty to the recruiter.
//!
//! So [`clean`] collapses whitespace-only input to `None` on the way in, at exactly one place,
//! and nothing below it has to remember the distinction.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// A user's CV details. Every field optional — a half-filled profile is the normal state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct CvProfile {
    pub full_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub preferred_name: Option<String>,

    pub email: Option<String>,
    pub phone: Option<String>,
    pub location: Option<String>,

    pub school: Option<String>,
    pub degree: Option<String>,
    pub major: Option<String>,
    pub gpa: Option<String>,
    pub graduation_month: Option<i64>,
    pub graduation_year: Option<i64>,

    pub github_url: Option<String>,
    pub linkedin_url: Option<String>,
    pub portfolio_url: Option<String>,

    pub work_authorization: Option<String>,
    /// Three-state: `None` = not stated, and it must stay that way. Answering a sponsorship
    /// question on someone's behalf is not a convenience.
    pub needs_sponsorship: Option<bool>,

    /// Shown as a reminder. **Never uploaded** — see the migration.
    pub resume_path: Option<String>,
}

/// The longest we will store in any single field. An unbounded text column filled by a client
/// is a denial-of-service vector, not a feature — same cap reasoning as the blog's body.
pub const MAX_FIELD_LENGTH: usize = 500;

/// Trim, and treat whitespace-only as absent.
///
/// The single place that decision is made. A form field the user cleared should become `None`,
/// not `Some("")`, or the autofill will confidently type nothing into it.
fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Whether every field is within the length cap.
fn within_limits(profile: &CvProfile) -> bool {
    [
        &profile.full_name,
        &profile.first_name,
        &profile.last_name,
        &profile.preferred_name,
        &profile.email,
        &profile.phone,
        &profile.location,
        &profile.school,
        &profile.degree,
        &profile.major,
        &profile.gpa,
        &profile.github_url,
        &profile.linkedin_url,
        &profile.portfolio_url,
        &profile.work_authorization,
        &profile.resume_path,
    ]
    .iter()
    .all(|field| field.as_ref().is_none_or(|text| text.len() <= MAX_FIELD_LENGTH))
}

/// Normalize every text field, so nothing below this stores a blank as though it were a value.
fn normalized(profile: CvProfile) -> CvProfile {
    CvProfile {
        full_name: clean(profile.full_name),
        first_name: clean(profile.first_name),
        last_name: clean(profile.last_name),
        preferred_name: clean(profile.preferred_name),
        email: clean(profile.email),
        phone: clean(profile.phone),
        location: clean(profile.location),
        school: clean(profile.school),
        degree: clean(profile.degree),
        major: clean(profile.major),
        gpa: clean(profile.gpa),
        github_url: clean(profile.github_url),
        linkedin_url: clean(profile.linkedin_url),
        portfolio_url: clean(profile.portfolio_url),
        work_authorization: clean(profile.work_authorization),
        resume_path: clean(profile.resume_path),
        ..profile
    }
}

/// This user's profile. An empty one for a user who has never saved — not an error, and not
/// `None`: "you have not filled this in yet" is a profile with nothing in it.
pub async fn get(pool: &SqlitePool, user_id: &str) -> Result<CvProfile> {
    let found: Option<CvProfile> = sqlx::query_as(
        "SELECT full_name, first_name, last_name, preferred_name, email, phone, location,
                school, degree, major, gpa, graduation_month, graduation_year,
                github_url, linkedin_url, portfolio_url,
                work_authorization, needs_sponsorship, resume_path
           FROM cv_profile WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(found.unwrap_or_default())
}

/// Replace this user's profile wholesale. Returns `None` if a field exceeded the length cap.
///
/// A full replace rather than a patch: the editor sends the whole form, and a partial update
/// would make "I cleared this field" indistinguishable from "I did not send this field".
pub async fn put(
    pool: &SqlitePool,
    user_id: &str,
    profile: CvProfile,
    now: DateTime<Utc>,
) -> Result<Option<CvProfile>> {
    let profile = normalized(profile);
    if !within_limits(&profile) {
        return Ok(None);
    }

    sqlx::query(
        "INSERT INTO cv_profile (
             user_id, full_name, first_name, last_name, preferred_name,
             email, phone, location,
             school, degree, major, gpa, graduation_month, graduation_year,
             github_url, linkedin_url, portfolio_url,
             work_authorization, needs_sponsorship, resume_path,
             created_at, updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?21)
         ON CONFLICT (user_id) DO UPDATE SET
             full_name = excluded.full_name,
             first_name = excluded.first_name,
             last_name = excluded.last_name,
             preferred_name = excluded.preferred_name,
             email = excluded.email,
             phone = excluded.phone,
             location = excluded.location,
             school = excluded.school,
             degree = excluded.degree,
             major = excluded.major,
             gpa = excluded.gpa,
             graduation_month = excluded.graduation_month,
             graduation_year = excluded.graduation_year,
             github_url = excluded.github_url,
             linkedin_url = excluded.linkedin_url,
             portfolio_url = excluded.portfolio_url,
             work_authorization = excluded.work_authorization,
             needs_sponsorship = excluded.needs_sponsorship,
             resume_path = excluded.resume_path,
             updated_at = excluded.updated_at",
    )
    .bind(user_id)
    .bind(&profile.full_name)
    .bind(&profile.first_name)
    .bind(&profile.last_name)
    .bind(&profile.preferred_name)
    .bind(&profile.email)
    .bind(&profile.phone)
    .bind(&profile.location)
    .bind(&profile.school)
    .bind(&profile.degree)
    .bind(&profile.major)
    .bind(&profile.gpa)
    .bind(profile.graduation_month)
    .bind(profile.graduation_year)
    .bind(&profile.github_url)
    .bind(&profile.linkedin_url)
    .bind(&profile.portfolio_url)
    .bind(&profile.work_authorization)
    .bind(profile.needs_sponsorship)
    .bind(&profile.resume_path)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;

    Ok(Some(profile))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    async fn test_pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("cv-profile-{}.db", Uuid::new_v4()));
        crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("migrations")
    }

    async fn user(pool: &SqlitePool, id: &str) {
        sqlx::query("INSERT INTO users (id, email, password_hash, created_at) VALUES (?1,?2,?3,?4)")
            .bind(id)
            .bind(format!("{id}@example.com"))
            .bind("x")
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await
            .expect("user");
    }

    fn filled() -> CvProfile {
        CvProfile {
            full_name: Some("Ada Lovelace".into()),
            email: Some("ada@example.com".into()),
            phone: Some("555-0100".into()),
            graduation_year: Some(2027),
            needs_sponsorship: Some(false),
            ..CvProfile::default()
        }
    }

    #[tokio::test]
    async fn a_user_who_has_never_saved_gets_an_empty_profile_not_an_error() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        assert_eq!(get(&pool, "u1").await.expect("get"), CvProfile::default());
    }

    #[tokio::test]
    async fn a_saved_profile_round_trips() {
        let pool = test_pool().await;
        user(&pool, "u1").await;

        put(&pool, "u1", filled(), Utc::now()).await.expect("put");
        assert_eq!(get(&pool, "u1").await.expect("get"), filled());
    }

    #[tokio::test]
    async fn a_blank_field_is_stored_as_absent_not_as_an_empty_string() {
        // THE rule this module exists for. `Some("")` would make the autofill type nothing
        // into a required field, which looks filled to the user and empty to the recruiter.
        let pool = test_pool().await;
        user(&pool, "u1").await;

        let mut profile = filled();
        profile.phone = Some("   ".into());
        profile.location = Some("".into());
        put(&pool, "u1", profile, Utc::now()).await.expect("put");

        let stored = get(&pool, "u1").await.expect("get");
        assert_eq!(stored.phone, None, "whitespace-only must become absent");
        assert_eq!(stored.location, None);
        assert_eq!(stored.full_name, Some("Ada Lovelace".into()), "and the rest survives");
    }

    #[tokio::test]
    async fn clearing_a_field_actually_clears_it() {
        // The reason this is a PUT and not a PATCH: a partial update cannot tell "I cleared
        // this" from "I did not send this".
        let pool = test_pool().await;
        user(&pool, "u1").await;

        put(&pool, "u1", filled(), Utc::now()).await.expect("put");
        let cleared = CvProfile { full_name: Some("Ada Lovelace".into()), ..CvProfile::default() };
        put(&pool, "u1", cleared, Utc::now()).await.expect("put");

        let stored = get(&pool, "u1").await.expect("get");
        assert_eq!(stored.phone, None);
        assert_eq!(stored.email, None);
        assert_eq!(stored.full_name, Some("Ada Lovelace".into()));
    }

    #[tokio::test]
    async fn not_stated_sponsorship_stays_not_stated() {
        // Three states, and the third is the point. Defaulting this to `false` answers a
        // legally meaningful question on the user's behalf.
        let pool = test_pool().await;
        user(&pool, "u1").await;

        put(&pool, "u1", CvProfile::default(), Utc::now()).await.expect("put");
        assert_eq!(get(&pool, "u1").await.expect("get").needs_sponsorship, None);

        let says_no = CvProfile { needs_sponsorship: Some(false), ..CvProfile::default() };
        put(&pool, "u1", says_no, Utc::now()).await.expect("put");
        assert_eq!(
            get(&pool, "u1").await.expect("get").needs_sponsorship,
            Some(false),
            "an explicit no is not the same as saying nothing"
        );
    }

    #[tokio::test]
    async fn an_oversized_field_is_refused_rather_than_truncated() {
        let pool = test_pool().await;
        user(&pool, "u1").await;

        let huge = CvProfile {
            full_name: Some("a".repeat(MAX_FIELD_LENGTH + 1)),
            ..CvProfile::default()
        };

        assert!(put(&pool, "u1", huge, Utc::now()).await.expect("put").is_none());
        assert_eq!(get(&pool, "u1").await.expect("get"), CvProfile::default());
    }

    #[tokio::test]
    async fn one_users_profile_is_invisible_to_another() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        user(&pool, "u2").await;

        put(&pool, "u1", filled(), Utc::now()).await.expect("put");
        assert_eq!(get(&pool, "u2").await.expect("get"), CvProfile::default());
    }
}
