//! Résumé variants: which résumé went with which application (Phase 12f).
//!
//! Full reasoning in `docs/HUNT.md` § Résumé variants. The three things this module exists to
//! enforce:
//!
//! - **A variant is a label, never a file.** Nothing here accepts bytes.
//! - **Renaming is an `UPDATE`**, because applications reference the id. History follows a
//!   rename automatically, which is the whole reason the foreign key is not on the label.
//! - **Retiring is `archived_at`; deleting a referenced variant is refused.** Cascading would
//!   delete the evidence and nulling would silently turn measured rows into unattributed ones,
//!   which is worse than an error message.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Longest label and note we store. Unbounded client text is a denial-of-service vector, not a
/// feature — the same cap the answer library and the blog body carry.
pub const MAX_LABEL_LENGTH: usize = 120;
pub const MAX_NOTES_LENGTH: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct ResumeVariant {
    pub id: String,
    pub label: String,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    /// How many applications reference it. The number that makes "can I delete this?"
    /// answerable in the UI before the answer arrives as a 409.
    pub application_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewVariant {
    pub label: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EditVariant {
    pub label: Option<String>,
    pub notes: Option<String>,
    /// `Some(true)` retires it, `Some(false)` brings it back, `None` leaves it alone.
    pub archived: Option<bool>,
}

/// Why a write was refused, so the route can answer with the right status rather than a 500.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// An empty label, or one past the cap.
    BadLabel,
    /// `UNIQUE (user_id, label)` — the report would be ambiguous.
    DuplicateLabel,
    /// Applications reference it, so deleting would destroy the attribution.
    InUse,
}

const COLUMNS: &str = "v.id, v.label, v.notes, v.created_at, v.archived_at,
     (SELECT COUNT(*) FROM internship_applications a WHERE a.resume_variant_id = v.id)
        AS application_count";

/// Active variants first, then archived — the order a picker wants.
pub async fn list(pool: &SqlitePool, user_id: &str) -> Result<Vec<ResumeVariant>> {
    Ok(sqlx::query_as::<_, ResumeVariant>(&format!(
        "SELECT {COLUMNS} FROM resume_variants v
          WHERE v.user_id = ?1
          ORDER BY v.archived_at IS NOT NULL, v.label COLLATE NOCASE"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get(pool: &SqlitePool, user_id: &str, id: &str) -> Result<Option<ResumeVariant>> {
    Ok(sqlx::query_as::<_, ResumeVariant>(&format!(
        "SELECT {COLUMNS} FROM resume_variants v WHERE v.id = ?1 AND v.user_id = ?2"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn create(
    pool: &SqlitePool,
    user_id: &str,
    body: NewVariant,
    now: DateTime<Utc>,
) -> Result<Result<ResumeVariant, Refused>> {
    let Some(label) = clean(&body.label, MAX_LABEL_LENGTH) else {
        return Ok(Err(Refused::BadLabel));
    };
    let notes = body.notes.as_deref().and_then(|n| clean(n, MAX_NOTES_LENGTH));

    let id = Uuid::new_v4().to_string();
    let insert = sqlx::query(
        "INSERT INTO resume_variants (id, user_id, label, notes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(&label)
    .bind(&notes)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await;

    if let Err(err) = insert {
        // UNIQUE (user_id, label) is a database guarantee rather than a check-then-insert
        // race, the same call `internship_applications` makes about "already applied".
        if err.as_database_error().is_some_and(|e| e.is_unique_violation()) {
            return Ok(Err(Refused::DuplicateLabel));
        }
        return Err(err.into());
    }

    Ok(Ok(get(pool, user_id, &id)
        .await?
        .expect("just inserted")))
}

/// Rename, re-note, retire or restore. Attribution is by id, so a rename changes nothing about
/// which applications belong to it.
pub async fn edit(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
    body: EditVariant,
    now: DateTime<Utc>,
) -> Result<Option<Result<ResumeVariant, Refused>>> {
    if get(pool, user_id, id).await?.is_none() {
        return Ok(None);
    }

    if let Some(raw) = body.label.as_deref() {
        let Some(label) = clean(raw, MAX_LABEL_LENGTH) else {
            return Ok(Some(Err(Refused::BadLabel)));
        };
        let renamed = sqlx::query("UPDATE resume_variants SET label = ?1 WHERE id = ?2 AND user_id = ?3")
            .bind(&label)
            .bind(id)
            .bind(user_id)
            .execute(pool)
            .await;
        if let Err(err) = renamed {
            if err.as_database_error().is_some_and(|e| e.is_unique_violation()) {
                return Ok(Some(Err(Refused::DuplicateLabel)));
            }
            return Err(err.into());
        }
    }

    if let Some(raw) = body.notes.as_deref() {
        let notes = clean(raw, MAX_NOTES_LENGTH);
        sqlx::query("UPDATE resume_variants SET notes = ?1 WHERE id = ?2 AND user_id = ?3")
            .bind(&notes)
            .bind(id)
            .bind(user_id)
            .execute(pool)
            .await?;
    }

    if let Some(archived) = body.archived {
        sqlx::query("UPDATE resume_variants SET archived_at = ?1 WHERE id = ?2 AND user_id = ?3")
            .bind(archived.then(|| now.to_rfc3339()))
            .bind(id)
            .bind(user_id)
            .execute(pool)
            .await?;
    }

    Ok(Some(Ok(get(pool, user_id, id).await?.expect("still there"))))
}

/// Delete, unless anything references it.
///
/// `Ok(None)` is "no such variant"; `Ok(Some(Err(InUse)))` is the refusal that protects the
/// attribution from being deleted by a tidy-up.
pub async fn delete(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
) -> Result<Option<Result<(), Refused>>> {
    let Some(variant) = get(pool, user_id, id).await? else {
        return Ok(None);
    };
    if variant.application_count > 0 {
        return Ok(Some(Err(Refused::InUse)));
    }

    sqlx::query("DELETE FROM resume_variants WHERE id = ?1 AND user_id = ?2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(Some(Ok(())))
}

/// Whether this variant may be attached to a new application: it must exist, belong to the
/// caller, and not be archived. Archived means "no longer sending this one".
pub async fn is_attachable(pool: &SqlitePool, user_id: &str, id: &str) -> Result<bool> {
    Ok(get(pool, user_id, id)
        .await?
        .is_some_and(|variant| variant.archived_at.is_none()))
}

fn clean(text: &str, max: usize) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().count() > max {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("variants-{}.db", Uuid::new_v4()));
        crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("migrations")
    }

    async fn user(pool: &SqlitePool) -> String {
        let id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO users (id, email, created_at) VALUES (?1, ?2, ?3)")
            .bind(&id)
            .bind(format!("{id}@example.com"))
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await
            .unwrap();
        id
    }

    async fn named(pool: &SqlitePool, user_id: &str, label: &str) -> ResumeVariant {
        create(
            pool,
            user_id,
            NewVariant { label: label.into(), notes: None },
            Utc::now(),
        )
        .await
        .unwrap()
        .unwrap()
    }

    /// Attach a variant to an application without going through the route, so these stay about
    /// storage rather than about HTTP.
    async fn application_using(pool: &SqlitePool, user_id: &str, variant_id: &str) -> String {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO internship_applications
                (id, user_id, company_name, title, url, snapshot_json, snapshot_at, status,
                 applied_at, status_changed_at, created_at, updated_at, resume_variant_id)
             VALUES (?1, ?2, 'Jump Trading', 'SWE Intern', 'https://x/j', '{}', ?3,
                     'applied', ?3, ?3, ?3, ?3, ?4)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(&now)
        .bind(variant_id)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn a_label_is_unique_per_user_because_a_duplicate_makes_every_report_ambiguous() {
        let pool = pool().await;
        let me = user(&pool).await;
        let someone_else = user(&pool).await;
        named(&pool, &me, "one-page, systems").await;

        assert_eq!(
            create(&pool, &me, NewVariant { label: " one-page, systems ".into(), notes: None }, Utc::now())
                .await
                .unwrap(),
            Err(Refused::DuplicateLabel),
            "trimmed to the same thing is the same thing"
        );
        // Another account is a separate namespace; nothing is shared here.
        assert!(
            create(&pool, &someone_else, NewVariant { label: "one-page, systems".into(), notes: None }, Utc::now())
                .await
                .unwrap()
                .is_ok()
        );
    }

    #[tokio::test]
    async fn an_empty_or_oversized_label_is_refused() {
        let pool = pool().await;
        let me = user(&pool).await;

        for label in ["", "   ", &"x".repeat(MAX_LABEL_LENGTH + 1)] {
            assert_eq!(
                create(&pool, &me, NewVariant { label: label.into(), notes: None }, Utc::now())
                    .await
                    .unwrap(),
                Err(Refused::BadLabel)
            );
        }
    }

    /// The property the whole schema is shaped around: applications reference the id, so a
    /// rename cannot orphan them.
    #[tokio::test]
    async fn renaming_keeps_every_application_that_used_it() {
        let pool = pool().await;
        let me = user(&pool).await;
        let variant = named(&pool, &me, "one-page").await;
        application_using(&pool, &me, &variant.id).await;

        let renamed = edit(
            &pool,
            &me,
            &variant.id,
            EditVariant { label: Some("one-page, systems".into()), notes: None, archived: None },
            Utc::now(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();

        assert_eq!(renamed.label, "one-page, systems");
        assert_eq!(renamed.application_count, 1, "history followed the rename");
    }

    /// Deleting a variant in use would delete the evidence for the only number this produces.
    #[tokio::test]
    async fn a_variant_in_use_cannot_be_deleted_but_can_be_retired() {
        let pool = pool().await;
        let me = user(&pool).await;
        let variant = named(&pool, &me, "two-page").await;
        application_using(&pool, &me, &variant.id).await;

        assert_eq!(
            delete(&pool, &me, &variant.id).await.unwrap(),
            Some(Err(Refused::InUse))
        );

        let archived = edit(
            &pool,
            &me,
            &variant.id,
            EditVariant { label: None, notes: None, archived: Some(true) },
            Utc::now(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert!(archived.archived_at.is_some());
        assert_eq!(archived.application_count, 1, "retiring keeps the history");
    }

    #[tokio::test]
    async fn an_unused_variant_is_deletable() {
        let pool = pool().await;
        let me = user(&pool).await;
        let variant = named(&pool, &me, "draft").await;

        assert_eq!(delete(&pool, &me, &variant.id).await.unwrap(), Some(Ok(())));
        assert!(get(&pool, &me, &variant.id).await.unwrap().is_none());
    }

    /// Archived means "no longer sending this one", so it must not be attachable to a NEW
    /// application — that would record something that did not happen.
    #[tokio::test]
    async fn an_archived_variant_is_not_attachable_but_still_exists() {
        let pool = pool().await;
        let me = user(&pool).await;
        let variant = named(&pool, &me, "old").await;
        assert!(is_attachable(&pool, &me, &variant.id).await.unwrap());

        edit(&pool, &me, &variant.id,
             EditVariant { label: None, notes: None, archived: Some(true) }, Utc::now())
            .await.unwrap().unwrap().unwrap();

        assert!(!is_attachable(&pool, &me, &variant.id).await.unwrap());
        assert!(get(&pool, &me, &variant.id).await.unwrap().is_some(), "still readable, and still in reports");
    }

    #[tokio::test]
    async fn another_accounts_variant_is_invisible_and_unattachable() {
        let pool = pool().await;
        let me = user(&pool).await;
        let someone_else = user(&pool).await;
        let theirs = named(&pool, &someone_else, "theirs").await;

        assert!(get(&pool, &me, &theirs.id).await.unwrap().is_none());
        assert!(!is_attachable(&pool, &me, &theirs.id).await.unwrap());
        assert!(list(&pool, &me).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn active_variants_are_listed_before_archived_ones() {
        let pool = pool().await;
        let me = user(&pool).await;
        let old = named(&pool, &me, "aaa-old").await;
        named(&pool, &me, "zzz-current").await;
        edit(&pool, &me, &old.id,
             EditVariant { label: None, notes: None, archived: Some(true) }, Utc::now())
            .await.unwrap().unwrap().unwrap();

        let labels: Vec<String> = list(&pool, &me).await.unwrap().into_iter().map(|v| v.label).collect();
        assert_eq!(labels, vec!["zzz-current", "aaa-old"], "a picker wants what you still send first");
    }
}
