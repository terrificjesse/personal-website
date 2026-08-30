//! The answer library (Phase 8g): questions you have already answered well.
//!
//! Retrieval only. Nothing here writes an answer into a form, and nothing here generates one —
//! the free-text answers are the part of an application that is actually you, and a
//! model-written one reads like every other model-written one. Surface, let the user pick, let
//! them edit.
//!
//! # Why this does not reuse `nlp.rs`
//!
//! `nlp::suggest_item_names` is a banded typeahead over short item names — prefix, substring
//! and edit-distance tiers tuned for "tomato" against a fridge inventory. Ranking
//! *question sentences* is a different problem, and its substring band is the specific reason:
//! nearly every application question shares words like "you", "your", "work" and "experience",
//! so a substring tier fires on everything and ranks by noise. This is the same conclusion
//! `internships::dedup` reached about pairwise identity, one subsystem over. `strsim` — which
//! `nlp.rs` itself uses — is applied directly here instead.
//!
//! # The company-specific trap
//!
//! "Why do you want to work at X" scores as near-identical across every application by any
//! measure a similarity function can see, and is the worst possible answer to reuse verbatim.
//! [`detect_company_specific`] flags those, and [`suggest`] never offers one written for a
//! different employer.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Below this similarity a stored answer is not offered at all. A weak match is worse than no
/// match: it invites a glance rather than a read, and the whole failure mode here is pasting
/// something that looked close enough.
pub const MIN_SIMILARITY: f64 = 0.45;

/// Longest question and answer we store. Unbounded text from a client is a denial-of-service
/// vector, not a feature — the blog's body cap, one subsystem over.
pub const MAX_QUESTION_LENGTH: usize = 2_000;
pub const MAX_ANSWER_LENGTH: usize = 20_000;

/// A stored answer.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct Answer {
    pub id: String,
    pub question_text: String,
    pub answer_text: String,
    pub is_company_specific: bool,
    /// Who it was written for, when known.
    pub company_name: Option<String>,
    pub tags: Option<String>,
    pub use_count: i64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An answer offered for a question, with the score that got it there.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    #[serde(flatten)]
    pub answer: Answer,
    /// `0.0..=1.0`. Published so a weak match is visibly weak rather than merely last.
    pub similarity: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewAnswer {
    pub question_text: String,
    pub answer_text: String,
    /// The employer whose form this was, when the caller knows it.
    pub company_name: Option<String>,
    pub tags: Option<String>,
}

/// Lowercase, drop punctuation, collapse whitespace.
///
/// Deliberately not stemming or removing stop words: the corpus is one person's answers, tens
/// of them at most, and every transformation that throws information away is one more way for
/// two different questions to look alike.
pub fn normalize_question(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Questions that are inherently about the employer, whatever company is named.
///
/// Matched against the normalized question. Generous by design — see the migration's note on
/// `is_company_specific`: a false positive costs one suggestion, a false negative costs the
/// application.
const COMPANY_QUESTION_MARKERS: &[&str] = &[
    "why do you want to work",
    "why would you like to work",
    "why are you interested in working",
    "why are you applying",
    "why this company",
    "why us",
    "why our",
    "what interests you about us",
    "what excites you about us",
    "what do you know about us",
    "about our company",
    "about our mission",
    "our team",
    "join us",
    "join our",
    "work here",
];

/// Whether this answer is about a particular employer and must not be reused for another.
///
/// Two signals: the question is inherently an employer question, or a company is named in the
/// question or the answer. The second catches the case the first cannot — a generic-sounding
/// question answered with "…which is why Stripe's approach appeals to me".
pub fn detect_company_specific(question: &str, answer: &str, company: Option<&str>) -> bool {
    let question_normalized = normalize_question(question);
    if COMPANY_QUESTION_MARKERS
        .iter()
        .any(|marker| question_normalized.contains(marker))
    {
        return true;
    }

    let Some(company) = company else {
        return false;
    };
    let company_normalized = normalize_question(company);
    if company_normalized.is_empty() {
        return false;
    }

    normalize_question(answer).contains(&company_normalized)
        || question_normalized.contains(&company_normalized)
}

/// Save an answer. Returns `None` if a field exceeded its cap.
pub async fn save(
    pool: &SqlitePool,
    user_id: &str,
    new: NewAnswer,
    now: DateTime<Utc>,
) -> Result<Option<Answer>> {
    let question = new.question_text.trim().to_string();
    let answer_text = new.answer_text.trim().to_string();
    if question.is_empty()
        || answer_text.is_empty()
        || question.len() > MAX_QUESTION_LENGTH
        || answer_text.len() > MAX_ANSWER_LENGTH
    {
        return Ok(None);
    }

    let company = new
        .company_name
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let company_specific = detect_company_specific(&question, &answer_text, company);
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO application_answers
             (id, user_id, question_text, question_normalized, answer_text,
              is_company_specific, company_name, tags, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(&question)
    .bind(normalize_question(&question))
    .bind(&answer_text)
    .bind(i64::from(company_specific))
    .bind(company)
    .bind(new.tags.as_deref().map(str::trim).filter(|t| !t.is_empty()))
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;

    Ok(get(pool, &id, user_id).await?)
}

const ANSWER_COLUMNS: &str = "id, question_text, answer_text, is_company_specific,
     company_name, tags, use_count, last_used_at, created_at, updated_at";

/// One answer, if it belongs to this user.
pub async fn get(pool: &SqlitePool, id: &str, user_id: &str) -> Result<Option<Answer>> {
    Ok(sqlx::query_as::<_, Answer>(&format!(
        "SELECT {ANSWER_COLUMNS} FROM application_answers WHERE id = ? AND user_id = ?"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?)
}

/// Every answer this user has, newest edit first.
pub async fn list(pool: &SqlitePool, user_id: &str) -> Result<Vec<Answer>> {
    Ok(sqlx::query_as::<_, Answer>(&format!(
        "SELECT {ANSWER_COLUMNS} FROM application_answers
          WHERE user_id = ? ORDER BY updated_at DESC"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

/// Answers worth offering for this question, best first.
///
/// # What is excluded, and why exclusion rather than a warning
///
/// A company-specific answer written for a *different* employer is not offered at all. The
/// alternative — showing it with a warning — assumes the warning is read at the moment of
/// copying, and the failure this guards against is precisely a hurried copy. An answer written
/// for the same employer is offered normally, because there it is exactly right.
pub async fn suggest(
    pool: &SqlitePool,
    user_id: &str,
    question: &str,
    company: Option<&str>,
    limit: usize,
) -> Result<Vec<Suggestion>> {
    let wanted = normalize_question(question);
    if wanted.is_empty() {
        return Ok(Vec::new());
    }

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, question_normalized FROM application_answers WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut scored: Vec<(String, f64)> = rows
        .into_iter()
        .map(|(id, stored)| {
            (
                id,
                strsim::normalized_damerau_levenshtein(&wanted, &stored),
            )
        })
        .filter(|(_, score)| *score >= MIN_SIMILARITY)
        .collect();

    // `f64` has no `Ord`; `total_cmp` rather than `partial_cmp().unwrap()`, which is the trap
    // `apps/fridge-app/CLAUDE.md` records from Phase 4.
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let asking_for = company.map(normalize_question);
    let mut out = Vec::new();
    for (id, similarity) in scored {
        let Some(answer) = get(pool, &id, user_id).await? else {
            continue;
        };
        if answer.is_company_specific {
            let written_for = answer.company_name.as_deref().map(normalize_question);
            // Offered only when we can positively tell it is the same employer. Unknown on
            // either side means we cannot, and cannot is treated as no.
            if written_for.is_none() || written_for != asking_for {
                continue;
            }
        }
        out.push(Suggestion { answer, similarity });
        if out.len() >= limit {
            break;
        }
    }

    Ok(out)
}

/// Replace an answer's text, keeping the previous version.
pub async fn edit(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    answer_text: &str,
    now: DateTime<Utc>,
) -> Result<Option<Answer>> {
    let text = answer_text.trim();
    if text.is_empty() || text.len() > MAX_ANSWER_LENGTH {
        return Ok(None);
    }

    let Some(existing) = get(pool, id, user_id).await? else {
        return Ok(None);
    };
    if existing.answer_text == text {
        return Ok(Some(existing));
    }

    // The revision first. If the update failed after the revision was written we would have a
    // harmless duplicate of the current text; the other order loses the old text outright.
    sqlx::query(
        "INSERT INTO answer_revisions (id, answer_id, answer_text, replaced_at)
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(id)
    .bind(&existing.answer_text)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;

    // The company-specific flag is recomputed: an edit that adds "…at Stripe" changes what
    // this answer is, and leaving the flag stale would let the edit smuggle it past the filter.
    let company_specific =
        detect_company_specific(&existing.question_text, text, existing.company_name.as_deref());

    sqlx::query(
        "UPDATE application_answers
            SET answer_text = ?1, is_company_specific = ?2, updated_at = ?3
          WHERE id = ?4 AND user_id = ?5",
    )
    .bind(text)
    .bind(i64::from(company_specific))
    .bind(now.to_rfc3339())
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;

    get(pool, id, user_id).await
}

/// Previous versions of an answer, newest first.
pub async fn revisions(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
) -> Result<Vec<(DateTime<Utc>, String)>> {
    if get(pool, id, user_id).await?.is_none() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_as(
        "SELECT replaced_at, answer_text FROM answer_revisions
          WHERE answer_id = ? ORDER BY replaced_at DESC",
    )
    .bind(id)
    .fetch_all(pool)
    .await?)
}

/// Record that an answer was actually used — not merely shown.
pub async fn mark_used(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let affected = sqlx::query(
        "UPDATE application_answers SET use_count = use_count + 1, last_used_at = ?1
          WHERE id = ?2 AND user_id = ?3",
    )
    .bind(now.to_rfc3339())
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Delete an answer and its revisions.
pub async fn delete(pool: &SqlitePool, id: &str, user_id: &str) -> Result<bool> {
    if get(pool, id, user_id).await?.is_none() {
        return Ok(false);
    }
    sqlx::query("DELETE FROM answer_revisions WHERE answer_id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    let affected = sqlx::query("DELETE FROM application_answers WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("answers-{}.db", Uuid::new_v4()));
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

    fn answer(question: &str, text: &str, company: Option<&str>) -> NewAnswer {
        NewAnswer {
            question_text: question.into(),
            answer_text: text.into(),
            company_name: company.map(str::to_string),
            tags: None,
        }
    }

    /// **The 8g checkpoint.** A "why do you want to work here" answer stored against one
    /// company is not offered for another, and a genuinely reusable one is.
    #[tokio::test]
    async fn a_company_specific_answer_is_not_offered_to_a_different_company() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let now = Utc::now();

        save(&pool, "u1", answer(
            "Why do you want to work here?",
            "I'm excited about Stripe's approach to developer experience.",
            Some("Stripe"),
        ), now).await.expect("save");

        save(&pool, "u1", answer(
            "Tell us about a personal project you're proud of.",
            "I built a fridge inventory app that estimates expiry dates from FoodKeeper data.",
            Some("Stripe"),
        ), now).await.expect("save");

        // Datadog asks both questions.
        let why = suggest(&pool, "u1", "Why do you want to work here?", Some("Datadog"), 5)
            .await
            .expect("suggest");
        assert!(
            why.is_empty(),
            "the Stripe answer must not be offered to Datadog, got {:?}",
            why.iter().map(|s| &s.answer.question_text).collect::<Vec<_>>()
        );

        let project = suggest(
            &pool, "u1", "Tell us about a personal project you're proud of.", Some("Datadog"), 5,
        )
        .await
        .expect("suggest");
        assert_eq!(project.len(), 1, "a reusable answer must still be offered");
        assert!(project[0].answer.answer_text.contains("fridge inventory"));
    }

    #[tokio::test]
    async fn the_same_company_still_gets_its_own_answer_back() {
        // Exclusion is about reuse across employers, not about hiding your own work from you.
        let pool = test_pool().await;
        user(&pool, "u1").await;
        save(&pool, "u1", answer(
            "Why do you want to work here?", "Stripe's docs culture.", Some("Stripe"),
        ), Utc::now()).await.expect("save");

        let again = suggest(&pool, "u1", "Why do you want to work here?", Some("Stripe"), 5)
            .await
            .expect("suggest");
        assert_eq!(again.len(), 1);
    }

    #[tokio::test]
    async fn an_unknown_asking_company_is_treated_as_a_different_one() {
        // "We cannot tell whose form this is" must not read as "it is fine to reuse".
        let pool = test_pool().await;
        user(&pool, "u1").await;
        save(&pool, "u1", answer(
            "Why do you want to work here?", "Stripe's docs culture.", Some("Stripe"),
        ), Utc::now()).await.expect("save");

        assert!(suggest(&pool, "u1", "Why do you want to work here?", None, 5)
            .await
            .expect("suggest")
            .is_empty());
    }

    #[test]
    fn a_company_named_only_in_the_answer_still_flags_it() {
        // The question looks generic; the answer is not. This is the case a question-pattern
        // check alone cannot catch.
        assert!(detect_company_specific(
            "What are you looking for in your next role?",
            "Somewhere with Stripe's engineering culture, ideally.",
            Some("Stripe"),
        ));
    }

    #[test]
    fn an_employer_question_flags_even_with_no_company_named() {
        for question in [
            "Why do you want to work here?",
            "Why are you applying to this role?",
            "What excites you about us?",
            "Why us?",
        ] {
            assert!(
                detect_company_specific(question, "Some answer.", None),
                "{question} should be flagged"
            );
        }
    }

    #[test]
    fn an_ordinary_question_is_not_flagged() {
        for question in [
            "Tell us about a project you're proud of.",
            "Describe a time you disagreed with a teammate.",
            "What is your greatest strength?",
        ] {
            assert!(
                !detect_company_specific(question, "Some answer.", Some("Stripe")),
                "{question} should not be flagged"
            );
        }
    }

    #[tokio::test]
    async fn a_weak_match_is_not_offered_at_all() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        save(&pool, "u1", answer(
            "Tell us about a personal project you're proud of.", "A fridge app.", None,
        ), Utc::now()).await.expect("save");

        let unrelated = suggest(&pool, "u1", "What is your expected salary?", None, 5)
            .await
            .expect("suggest");
        assert!(unrelated.is_empty(), "got {unrelated:?}");
    }

    #[tokio::test]
    async fn an_edit_keeps_the_previous_version() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let now = Utc::now();
        let saved = save(&pool, "u1", answer("A project?", "First draft.", None), now)
            .await
            .expect("save")
            .expect("some");

        edit(&pool, &saved.id, "u1", "A much better second version.", now)
            .await
            .expect("edit");

        assert_eq!(
            get(&pool, &saved.id, "u1").await.expect("get").unwrap().answer_text,
            "A much better second version."
        );
        let history = revisions(&pool, &saved.id, "u1").await.expect("revisions");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].1, "First draft.", "the regrettable rewrite is recoverable");
    }

    #[tokio::test]
    async fn an_edit_that_names_a_company_starts_being_treated_as_company_specific() {
        // Otherwise an edit smuggles a company mention past a flag computed at save time.
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let now = Utc::now();
        let saved = save(&pool, "u1", answer(
            "What are you looking for in your next role?", "Good mentorship.", Some("Stripe"),
        ), now).await.expect("save").expect("some");
        assert!(!saved.is_company_specific);

        let edited = edit(&pool, &saved.id, "u1", "Exactly what Stripe describes.", now)
            .await
            .expect("edit")
            .expect("some");
        assert!(edited.is_company_specific, "the edit changed what this answer is");
    }

    #[tokio::test]
    async fn one_users_answers_are_invisible_to_another() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        user(&pool, "u2").await;
        let saved = save(&pool, "u1", answer("A project?", "Mine.", None), Utc::now())
            .await.expect("save").expect("some");

        assert!(get(&pool, &saved.id, "u2").await.expect("get").is_none());
        assert!(list(&pool, "u2").await.expect("list").is_empty());
        assert!(suggest(&pool, "u2", "A project?", None, 5).await.expect("s").is_empty());
        assert!(!delete(&pool, &saved.id, "u2").await.expect("delete"));
        assert!(get(&pool, &saved.id, "u1").await.expect("get").is_some());
    }

    #[tokio::test]
    async fn use_is_counted_only_when_an_answer_is_actually_used() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let saved = save(&pool, "u1", answer("A project?", "A fridge app.", None), Utc::now())
            .await.expect("save").expect("some");

        // Being suggested is not being used.
        suggest(&pool, "u1", "A project?", None, 5).await.expect("suggest");
        assert_eq!(get(&pool, &saved.id, "u1").await.expect("g").unwrap().use_count, 0);

        mark_used(&pool, &saved.id, "u1", Utc::now()).await.expect("used");
        let after = get(&pool, &saved.id, "u1").await.expect("g").unwrap();
        assert_eq!(after.use_count, 1);
        assert!(after.last_used_at.is_some());
    }
}
