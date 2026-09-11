//! Matching an email to an application, and deciding what it may do to that application's
//! status. **Rules 2 and 3 live here**, as pure functions.
//!
//! No database, no network, no side effects — everything expensive to get wrong is decidable
//! from arguments, so it is testable without a mailbox. The writing happens in `sync`.

use crate::internships::models::ApplicationStatus;
use crate::internships::normalize::company_key;

use super::classify::Category;

/// How far along a status is.
///
/// **Rule 3: status advances; it does not follow the newest email.** Email order is not event
/// order — the OA arrives, then a bulk "thanks for applying" autoresponder lands three days
/// late. Naive "latest email wins" drags an interview back to `applied`, and Phase 7 made
/// `status_changed_at` load-bearing ("how long have I been at this stage"), so that is real
/// state destroyed.
fn rank(status: ApplicationStatus) -> u8 {
    match status {
        ApplicationStatus::Applied => 1,
        ApplicationStatus::Oa => 2,
        ApplicationStatus::Interview => 3,
        // Both terminal, and equal: neither follows the other.
        ApplicationStatus::Offer | ApplicationStatus::Rejected => 4,
    }
}

/// Whether a status ends the story. A terminal verdict may move *backwards in rank*, because
/// it is genuinely later in truth: being rejected after an interview is not a regression.
fn is_terminal(status: ApplicationStatus) -> bool {
    matches!(status, ApplicationStatus::Offer | ApplicationStatus::Rejected)
}

/// The status an email of this category implies, if any.
pub fn implied_status(category: Category) -> Option<ApplicationStatus> {
    match category {
        Category::Confirmation => Some(ApplicationStatus::Applied),
        Category::Oa => Some(ApplicationStatus::Oa),
        Category::Interview => Some(ApplicationStatus::Interview),
        Category::Offer => Some(ApplicationStatus::Offer),
        Category::Rejection => Some(ApplicationStatus::Rejected),
        // Neither says anything about an application's stage. Outreach is a terminal bucket,
        // not a pipeline stage — it never even creates a tracker row.
        Category::Outreach | Category::Disregarded => None,
    }
}

/// Whether moving from `from` to `to` is a move this system is allowed to propose.
///
/// Forward only, except a terminal verdict, which may land from anywhere. Everything else is
/// refused — including a same-rank move, which proposes nothing and is noise.
pub fn may_advance(from: ApplicationStatus, to: ApplicationStatus) -> bool {
    if from == to {
        return false;
    }
    if is_terminal(to) {
        // ...but not from another terminal state. An offer does not become a rejection because
        // a late autoresponder arrived, and a rejection does not become an offer.
        return !is_terminal(from);
    }
    rank(to) > rank(from)
}

/// Whether this proposal may be applied without a human looking at it.
///
/// **Rule 2.** Three conditions, and all three matter:
///
/// - it must be a forward move, which [`may_advance`] already guarantees;
/// - it must clear the confidence threshold;
/// - and it must not be terminal. **`offer` and `rejected` end the story and are never
///   auto-applied**, at any confidence. A false positive there destroys the record of where
///   you actually were, and there is no signal that says "actually, still interviewing".
///
/// `threshold` is `None` by default — nothing auto-applies until the classifier has been
/// measured against real mail. Guessing a number is worse than measuring one, and 8b's
/// checkpoint is what supplies it.
pub fn may_auto_apply(to: ApplicationStatus, confidence: f64, threshold: Option<f64>) -> bool {
    if is_terminal(to) {
        return false;
    }
    threshold.is_some_and(|threshold| confidence >= threshold)
}

/// An application as the matcher needs to see it.
///
/// Carries the title because the unit of matching is a (company, role) pair — see
/// [`match_application`].
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TrackedApplication {
    pub id: String,
    pub company: String,
    pub title: Option<String>,
}

/// The best application for an email, by company and role.
///
/// Fuzzy, and **enrichment rather than a gate** — rule 8. A miss means the email is still
/// classified, still labelled and still alerted, with no application attached.
///
/// Reuses `internships::normalize::company_key`, which already collapses "Roblox", "Roblox
/// Corporation" and "roblox," to one key, rather than being a third normalizer that drifts
/// from the two that exist.
pub fn match_application<'a>(
    company_guess: Option<&str>,
    role_guess: Option<&str>,
    applications: &'a [TrackedApplication],
) -> Option<&'a str> {
    let wanted = company_key(company_guess?);
    if wanted.is_empty() {
        return None;
    }
    // The role the email names, if it names one. Empty means "this email does not say", which
    // is not the same as "a different role" — see the filter below.
    let wanted_role = super::untracked::role_key(role_guess);

    let mut best: Option<(&str, f64)> = None;
    for TrackedApplication { id, company, title } in applications {
        let key = company_key(company);
        if key.is_empty() {
            continue;
        }
        // **An application is a (company, role) pair, not a company.** Nine Microsoft
        // confirmations for nine different roles all matched the one tracked Microsoft
        // application, so eight real applications were never proposed and the tracker read 14
        // when it should have read far more.
        //
        // Asymmetric on purpose: when the email names no role we fall back to company-only
        // matching, because refusing to match there would propose a duplicate for every
        // roleless autoresponder. When it DOES name one, it has to agree.
        if !wanted_role.is_empty() {
            let their_role = super::untracked::role_key(title.as_deref());
            if !their_role.is_empty() && their_role != wanted_role {
                continue;
            }
        }
        let score = if key == wanted {
            1.0
        } else {
            strsim::normalized_damerau_levenshtein(&wanted, &key)
        };
        // A high bar on purpose. The cost of a wrong match is an email attached to somebody
        // else's application, which then proposes a status change on it — far worse than the
        // no-match case, which rule 8 already makes safe.
        if score >= 0.9 && best.is_none_or(|(_, current)| score > current) {
            best = Some((id.as_str(), score));
        }
    }
    best.map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ApplicationStatus::*;

    #[test]
    fn a_late_autoresponder_cannot_drag_an_interview_back_to_applied() {
        // **8c's checkpoint.** The OA arrives, then a bulk "thanks for applying" lands three
        // days late. Naive "latest email wins" destroys real state.
        assert!(!may_advance(Interview, Applied));
        assert!(!may_advance(Oa, Applied));
        assert!(!may_advance(Interview, Oa));
    }

    #[test]
    fn forward_moves_are_allowed() {
        assert!(may_advance(Applied, Oa));
        assert!(may_advance(Applied, Interview));
        assert!(may_advance(Oa, Interview));
        assert!(may_advance(Interview, Offer));
    }

    #[test]
    fn a_terminal_verdict_may_arrive_from_anywhere() {
        // Being rejected after an interview is not a regression — it is genuinely later.
        assert!(may_advance(Interview, Rejected));
        assert!(may_advance(Applied, Rejected));
        assert!(may_advance(Applied, Offer));
    }

    #[test]
    fn one_terminal_state_does_not_become_another() {
        // An offer does not turn into a rejection because a late autoresponder arrived.
        assert!(!may_advance(Offer, Rejected));
        assert!(!may_advance(Rejected, Offer));
        assert!(!may_advance(Rejected, Rejected));
    }

    #[test]
    fn a_move_to_the_same_status_proposes_nothing() {
        for status in [Applied, Oa, Interview, Offer, Rejected] {
            assert!(!may_advance(status, status), "{status:?}");
        }
    }

    #[test]
    fn offers_and_rejections_are_never_auto_applied_at_any_confidence() {
        // Rule 2, and the one that has no undo: they end the story, and nothing later says
        // "actually, still interviewing".
        for terminal in [Offer, Rejected] {
            assert!(!may_auto_apply(terminal, 1.0, Some(0.5)), "{terminal:?}");
            assert!(!may_auto_apply(terminal, 0.99, Some(0.1)), "{terminal:?}");
        }
    }

    #[test]
    fn nothing_auto_applies_while_the_threshold_is_unset() {
        // The default. Guessing a threshold is worse than measuring one, and 8b's checkpoint
        // is what supplies it.
        for status in [Oa, Interview] {
            assert!(!may_auto_apply(status, 1.0, None), "{status:?}");
        }
    }

    #[test]
    fn a_confident_forward_move_auto_applies_once_a_threshold_exists() {
        assert!(may_auto_apply(Interview, 0.9, Some(0.8)));
        assert!(!may_auto_apply(Interview, 0.7, Some(0.8)));
    }

    #[test]
    fn only_the_categories_that_say_something_about_a_stage_imply_one() {
        assert_eq!(implied_status(Category::Oa), Some(Oa));
        assert_eq!(implied_status(Category::Rejection), Some(Rejected));
        assert_eq!(implied_status(Category::Confirmation), Some(Applied));
        assert_eq!(implied_status(Category::Outreach), None);
        assert_eq!(implied_status(Category::Disregarded), None);
    }

    fn app(id: &str, company: &str, title: Option<&str>) -> TrackedApplication {
        TrackedApplication {
            id: id.into(),
            company: company.into(),
            title: title.map(str::to_string),
        }
    }

    fn applications() -> Vec<TrackedApplication> {
        vec![
            app("a1", "Roblox", Some("Software Engineer Intern")),
            app("a2", "Jump Trading", None),
            app("a3", "Tesla", Some("Firmware Intern")),
        ]
    }

    #[test]
    fn a_company_matches_its_application() {
        assert_eq!(match_application(Some("roblox"), None, &applications()), Some("a1"));
        assert_eq!(match_application(Some("Jump Trading"), None, &applications()), Some("a2"));
    }

    #[test]
    fn a_suffix_variant_still_matches() {
        // company_key already collapses these; this asserts we are actually using it.
        assert_eq!(
            match_application(Some("Roblox Corporation"), None, &applications()),
            Some("a1")
        );
    }

    #[test]
    fn an_unknown_company_matches_nothing_rather_than_the_nearest_one() {
        // The expensive failure is attaching an email to someone else's application, which
        // then proposes a status change on it. Rule 8 already makes the no-match case safe.
        assert_eq!(match_application(Some("Datadog"), None, &applications()), None);
        assert_eq!(match_application(Some("Stripe"), None, &applications()), None);
        assert_eq!(match_application(None, None, &applications()), None);
    }

    #[test]
    fn a_different_role_at_the_same_company_is_a_different_application() {
        // The bug that made the tracker read 14. Nine Microsoft confirmations for nine roles
        // all matched the one tracked Microsoft application, so eight were never proposed.
        assert_eq!(
            match_application(Some("Roblox"), Some("Software Engineer Intern"), &applications()),
            Some("a1"),
            "the same role still matches"
        );
        assert_eq!(
            match_application(Some("Roblox"), Some("Firmware Engineering Intern"), &applications()),
            None,
            "a different role at the same company is a different application"
        );
    }

    #[test]
    fn an_email_that_names_no_role_still_matches_on_company_alone() {
        // Asymmetric on purpose. Refusing to match a roleless autoresponder would propose a
        // duplicate for every "thanks for applying" that does not name the job — which is the
        // failure the original per-company key was written to prevent, and it is still real.
        assert_eq!(match_application(Some("Roblox"), None, &applications()), Some("a1"));
    }

    #[test]
    fn a_tracked_application_with_no_title_matches_any_role() {
        // We know nothing about a2's role, so an email naming one cannot contradict it. The
        // alternative is proposing a duplicate for an application we already have.
        assert_eq!(
            match_application(Some("Jump Trading"), Some("Quant Intern"), &applications()),
            Some("a2")
        );
    }
}
