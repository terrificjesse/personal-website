//! The classifier. **A stub until 8b** — it is here so the sync has the shape it will need,
//! not because it decides anything.
//!
//! # Why the shape matters before the implementation does
//!
//! Rule 1: the real classifier sits upstream of a token that will eventually be able to
//! relabel a mailbox, and it reads content written by strangers. So it is a **pure function**
//! — email in, a constrained enum out. It gets no tools, no database handle, and no ability to
//! act. Every write happens in Rust, outside it, switching on the value it returned.
//!
//! Fixing that signature now means 8b fills in a body rather than choosing an architecture
//! under time pressure. A classifier that could act would be a different thing entirely, and
//! much harder to take the power back from later.
//!
//! Rule 8: the category is decided **from the email alone**, before any match against an
//! application is attempted. An unmatched interview invite is still an interview invite.

use serde::{Deserialize, Serialize};

/// What an email is about.
///
/// Mirrors `internship_applications.status` where it can, because that is the structural idea
/// of the whole phase: the folders already exist as application statuses, so classification is
/// "propose a transition", not "pick a folder".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Confirmation,
    Oa,
    Interview,
    Offer,
    Rejection,
    /// Job-specific, addressed to you, but matching no application — a recruiter about an
    /// opening, an ATS invite for something you did not apply to. A **terminal bucket, not a
    /// pipeline stage**: it never creates a tracker row, because `applied_at` means you
    /// applied.
    Outreach,
    /// Correctly ignored. The highest-volume path, and still **recorded** — rule 7.
    Disregarded,
}

impl Category {
    /// Whether this is one of the categories worth interrupting someone for.
    ///
    /// Rule 8: a pressing email is labelled and alerted **even with no matched application**.
    /// An unmatched interview invite is the single most costly thing this tool could drop.
    pub fn is_pressing(self) -> bool {
        matches!(self, Category::Oa | Category::Interview | Category::Offer)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Category::Confirmation => "confirmation",
            Category::Oa => "oa",
            Category::Interview => "interview",
            Category::Offer => "offer",
            Category::Rejection => "rejection",
            Category::Outreach => "outreach",
            Category::Disregarded => "disregarded",
        }
    }
}

/// What the classifier returns. Never an action, never a label name, never SQL.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EmailVerdict {
    pub category: Category,
    pub confidence: f64,
    /// A company name guessed from the email, for the matcher to use as a hint. The match
    /// itself happens afterwards and separately — enrichment, not a gate.
    pub company_guess: Option<String>,
    /// Why. Also where text in an email that was *addressed at the agent* gets surfaced:
    /// that is data worth recording, never an instruction to follow.
    pub evidence: String,
}

/// The 8a stub: everything is disregarded, and says so.
///
/// Deliberately not a guess. A half-built rules layer shipped early would produce numbers that
/// look like classification and measure nothing, and 8b's checkpoint depends on measuring
/// against real mail — a stub that is obviously a stub cannot be mistaken for a baseline.
pub fn classify(_from: Option<&str>, _subject: Option<&str>, _snippet: Option<&str>) -> EmailVerdict {
    EmailVerdict {
        category: Category::Disregarded,
        confidence: 0.0,
        company_guess: None,
        evidence: "8a stub: no classification has been attempted".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_classifies_nothing_and_admits_it() {
        let verdict = classify(Some("a@b.com"), Some("Interview invitation"), Some("..."));
        assert_eq!(verdict.category, Category::Disregarded);
        assert_eq!(verdict.confidence, 0.0);
        assert!(verdict.evidence.contains("stub"));
    }

    #[test]
    fn the_pressing_categories_are_the_three_that_cost_you_something() {
        for category in [Category::Oa, Category::Interview, Category::Offer] {
            assert!(category.is_pressing(), "{category:?}");
        }
        for category in [
            Category::Confirmation,
            Category::Rejection,
            Category::Outreach,
            Category::Disregarded,
        ] {
            assert!(!category.is_pressing(), "{category:?}");
        }
    }

    #[test]
    fn every_category_matches_the_migration_check_constraint() {
        // The stored spelling is a contract with SQL, which the compiler cannot check — the
        // "Rust cannot check the inside of a string" trap this repo records.
        let allowed = [
            "confirmation", "oa", "interview", "offer", "rejection", "outreach", "disregarded",
        ];
        for category in [
            Category::Confirmation, Category::Oa, Category::Interview, Category::Offer,
            Category::Rejection, Category::Outreach, Category::Disregarded,
        ] {
            assert!(allowed.contains(&category.as_str()), "{category:?}");
        }
    }
}
