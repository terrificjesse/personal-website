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

    /// The inverse of [`as_str`](Category::as_str), for reading a stored verdict back.
    ///
    /// Exhaustive over the same match rather than a lookup table, so adding a category is a
    /// compile error here instead of a silent `None` at the point something reads the database.
    pub fn parse(raw: &str) -> Option<Self> {
        [
            Category::Confirmation,
            Category::Oa,
            Category::Interview,
            Category::Offer,
            Category::Rejection,
            Category::Outreach,
            Category::Disregarded,
        ]
        .into_iter()
        .find(|category| category.as_str() == raw)
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
    // Pronoun-agnostic. `"we regret"` was here alone and missed *"I regret to inform you"* on a
    // real rejection — the formulation a named recruiter uses rather than a template.
    "regret to inform",
    "we regret",
    "not be moving forward",
    "not moving forward",
    // "not be moving forward" is not a substring of the contraction, and the contraction is how
    // most of these are actually written.
    "won't be moving forward",
    "will not be progressing",
    "not be proceeding",
    "decided not to move forward",
    "decided to move forward with other",
    "no longer under consideration",
    "were not selected",
    "not selected for",
    "pursue other candidates",
    // Fit-based refusals, which name no decision verb at all. Every one is negated on purpose:
    // a bare "ideal fit" would read an enthusiastic offer as a rejection, and REJECTION is
    // checked first so it would win.
    "not an ideal fit",
    "isn't an ideal fit",
    "not the right fit",
    "not a fit at this time",
    "position has been filled",
    "role has been filled",
];

const OFFER: &[&str] = &[
    "offer of employment",
    "pleased to offer",
    "extend an offer",
    "your offer",
    "offer letter",
];

/// An interview you are being **invited to** — not one merely described.
///
/// Bare `"interview"` was here and it was wrong on real mail, in exactly the way bare
/// `"assessment"` was wrong below. Two confirmations were classified as interviews on 2026-09-10:
/// *"help prepare you for the interview process"* and *"details on the interview"*. Both are
/// describing a process; neither is an invitation. The word is the topic, not the ask.
///
/// **ASSESSMENT already learned this and the lesson was never carried across** — its own comment
/// says the marker has to carry the ask rather than the topic. This list now does too.
///
/// Kept deliberately broad within that constraint. Rule 8: an unmatched interview invitation is
/// the single most costly thing this tool can drop, so the bar is "does this phrase carry an
/// invitation", not "is this phrase common".
const INTERVIEW: &[&str] = &[
    "invite you to interview",
    "invitation to interview",
    "interview invitation",
    "invite you to an interview",
    "like to interview you",
    "would like to interview",
    "to interview you",
    "schedule an interview",
    "schedule your interview",
    "scheduling your interview",
    "set up an interview",
    "book an interview",
    "book your interview",
    "confirm your interview",
    "your interview is",
    "interview has been scheduled",
    "interview is scheduled",
    "phone screen",
    "schedule a call",
    "schedule some time",
    "schedule time",
    "meet with the team",
    "like to speak with you",
    // Rounds and slot-booking, which invite you without using the word "interview" as a verb.
    // The 13f gate caught the absence of these: tightening this list dropped `syn-012`,
    // *"Please book a time — final round"*, whose snippet reads "select a slot for your final
    // round interview". That is an invitation with no invitation verb anywhere in it, and it is
    // exactly the miss rule 8 says costs the most.
    "next round",
    "final round",
    "round interview",
    "book a time",
    "select a slot",
    "select a time",
    "choose a time",
    "pick a time",
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
    let joined = format!("{} {}", subject.unwrap_or(""), snippet.unwrap_or(""));
    decode_entities(&joined)
        .to_lowercase()
        .replace(['\u{2018}', '\u{2019}'], "'")
        .replace(['\u{201c}', '\u{201d}'], "\"")
        .replace(['\u{2013}', '\u{2014}'], "-")
}

/// Undo the HTML escaping Gmail applies to `snippet`.
///
/// **Snippets arrive escaped**, so a contraction reaches the markers as `isn&#39;t` and any
/// marker containing an apostrophe silently never fires. Found 2026-09-10 on a real rejection —
/// *"there isn&#39;t an ideal fit at this time"* — which no marker could have caught.
///
/// The existing marker lists contain no apostrophes at all, which reads like a style choice and
/// was really the bug: the lists were written around it rather than it being fixed. Decoding
/// here means a marker can be written the way the sentence is actually spoken.
///
/// Deliberately a short fixed table rather than a dependency. These are the entities Gmail
/// actually emits in snippets; anything else passes through unchanged and simply fails to match,
/// which is the safe direction.
fn decode_entities(text: &str) -> String {
    text.replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&rsquo;", "'")
        .replace("&lsquo;", "'")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&ldquo;", "\"")
        .replace("&rdquo;", "\"")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        // Last: an escaped ampersand can encode another entity, so unescaping it first would
        // let "&amp;#39;" become an apostrophe that was never in the text.
        .replace("&amp;", "&")
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
        // The corpus contains names that are not employers — `internship` and `internship
        // list` are really in it. Because this loop prefers the LONGEST match, junk like that
        // outranks a real company whose name is shorter, and `internship` beats `tesla` on any
        // message that says the word. Skipping them here is what stops a 119-posting employer
        // losing to one posting of parsing debris.
        if !crate::internships::company_match::is_company_name(company) {
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

    #[test]
    fn junk_in_the_company_corpus_cannot_outrank_a_real_employer() {
        // A live defect, 2026-09-05. The postings corpus really contains `Internship` and
        // `Internship List` as company names, and this function prefers the LONGEST match — so
        // `internship` (10 chars) beat `tesla` (5) on a genuine Tesla confirmation, against a
        // company with 119 postings. The email was then never proposed as an application at
        // all, because a guess of "internship" is filtered downstream as not-a-company.
        let companies: Vec<String> = ["tesla", "internship", "internship list"]
            .iter()
            .map(|c| c.to_string())
            .collect();
        let context = Context { known_companies: &companies };

        let verdict = classify(
            Some("Tesla <noreply@tesla.com>"),
            Some("Thank you \u{2013} we\u{2019}ve received your Tesla application"),
            Some("Your internship application is being reviewed."),
            &context,
        );
        assert_eq!(
            verdict.company_guess.as_deref(),
            Some("tesla"),
            "a real employer must win against parsing debris in its own corpus"
        );
    }

    #[test]
    fn an_ats_hostname_is_not_read_as_the_employer() {
        // `workiva@myworkday.com` and `no-reply@ashbyhq.com` send for hundreds of companies.
        // Guessing the ATS would attach the mail to the wrong employer with full confidence.
        let companies: Vec<String> = ["workday", "ashby", "greenhouse"]
            .iter()
            .map(|c| c.to_string())
            .collect();
        let context = Context { known_companies: &companies };

        let verdict = classify(
            Some("Workiva Careers <workiva@myworkday.com>"),
            Some("Workiva Careers: Application for Summer 2027 Intern has been Received!"),
            None,
            &context,
        );
        assert_eq!(verdict.company_guess, None, "no employer is better than the wrong one");
    }

    // ---- 2026-09-10: rejections missed, confirmations read as interviews ----

    fn verdict_for(subject: &str, snippet: &str) -> Category {
        let companies = vec!["stripe".to_string(), "optiver".to_string()];
        let ctx = Context { known_companies: &companies };
        classify(Some("no-reply@example.com"), Some(subject), Some(snippet), &ctx).category
    }

    #[test]
    fn a_rejection_does_not_have_to_say_we() {
        // `"we regret"` was the only regret marker and missed the formulation a named recruiter
        // actually uses. Both must land.
        assert_eq!(
            verdict_for("Application Status", "I regret to inform you the decision has been made"),
            Category::Rejection
        );
        assert_eq!(
            verdict_for("Application Status", "We regret to inform you that we have decided"),
            Category::Rejection
        );
    }

    #[test]
    fn an_html_escaped_apostrophe_does_not_hide_a_rejection() {
        // Gmail escapes snippets, so a contraction arrives as `isn&#39;t`. Every marker
        // containing an apostrophe silently failed, and the marker lists had been written
        // around that rather than the escaping being undone. This is the real wording of a
        // rejection that classified as a confirmation.
        assert_eq!(
            verdict_for(
                "Application Update",
                "After reviewing your application we&#39;ve determined that there isn&#39;t an ideal fit at this time"
            ),
            Category::Rejection
        );
        // And the decoding itself, independent of any marker.
        assert_eq!(haystack(Some("A&amp;B"), Some("don&#39;t")), "a&b don't");
        // An escaped ampersand must not be unescaped into another entity.
        assert_eq!(haystack(None, Some("&amp;#39;")).trim(), "&#39;");
    }

    #[test]
    fn the_contraction_form_of_a_refusal_counts() {
        assert_eq!(
            verdict_for("Update", "we won't be moving forward with your application"),
            Category::Rejection
        );
        assert_eq!(
            verdict_for("Update", "the position has been filled"),
            Category::Rejection
        );
    }

    #[test]
    fn a_fit_marker_is_negated_or_it_is_not_a_marker() {
        // A bare "ideal fit" would read this as a rejection, and REJECTION is checked first, so
        // it would win outright and flip a real offer to rejected.
        assert_ne!(
            verdict_for("Great news", "we think you are an ideal fit and are pleased to offer you the role"),
            Category::Rejection
        );
    }

    #[test]
    fn describing_an_interview_is_not_inviting_you_to_one() {
        // Both are real confirmations that classified as interviews. Bare "interview" matched
        // the topic; neither email asks for anything.
        assert_eq!(
            verdict_for(
                "Prepare for your application process",
                "Your application has been received. Here is some information to help prepare you for the interview process."
            ),
            Category::Confirmation
        );
        assert_eq!(
            verdict_for(
                "Thank you for applying",
                "Please see below for information on the program and details on the interview"
            ),
            Category::Confirmation
        );
    }

    #[test]
    fn a_real_invitation_still_lands_as_an_interview() {
        // The corpus contains no genuine invitation, so tightening INTERVIEW is unvalidated in
        // the direction that matters. Rule 8: an unmatched invitation is the costliest miss in
        // the system, so these are the guard on that tightening.
        for snippet in [
            "We would like to invite you to interview for the role",
            "Please schedule an interview using the link below",
            "Let's set up an interview next week",
            "Your interview is confirmed for Tuesday",
            "We'd like to schedule a call to discuss the role",
            "The next step is a phone screen with the team",
            "You have been invited to the next round",
            "We would like to interview you for this position",
            // No invitation verb at all — the shape the 13f gate caught when this list was
            // first tightened.
            "Use the link below to select a slot for your final round interview",
            "Please book a time that works for you",
        ] {
            assert_eq!(
                verdict_for("Next steps", snippet),
                Category::Interview,
                "must read as an invitation: {snippet:?}"
            );
        }
    }

    #[test]
    fn a_rejection_that_mentions_an_interview_is_still_a_rejection() {
        // The ordering guard REJECTION's own doc calls out. Checked first for exactly this.
        assert_eq!(
            verdict_for(
                "Thank you for interviewing with us",
                "Unfortunately we will not be moving forward with your application"
            ),
            Category::Rejection
        );
    }

    /// Re-classify a COPY of the live mailbox and print what changed. Ignored; never in CI.
    ///
    ///   INBOX_PROBE_DB=/path/copy.db cargo test classify::tests::probe -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn probe_reclassify_the_live_mailbox() {
        use sqlx::SqlitePool;
        let path = std::env::var("INBOX_PROBE_DB").expect("INBOX_PROBE_DB");
        let pool = SqlitePool::connect(&format!("sqlite://{path}")).await.expect("open");
        let companies: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT lower(company_name) FROM internship_postings WHERE company_name IS NOT NULL",
        ).fetch_all(&pool).await.expect("companies");
        let ctx = Context { known_companies: &companies };

        let rows: Vec<(String, Option<String>, Option<String>, String)> = sqlx::query_as(
            "SELECT m.subject, m.from_address, m.snippet, v.category
               FROM email_messages m JOIN email_verdicts v ON v.message_id = m.id
              ORDER BY m.received_at",
        ).fetch_all(&pool).await.expect("rows");

        let mut changed = 0;
        let mut after = std::collections::BTreeMap::new();
        for (subject, from, snippet, was) in &rows {
            let v = classify(from.as_deref(), Some(subject), snippet.as_deref(), &ctx);
            let now = format!("{:?}", v.category).to_lowercase();
            *after.entry(now.clone()).or_insert(0usize) += 1;
            if now != *was {
                changed += 1;
                println!("  {was:>12} -> {now:<12} {}", &subject.chars().take(58).collect::<String>());
                println!("               {}", v.evidence);
            }
        }
        println!("\n{changed} of {} messages change category", rows.len());
        println!("new distribution: {after:?}");
    }
}
