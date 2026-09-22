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

    /// Every category, in the order the matrices and reports use.
    ///
    /// The doc on `parse` used to claim that adding a variant was "a compile error here", and
    /// it was not: `parse` held an array literal, so a new variant would have compiled and
    /// returned `None` for its own name at the point something read the database. The guard
    /// that makes the claim true is [`Category::index`] below, which is a real exhaustive
    /// match — the trick `labelset` had already used for the same list.
    pub const ALL: [Category; 7] = [
        Category::Confirmation,
        Category::Oa,
        Category::Interview,
        Category::Offer,
        Category::Rejection,
        Category::Outreach,
        Category::Disregarded,
    ];

    /// Sort order, and the compile-time guard on [`Category::ALL`].
    ///
    /// Add a variant and this match stops compiling. That is the whole job.
    pub fn index(self) -> usize {
        match self {
            Category::Confirmation => 0,
            Category::Oa => 1,
            Category::Interview => 2,
            Category::Offer => 3,
            Category::Rejection => 4,
            Category::Outreach => 5,
            Category::Disregarded => 6,
        }
    }

    /// The inverse of [`as_str`](Category::as_str), for reading a stored verdict back.
    pub fn parse(raw: &str) -> Option<Self> {
        Category::ALL.into_iter().find(|category| category.as_str() == raw)
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
    // Bare "unfortunately" is NOT here, and was until the classifier started reading bodies.
    // In a 200-character snippet it was a fair proxy for a refusal. In four thousand it is not:
    // a real OA invitation reads "complete the test in one go. Unfortunately, we cannot send a
    // new test link", and that flipped a live assessment to `rejected`. It now has to appear
    // beside a decision — see REJECTION_PAIRS.
    "move forward with other",
    "made the decision to move forward",
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
    // "not the right fit" is NOT here. In a body it matches "if you decide that this location
    // is not the right fit FOR YOU, we will not be able to proceed" — a conditional about your
    // preference inside a live OA invitation, which it flipped to `rejected`. The "ideal fit"
    // forms carry the decision; this one does not.
    "not a fit at this time",
    "position has been filled",
    "role has been filled",
];

/// Phrases that mean an assessment is being *described*, not assigned to you.
///
/// A campus-event invitation reads "before you dive into **our** online assessments, please
/// bring your laptop" — an event, not a test with your name on it. With message bodies in
/// scope that flipped two real outreach emails to pressing.
///
/// Narrower than it looks, and narrow on purpose: an earlier attempt required the marker itself
/// to carry a possessive, which dropped the real OA subject "[Action Required] Your Roblox
/// Application - Online Assessment", where "your" and the marker sit either side of a dash.
/// Excluding the one phrase that means "ours in general" keeps that and drops the event.
/// **Each begins with a space, and that is load-bearing.** `hit` is substring matching with no
/// word boundary, so the marker "our online assessment" matches inside "y*our online
/// assessment*" — which is the exact wording of a real OA, and of the committed fixture
/// `syn-002`. The gate caught it. The extension's field matcher learned the same lesson
/// separately, where three-letter company names matched inside ordinary words.
const ASSESSMENT_IS_DESCRIBED: &[&str] = &[
    " our online assessment",
    " our assessments",
    " into our online",
];

/// An assessment handed TO YOU, which outranks a describing phrase elsewhere in the same mail.
///
/// # Why a veto needed a counter-veto
///
/// [`ASSESSMENT_IS_DESCRIBED`] was written against a ~200-character snippet, where "learn about
/// our assessments" really is the whole message. Against four thousand characters of body it
/// became a **global** veto, and legal boilerplate at the foot of a genuine invitation switched
/// the entire OA branch off.
///
/// Live, on 2026-09-22: an Optiver invitation reading "We would like to invite you to complete
/// the Optiver assessments … Please complete the assessments by September 18, 2026" was
/// classified `disregarded`, because 2,500 characters further down it also said "A note on
/// assessment integrity: **our assessments** are designed to evaluate your skills". A real
/// assessment with a real deadline, dropped by its own integrity notice. Rule 8 names that the
/// costliest failure in the system, and this is what it looks like in practice.
///
/// So the gate is no longer "any describing phrase disqualifies". A describing phrase
/// disqualifies **unless the same mail also tells you to go and do one**. Both halves stay
/// narrow: this list is instructions addressed at the reader, never topic words.
const ASSESSMENT_IS_ASSIGNED: &[&str] = &[
    "invite you to complete",
    "invite you to take",
    "complete the assessment",
    "complete your assessment",
    "completed the assessment",
    "completed your assessment",
    "assessment invitation",
    "assessments invitation",
    "your assessment link",
];

/// Assessment platforms, which are evidence about the SENDER and never a verdict on their own.
///
/// These four were bare entries in [`ASSESSMENT`] until 2026-09-22, and the cost was live:
/// "Verify your CodeSignal account" — an account-admin mail with no assessment in it — was
/// stored `oa` at confidence 0.8 on the single marker "codesignal". Meanwhile "Assessment
/// completed: Roblox Assessment", from the same platform, was `disregarded`, because the word
/// "codesignal" happened not to appear in its text. One list, opposite errors, both wrong.
///
/// A platform name now needs [`ASSESSMENT_TOPIC`] beside it. That is the same corroboration
/// rule the pair lists use, applied to a signal that reads the sender.
const ASSESSMENT_PLATFORMS: &[&str] =
    &["hackerrank", "codesignal", "codility", "karat", "coderpad", "hirevue"];

/// Deliberately useless alone — admissible only as corroboration for a platform.
const ASSESSMENT_TOPIC: &[&str] = &["assessment", "coding challenge", "coding test", "challenge"];

/// "Unfortunately" plus something that makes it a decision about you.
///
/// The word alone is a tone, not a verdict — a rejection, a scheduling apology and a broken link
/// all use it. Requiring a decision word alongside keeps every real refusal and drops the
/// logistics. Order is not required; templates vary.
const REJECTION_PAIRS: &[(&str, &str)] = &[
    ("unfortunately", "not be moving forward"),
    ("unfortunately", "not moving forward"),
    ("unfortunately", "other candidates"),
    ("unfortunately", "made the decision"),
    ("unfortunately", "not selected"),
    ("unfortunately", "unable to offer"),
    ("unfortunately", "not be progressing"),
    // NOT "will not be", "we have decided" or "no longer". Each is generic enough to appear in
    // ordinary logistics, and proximity does not save them: a real OA invitation reads
    // "unfortunately, we cannot send a new test link" and, one sentence later, "we will not be
    // able to proceed with this part" — both inside the window, neither a rejection. The second
    // half has to be decisive on its own.
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
    "invited to complete",
    "assessment invitation",
    "assessments invitation",
    "complete the assessment",
    "complete your assessment",
    "coding challenge",
    "code challenge",
    "take home",
    "take-home",
    "technical screen",
    // An assessment you already have and are about to LOSE. Real mail, disregarded:
    // "[Action Required] Your Roblox Assessments Expire in 24 hours". Rule 8 calls a disregarded
    // pressing message the costliest failure in the system, and an expiring OA is exactly that —
    // the deadline is the whole point of telling you.
    "assessment expires",
    "assessments expire",
    "assessment will expire",
    "assessments will expire",
    "assessment have expired",
    "assessments have expired",
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
    // Microsoft Careers' wording. Nine real confirmations — nine DIFFERENT roles — were
    // disregarded because every marker above wants "applying", "received" or "submitted", and
    // this says *"thank you for taking the time to submit your application for …"*.
    //
    // "taking the time to submit" and not the shorter "taking the time to apply", deliberately.
    // The short form opens a polite REJECTION just as often — Epic Games' begins "thank you so
    // much for taking the time to apply", and its refusal sits past the 200-character snippet
    // where nothing can see it. Matching the short form would file that as a confirmation,
    // which is worse than leaving it unclassified.
    "taking the time to submit your application",
    "time to submit your application",
    // Handshake forwards an "Application sent to <employer>" receipt. Two real ones were
    // disregarded — both for small companies that appear in no posting, which is why the
    // company guesser could not help either.
    "application sent to",
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
];

/// Event registrations — matched against the SUBJECT only. See [`BULK`] for why the split.
///
/// # An event word in the subject is the event; in the body it is a mention
///
/// These lived in [`BULK`] and were matched body-wide. Live cost, 2026-09-22: a named recruiter
/// at a company in the corpus wrote about an on-campus week, and the mail was `disregarded` on
/// the marker "rsvp" — which appeared nowhere in her subject and only in the footer of her
/// body. The relevance gate is checked before the outreach test, so a footer beat a person.
///
/// The subject is where a bulk sender says what the mail IS, which is the same headline/text
/// distinction `classify_with_body` already draws for company guessing. The RSVP *receipt* this
/// list was written for — "Thanks for RSVPing to …" — says so in its subject and is still
/// caught.
const BULK_EVENT: &[&str] = &[
    "rsvp",
    "thanks for your response to",
    "thank you for your response to",
    "you are registered",
    "thanks for registering",
];

/// Job-topic words. Useless alone, and never a verdict on their own.
///
/// This exists only to corroborate a machine sender at a known employer — the question it
/// answers is "is this mail about employment at all", not "what kind of mail is it". Every
/// entry is a word that appears in half the recruiting mail ever written, which is exactly why
/// it may never decide anything by itself.
const JOB_TOPIC: &[&str] = &[
    "assessment",
    "application",
    "applied",
    "interview",
    "candidate",
    "recruit",
    "position",
    "internship",
    "hiring",
    "job offer",
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
fn haystack_with_body(subject: Option<&str>, snippet: Option<&str>, body: Option<&str>) -> String {
    let joined = format!(
        "{} {} {}",
        subject.unwrap_or(""),
        snippet.unwrap_or(""),
        body.unwrap_or("")
    );
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
pub fn decode_entities(text: &str) -> String {
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

/// How far apart two halves of a pair may sit and still be one statement.
///
/// Roughly a long sentence. Unbounded co-occurrence was fine when the haystack was a subject
/// and a 200-character snippet; across a whole body it is not. A real OA invitation says
/// "unfortunately, we cannot send a new test link" in one paragraph and "we will not be able to
/// proceed" in another, and an unbounded pair read those as a rejection.
///
/// **All three pair lists are bounded, as of 2026-09-22.** Only `REJECTION_PAIRS` was, because
/// it was the list that produced the bug above; the other two kept an unbounded matcher across
/// four thousand characters for another eleven days. Live evidence that this was not
/// theoretical: an Optiver mail subject "Prepare for your application process" was stored
/// `confirmation` on the pair "application" + "has been received", two phrases with no
/// relationship to each other in that message. Right answer, unrelated reason — and because the
/// confirmation branch carried no quote, the evidence could not be checked either.
const PAIR_WINDOW: usize = 160;

/// Both halves present, in either order, within [`PAIR_WINDOW`] of each other.
///
/// The unbounded variant this replaced is gone rather than deprecated: leaving it in the file
/// is an invitation for the next list to be added with the wrong matcher, which is exactly how
/// two of the three ended up unbounded.
fn hit_pair_near<'a>(text: &str, pairs: &[(&'a str, &'a str)]) -> Option<(&'a str, &'a str)> {
    pairs.iter().copied().find(|(a, b)| {
        text.match_indices(a).any(|(ai, _)| {
            text.match_indices(b).any(|(bi, _)| {
                let (lo, hi) = if ai < bi { (ai + a.len(), bi) } else { (bi + b.len(), ai) };
                hi.saturating_sub(lo) <= PAIR_WINDOW
            })
        })
    })
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
    classify_with_body(from, subject, snippet, None, context)
}

/// [`classify`], plus the message body when one was fetched.
///
/// Split rather than adding a parameter to `classify` so the existing call sites and their
/// tests keep meaning what they meant: "decide this from the metadata alone". The body is
/// strictly additional evidence, and every rule reads the same haystack whether it is there.
///
/// **Rule 1 still holds with the body in hand, and matters more.** The body is the
/// prompt-injection surface: this is a pure function, it gets no tools, and text in an email
/// addressed at the agent is data to classify rather than an instruction. `gmail::fetch_message`
/// strips tags and truncates before anything reaches here.
pub fn classify_with_body(
    from: Option<&str>,
    subject: Option<&str>,
    snippet: Option<&str>,
    body: Option<&str>,
    context: &Context<'_>,
) -> EmailVerdict {
    let text = haystack_with_body(subject, snippet, body);
    // Sender and subject only. Every employer worth naming appears in one of them, and a body
    // is four thousand characters of prose in which a one-word company name — the corpus really
    // contains "Secure" — turns up by accident. A real case: "Join Workiva's Internship
    // Information Session" guessed `secure`, from a word in the body.
    let headline = haystack_with_body(subject, None, None);
    let sender = from.unwrap_or("").to_lowercase();

    let verdict = |category: Category, confidence: f64, evidence: String| EmailVerdict {
        category,
        confidence,
        company_guess: guess_company(&sender, &headline, &text, context),
        evidence,
    };

    // A marker plus the words around it.
    //
    // `rejection marker: "unfortunately"` names the rule and tells a reader nothing about
    // whether it was right — and since the classifier started reading message bodies, a marker
    // can match four thousand characters in rather than inside a 200-character snippet, where
    // a human could see the whole haystack anyway. The evidence column exists to make a verdict
    // checkable; a bare marker had stopped doing that.
    let quote = |marker: &str| -> String {
        match text.find(marker) {
            Some(at) => {
                let start = text[..at].char_indices().rev().nth(45).map_or(0, |(i, _)| i);
                let end = text[at..]
                    .char_indices()
                    .nth(marker.chars().count() + 55)
                    .map_or(text.len(), |(i, _)| at + i);
                format!("…{}…", text[start..end].trim())
            }
            None => String::new(),
        }
    };

    // Terminal outcomes first. See REJECTION's note on why it leads.
    if let Some(marker) = hit(&text, REJECTION) {
        return verdict(
            Category::Rejection,
            0.9,
            format!("rejection marker: {marker:?} — {}", quote(marker)),
        );
    }
    if let Some((a, b)) = hit_pair_near(&text, REJECTION_PAIRS) {
        return verdict(
            Category::Rejection,
            0.85,
            format!("rejection pair: {a:?} + {b:?} — {}", quote(a)),
        );
    }
    if let Some(marker) = hit(&text, OFFER) {
        return verdict(
            Category::Offer,
            0.85,
            format!("offer marker: {marker:?} — {}", quote(marker)),
        );
    }

    // Then the two that need a response from you. Interview before assessment: "interview" is
    // the more specific claim, and an email that mentions both is usually inviting you to one.
    if let Some(marker) = hit(&text, INTERVIEW) {
        return verdict(
            Category::Interview,
            0.8,
            format!("interview marker: {marker:?} — {}", quote(marker)),
        );
    }
    // An assessment somebody is telling you about is not one you have been given — unless the
    // same mail also hands you one. See `ASSESSMENT_IS_ASSIGNED` for the invitation this veto
    // was silently eating.
    let assessment_is_yours = hit(&text, ASSESSMENT_IS_DESCRIBED).is_none()
        || hit(&text, ASSESSMENT_IS_ASSIGNED).is_some();
    if assessment_is_yours
        && let Some(marker) = hit(&text, ASSESSMENT)
    {
        return verdict(
            Category::Oa,
            0.8,
            format!("assessment marker: {marker:?} — {}", quote(marker)),
        );
    }
    // A platform's own mail, corroborated. Neither half is a verdict alone: the platform name
    // by itself made an account-verification mail pressing, and the topic word by itself is in
    // half the recruiting mail ever written.
    if assessment_is_yours
        && let Some(platform) = hit(&sender, ASSESSMENT_PLATFORMS).or_else(|| hit(&text, ASSESSMENT_PLATFORMS))
        && let Some(topic) = hit(&text, ASSESSMENT_TOPIC)
    {
        return verdict(
            Category::Oa,
            0.75,
            format!("assessment platform: {platform:?} + {topic:?} — {}", quote(topic)),
        );
    }
    if assessment_is_yours
        && let Some((a, b)) = hit_pair_near(&text, ASSESSMENT_PAIRS)
    {
        return verdict(
            Category::Oa,
            0.7,
            format!("assessment pair: {a:?} + {b:?} — {} / {}", quote(a), quote(b)),
        );
    }

    if let Some(marker) = hit(&text, CONFIRMATION) {
        return verdict(
            Category::Confirmation,
            0.85,
            format!("confirmation marker: {marker:?} — {}", quote(marker)),
        );
    }
    if let Some((a, b)) = hit_pair_near(&text, CONFIRMATION_PAIRS) {
        return verdict(
            Category::Confirmation,
            0.8,
            format!("confirmation pair: {a:?} + {b:?} — {} / {}", quote(a), quote(b)),
        );
    }

    // The relevance gate. Checked AFTER the pressing categories, never before: a digest
    // subject line must not be able to swallow a real interview invite that happens to
    // contain the word "jobs".
    //
    // The event half is matched against the SUBJECT only — see `BULK_EVENT` for the recruiter
    // this cost when it was matched body-wide.
    if let Some(marker) = hit(&headline, BULK_EVENT) {
        return verdict(
            Category::Disregarded,
            0.7,
            format!("event marker in the subject: {marker:?} — {}", quote(marker)),
        );
    }
    if let Some(marker) = hit(&text, BULK) {
        return verdict(
            Category::Disregarded,
            0.7,
            format!("bulk mail marker: {marker:?} — {}", quote(marker)),
        );
    }

    // Job-specific and addressed to you, but about no application you made.
    let from_ats = ATS_DOMAINS.iter().any(|domain| sender.contains(domain));
    // **The corpus-backed guess only, deliberately not the sender fallback.**
    //
    // This decides whether an email we have no other reason to care about is job-related, and
    // the bar is "it names a specific employer we have seen hiring". `employer_from_sender`
    // clears no such bar: it reads a name off almost any domain, so wiring it in here made
    // ordinary mail look like recruiter outreach — the 13f gate measured junk leaking into
    // Outreach rise from 0 to 2 the moment it was.
    //
    // The fallback is still what names the employer on the verdict. Knowing WHICH company an
    // application belongs to, once we believe it is one, is a different question from whether
    // a stranger's email is about a job.
    let named_company = best_company_named(&sender, &headline, &text, context);
    let from_a_person = !sender.is_empty() && !is_machine_sender(&sender);

    // An assessment platform only ever writes to you in a hiring context, so its mail belongs
    // in the folder even when no branch above claimed it. "Verify your CodeSignal account" is
    // not an assessment — that was the false `oa` this pass removed — but it is often the step
    // that GATES one, and disregarding it is how you find out too late.
    let from_a_platform = hit(&sender, ASSESSMENT_PLATFORMS);

    if from_ats || from_a_platform.is_some() || (named_company.is_some() && from_a_person) {
        // The evidence names the condition that actually opened the gate. Reporting the
        // company first looks tidier and lies when a machine sent the mail: a dry run said
        // "names reply in the sender, and a person sent it" about `no-reply@codesignal.com`,
        // which was wrong twice over in one sentence.
        let evidence = match (&named_company, from_a_platform) {
            (Some((company, found_in)), _) if from_a_person => {
                format!("names {company} in the {found_in}, and a person sent it")
            }
            (_, Some(platform)) => {
                format!("from the assessment platform {platform:?}, but names no application")
            }
            _ => "from an ATS domain, but matches no application".to_string(),
        };
        return verdict(Category::Outreach, 0.5, evidence);
    }

    // A machine at an employer we have seen hiring, writing about employment.
    //
    // The gate above requires a human, so `no-reply@` mail from a real employer had no path at
    // all and fell straight to disregarded. Live examples, 2026-09-22: a CodeSignal "Assessment
    // completed", a Roblox "you have completed the assessments", an Optiver assessment-portal
    // login code — all real, all dropped.
    //
    // **Three conditions at once, because any one of them alone is the junk leak.** The company
    // must be a CORPUS company found in the sender or the subject — never body prose, never
    // `employer_from_sender`, which is what moved junk-leaked-to-outreach from 0 to 2 when it
    // was tried here. And `JOB_TOPIC` must corroborate, which is what keeps a Google security
    // alert out: "google" really is in the corpus, and the alert says nothing about employment.
    let named_by_the_sender_or_subject = named_company
        .as_ref()
        .filter(|(_, found_in)| !matches!(found_in, NamedIn::Body));
    if let Some((company, _)) = named_by_the_sender_or_subject
        && let Some(topic) = hit(&text, JOB_TOPIC)
    {
        return verdict(
            Category::Outreach,
            0.45,
            format!("machine sender at {company}, job topic {topic:?} — {}", quote(topic)),
        );
    }

    // Everything else. The highest-volume path, and still recorded — rule 7.
    verdict(
        Category::Disregarded,
        0.6,
        match &named_company {
            Some((company, found_in)) => format!(
                "names {company} in the {found_in} but nothing job-specific, and a machine sent it"
            ),
            None => "no application, employer or job-specific signal".to_string(),
        },
    )
}

/// A company named in the sender's domain or the text, if we know of one.
///
/// A hint for the matcher, never a gate. Longest match wins so "jump trading" beats "jump".
/// The employer this email is about.
///
/// **Two passes, sender and subject first.** Matching anywhere in a body makes every short
/// company name a coin flip — see the `headline` note in `classify_with_body`. The body is still
/// searched, but only when the reliable fields name nobody, so it adds companies rather than
/// outvoting them.
fn guess_company(
    sender: &str,
    headline: &str,
    text: &str,
    context: &Context<'_>,
) -> Option<String> {
    // Corpus first, both passes, THEN the sender. The corpus knows how a company spells its own
    // name and a domain label does not: "Chicago Trading Company" appears in the body of its own
    // confirmation, and putting the fallback ahead of that pass renamed it "chicagotrading".
    best_company(sender, headline, context)
        .or_else(|| {
        // The body pass only accepts a name distinctive enough to survive four thousand
        // characters of prose. A short single word is not: the corpus contains "Secure", and it
        // matched inside the body of a Workiva information-session invitation. Two words, or
        // eight-plus characters — anything shorter that is genuinely the employer is named in
        // the sender or the subject too, which the first pass already read.
        best_company(sender, text, context)
            .filter(|company| company.contains(' ') || company.len() >= 8)
    })
    .or_else(|| employer_from_sender(sender, headline, context))
}

/// Words a careers mailbox appends to its employer's name.
const SENDER_ROLE_WORDS: &[&str] = &[
    "careers", "career", "early", "recruiting", "recruitment", "talent", "acquisition",
    "hiring", "team", "university", "campus", "support", "notifications", "notification",
    "reply", "noreply", "donotreply", "jobs", "job", "hr", "people", "no", "do", "not",
    "the", "at", "via", "info", "admin", "mail", "mailer", "system", "systemmessage",
    "message", "messages", "alerts", "alert", "service", "services", "us", "inc", "llc",
];

/// Hosts that send on somebody else's behalf, so their domain names no employer.
const RELAY_DOMAINS: &[&str] = &[
    "joinhandshake.com", "handshake.com", "gmail.com", "googlemail.com", "outlook.com",
    "hotmail.com", "yahoo.com", "icloud.com", "andrew.cmu.edu", "cmu.edu", "sendgrid.net",
    "mailgun.org", "amazonses.com", "hackerrank.com", "codesignal.com", "eightfold.ai",
];

/// The employer, guessed from who sent it and what the subject says, when the postings corpus
/// has never heard of them.
///
/// **Most employers are not in the corpus.** It only knows companies we have collected postings
/// from, so Workiva (0 postings), the Oklahoma City Thunder and two companies reached through
/// Handshake produced no guess at all and were dropped — real applications, invisible. Every one
/// of them names its employer plainly: in the sender's display name, in its domain, in the local
/// part, or in the subject.
///
/// Ordered by how much the source can be trusted, and each candidate still has to pass
/// [`company_match::is_company_name`], which is what rejects "careers" and "no reply".
fn employer_from_sender(sender: &str, headline: &str, context: &Context<'_>) -> Option<String> {
    // 1. "Application sent to <employer>" — Handshake's receipt, forwarded by hand, where the
    //    sender is the user themselves and says nothing.
    for marker in ["application sent to ", "application to "] {
        if let Some(at) = headline.find(marker) {
            let tail = &headline[at + marker.len()..];
            let name = tail
                .split(['-', '—', ':', '|', ','])
                .next()
                .unwrap_or(tail)
                .trim();
            if let Some(name) = clean_employer(name) {
                return Some(canonicalize(name, context));
            }
        }
    }

    let (display, address) = split_sender(sender);
    let from_relay = RELAY_DOMAINS
        .iter()
        .any(|relay| address.ends_with(relay) || address.ends_with(&format!(".{relay}")));

    // 2. The display name — human-written, and spells the employer the way the employer does.
    //    Checked BEFORE the domain, which gives a squashed slug: one Chicago Trading Company
    //    email carries the name and another only `chicagotrading.com`, and taking the domain
    //    first produced two employers for one company.
    //
    //    Skipped on a relay. There the display name is a person — on a forwarded Handshake
    //    receipt it is the user themselves, and reading that as an employer would file their
    //    applications under their own name.
    if !from_relay
        && let Some(name) = clean_employer(&display)
    {
        return Some(canonicalize(name, context));
    }

    // 3. The domain, unless it belongs to an ATS or a relay — `recruiting.workiva.com` is
    //    Workiva, `myworkday.com` is nobody.
    if let Some(domain) = address.rsplit('@').next() {
        let is_relay = ATS_DOMAINS.iter().chain(RELAY_DOMAINS.iter()).any(|d| domain.ends_with(d));
        if !is_relay {
            let labels: Vec<&str> = domain.split('.').collect();
            if labels.len() >= 2
                && let Some(name) = clean_employer(labels[labels.len() - 2])
            {
                return Some(canonicalize(name, context));
            }
        }
    }

    // 4. The local part, where an employer's name ends up when the domain is an ATS:
    //    `workiva@myworkday.com`.
    //
    //    Gated on the sender not being a machine mailbox, because that is precisely where a
    //    local part stops naming anybody: `systemmessage@paycomonline.com` yielded
    //    "systemmessage", which is not a company and reads like one to every check downstream.
    if from_relay || is_machine_sender(sender) {
        return None;
    }
    address
        .split('@')
        .next()
        .and_then(|local| clean_employer(&local.replace(['-', '.', '_'], " ")))
        .map(|name| canonicalize(name, context))
}

/// Spell a guessed employer the way the corpus spells it, when the corpus knows them.
///
/// A domain gives "chicagotrading" and the corpus says "chicago trading company". Left alone
/// those are two different `company_key`s and therefore two applications at one employer — the
/// duplicate that showed up the first time this fallback ran. Compared with spaces removed,
/// because a domain has none.
fn canonicalize(name: String, context: &Context<'_>) -> String {
    let squashed = name.replace(' ', "");
    context
        .known_companies
        .iter()
        .filter(|known| {
            let theirs = known.replace(' ', "");
            theirs == squashed || theirs.starts_with(&squashed) && squashed.len() >= 6
        })
        // The shortest match, so "chicagotrading" prefers "chicago trading company" over any
        // longer name that merely begins the same way.
        .min_by_key(|known| known.len())
        .cloned()
        .unwrap_or(name)
}

/// `"Name" <a@b>` split into its two halves, lowercased.
fn split_sender(sender: &str) -> (String, String) {
    match sender.split_once('<') {
        Some((display, rest)) => (
            display.trim().trim_matches('"').to_lowercase(),
            rest.trim_end_matches('>').trim().to_lowercase(),
        ),
        None => (String::new(), sender.trim().to_lowercase()),
    }
}

/// Strip the role words a careers mailbox appends, and refuse what is left if it is not a name.
fn clean_employer(raw: &str) -> Option<String> {
    // Token-wise, from both ends. Phrase-matching left dangling halves: "Workiva Early Career"
    // lost "career" and kept "early", yielding "workiva early" — a company that does not exist
    // and would key separately from Workiva.
    let mut tokens: Vec<String> = raw
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect();

    while tokens.last().is_some_and(|t| SENDER_ROLE_WORDS.contains(&t.as_str())) {
        tokens.pop();
    }
    while tokens.first().is_some_and(|t| SENDER_ROLE_WORDS.contains(&t.as_str())) {
        tokens.remove(0);
    }

    let name = tokens.join(" ");
    // The same bar every other company guess clears: not a generic word, not parsing debris.
    if crate::internships::company_match::is_company_name(&name) && name.len() >= 3 {
        Some(name)
    } else {
        None
    }
}

/// Where a corpus company name turned up. Not cosmetic: it is the difference between "an
/// employer wrote this" and "this mentions an employer", and only the first is evidence about
/// a machine sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NamedIn {
    Sender,
    Subject,
    Body,
}

impl std::fmt::Display for NamedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            NamedIn::Sender => "sender",
            NamedIn::Subject => "subject",
            NamedIn::Body => "body",
        })
    }
}

/// [`best_company`], but it says where it looked, and it filters the body the way
/// `guess_company` already does.
///
/// # The asymmetry this fixes
///
/// The relevance gate used to pass the whole body-inclusive haystack to `best_company` with no
/// length filter, while `guess_company`'s body pass filtered to names with a space or eight
/// characters. So a short corpus name — the list really contains "Secure" — could decide the
/// CATEGORY from body prose while being too flimsy to be reported as the company.
///
/// Live, 2026-09-22: "Join Workiva's Internship Information Session!" was classified Outreach
/// with the evidence "names secure, and a person sent it", because its body said "**Secure**
/// your spot at our upcoming session". Right category, by accident, for a reason that was
/// nonsense — and the test that was supposed to cover this asserted only on `company_guess`,
/// which the filter had already cleaned.
fn best_company_named(
    sender: &str,
    headline: &str,
    text: &str,
    context: &Context<'_>,
) -> Option<(String, NamedIn)> {
    if let Some(name) = best_company(sender, "", context) {
        return Some((name, NamedIn::Sender));
    }
    if let Some(name) = best_company("", headline, context) {
        return Some((name, NamedIn::Subject));
    }
    best_company("", text, context)
        .filter(|name| name.contains(' ') || name.len() >= 8)
        .map(|name| (name, NamedIn::Body))
}

fn best_company(sender: &str, text: &str, context: &Context<'_>) -> Option<String> {
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
        // A company name that is also a word addresses are built out of cannot be read off an
        // address. The corpus really contains "Reply" — an Italian consultancy — and
        // `no-reply@codesignal.com` offers it a clean whole-word match between a hyphen and an
        // at-sign. `SENDER_ROLE_WORDS` already exists to name exactly these parts of an
        // address, so the same list answers this question.
        let names_a_role_not_a_company = SENDER_ROLE_WORDS.contains(&squashed.as_str());
        let mentioned = contains_whole_word(text, company.as_str())
            || (!names_a_role_not_a_company && contains_whole_word(sender, &squashed));
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
            // **The property is that no name is matched out of the MIDDLE of a word**, not that
            // no name is produced at all. Since the sender fallback landed, "Oklahoma City
            // Thunder" is correctly read from the display name and "somecorp" from the domain —
            // both real employers, neither a substring accident. What must never appear is a
            // known-company name that is only there by coincidence.
            let guess = verdict.company_guess.clone().unwrap_or_default();
            for accident in &known {
                assert_ne!(
                    &guess, accident,
                    "{from} matched {accident:?} inside an ordinary word"
                );
            }
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
        // Since the sender fallback landed this DOES name an employer — and the property the
        // test protects is unchanged: it must not be the ATS. `myworkday.com` is in the list
        // above and is still never the answer.
        assert_eq!(verdict.company_guess.as_deref(), Some("workiva"));
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
        assert_eq!(haystack_with_body(Some("A&amp;B"), Some("don&#39;t"), None).trim(), "a&b don't");
        // An escaped ampersand must not be unescaped into another entity.
        assert_eq!(haystack_with_body(None, Some("&amp;#39;"), None).trim(), "&#39;");
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

    #[test]
    fn microsofts_wording_is_a_confirmation() {
        // Nine real confirmations, nine different roles, all disregarded: every confirmation
        // marker wanted "applying", "received" or "submitted", and this says neither.
        assert_eq!(
            verdict_for(
                "Thank you for your application!",
                "Hi Jesse, Thank you for taking the time to submit your application for Software Engineer: Data Platform"
            ),
            Category::Confirmation
        );
    }

    #[test]
    fn the_short_polite_opener_is_not_claimed_by_either_side() {
        // "taking the time to APPLY" opens Microsoft-style confirmations and polite rejections
        // alike — Epic Games' rejection begins with it and puts the refusal past the snippet.
        // Claiming it for confirmation would file a rejection as an open application, which is
        // worse than leaving it unclassified, so only the longer "submit your application"
        // form is a marker.
        assert_eq!(
            verdict_for(
                "Update from Epic Games",
                "Thank you so much for taking the time to apply for the Gameplay Programmer Intern role. We know a lot of thought went into"
            ),
            Category::Disregarded
        );
    }

    #[test]
    fn a_forwarded_handshake_receipt_is_a_confirmation() {
        // Two real ones, both for companies that appear in no posting — so the company guesser
        // could not help either, and the message was dropped entirely.
        assert_eq!(
            verdict_for(
                "Fwd: Application sent to Glencliff Labs — here's what's next",
                "---------- Forwarded message --------- From: Handshake"
            ),
            Category::Confirmation
        );
    }

    #[test]
    fn an_expiring_assessment_is_pressing() {
        // Rule 8: a disregarded pressing message is the costliest failure here, and an OA you
        // are about to lose is exactly that. Both real subjects.
        assert_eq!(
            verdict_for("[Action Required] Your Roblox Assessments Expire in 24 hours", ""),
            Category::Oa
        );
        assert_eq!(
            verdict_for("Your Roblox Assessments Have Expired", ""),
            Category::Oa
        );
    }

    #[test]
    fn unfortunately_alone_is_a_tone_not_a_verdict() {
        // A real OA invitation, which bare "unfortunately" flipped to `rejected` the moment the
        // classifier started reading bodies. In a 200-character snippet the word was a fair
        // proxy for a refusal; in four thousand it is not.
        assert_eq!(
            verdict_for(
                "Microsoft HackerRank Online Technical Screen",
                "Complete the test in one go. Unfortunately, we cannot send a new test link. After submitting"
            ),
            Category::Oa
        );
        // And it still is a refusal when a decision stands beside it.
        assert_eq!(
            verdict_for(
                "Update from Epic Games",
                "Thank you for your interest in joining the team. Unfortunately, we have made the decision to move forward with other candidates"
            ),
            Category::Rejection
        );
    }

    #[test]
    fn a_pair_must_be_one_statement_not_two_paragraphs() {
        // The live OA this cost. "unfortunately, we cannot send a new test link" and "we will
        // not be able to proceed with this part" are one sentence apart and unrelated; an
        // unbounded pair read them as a refusal and marked a live assessment rejected.
        assert_eq!(
            verdict_for(
                "Microsoft HackerRank Online Technical Screen",
                "Complete the test in one go. Unfortunately, we cannot send a new test link. After submitting, \
                 if you decide that this location is not the right fit for you, we will not be able to proceed with this part"
            ),
            Category::Oa
        );
    }

    #[test]
    fn an_employer_absent_from_the_corpus_is_still_named() {
        // Every one of these is a real sender whose application was dropped entirely, because
        // the guesser only knew companies we had collected postings from. Workiva has none.
        let companies: Vec<String> = vec!["stripe".to_string()];
        let ctx = Context { known_companies: &companies };
        let guess = |from: &str, subject: &str| {
            classify(Some(from), Some(subject), None, &ctx).company_guess
        };

        // The local part, when the domain belongs to an ATS.
        assert_eq!(
            guess("workiva@myworkday.com", "Workiva Careers: Application for Summer 2027 Intern"),
            Some("workiva".to_string())
        );
        // The display name, whole — a name that is three ordinary words and still an employer.
        assert_eq!(
            guess("Oklahoma City Thunder <donotreply@msg.paycomonline.com>", "Password setup"),
            Some("oklahoma city thunder".to_string())
        );
        // The domain, with the careers subdomain ignored.
        assert_eq!(
            guess("Workiva Early Career <EarlyCareer@recruiting.workiva.com>", "Info session"),
            Some("workiva".to_string())
        );
        // The subject, when the sender is you forwarding a Handshake receipt.
        assert_eq!(
            guess("Jesse Li <jesseli@andrew.cmu.edu>", "Fwd: Application sent to Glencliff Labs — here's what's next"),
            Some("glencliff labs".to_string())
        );
    }

    #[test]
    fn the_fallback_refuses_a_relay_and_a_role_word() {
        let companies: Vec<String> = vec!["stripe".to_string()];
        let ctx = Context { known_companies: &companies };
        let guess = |from: &str| classify(Some(from), Some("Hello"), None, &ctx).company_guess;

        // An ATS is not an employer, and neither is the mailbox it sends from.
        assert_eq!(guess("no-reply@greenhouse.io"), None);
        assert_eq!(guess("no-reply@ashbyhq.com"), None);
        // Nor a relay, nor the user's own address.
        assert_eq!(guess("Jesse Li <jesseli@andrew.cmu.edu>"), None);
        assert_eq!(guess("notifications@joinhandshake.com"), None);
        // Nor a generic careers mailbox with nothing else in it.
        assert_eq!(guess("Careers <careers@myworkday.com>"), None);
    }

    #[test]
    fn a_known_company_still_beats_the_fallback() {
        // The fallback is a last resort: the corpus knows how a company spells its own name,
        // and a domain label does not.
        let companies: Vec<String> = vec!["jump trading".to_string()];
        let ctx = Context { known_companies: &companies };
        assert_eq!(
            classify(
                Some("no-reply@jumptrading.com"),
                Some("Thank you for applying to Jump Trading"),
                None,
                &ctx
            )
            .company_guess
            .as_deref(),
            Some("jump trading")
        );
    }

    #[test]
    fn a_short_company_name_is_not_guessed_from_prose() {
        // The corpus really contains "Secure", and it matched inside the body of a Workiva
        // information-session invitation. A name short enough to be an ordinary word is only
        // trusted from the sender or the subject, which the first pass reads.
        let companies: Vec<String> = vec!["secure".to_string(), "jump trading".to_string()];
        let ctx = Context { known_companies: &companies };
        // A relay sender, so the fallback cannot name anyone either and the body is the only
        // candidate — which is the case under test.
        let verdict = classify_with_body(
            Some("Someone <someone@gmail.com>"),
            Some("An invitation"),
            None,
            Some("Please secure your spot by registering before Friday."),
            &ctx,
        );
        assert_eq!(verdict.company_guess, None, "a word in prose is not an employer");

        // A distinctive name in the body is still found.
        let verdict = classify_with_body(
            Some("no-reply@greenhouse.io"),
            Some("Your application"),
            None,
            Some("Thank you for applying to Jump Trading."),
            &ctx,
        );
        assert_eq!(verdict.company_guess.as_deref(), Some("jump trading"));
    }

    #[test]
    fn an_assessment_someone_describes_is_not_one_you_were_given() {
        // A campus event, flipped to pressing once bodies were in scope.
        // `assert_ne` against Oa rather than `assert_eq` to a category: what matters is that it
        // is not PRESSING. Whether it lands on outreach or disregarded depends on the known-
        // company list, which this harness deliberately keeps tiny.
        assert_ne!(
            verdict_for(
                "Roblox Week @ CMU",
                "Bring any questions you have before you dive into our online assessments. Please bring your laptop"
            ),
            Category::Oa
        );
    }

    #[test]
    fn the_exclusion_does_not_match_inside_the_word_your() {
        // "your online assessment" CONTAINS "our online assessment". `hit` is substring
        // matching with no word boundary, so the exclusion silently ate a real OA — and the
        // committed fixture syn-002 is worded exactly this way, which is what caught it.
        assert_eq!(
            verdict_for(
                "Example Corp — Online Assessment",
                "Please complete your online assessment within 5 days to continue."
            ),
            Category::Oa
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
            // The NEWEST verdict per message. Joining all of them counts a re-classified
            // message twice and reports more rows than there are messages, which is how this
            // probe first read 87 rows for 83 messages.
            "SELECT m.subject, m.from_address, m.snippet, v.category
               FROM email_messages m
               JOIN email_verdicts v ON v.id = (
                   SELECT id FROM email_verdicts
                    WHERE message_id = m.id ORDER BY created_at DESC LIMIT 1)
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

    // --- The 2026-09-22 quality-control pass ---------------------------------------------
    //
    // Every case below is a real message from the burner inbox, PARAPHRASED — `labelsets/` is
    // gitignored precisely so real subject lines never reach the repository, and a test file is
    // not an exception. What is verbatim is the classifier's behaviour, which each of these
    // reproduced before its fix landed.

    /// A corpus company with a body long enough to be a real email.
    fn classify_body(from: &str, subject: &str, body: &str) -> EmailVerdict {
        let companies = companies();
        let context = Context { known_companies: &companies };
        classify_with_body(Some(from), Some(subject), None, Some(body), &context)
    }

    /// Integrity boilerplate at the foot of a genuine invitation used to switch the whole
    /// assessment branch off, because `ASSESSMENT_IS_DESCRIBED` vetoed the entire haystack.
    ///
    /// The live message was an Optiver invitation with a stated deadline, classified
    /// `disregarded`. Rule 8 calls a disregarded pressing email the costliest failure in this
    /// system, so this is the most expensive defect the pass found.
    #[test]
    fn an_integrity_notice_does_not_cancel_the_assessment_it_is_attached_to() {
        let body = "We would like to invite you to complete the assessments. Please complete \
                    the assessment by next Friday. It takes about an hour. \
                    A note on assessment integrity: our assessments are designed to evaluate \
                    your own skills, so please do not seek outside help.";
        let verdict = classify_body("Assessments <no-reply@tesla.com>", "Invitation for assessments", body);
        assert_eq!(verdict.category, Category::Oa, "{verdict:?}");
    }

    /// The other half of the same gate: a describing phrase with nothing assigned to you still
    /// disqualifies, which is what the veto was written for.
    #[test]
    fn an_email_describing_assessments_it_is_not_giving_you_is_still_not_an_assessment() {
        let body = "Come along to our info session to learn about our assessments and what we \
                    look for. Bring your laptop.";
        let verdict = classify_body("Someone <someone@tesla.com>", "Info session next week", body);
        assert_ne!(verdict.category, Category::Oa, "{verdict:?}");
    }

    /// "codesignal", "hackerrank", "karat" and "codility" were bare `ASSESSMENT` markers, so a
    /// platform's account-admin mail was `oa` at 0.8 while its actual assessment mail —
    /// which never says the platform's name — was `disregarded`. One list, opposite errors.
    #[test]
    fn a_platform_name_alone_is_not_an_assessment() {
        let verdict = classify_body(
            "CodeSignal <no-reply@codesignal.com>",
            "Verify your CodeSignal account",
            "Click below to confirm your email address and finish setting up your account.",
        );
        assert_ne!(verdict.category, Category::Oa, "{verdict:?}");
        // Not dropped either: a verification link often gates the assessment itself.
        assert_eq!(verdict.category, Category::Outreach, "{verdict:?}");
    }

    #[test]
    fn a_platform_name_with_an_assessment_beside_it_is_one() {
        let verdict = classify_body(
            "CodeSignal <no-reply@codesignal.com>",
            "Assessment completed: Roblox Assessment",
            "You have completed the Roblox assessment. Your results have been sent on.",
        );
        assert_eq!(verdict.category, Category::Oa, "{verdict:?}");
    }

    /// The outreach gate demanded a human, so `no-reply@` mail from a real employer had no path
    /// at all. Three live messages fell straight through to disregarded this way.
    #[test]
    fn a_machine_at_a_known_employer_writing_about_hiring_is_outreach() {
        let verdict = classify_body(
            "Roblox Careers <donotreply@careers.roblox.com>",
            "Welcome to Roblox Careers",
            "Your profile is set up. You can now apply for openings and follow the recruiting \
             process from your action center.",
        );
        assert_eq!(verdict.category, Category::Outreach, "{verdict:?}");
    }

    /// The guard that keeps the branch above from becoming the junk leak. "google" really is in
    /// a corpus built from job postings, so a corpus match alone would sweep in every account
    /// notice Google sends; the job-topic corroboration is what stops it.
    #[test]
    fn a_machine_at_a_known_employer_writing_about_your_account_is_not() {
        let verdict = classify_body(
            "Roblox <no-reply@accounts.roblox.com>",
            "Security alert",
            "A new sign-in on a Windows device. If this was you, no action is needed.",
        );
        assert_eq!(verdict.category, Category::Disregarded, "{verdict:?}");
    }

    /// An event word in the SUBJECT is the event. In the body it is a mention, and a named
    /// recruiter was being disregarded on the word "rsvp" in her own footer.
    #[test]
    fn an_event_word_in_the_body_does_not_turn_a_recruiter_into_a_digest() {
        let verdict = classify_body(
            "Dana Whitfield <dwhitfield@roblox.com>",
            "Roblox Week at CMU",
            "Hi Jesse, I wanted to reach out since you are at the application stage with us. \
             Our team is on campus next week. You can rsvp for any of the sessions here.",
        );
        assert_eq!(verdict.category, Category::Outreach, "{verdict:?}");
    }

    #[test]
    fn an_event_word_in_the_subject_still_is_one() {
        let verdict = classify_body(
            "Campus Events <events@connect.roblox.com>",
            "Thanks for RSVPing to Roblox Week",
            "We have your response. See you there.",
        );
        assert_eq!(verdict.category, Category::Disregarded, "{verdict:?}");
    }

    /// The relevance gate matched company names in the body with no length filter, while the
    /// company-naming pass beside it filtered to distinctive names. So a short corpus name
    /// could decide the CATEGORY while being too flimsy to be reported as the company — live,
    /// a Workiva mail was Outreach because its body said "Secure your spot".
    ///
    /// This asserts on the category, which is the assertion the older test was missing.
    #[test]
    fn a_short_company_name_in_body_prose_does_not_decide_the_category() {
        let companies = ["secure".to_string(), "roblox".to_string()];
        let context = Context { known_companies: &companies };
        let verdict = classify_with_body(
            Some("Events <hello@example.org>"),
            Some("Join our information session"),
            None,
            Some("Secure your spot at our upcoming session. Doors open at six."),
            &context,
        );
        assert_eq!(verdict.category, Category::Disregarded, "{verdict:?}");
    }

    /// `REJECTION_PAIRS` was bounded in 2026-09-11 after a real defect. The other two lists kept
    /// an unbounded matcher across four thousand characters for another eleven days.
    #[test]
    fn an_assessment_pair_must_be_one_statement_not_two_paragraphs() {
        let far = "x ".repeat(120);
        let body = format!(
            "We would like to invite you to our autumn information session. {far} \
             Separately, here is some general reading about what an assessment involves."
        );
        let verdict = classify_body("Events <events@tesla.com>", "Information session", &body);
        assert_ne!(verdict.category, Category::Oa, "{verdict:?}");
    }

    #[test]
    fn a_confirmation_pair_must_be_one_statement_not_two_paragraphs() {
        let far = "x ".repeat(120);
        let body = format!(
            "Prepare for your application process with us. {far} \
             Separately: once a referral has been received it is reviewed within a week."
        );
        let verdict = classify_body("Careers <no-reply@tesla.com>", "Preparing for your application", &body);
        assert_ne!(verdict.category, Category::Confirmation, "{verdict:?}");
    }

    /// Seven of the eleven evidence branches named a marker and quoted nothing, so a verdict
    /// could not be checked against the text that produced it — which is the entire reason the
    /// `evidence` column exists.
    #[test]
    fn every_branch_quotes_the_text_that_convinced_it() {
        let cases: &[(&str, &str, &str, Category)] = &[
            ("a@tesla.com", "Update", "We regret to inform you that we are moving on.", Category::Rejection),
            ("a@tesla.com", "Update", "Unfortunately we have decided to move forward with other candidates.", Category::Rejection),
            ("a@tesla.com", "Good news", "We are pleased to offer you the internship.", Category::Offer),
            ("a@tesla.com", "Next steps", "We would like to invite you to interview with the team.", Category::Interview),
            ("a@tesla.com", "Next steps", "Please complete your online assessment this week.", Category::Oa),
            ("no-reply@codesignal.com", "Done", "Your assessment has been submitted.", Category::Oa),
            ("a@tesla.com", "Received", "Thank you for applying to our internship programme.", Category::Confirmation),
            ("a@tesla.com", "Received", "We have received your application for the role.", Category::Confirmation),
            ("a@example.org", "Weekly digest", "New jobs for you this week.", Category::Disregarded),
            ("a@example.org", "Thanks for registering for our event", "See you there.", Category::Disregarded),
        ];
        for (from, subject, body, expected) in cases {
            let verdict = classify_body(from, subject, body);
            assert_eq!(verdict.category, *expected, "{subject:?} -> {verdict:?}");
            assert!(
                verdict.evidence.contains('…'),
                "{subject:?} names a marker but quotes nothing: {:?}",
                verdict.evidence
            );
        }
    }

    /// The corpus contains "Reply", and every no-reply address offers it a clean whole-word
    /// match. Found by reading a dry run whose evidence read "names reply in the sender, and a
    /// person sent it" — wrong about the company and wrong about the person.
    #[test]
    fn a_word_addresses_are_built_from_is_not_a_company_in_the_sender() {
        let companies = ["reply".to_string(), "roblox".to_string()];
        let context = Context { known_companies: &companies };
        let verdict = classify_with_body(
            Some("Someone <no-reply@somewhere.example>"),
            Some("Your account"),
            None,
            Some("Confirm your email address to finish signing up."),
            &context,
        );
        assert_eq!(verdict.category, Category::Disregarded, "{verdict:?}");
        // The sender fallback may still name something off the display name — that is its job,
        // and it is not a claim that the mail is job-related. What must not happen is the
        // CORPUS matching "reply", because that is what opens the relevance gate.
        assert_ne!(verdict.company_guess.as_deref(), Some("reply"), "{verdict:?}");
    }

    /// A machine is never described as a person, whichever condition opened the gate.
    #[test]
    fn the_outreach_evidence_never_calls_a_machine_a_person() {
        let companies = ["roblox".to_string()];
        let context = Context { known_companies: &companies };
        let verdict = classify_with_body(
            Some("Roblox <no-reply@notifications.roblox.com>"),
            Some("Your application"),
            None,
            Some("We have your application on file for the internship."),
            &context,
        );
        assert!(
            !verdict.evidence.contains("a person sent it"),
            "a no-reply sender is not a person: {:?}",
            verdict.evidence
        );
    }

    /// `parse` held an array literal while its doc claimed adding a variant was a compile error.
    /// `Category::index` is the exhaustive match that makes the claim true; these are the tests
    /// the function never had.
    #[test]
    fn every_category_parses_back_from_the_name_it_stores() {
        for category in Category::ALL {
            assert_eq!(Category::parse(category.as_str()), Some(category));
        }
        assert_eq!(Category::ALL.len(), 7);
        assert_eq!(Category::parse("rejections"), None);
        assert_eq!(Category::parse(""), None);
        assert_eq!(Category::parse("Rejection"), None);
    }

    #[test]
    fn no_two_categories_share_an_index() {
        let mut seen = Category::ALL.map(Category::index);
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2, 3, 4, 5, 6]);
    }

}
