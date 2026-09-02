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

/// What the classifier is allowed to know, beyond the email itself.
///
/// Passed in rather than looked up, because [`classify`] is a **pure function** — rule 1. It
/// gets no database handle and no tools, so anything it needs about the world arrives here
/// and the caller decides what that is.
#[derive(Debug, Clone, Default)]
pub struct Context<'a> {
    /// Companies we have collected postings from, lowercased. Used only to decide whether an
    /// email names a *specific* employer, which is the line between outreach and junk.
    pub known_companies: &'a [String],
}

/// Terminal verdicts, checked before anything else and in this order.
///
/// **Rejection comes first on purpose.** "Thank you for interviewing with us — unfortunately
/// we will not be moving forward" contains an interview marker and is not an interview. Every
/// other order lets a rejection read as the stage it is rejecting you from, which is rule 3's
/// trap arriving through the classifier instead of through timestamps.
const REJECTION: &[&str] = &[
    "unfortunately",
    "we regret",
    "not be moving forward",
    "not moving forward",
    "will not be progressing",
    "decided to move forward with other",
    "no longer under consideration",
    "were not selected",
    "not selected for",
    "pursue other candidates",
];

const OFFER: &[&str] = &[
    "offer of employment",
    "pleased to offer",
    "extend an offer",
    "your offer",
    "offer letter",
];

const INTERVIEW: &[&str] = &[
    "interview",
    "phone screen",
    "schedule a call",
    "schedule some time",
    "schedule time",
    "meet with the team",
    "speak with you",
    "next round",
];

/// An assessment you are being **asked to do** — not one merely mentioned.
///
/// Bare "assessment" was here first and it was wrong on real mail: a recruiter wrote "you are
/// currently at the application/assessment stage with Roblox", which is context, and it
/// classified as an OA. That inflates `pressing`, which is the count that decides whether you
/// get interrupted — so the marker has to carry the *ask*, not the topic.
///
/// The named platforms stay bare because you are never sent a HackerRank link for reference.
const ASSESSMENT: &[&str] = &[
    "online assessment",
    "assessment invitation",
    "assessments invitation",
    "complete the assessment",
    "complete your assessment",
    "coding challenge",
    "code challenge",
    "take home",
    "take-home",
    "technical screen",
    "hackerrank",
    "codesignal",
    "codility",
    "karat",
];

/// An invitation *to* an assessment, where the two words are separated by template prose.
/// "We're thrilled to invite you to the next step of the recruiting process — the assessments!"
const ASSESSMENT_PAIRS: &[(&str, &str)] = &[
    ("invite you", "assessment"),
    ("invitation", "assessment"),
    ("next step", "assessment"),
];

const CONFIRMATION: &[&str] = &[
    "thank you for applying",
    "thanks for applying",
    "we have received your application",
    "we've received your application",
    "application received",
    "received your application",
    "your application to",
    "thank you for your interest in",
    // Paycom's wording, from a real application confirmation that was disregarded because
    // only the "your interest" phrasing was listed.
    "expressing interest in",
    "expressed interest in",
    "application was submitted",
    // Real mail: "Thank you for submitting your application for a position at Roblox!" —
    // the "applying"/"received" families both miss it.
    "submitting your application",
    "submitted your application",
    "for submitting your",
];

/// Bulk mail that is *literally* job-related and still junk.
///
/// This is the relevance gate, and the line is **specificity, not topic**. A burner inbox used
/// for applications fills with Indeed digests, staffing blasts and bootcamp marketing — all
/// about jobs, none about *your* application. Key the rules on the word "job" and
/// `Hunt/Outreach` becomes the same undifferentiated pile the inbox already is.
const BULK: &[&str] = &[
    "jobs for you",
    "new jobs",
    "job alert",
    "jobs you may be interested",
    "recommended for you",
    "top picks for you",
    "hiring now",
    "apply now to",
    "we found jobs",
    "your job search",
    "unsubscribe from job",
    "webinar",
    "master's program",
    "masters program",
    "bootcamp",
    // Event RSVPs and registrations. A recruiting event you replied to is not an application
    // and not a role — and this one reached Hunt/Outreach only because the sender's domain
    // was connect.roblox.com and the address did not happen to contain "noreply", which is a
    // thin basis for deciding a human wrote to you.
    "rsvp",
    "thanks for your response to",
    "thank you for your response to",
    "you are registered",
    "thanks for registering",
];

/// Senders that are machines. Not junk by itself — most ATS mail is a no-reply — but it is the
/// difference between a person writing to you and a system announcing something.
fn is_machine_sender(from: &str) -> bool {
    let from = from.to_lowercase();
    [
        "no-reply",
        "noreply",
        "donotreply",
        "do-not-reply",
        "notifications@",
        "mailer@",
        "systemmessage@",
    ]
    .iter()
    .any(|marker| from.contains(marker))
}

/// Domains that only ever carry application mail.
const ATS_DOMAINS: &[&str] = &[
    "greenhouse.io",
    "lever.co",
    "ashbyhq.com",
    "myworkday.com",
    "workday.com",
    "smartrecruiters.com",
    "icims.com",
    "workable.com",
    "rippling.com",
    // Found in real mail: an application confirmation arrived from msg.paycomonline.com and
    // matched nothing. The same shape as Phase 7's ATS-coverage gap, one subsystem over.
    "paycomonline.com",
    "myworkdayjobs.com",
    "taleo.net",
    "brassring.com",
    "jobvite.com",
];

/// Lowercase, and flatten the punctuation real subject lines actually use.
///
/// Mail clients and ATS templates emit typographic quotes and dashes constantly — Tesla's
/// confirmation is "Thank you – we've received your Tesla application", with an en-dash and a
/// curly apostrophe. A marker written with an ASCII apostrophe silently never matches it, and
/// silently-never-matching is the failure mode this whole classifier is judged on.
fn haystack(subject: Option<&str>, snippet: Option<&str>) -> String {
    format!("{} {}", subject.unwrap_or(""), snippet.unwrap_or(""))
        .to_lowercase()
        .replace(['\u{2018}', '\u{2019}'], "'")
        .replace(['\u{201c}', '\u{201d}'], "\"")
        .replace(['\u{2013}', '\u{2014}'], "-")
}

fn hit<'a>(text: &str, markers: &[&'a str]) -> Option<&'a str> {
    markers.iter().copied().find(|marker| text.contains(marker))
}

/// Markers that only work as a pair, because something sits between them.
///
/// "We've received your **Tesla** application" is the case that forced this: the company name
/// is inside the phrase, so no single substring matches. Both halves must be present, and
/// order is not required — templates vary.
const CONFIRMATION_PAIRS: &[(&str, &str)] = &[
    ("received your", "application"),
    ("application", "has been received"),
    ("application", "was submitted"),
];

fn hit_pair<'a>(text: &str, pairs: &[(&'a str, &'a str)]) -> Option<(&'a str, &'a str)> {
    pairs
        .iter()
        .copied()
        .find(|(a, b)| text.contains(a) && text.contains(b))
}

/// Classify one email from its metadata alone.
///
/// **Rule 8: the category is decided here, from the email, before any match against an
/// application is attempted.** The matcher is fuzzy and will miss — a company styled
/// differently in mail than on its posting, a subsidiary, an ATS sending as
/// `no-reply@greenhouse.io` — and if "unmatched" routed to disregard, one miss would silently
/// eat an interview invite.
pub fn classify(
    from: Option<&str>,
    subject: Option<&str>,
    snippet: Option<&str>,
    context: &Context<'_>,
) -> EmailVerdict {
    let text = haystack(subject, snippet);
    let sender = from.unwrap_or("").to_lowercase();

    let verdict = |category: Category, confidence: f64, evidence: String| EmailVerdict {
        category,
        confidence,
        company_guess: guess_company(&sender, &text, context),
        evidence,
    };

    // Terminal outcomes first. See REJECTION's note on why it leads.
    if let Some(marker) = hit(&text, REJECTION) {
        return verdict(Category::Rejection, 0.9, format!("rejection marker: {marker:?}"));
    }
    if let Some(marker) = hit(&text, OFFER) {
        return verdict(Category::Offer, 0.85, format!("offer marker: {marker:?}"));
    }

    // Then the two that need a response from you. Interview before assessment: "interview" is
    // the more specific claim, and an email that mentions both is usually inviting you to one.
    if let Some(marker) = hit(&text, INTERVIEW) {
        return verdict(Category::Interview, 0.8, format!("interview marker: {marker:?}"));
    }
    if let Some(marker) = hit(&text, ASSESSMENT) {
        return verdict(Category::Oa, 0.8, format!("assessment marker: {marker:?}"));
    }
    if let Some((a, b)) = hit_pair(&text, ASSESSMENT_PAIRS) {
        return verdict(Category::Oa, 0.7, format!("assessment pair: {a:?} + {b:?}"));
    }

    if let Some(marker) = hit(&text, CONFIRMATION) {
        return verdict(Category::Confirmation, 0.85, format!("confirmation marker: {marker:?}"));
    }
    if let Some((a, b)) = hit_pair(&text, CONFIRMATION_PAIRS) {
        return verdict(Category::Confirmation, 0.8, format!("confirmation pair: {a:?} + {b:?}"));
    }

    // The relevance gate. Checked AFTER the pressing categories, never before: a digest
    // subject line must not be able to swallow a real interview invite that happens to
    // contain the word "jobs".
    if let Some(marker) = hit(&text, BULK) {
        return verdict(Category::Disregarded, 0.7, format!("bulk mail marker: {marker:?}"));
    }

    // Job-specific and addressed to you, but about no application you made.
    let from_ats = ATS_DOMAINS.iter().any(|domain| sender.contains(domain));
    let named_company = guess_company(&sender, &text, context);
    let from_a_person = !sender.is_empty() && !is_machine_sender(&sender);

    if from_ats || (named_company.is_some() && from_a_person) {
        return verdict(
            Category::Outreach,
            0.5,
            match &named_company {
                Some(company) => format!("names {company}, and a person sent it"),
                None => "from an ATS domain, but matches no application".to_string(),
            },
        );
    }

    // Everything else. The highest-volume path, and still recorded — rule 7.
    verdict(
        Category::Disregarded,
        0.6,
        "no application, employer or job-specific signal".to_string(),
    )
}

/// A company named in the sender's domain or the text, if we know of one.
///
/// A hint for the matcher, never a gate. Longest match wins so "jump trading" beats "jump".
fn guess_company(sender: &str, text: &str, context: &Context<'_>) -> Option<String> {
    let mut best: Option<&String> = None;
    for company in context.known_companies {
        if company.len() < 3 {
            continue;
        }
        let squashed = company.replace(' ', "");
        let mentioned =
            contains_whole_word(text, company.as_str()) || contains_whole_word(sender, &squashed);
        if mentioned && best.is_none_or(|current| company.len() > current.len()) {
            best = Some(company);
        }
    }
    best.cloned()
}

/// Whether `needle` occurs in `haystack` bounded by non-alphanumeric characters on both sides.
///
/// # Why a bare `contains` was wrong
///
/// The company list is real and contains three-letter names — `exa`, `kla`, `zip`, `imc`,
/// `amd`, `sage`. As bare substrings those match the *inside of ordinary words*, and every one
/// of these was observed on live burner-inbox mail:
///
/// | Sender | Matched | Because |
/// |---|---|---|
/// | `systemmessage@paycomonline.com` | Sage | "mes**sage**" |
/// | `oklahoma city thunder <donotreply@…>` | KLA | "o**kla**homa" |
/// | `jobs@ziprecruiter.com` | Zip | "**zip**recruiter" |
///
/// That is not a cosmetic defect. `company_guess` is the hint `advance::match_application`
/// keys on, so a name invented out of the middle of a word is rule 2's failure — an email
/// matched to an application it has nothing to do with. Pointed at the relevance gate it is
/// the other one: a job-board digest that "names a specific employer" is exactly the junk that
/// is supposed to fall through to disregarded.
///
/// Requiring a boundary keeps every true positive in the live corpus, because a company name
/// that is really there is delimited by something — `@`, `.`, a space, or the end of the
/// string. `no-reply@jumptrading.com` still finds Jump Trading via the squashed form, and a
/// display name like `Zip Hiring Team <no-reply@ashbyhq.com>` still finds Zip.
fn contains_whole_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(needle) {
        let start = from + offset;
        let end = start + needle.len();
        // Both indices land on char boundaries — `find` returns one, and the other is the end
        // of the matched needle — so these slices are safe. Compared as `char`s rather than
        // bytes: a byte-level check reads the second half of a two-byte letter like `ç` as a
        // non-alphanumeric and would call the middle of "çzip" a word boundary.
        let open = haystack[..start].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let close = haystack[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        if open && close {
            return true;
        }
        from = start + haystack[start..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn companies() -> Vec<String> {
        ["roblox", "tesla", "jump trading", "stripe", "datadog"]
            .iter()
            .map(|c| c.to_string())
            .collect()
    }

    fn classify_with(from: &str, subject: &str, snippet: &str) -> EmailVerdict {
        let known = companies();
        classify(
            Some(from),
            Some(subject),
            Some(snippet),
            &Context { known_companies: &known },
        )
    }

    // --- Every one of these is a real message from the burner inbox, verbatim. ------------

    #[test]
    fn the_real_confirmations_are_confirmations() {
        for (from, subject) in [
            ("Tesla <noreply@tesla.com>", "Thank you – we've received your Tesla application"),
            ("no-reply@roblox.com", "Thank you for applying to Roblox!"),
            ("no-reply@jumptrading.com", "Thank you for applying to Jump Trading!"),
        ] {
            let verdict = classify_with(from, subject, "");
            assert_eq!(verdict.category, Category::Confirmation, "{subject} -> {verdict:?}");
        }
    }

    #[test]
    fn the_real_assessment_emails_are_pressing() {
        // The two that actually needed something from the user. Getting these wrong is the
        // expensive direction — a missed OA is a lost application.
        for subject in [
            "[Action Required] Your Roblox Application - Online Assessment",
            "[Action Required] Your Roblox Assessments Invitation",
        ] {
            let verdict = classify_with("Roblox Assessment <assessment@email.roblox.com>", subject, "");
            assert_eq!(verdict.category, Category::Oa, "{subject} -> {verdict:?}");
            assert!(verdict.category.is_pressing());
        }
    }

    #[test]
    fn a_security_alert_is_not_employment() {
        let verdict = classify_with(
            "Google <no-reply@accounts.google.com>",
            "Security alert",
            "A new sign-in on Windows",
        );
        assert_eq!(verdict.category, Category::Disregarded);
    }

    #[test]
    fn a_named_person_at_a_known_company_is_outreach() {
        // Borderline, and decided deliberately: a human at an employer we know of, about
        // something job-related, is the "addressed to you" side of the line. If this proves
        // too generous the boundary moves HERE, in one place, rather than by loosening rules
        // elsewhere.
        let verdict = classify_with(
            "Sophia Pressman <spressman@roblox.com>",
            "Roblox Week @ CMU - 9/8-9/10",
            "Come meet the team",
        );
        assert_eq!(verdict.category, Category::Outreach);
        assert_eq!(verdict.company_guess.as_deref(), Some("roblox"));
    }

    #[test]
    fn an_event_rsvp_is_not_an_application_confirmation() {
        // "Thanks for RSVPing" must not trip the "thanks for applying" family.
        let verdict = classify_with(
            "On Campus 2026 <oncampus2026@example.com>",
            "Thanks for RSVPing to On Campus 2026 CMU Kaiju Cats x Cookies",
            "",
        );
        assert_ne!(verdict.category, Category::Confirmation, "{verdict:?}");
    }

    // --- Ordering, which is where this gets subtly wrong ---------------------------------

    #[test]
    fn a_rejection_that_mentions_an_interview_is_a_rejection() {
        // THE ordering trap. Every other order lets a rejection read as the stage it is
        // rejecting you from — rule 3 arriving through the classifier instead of timestamps.
        let verdict = classify_with(
            "no-reply@greenhouse.io",
            "Your application to Datadog",
            "Thank you for interviewing with us. Unfortunately we will not be moving forward.",
        );
        assert_eq!(verdict.category, Category::Rejection, "{verdict:?}");
    }

    #[test]
    fn a_rejection_after_an_assessment_is_still_a_rejection() {
        let verdict = classify_with(
            "no-reply@lever.co",
            "Update on your application",
            "Thanks for completing the online assessment. We regret to say we are moving on.",
        );
        assert_eq!(verdict.category, Category::Rejection, "{verdict:?}");
    }

    #[test]
    fn a_digest_cannot_swallow_a_real_interview_invite() {
        // The relevance gate is checked AFTER the pressing categories for exactly this: a
        // subject containing "new jobs" must not be able to disregard an interview.
        let verdict = classify_with(
            "recruiter@stripe.com",
            "Interview invitation — and some new jobs you may like",
            "",
        );
        assert_eq!(verdict.category, Category::Interview, "{verdict:?}");
    }

    // --- The relevance gate ---------------------------------------------------------------

    #[test]
    fn job_related_bulk_mail_is_disregarded() {
        // Literally about jobs, and still junk. Key the rules on the word "job" and the
        // outreach folder becomes the pile the inbox already is.
        for subject in [
            "10 new jobs for you this week",
            "Your job alert: software intern",
            "Jobs you may be interested in",
            "Free webinar: break into tech",
            "Apply now to our data science bootcamp",
        ] {
            let verdict = classify_with("noreply@jobboard.example.com", subject, "");
            assert_eq!(verdict.category, Category::Disregarded, "{subject} -> {verdict:?}");
        }
    }

    #[test]
    fn a_machine_at_an_unknown_domain_saying_nothing_specific_is_disregarded() {
        let verdict = classify_with("noreply@shop.example.com", "Your receipt", "Order #123");
        assert_eq!(verdict.category, Category::Disregarded);
    }

    #[test]
    fn paycom_systemmessage_sender_is_a_machine() {
        assert!(is_machine_sender("systemmessage@paycomonline.com"));
    }

    #[test]
    fn ats_mail_that_matches_nothing_is_still_kept_as_outreach() {
        // Rule 8's spirit: an ATS wrote to you about something. It is not junk just because
        // no application of ours matches it.
        let verdict = classify_with(
            "no-reply@ashbyhq.com",
            "An update from the hiring team",
            "",
        );
        assert_eq!(verdict.category, Category::Outreach);
    }

    // --- Shape --------------------------------------------------------------------------


    // --- Real snippets that the first version of these rules got wrong -------------------

    #[test]
    fn an_assessment_merely_mentioned_is_not_an_assessment_invitation() {
        // Verbatim from a recruiter's email. Bare "assessment" matched this and called it an
        // OA, inflating the count that decides whether you get interrupted.
        let verdict = classify_with(
            "Sophia Pressman <spressman@roblox.com>",
            "Roblox Week @ CMU - 9/8-9/10",
            "Hi Jesse, Hope you're having a great weekend! I wanted to reach out since you \
             are currently at the application/assessment stage with Roblox.",
        );
        assert_ne!(verdict.category, Category::Oa, "{verdict:?}");
        assert_eq!(verdict.category, Category::Outreach);
    }

    #[test]
    fn an_actual_assessment_invitation_still_registers() {
        // The other half: the ask, separated from the noun by template prose.
        let verdict = classify_with(
            "Roblox Assessment <noreply@email.roblox.com>",
            "[Action Required] Your Roblox Assessments Invitation",
            "Hi Jesse, We're thrilled to invite you to the next step of the recruiting \
             process — the assessments!",
        );
        assert_eq!(verdict.category, Category::Oa, "{verdict:?}");
    }

    #[test]
    fn a_verify_your_email_application_receipt_is_a_confirmation() {
        // Subject says "[Action Required] Your Roblox Application", which reads pressing and
        // is not: the body is an email verification for an application just submitted.
        let verdict = classify_with(
            "no-reply@roblox.com",
            "[Action Required] Your Roblox Application",
            "Email Verification Hi Jesse, Thank you for submitting your application for a \
             position at Roblox! Please click here to verify your email address",
        );
        assert_eq!(verdict.category, Category::Confirmation, "{verdict:?}");
    }


    // --- The held-out set: real mail that arrived AFTER the rules were written -----------

    #[test]
    fn an_event_rsvp_confirmation_is_disregarded_not_outreach() {
        // Verbatim. It reached Hunt/Outreach because the sender's domain contains "roblox"
        // and the address lacks "noreply" — a thin basis for deciding a human wrote to you.
        let verdict = classify_with(
            "On Campus 2026 CMU Kaiju Cats x Cookies <oncampus2026kaijucatsxcookies@connect.roblox.com>",
            "Thanks for RSVPing to On Campus 2026 CMU Kaiju Cats x Cookies",
            "Thanks for your response to On Campus 2026 CMU Kaiju Cats x Cookies Name: Jesse Li",
        );
        assert_eq!(verdict.category, Category::Disregarded, "{verdict:?}");
    }

    #[test]
    fn an_application_account_setup_is_a_confirmation() {
        // Verbatim, and the expensive direction: real application mail was being dropped.
        // "Thank you for expressing interest in" is Paycom's wording, and only the
        // "your interest in" phrasing was listed.
        let verdict = classify_with(
            "Oklahoma City Thunder <donotreply@msg.paycomonline.com>",
            "Oklahoma City Thunder Password setup",
            "You have received a new message from Oklahoma City Thunder. Hi Jesse Li! Thank \
             you for expressing interest in the Software Engineer Intern position",
        );
        assert_eq!(verdict.category, Category::Confirmation, "{verdict:?}");
    }

    #[test]
    fn a_real_recruiter_email_is_still_outreach() {
        // The RSVP rule must not swallow the case Hunt/Outreach exists for.
        let verdict = classify_with(
            "Sophia Pressman <spressman@roblox.com>",
            "Roblox Week @ CMU - 9/8-9/10",
            "Hi Jesse, I wanted to reach out about the team.",
        );
        assert_eq!(verdict.category, Category::Outreach, "{verdict:?}");
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

    #[test]
    fn every_verdict_carries_its_reason() {
        // A verdict with no evidence is a number nobody can argue with. `posting_rejects`
        // one subsystem over exists for the same reason.
        let verdict = classify_with("no-reply@roblox.com", "Thank you for applying to Roblox!", "");
        assert!(!verdict.evidence.is_empty());
        assert!(verdict.evidence.contains("thank you for applying"), "{verdict:?}");
    }

    #[test]
    fn the_longest_known_company_wins_the_guess() {
        let known = vec!["jump".to_string(), "jump trading".to_string()];
        let verdict = classify(
            Some("no-reply@jumptrading.com"),
            Some("Thank you for applying to Jump Trading!"),
            Some(""),
            &Context { known_companies: &known },
        );
        assert_eq!(verdict.company_guess.as_deref(), Some("jump trading"));
    }

    /// The company list really does contain three-letter names, and these three senders are
    /// verbatim from the burner inbox. Each one used to "name a company" out of the middle of
    /// an ordinary word.
    #[test]
    fn a_company_name_inside_an_ordinary_word_is_not_a_company_mention() {
        let known: Vec<String> =
            ["sage", "kla", "zip", "exa"].iter().map(|c| c.to_string()).collect();
        let context = Context { known_companies: &known };

        for (from, subject) in [
            // "mes-SAGE-".
            ("systemmessage@paycomonline.com", "Your application"),
            // "o-KLA-homa".
            ("Oklahoma City Thunder <donotreply@msg.paycomonline.com>", "Thanks"),
            // "ZIP-recruiter" — a job board, not the company called Zip.
            ("jobs@ziprecruiter.com", "Openings near you"),
            // "-EXA-mple".
            ("hr@somecorp.example", "Hello"),
        ] {
            let verdict = classify(Some(from), Some(subject), Some(""), &context);
            assert_eq!(
                verdict.company_guess, None,
                "{from} should name no company, got {:?}",
                verdict.company_guess
            );
        }
    }

    /// The other half of the same fix: a name that is genuinely there is still found, whether
    /// it arrives in the domain or in the display name.
    #[test]
    fn a_company_named_at_a_word_boundary_is_still_found() {
        let known: Vec<String> =
            ["zip", "roblox", "jump trading"].iter().map(|c| c.to_string()).collect();
        let context = Context { known_companies: &known };

        // Squashed, as a whole domain label.
        let from_domain = classify(Some("no-reply@jumptrading.com"), Some("Hi"), Some(""), &context);
        assert_eq!(from_domain.company_guess.as_deref(), Some("jump trading"));

        // In the display name, where the domain belongs to the ATS rather than the employer.
        let from_display =
            classify(Some("Zip Hiring Team <no-reply@ashbyhq.com>"), Some("Hi"), Some(""), &context);
        assert_eq!(from_display.company_guess.as_deref(), Some("zip"));

        // In the subject line.
        let from_text = classify(Some("a@b.test"), Some("Roblox Week @ CMU"), Some(""), &context);
        assert_eq!(from_text.company_guess.as_deref(), Some("roblox"));
    }

    #[test]
    fn whole_word_matching_handles_the_edges() {
        assert!(contains_whole_word("zip hiring team", "zip"), "start of string");
        assert!(contains_whole_word("team at zip", "zip"), "end of string");
        assert!(contains_whole_word("a@zip.com", "zip"), "delimited by punctuation");
        assert!(!contains_whole_word("ziprecruiter", "zip"), "prefix of a longer word");
        assert!(!contains_whole_word("unzip", "zip"), "suffix of a longer word");
        assert!(!contains_whole_word("message", "sage"), "inside a word");
        assert!(!contains_whole_word("anything", ""), "an empty needle names nothing");
        // A non-ASCII neighbour is a boundary, and must not panic on a byte index mid-char.
        assert!(contains_whole_word("café zip", "zip"));
        assert!(!contains_whole_word("çzip", "zip"));
    }
}
