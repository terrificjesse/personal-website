//! Applications the mailbox knows about and the tracker does not.
//!
//! # The gap
//!
//! [`super::sync::propose_status`] matches an email to an application and proposes a
//! transition. When the match misses, rule 8 says that is not a failure — the email is still
//! classified, stored, labelled and alerted — and the proposal is simply skipped. That is right
//! when the application exists and the matcher was wrong. It is *not* right when the
//! application was never tracked at all, which is the common case for anyone who applies
//! outside the extension.
//!
//! Measured on the live mailbox 2026-09-04: **24 confirmations and 4 OA invitations** naming
//! Tesla, Stripe, Adobe, Microsoft, Amazon, Jump Trading, Workiva and others, against **2**
//! tracked applications. Every one of those emails was classified correctly and then had
//! nowhere to go. Twelve distinct companies were recoverable.
//!
//! # It proposes; it never creates
//!
//! Accepting is a click, and that is the whole design rather than a step toward automation.
//! `INBOX_AUTO_APPLY_CONFIDENCE` is unset until Checkpoint 13 measures the classifier, and this
//! path is held to the same standard for a sharper reason: the failures are not symmetric. A
//! wrong *status change* lands on an application you recognise and can undo. A wrong
//! *application* is a row you must first notice before you can delete — and of the twelve
//! companies this would have surfaced, one is the generic word `internship`.
//!
//! See `docs/HUNT.md` § "Untracked applications" for the table and endpoint contract.

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::internships::models::ApplicationStatus;
use crate::internships::normalize::company_key;

/// Whether a guessed company is worth asking a human about.
///
/// Delegates to [`company_match::is_company_name`], which is also what
/// `classify::guess_company` filters its candidates with. One definition of "not an employer",
/// so a name that cannot be guessed cannot be proposed either.
pub fn is_proposable_company(company: &str) -> bool {
    crate::internships::company_match::is_company_name(&company_key(company))
}

/// A status an email may *create* an application at.
///
/// `offer` and `rejected` are excluded, and not as a policy that could be relaxed: both
/// presuppose a history this row does not have. An offer from a company you never applied to is
/// a classifier error every time, and creating a row at `rejected` records an ending with no
/// beginning. The database CHECK enforces the same set from the other side.
pub fn creatable_status(status: ApplicationStatus) -> bool {
    matches!(
        status,
        ApplicationStatus::Applied | ApplicationStatus::Oa | ApplicationStatus::Interview
    )
}

/// Record that an email implies an application nothing is tracking.
///
/// Returns `true` when a proposal was written. `false` covers every "nothing to ask about"
/// case — a junk company, a terminal status, or a question already answered for this company.
///
/// The `UNIQUE (user_id, company_key)` index is what makes the last one a database guarantee
/// rather than a check-then-insert race: five Stripe confirmations arriving in one pass produce
/// one proposal, and `ON CONFLICT DO NOTHING` is the whole of the dedup logic.
pub async fn propose(
    pool: &SqlitePool,
    user_id: &str,
    verdict_id: &str,
    company_guess: &str,
    title_guess: Option<&str>,
    status: ApplicationStatus,
    now: DateTime<Utc>,
) -> Result<bool> {
    if !creatable_status(status) || !is_proposable_company(company_guess) {
        return Ok(false);
    }

    let mut key = company_key(company_guess);
    let role = role_key(title_guess);

    // **Reuse a spelling we already hold for this employer.**
    //
    // The company name comes from whatever the email offered — a display name on one, a domain
    // label on the next. Chicago Trading Company sent both: one carries the name, one only
    // `chicagotrading.com`, and the two key differently, so one employer became two. Compared
    // with spaces removed, because a domain has none, and only on a prefix of six or more so
    // "imc" cannot absorb "imc trading" by accident.
    let squashed = key.replace(' ', "");
    if squashed.len() >= 6 {
        let known: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT company_key FROM application_proposals WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_all(pool)
        .await?;
        if let Some(existing) = known
            .into_iter()
            .filter(|k| {
                // Compared SQUASHED, and equality there is the main case rather than an
                // excluded one: "chicago trading" and "chicagotrading" are different keys whose
                // squashed forms are identical, which is exactly the collision to collapse.
                let theirs = k.replace(' ', "");
                k != &key
                    && theirs.len() >= 6
                    && (theirs.starts_with(&squashed) || squashed.starts_with(&theirs))
            })
            // The longest, which is the more fully spelled of the two.
            .max_by_key(String::len)
        {
            key = existing;
        }
    }

    // **A roleless email is not evidence of a second application.**
    //
    // The role is only sometimes in the text. Two emails about one opening — one naming the
    // job, one a bare "thanks for applying" — would otherwise key differently and ask twice.
    // Seen on the first real run: Adobe, Datadog, Two Sigma and Vercel each produced a roleless
    // proposal beside a roled one. So:
    //
    //   - a roleless email proposes nothing when that company already has any proposal, and
    //   - a roled email UPGRADES an unreviewed roleless proposal rather than sitting beside it.
    //
    // The per-role key still does its job for Microsoft's nine, because those nine all name
    // their role. This only collapses the case where we genuinely cannot tell them apart.
    if role.is_empty() {
        let existing: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM application_proposals WHERE user_id = ?1 AND company_key = ?2",
        )
        .bind(user_id)
        .bind(&key)
        .fetch_one(pool)
        .await?;
        if existing > 0 {
            return Ok(false);
        }
    } else {
        let upgraded = sqlx::query(
            "UPDATE application_proposals
                SET role_key = ?3, title = ?4, verdict_id = ?5
              WHERE user_id = ?1 AND company_key = ?2
                AND role_key = '' AND reviewed_at IS NULL",
        )
        .bind(user_id)
        .bind(&key)
        .bind(&role)
        .bind(title_guess)
        .bind(verdict_id)
        .execute(pool)
        .await?
        .rows_affected();
        if upgraded == 1 {
            return Ok(true);
        }
    }

    let inserted = sqlx::query(
        "INSERT INTO application_proposals
             (id, user_id, verdict_id, company_key, company_name, title, role_key,
              implied_status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (user_id, company_key, role_key) DO NOTHING",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(verdict_id)
    .bind(&key)
    .bind(display_name_for(pool, &key).await?)
    .bind(title_guess)
    .bind(&role)
    .bind(status.as_str())
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?
    .rows_affected();

    Ok(inserted == 1)
}

/// The company as it should appear on screen, preferring a spelling we already hold.
///
/// The guesser lower-cases everything, so the display name has to come from somewhere. Naive
/// title-casing gets `Igs Energy` and `Imc Trading` — both real, both wrong, and both sitting
/// in the tracker afterwards under a name their employer does not use.
///
/// `internship_postings.company_name` is 2,157 rows of employer names spelled the way the
/// employers spell them, keyed by the same `company_key` this table dedups on. Asking it is
/// strictly better than any casing heuristic, and title-casing stays as the fallback for a
/// company that has never appeared in a posting. Nothing keys off the result either way.
async fn display_name_for(pool: &SqlitePool, company: &str) -> Result<String> {
    let key = company_key(company);
    let known: Option<String> = sqlx::query_scalar(
        "SELECT company_name FROM internship_postings
          WHERE company_key = ?1 AND company_name IS NOT NULL
          ORDER BY length(company_name) ASC LIMIT 1",
    )
    .bind(&key)
    .fetch_optional(pool)
    .await?;

    Ok(known.unwrap_or_else(|| title_case(company)))
}

fn title_case(company: &str) -> String {
    company
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A role from a subject line, when it plainly states one.
///
/// **Returns `None` far more often than it returns a title, on purpose.** `title` is `NULL`-able
/// precisely so the panel can say "role unknown" instead of showing a guess, and a subject like
/// "Thank you for applying to Stripe!" names no role at all. Inventing "Software Engineer
/// Intern" there would put a fabrication in the column an audit trail reads.
pub fn role_from(subject: Option<&str>, snippet: Option<&str>) -> Option<String> {
    // Subject first: when it names the role it names it cleanly. Microsoft's does not — nine
    // real confirmations all read "Thank you for your application!" and put the role in the
    // body preview, which is why the snippet is tried at all.
    title_from_subject(subject).or_else(|| title_from_subject(snippet))
}

/// A role reduced to a comparison key: lowercase, alphanumerics and single spaces.
///
/// Two emails about the same application must produce the same key, and two applications at one
/// company must not. `None` becomes the empty string — an email that names no role is not
/// evidence of a *different* application, so every roleless email at a company collapses
/// together, which is what keeps five Stripe autoresponders one proposal instead of five.
pub fn role_key(role: Option<&str>) -> String {
    role.map(|role| {
        let words: Vec<&str> = role
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        // Drop a trailing requisition id. Tesla sent the same opening twice, once as
        // "…Access Control Systems (Fall 2026), 277192" and once without, and a key that kept
        // the number proposed one application as two.
        let mut end = words
            .iter()
            .rposition(|word| !word.chars().all(|c| c.is_ascii_digit()) || word.len() < 5)
            .map_or(0, |i| i + 1);
        // …and the words that introduced it, but ONLY if an id was actually dropped.
        //
        // This key is compared across two sources that punctuate differently. A tracked title
        // carries the posting's "(Job number: 200042200)"; the same role read out of an email
        // does not. Splitting on non-alphanumerics turns the parenthetical into the words
        // "job number 200042200", so stripping the digits alone left the two keys differing by
        // "job number" — and on 2026-09-23 seven Microsoft rejections whose role matched the
        // tracked title character for character were reported as matching no application.
        //
        // Conditional on an id having been found, so a role that genuinely ends in "Job" keeps
        // it.
        if end < words.len() {
            while end > 0
                && matches!(
                    words[end - 1].to_lowercase().as_str(),
                    "job" | "number" | "no" | "req" | "id" | "requisition" | "ref" | "posting"
                )
            {
                end -= 1;
            }
        }
        words[..end]
            .iter()
            .map(|word| word.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ")
    })
    .unwrap_or_default()
}

/// Whether `text` carries a requisition id: a run of at least five digits.
///
/// Five because a year is four, and years appear in role names as terms — "(Summer 2027)" —
/// that must survive. Every live requisition id seen so far is longer: 200042195, 10529525,
/// 277192.
fn has_requisition_id(text: &str) -> bool {
    let mut run = 0;
    for c in text.chars() {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        if run >= 5 {
            return true;
        }
    }
    false
}

/// Where a sentence stops naming the job and starts telling you about it. Lowercase, and matched
/// case-insensitively by [`cut_before_any`]: Workiva capitalises its "has been Received".
const SENTENCE_CONTINUATIONS: &[&str] = &[
    " and are ",
    " and we ",
    " and you ",
    " what happens",
    // Added 2026-11-02. Each is a title that reached the tracker or the review queue with the
    // rest of its sentence still attached, paraphrased:
    //   NVIDIA, Databricks: "…Compiler Software Engineering Intern - Summer 2027 role has been received"
    //   Workiva:            "…Intern - Software Engineering has been Received"
    //   Allegion:           "…Software Engineering and it is being reviewed by our recruiting team"
    //   OKC Thunder:        "…Basketball Operations position with the Oklahoma City Thunder"
    // The Thunder one cost more than a long title: the tail kept its key from equalling the
    // tracked title's, so an application already in the tracker was proposed as untracked.
    // `" and it is "`, not `" and it "`, so an "Information Systems and IT Intern" keeps its IT.
    " has been ",
    " and it is ",
    " is being ",
    " position with ",
    " role with ",
    // Optiver: "…your application for the Institutional Trader Intern (2027 Start - Chicago)
    // role here at Optiver". " at " already ends the role; this is the "role here" it leaves.
    " role here",
];

/// `text` up to the first of `markers`, compared ASCII-case-insensitively.
///
/// `to_ascii_lowercase` rather than `to_lowercase` because it keeps every byte where it was, so
/// an index found in the lowered copy is a valid cut in the original. Full Unicode lowercasing
/// can change a string's length and would cut in the wrong place.
fn cut_before_any<'a>(text: &'a str, markers: &[&str]) -> &'a str {
    let lower = text.to_ascii_lowercase();
    let end = markers.iter().filter_map(|marker| lower.find(marker)).min().unwrap_or(text.len());
    &text[..end]
}

pub fn title_from_subject(subject: Option<&str>) -> Option<String> {
    // Real mail is HTML-escaped, so a role reaches this as "Summer &#39;27" and would be stored
    // with the entity in it, in the column the tracker renders.
    let subject = crate::inbox::classify::decode_entities(subject?);
    let subject = subject.as_str();
    let lower = subject.to_lowercase();
    // Only the shapes that genuinely carry a role, and only the text after the marker.
    for marker in [
        "your application for ",
        "application for ",
        "applying to the ",
        "application to the ",
        "applying to our ",
        "application: ",
        "following position: ",
        "position: ",
        "for the position of ",
        "interest in the ",
        // Added 2026-09-30 from the phrasings `backfill-status` could not read. Each is guarded
        // by the "intern" gate below, which is what stops "your interest in Two Sigma!" from
        // yielding "Two Sigma" as a role: these markers are broad on purpose, and the gate is
        // what makes that safe.
        //   Amazon:  "Thank you for your interest in Software Development Engineer Intern, …"
        //   HRT:     "Thank you for your interest in Hudson River Trading's Summer 2027 …"
        //   Microsoft: "Thank you for applying for the Microsoft Full Stack Product … Req ID (…)"
        "your interest in ",
        "applying for the ",
        "applying for ",
    ] {
        if let Some(at) = lower.find(marker) {
            let tail = subject[at + marker.len()..].trim();
            let tail = tail
                .trim_end_matches(['!', '.', '?'])
                .split(" at ")
                .next()
                .unwrap_or(tail)
                .trim();
            // A leading company possessive: "Hudson River Trading's Summer 2027 Algorithm
            // Development Internship". The owner is the employer, not part of the role, and
            // keeping it means the title can never agree with the posting's. Once only, and only
            // for a short digit-free owner, so a role that merely contains a possessive later on
            // — a "Master's" programme — keeps it.
            let tail = match tail.find("'s ").map(|at| (at, "'s ".len())).or_else(|| {
                tail.find("\u{2019}s ").map(|at| (at, "\u{2019}s ".len()))
            }) {
                Some((at, width))
                    if (1..=4).contains(&tail[..at].split_whitespace().count())
                        && !tail[..at].chars().any(|c| c.is_ascii_digit()) =>
                {
                    tail[at + width..].trim_start()
                }
                _ => tail,
            };
            // "…your interest in TetraMem Inc and the US 2026 Software - Machine Learning Intern":
            // the employer, then the role. Only after an "interest in" marker, where an employer
            // is what usually comes next, and only for a short head that does not itself name an
            // internship, so a role with "and the" inside it keeps it.
            let tail = match tail.to_ascii_lowercase().find(" and the ") {
                Some(at)
                    if marker.contains("interest in")
                        && tail[..at].split_whitespace().count() <= 4
                        && !tail[..at].to_ascii_lowercase().contains("intern") =>
                {
                    tail[at + " and the ".len()..].trim_start()
                }
                _ => tail,
            };
            // Leading noise the surrounding sentence leaves behind: an article, a possessive,
            // or a requisition number the ATS prefixes to the title. All real —
            // "our [Summer 2027] Software Engineer Intern", "R171519 2027 Intern - …".
            let mut tail = tail;
            loop {
                let before = tail;
                for prefix in [
                    "the ", "The ", "our ", "Our ", "a ", "an ",
                    // "…your application for the following position: Software Engineer…"
                    "following position: ", "Following position: ",
                    "following role: ", "position: ", "Position: ", "role of ",
                    // "…your application for the position of ML Systems … Intern" and
                    // "…for position 130043 - QA/Test Engineer Intern" (the id goes below).
                    // "position of " is listed first so it is never left as a stray "of ".
                    "position of ", "Position of ", "position ", "Position ",
                ] {
                    tail = tail.strip_prefix(prefix).unwrap_or(tail);
                }
                tail = tail.trim_start_matches(['[', ']', '-', ':', ' ']);
                // A requisition id: one leading token that is letters-then-digits or all digits,
                // and is not itself a word. "R171519", "20005432".
                if let Some((first, rest)) = tail.split_once(' ')
                    && first.len() >= 5
                    && first.chars().any(|c| c.is_ascii_digit())
                    && first.chars().all(|c| c.is_ascii_alphanumeric())
                    && !first.chars().all(|c| c.is_alphabetic())
                {
                    tail = rest.trim_start();
                }
                if tail == before {
                    break;
                }
            }
            // The subject's own sentence, not part of the role: "... Intern role",
            // "... Position". Trimmed so the tracker shows a title and not a clause.
            // A role is a noun phrase, not the rest of the paragraph. Real extractions ran on
            // into "… Software Engineer role. We loved reading about…", so the title is cut at
            // the first sentence end before anything else is trimmed.
            let tail = tail
                .split_inclusive(['.', '!', '?', ';'])
                .next()
                .unwrap_or(tail);
            // …and where the sentence stops naming the job and starts addressing you. A
            // heuristic, and an admitted one: extracting a noun phrase from prose with
            // substrings has a floor, and the honest failure is a title that is too long rather
            // than one that is wrong. The role is still identifiable, and editable once tracked.
            let tail = cut_before_any(tail, SENTENCE_CONTINUATIONS);
            // A requisition id in a trailing parenthetical is not part of the role's name, and
            // carrying it costs more than the twenty characters it occupies.
            //
            // Microsoft's rejections read "…Intern Opportunities for University Students,
            // (Job number: 200042195)". With the parenthetical kept, that title is 123
            // characters and the 120 gate below discards it — so `role_from` returned `None`,
            // `match_application` fell back to company-only, and on 2026-09-23 three rejections
            // for three different Microsoft roles all proposed against the same application.
            // `role_key` already strips a trailing requisition id; doing it here too is what
            // lets the title survive long enough to reach it.
            //
            // **A requisition id, not any number.** This first matched on "contains a digit",
            // which also stripped terms — "(Fall 2026)", "(Summer 2027)" — and took whatever
            // followed them. Tesla's rejection named "…Access Control Systems (Fall 2026)
            // position"; the term went, the key no longer equalled the tracked title's, and the
            // rejection sat unmatched from 2026-09-23 to 2026-09-30. An id is a run of five or
            // more digits. A year is four.
            let tail = match (tail.rfind('('), tail.rfind(')')) {
                (Some(open), Some(close))
                    if close > open
                        && has_requisition_id(&tail[open + 1..close])
                        && tail[open + 1..close].len() <= 30 =>
                {
                    tail[..open].trim_end()
                }
                _ => tail,
            };
            // An opening bracket that never closes is where the snippet was cut, not part of the
            // role. Gmail's preview stops at ~200 characters, so "Software Engineering Internship
            // (Summer 2027)" arrives as "…Internship (Summer 2", and the fragment "summer 2"
            // matches no word of the real title. Cutting it leaves a clean prefix, which the
            // matcher can then recognise as one — see `advance::match_application`.
            let tail = match (tail.rfind(['(', '[']), tail.rfind([')', ']'])) {
                (Some(open), close) if close.is_none_or(|close| close < open) => {
                    tail[..open].trim_end()
                }
                _ => tail,
            };
            // Requisition residue the parenthetical leaves behind: "…(Web + Services) Req ID"
            // once "(200042195)" is gone, or "Software Engineer Req #2" in a screen invitation.
            let mut tail = tail.trim_end();
            loop {
                let before = tail;
                if let Some((head, last)) = tail.rsplit_once(' ') {
                    let last = last.trim_end_matches([',', '.', ':']);
                    let is_residue = matches!(last.to_lowercase().as_str(), "req" | "id" | "#")
                        || (last.starts_with('#') && last[1..].chars().all(|c| c.is_ascii_digit()));
                    if is_residue {
                        tail = head.trim_end();
                    }
                }
                if tail == before {
                    break;
                }
            }
            let mut tail = tail.trim_end().trim_end_matches(['.', '!', '?', ';', ',', ' ']);
            loop {
                let before = tail;
                for suffix in ["role", "Role", "position", "Position", "job", "Job", "opportunity"] {
                    tail = tail.trim_end().trim_end_matches(suffix);
                }
                tail = tail.trim_end().trim_end_matches(['.', '!', ',', ']', ' ']);
                if tail == before {
                    break;
                }
            }
            let tail = tail.trim();
            if tail.len() >= 4 && tail.len() <= 120 && tail.to_lowercase().contains("intern") {
                return Some(tail.to_string());
            }
        }
    }
    None
}

/// What deciding a proposal did.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// Accepted: the application now exists, with this id.
    Created(String),
    /// Rejected: nothing was created, and the question is settled for that company.
    Rejected,
    /// No such open proposal for this user. Also what a second accept returns, which is what
    /// makes a double-click safe.
    NotFound,
}

/// Accept or reject one proposal.
///
/// **One transaction, because this is two writes.** A reviewed proposal whose application was
/// never created reads as settled while nothing exists, and nothing anywhere records that the
/// two disagree — the same reasoning `routes::inbox::decide` gives for status proposals.
///
/// Lives here rather than in the route so the HTTP handler and the CLI share it. Two
/// implementations would be two chances for the halves to come apart.
pub async fn decide(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
    accept: bool,
) -> Result<Decision> {
    let mut tx = crate::db::begin_write(pool).await?;

    let row: Option<(String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT company_name, title, implied_status, verdict_id
           FROM application_proposals
          WHERE id = ? AND user_id = ? AND reviewed_at IS NULL",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((company_name, title, implied_status, verdict_id)) = row else {
        return Ok(Decision::NotFound);
    };

    let now = Utc::now();

    // **When the application happened, which is not when you pressed the button.**
    //
    // A confirmation email lands essentially at apply time, so the message's `received_at` is
    // by far the best estimate of `applied_at` — and `applied_at` is not decoration. It is the
    // cohort key for the monthly breakdown, the left edge of every time-to-first-response
    // measurement, and what the analytics date window filters on. Stamping `now` instead put
    // eleven applications made between 30 August and 4 September into a single 5 September
    // cohort and made their response times a week short.
    //
    // Falls back to `now` if the message cannot be resolved, which is the only honest answer
    // left when the evidence is gone.
    let applied_at: DateTime<Utc> = sqlx::query_scalar::<_, String>(
        "SELECT m.received_at
           FROM email_verdicts v
           JOIN email_messages m ON m.id = v.message_id
          WHERE v.id = ?",
    )
    .bind(&verdict_id)
    .fetch_optional(&mut *tx)
    .await?
    .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
    .map(|at| at.with_timezone(&Utc))
    .unwrap_or(now);

    let mut created = None;

    if accept {
        let status = ApplicationStatus::parse(&implied_status)
            .ok_or_else(|| anyhow::anyhow!("proposal {id} holds an unparseable status"))?;
        let application_id = Uuid::new_v4().to_string();

        // `posting_id` is NULL: there is no posting behind this, and that column is already
        // nullable with a read path that treats a non-resolving posting exactly like a null
        // one. `snapshot_json` holds the email that caused it rather than a posting snapshot —
        // a fabricated posting record in the column an audit trail reads would be worse than
        // the truth, which is that this came from mail.
        let snapshot = serde_json::json!({
            "origin": "email",
            "verdict_id": verdict_id,
            "company_name": company_name,
            "title": title,
        })
        .to_string();

        sqlx::query(
            // `applied_at` and `status_changed_at` are when the thing happened; `snapshot_at`,
            // `created_at` and `updated_at` are when we recorded it. Conflating the two is what
            // put every one of these in the wrong month.
            "INSERT INTO internship_applications
                (id, user_id, posting_id, company_name, title, url,
                 source, snapshot_json, snapshot_at,
                 status, applied_at, status_changed_at, created_at, updated_at)
             VALUES (?1, ?2, NULL, ?3, ?4, '', 'email', ?5, ?6, ?7, ?8, ?8, ?6, ?6)",
        )
        .bind(&application_id)
        .bind(user_id)
        .bind(&company_name)
        .bind(title.as_deref().unwrap_or("Unknown role"))
        .bind(&snapshot)
        .bind(now)
        .bind(status.as_str())
        .bind(applied_at)
        .execute(&mut *tx)
        .await?;

        // 10e: every writer emits, and this one has to say *why the row exists*.
        crate::internships::application_events::record(
            &mut tx,
            crate::internships::application_events::NewApplicationEvent {
                application_id: &application_id,
                from_status: None,
                to_status: status,
                actor: crate::internships::application_events::Actor::Email,
                cause: Some(crate::internships::application_events::Cause::EmailVerdict(
                    &verdict_id,
                )),
                // The transition happened when the mail did, so the event fold and
                // `applied_at` agree rather than reporting a response before its application.
                at: applied_at,
                note: None,
            },
        )
        .await?;

        created = Some(application_id);
    }

    sqlx::query(
        "UPDATE application_proposals SET reviewed_at = ?3, accepted = ?4
          WHERE id = ?1 AND user_id = ?2",
    )
    .bind(id)
    .bind(user_id)
    .bind(now.to_rfc3339())
    .bind(i64::from(accept))
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(match created {
        Some(application_id) => Decision::Created(application_id),
        None => Decision::Rejected,
    })
}

/// Just enough of a stored message to re-classify it.
#[derive(sqlx::FromRow)]
struct StoredMessage {
    id: String,
    user_id: String,
    from_address: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
}

/// What a backfill pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BackfillReport {
    /// Messages whose category implies an application at all.
    pub implying: usize,
    /// Already matched to a tracked application — nothing to propose.
    pub already_tracked: usize,
    /// No company could be guessed, or the guess was not a company.
    pub unusable_company: usize,
    /// **And which ones**, because a count is not findable.
    ///
    /// "5 had no usable company" tells you a number and gives you no way to reach the rows it
    /// counted, which is the same defect as a source silently returning zero. These are real
    /// applications that will never be proposed until the guesser improves, so the only way to
    /// act on them is to be told which they are.
    pub unusable_subjects: Vec<String>,
    /// Proposals written. Lower than the message count by design: one per company.
    pub proposed: usize,
    /// A company already asked about — the second Stripe confirmation and the third.
    pub already_asked: usize,
}

/// Re-read the stored mailbox and propose the applications it implies.
///
/// The forward path only sees mail as it arrives, and the whole reason this exists is a mailbox
/// that already accumulated a month of confirmations with nowhere to put them. This re-runs the
/// **same classifier** over stored messages — not a second implementation of the rules — and
/// proposes through the same [`propose`] the sync pass uses, so a backfilled proposal and a
/// live one are the same row written the same way.
///
/// Idempotent through `ON CONFLICT DO NOTHING` on `(user_id, company_key)`: running it twice
/// proposes nothing the second time, and running it after a rejection does not re-ask.
pub async fn backfill(pool: &SqlitePool, now: DateTime<Utc>) -> Result<BackfillReport> {
    let companies: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT lower(company_name) FROM internship_postings
          WHERE company_name IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    let context = super::classify::Context {
        known_companies: &companies,
    };

    let messages: Vec<StoredMessage> = sqlx::query_as(
        "SELECT id, user_id, from_address, subject, snippet
           FROM email_messages ORDER BY received_at",
    )
    .fetch_all(pool)
    .await?;

    let mut report = BackfillReport::default();
    for StoredMessage {
        id: message_id,
        user_id,
        from_address: from,
        subject,
        snippet,
    } in &messages
    {
        let verdict = super::classify::classify(
            from.as_deref(),
            subject.as_deref(),
            snippet.as_deref(),
            &context,
        );
        let Some(status) = super::advance::implied_status(verdict.category) else {
            continue;
        };
        if !creatable_status(status) {
            continue;
        }
        report.implying += 1;

        // Scoped to this message's own user, so a second account's applications can never
        // satisfy the match for the first one's mail.
        let applications: Vec<super::advance::TrackedApplication> =
            sqlx::query_as("SELECT id, company_name AS company, title FROM internship_applications WHERE user_id = ?")
                .bind(user_id)
                .fetch_all(pool)
                .await?;
        if super::advance::match_application(
            verdict.company_guess.as_deref(),
            role_from(subject.as_deref(), snippet.as_deref()).as_deref(),
            &applications,
        )
        .is_some()
        {
            report.already_tracked += 1;
            continue;
        }

        let Some(company) = verdict.company_guess.as_deref().filter(|c| is_proposable_company(c))
        else {
            report.unusable_company += 1;
            report.unusable_subjects.push(format!(
                "{}  [{}]",
                subject.as_deref().unwrap_or("(no subject)"),
                verdict.company_guess.as_deref().unwrap_or("no company guessed")
            ));
            continue;
        };

        // The verdict row for this stored message, which is what makes the proposal traceable
        // back to the mail. A message with no stored verdict is skipped rather than proposed
        // without evidence.
        let verdict_id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM email_verdicts WHERE message_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(message_id)
        .fetch_optional(pool)
        .await?;
        let Some(verdict_id) = verdict_id else {
            report.unusable_company += 1;
            report.unusable_subjects.push(format!(
                "{}  [no stored verdict]",
                subject.as_deref().unwrap_or("(no subject)")
            ));
            continue;
        };

        if propose(
            pool,
            user_id,
            &verdict_id,
            company,
            role_from(subject.as_deref(), snippet.as_deref()).as_deref(),
            status,
            now,
        )
        .await?
        {
            report.proposed += 1;
        } else {
            report.already_asked += 1;
        }
    }

    Ok(report)
}

/// What `inbox retitle` changed, or would change.
#[derive(Debug, Default)]
pub struct RetitleReport {
    /// Pending proposals given a corrected title: company, old title, new title.
    pub retitled: Vec<(String, Option<String>, String)>,
    /// Pending proposals whose corrected role is already in the tracker: company, role. They
    /// were never untracked, and the garbled title is what stopped them matching. Removed.
    pub already_tracked: Vec<(String, String)>,
    /// Pending proposals that, corrected, ask the same question as another proposal for that
    /// company and role: company, role, and the status the kept one was raised to when this one
    /// was further along. Removed; the other one stays.
    pub duplicates: Vec<(String, String, Option<String>)>,
    /// Pending proposals whose corrected role is tracked, but which say more than the tracker
    /// does: company, role, tracked status, implied status. Left for you, not removed.
    pub ahead_of_tracker: Vec<(String, String, String, String)>,
    /// Tracked applications whose title carried the same garbling: company, old, new.
    pub applications: Vec<(String, String, String)>,
    /// Pending proposals whose employer was re-guessed: old company, new company. Added
    /// 2026-11-02 with the sender-name fix: a recruiter ("Aaron Simonds") or a department
    /// ("Epic Human Resources") had been stored as the employer.
    pub renamed: Vec<(String, String)>,
}

#[derive(sqlx::FromRow)]
struct PendingTitle {
    id: String,
    user_id: String,
    company_key: String,
    company_name: String,
    title: Option<String>,
    implied_status: String,
    verdict_id: String,
    from_address: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
}

/// Whether a stored title carries one of the garblings [`title_from_subject`] used to leave:
/// the rest of its sentence, or the word "position" in front.
fn carries_sentence_residue(title: &str) -> bool {
    cut_before_any(title, SENTENCE_CONTINUATIONS).len() < title.len()
        || title.to_ascii_lowercase().starts_with("position ")
}

/// Re-read titles with the current parser, after it learns a phrasing it used to garble.
///
/// Fixing [`title_from_subject`] fixes the next email, not the rows already written, and the
/// rows matter twice over: a title is what the tracker shows, and its [`role_key`] is what the
/// next email about that application has to equal. Added 2026-11-02, when 44 pending proposals
/// included seven titles ending "…role has been received" and one tracked application was
/// being re-proposed because its title ran on into the employer's name.
///
/// **A dry run unless `apply`.** The dry run is the same transaction, rolled back, so what it
/// prints is what the real run does, unique constraints included.
///
/// - **Pending proposals** are re-derived exactly as the live sync derives them, by
///   [`role_from`] on the verdict's message. A corrected role that a tracked application
///   already has removes the proposal: it was never untracked. One that now equals another
///   proposal for the same company removes this one and keeps the other. Anything else is
///   retitled. **Reviewed proposals are never touched**: each is an answer you gave.
/// - **Tracked applications** are touched only where the stored title itself carries the
///   garbling ([`carries_sentence_residue`]). A title copied from a posting is never re-parsed,
///   because the parser's other rules (splitting on " at ", dropping a leading possessive) are
///   written for email sentences and would damage it. The snapshot is left as it was taken.
pub async fn retitle(pool: &SqlitePool, now: DateTime<Utc>, apply: bool) -> Result<RetitleReport> {
    let mut tx = pool.begin().await?;
    let mut report = RetitleReport::default();

    let pending: Vec<PendingTitle> = sqlx::query_as(
        "SELECT p.id, p.user_id, p.company_key, p.company_name, p.title, p.implied_status,
                p.verdict_id, m.from_address, m.subject, m.snippet
           FROM application_proposals p
           JOIN email_verdicts v ON v.id = p.verdict_id
           JOIN email_messages m ON m.id = v.message_id
          WHERE p.reviewed_at IS NULL
          ORDER BY p.created_at, p.id",
    )
    .fetch_all(&mut *tx)
    .await?;

    // The corpus the employer is guessed against, exactly as `backfill` builds it.
    let corpus: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT lower(company_name) FROM internship_postings WHERE company_name IS NOT NULL",
    )
    .fetch_all(&mut *tx)
    .await?;
    let context = super::classify::Context { known_companies: &corpus };

    for row in pending {
        // Exactly what `sync::propose_status` derives for this message today. A role it can no
        // longer read keeps the stored one rather than losing it.
        let title = role_from(row.subject.as_deref(), row.snippet.as_deref()).or_else(|| row.title.clone());
        // The employer, re-guessed by the current classifier from the sender and the subject —
        // the fields `guess_company` trusts — and kept as stored when today's guess is not one.
        let guessed = super::classify::classify(
            row.from_address.as_deref(),
            row.subject.as_deref(),
            row.snippet.as_deref(),
            &context,
        )
        .company_guess
        .filter(|company| is_proposable_company(company));
        let company_changed = guessed.as_deref().is_some_and(|company| company_key(company) != row.company_key);
        let title_changed = title != row.title;
        if !company_changed && !title_changed {
            continue;
        }
        let company_name = match guessed.as_deref().filter(|_| company_changed) {
            Some(company) => display_name_for(pool, company).await?,
            None => row.company_name.clone(),
        };
        let row_company_key = if company_changed { company_key(&company_name) } else { row.company_key.clone() };
        // The label every report line below uses for this proposal's role.
        let role = title.clone().unwrap_or_else(|| "(no role)".to_string());

        let applications: Vec<super::advance::TrackedApplication> = sqlx::query_as(
            "SELECT id, company_name AS company, title FROM internship_applications WHERE user_id = ?",
        )
        .bind(&row.user_id)
        .fetch_all(&mut *tx)
        .await?;
        let implied = ApplicationStatus::parse(&row.implied_status);
        if let Some(tracked_id) = super::advance::match_application(Some(&company_name), title.as_deref(), &applications) {
            // Removed only when the tracker already knows as much as this proposal does. An OA
            // proposal for an application tracked at `applied` is news, not a duplicate, and
            // deleting it would lose the one record that the assessment arrived.
            let tracked: String = sqlx::query_scalar("SELECT status FROM internship_applications WHERE id = ?")
                .bind(tracked_id)
                .fetch_one(&mut *tx)
                .await?;
            let ahead = match (ApplicationStatus::parse(&tracked), implied) {
                (Some(current), Some(implied)) => super::advance::may_advance(current, implied),
                _ => true,
            };
            if ahead {
                report.ahead_of_tracker.push((company_name, role, tracked, row.implied_status));
                continue;
            }
            sqlx::query("DELETE FROM application_proposals WHERE id = ?")
                .bind(&row.id)
                .execute(&mut *tx)
                .await?;
            report.already_tracked.push((company_name, role));
            continue;
        }

        let key = role_key(title.as_deref());
        let taken: Option<(String, String)> = sqlx::query_as(
            "SELECT id, implied_status FROM application_proposals
              WHERE user_id = ? AND company_key = ? AND role_key = ? AND id <> ?",
        )
        .bind(&row.user_id)
        .bind(&row_company_key)
        .bind(&key)
        .bind(&row.id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((kept_id, kept_status)) = taken {
            // The removed one must not take the further status with it. IBM's assessment
            // reminder and its confirmation were one role; deleting the OA proposal and keeping
            // the `applied` one would create the application a step behind the mail.
            let raised = match (ApplicationStatus::parse(&kept_status), implied) {
                (Some(kept), Some(implied)) if super::advance::may_advance(kept, implied) => {
                    // The title moves with the evidence. A title read from one email beside a
                    // verdict citing another is a row the next run would "correct" again.
                    sqlx::query(
                        "UPDATE application_proposals SET implied_status = ?, verdict_id = ?, title = ? WHERE id = ?",
                    )
                        .bind(&row.implied_status)
                        .bind(&row.verdict_id)
                        .bind(&title)
                        .bind(&kept_id)
                        .execute(&mut *tx)
                        .await?;
                    Some(row.implied_status.clone())
                }
                _ => None,
            };
            sqlx::query("DELETE FROM application_proposals WHERE id = ?")
                .bind(&row.id)
                .execute(&mut *tx)
                .await?;
            report.duplicates.push((company_name, role, raised));
            continue;
        }

        sqlx::query(
            "UPDATE application_proposals SET title = ?, role_key = ?, company_name = ?, company_key = ?
              WHERE id = ?",
        )
        .bind(&title)
        .bind(&key)
        .bind(&company_name)
        .bind(&row_company_key)
        .bind(&row.id)
        .execute(&mut *tx)
        .await?;
        if company_changed {
            report.renamed.push((row.company_name.clone(), company_name.clone()));
        }
        if title_changed {
            report.retitled.push((company_name, row.title, role));
        }
    }

    let tracked: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, company_name, title FROM internship_applications ORDER BY company_name")
            .fetch_all(&mut *tx)
            .await?;
    for (id, company, title) in tracked {
        if !carries_sentence_residue(&title) {
            continue;
        }
        let Some(fixed) = title_from_subject(Some(&format!("application for {title}"))) else {
            continue;
        };
        if fixed == title {
            continue;
        }
        sqlx::query("UPDATE internship_applications SET title = ?, updated_at = ? WHERE id = ?")
            .bind(&fixed)
            .bind(now.to_rfc3339())
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        report.applications.push((company, title, fixed));
    }

    if apply {
        tx.commit().await?;
    } else {
        tx.rollback().await?;
    }
    Ok(report)
}

const USAGE: &str = "\
usage:
  inbox backfill-untracked        re-read stored mail and propose what it implies
  inbox retitle [--apply]         re-read pending proposals' titles and employers, and tracked
                                  titles, with the current rules (a dry run without --apply)
  inbox untracked reject-id <id>  reject one proposal by id, where its company is ambiguous
  inbox untracked                 list proposals awaiting your review
  inbox untracked accept <company>   track it  (use `all` for every pending one)
  inbox untracked reject <company>   record that you did not apply there

Company matching is case-insensitive and matches on the normalized key, so `jump` will not
match `jump trading` but `Jump Trading` will. Ambiguity is refused rather than guessed.
";

/// The single user this CLI acts for.
///
/// Refuses rather than guesses when there is more than one: accepting a proposal creates a row
/// in somebody's tracker, and picking the wrong somebody silently is not a recoverable mistake.
async fn only_user(pool: &SqlitePool) -> Result<String> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM users ORDER BY created_at")
        .fetch_all(pool)
        .await?;
    match ids.len() {
        1 => Ok(ids.into_iter().next().expect("one")),
        0 => bail!("no users exist"),
        n => bail!("{n} users exist; this command will not guess which one. Use the web UI."),
    }
}

async fn pending(pool: &SqlitePool, user_id: &str) -> Result<Vec<(String, String, Option<String>, String)>> {
    Ok(sqlx::query_as(
        "SELECT id, company_name, title, implied_status
           FROM application_proposals
          WHERE user_id = ? AND reviewed_at IS NULL
          ORDER BY company_name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

pub async fn main(pool: &SqlitePool, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("backfill-untracked") if args.len() == 1 => {}
        Some("retitle") => {
            let apply = match &args[1..] {
                [] => false,
                [flag] if flag == "--apply" => true,
                _ => {
                    print!("{USAGE}");
                    return Ok(());
                }
            };
            return retitle_cli(pool, apply).await;
        }
        Some("untracked") => return untracked_cli(pool, &args[1..]).await,
        _ => {
            print!("{USAGE}");
            return Ok(());
        }
    }

    let report = backfill(pool, Utc::now()).await?;
    println!(
        "inbox backfill-untracked:\n  \
         {} message(s) imply an application\n  \
         {} already matched a tracked application\n  \
         {} had no usable company\n  \
         {} already asked about\n  \
         {} NEW proposal(s) awaiting your review",
        report.implying,
        report.already_tracked,
        report.unusable_company,
        report.already_asked,
        report.proposed,
    );

    if !report.unusable_subjects.is_empty() {
        println!(
            "\nthese look like applications but no employer could be named — track them by hand:"
        );
        for subject in &report.unusable_subjects {
            println!("  {subject}");
        }
    }
    Ok(())
}

async fn retitle_cli(pool: &SqlitePool, apply: bool) -> Result<()> {
    let report = retitle(pool, Utc::now(), apply).await?;
    let mode = if apply { "applied" } else { "DRY RUN, nothing written; --apply to write" };
    println!("inbox retitle ({mode}):");
    println!("\n  {} pending proposal(s) retitled", report.retitled.len());
    for (company, old, new) in &report.retitled {
        println!("    {company}: {:?}\n      -> {new:?}", old.as_deref().unwrap_or("(no role)"));
    }
    println!("\n  {} pending proposal(s) given a corrected employer", report.renamed.len());
    for (old, new) in &report.renamed {
        println!("    {old:?} -> {new:?}");
    }
    println!("\n  {} pending proposal(s) removed: already in your tracker", report.already_tracked.len());
    for (company, role) in &report.already_tracked {
        println!("    {company}: {role:?}");
    }
    println!("\n  {} pending proposal(s) removed: duplicate of another proposal", report.duplicates.len());
    for (company, role, raised) in &report.duplicates {
        match raised {
            Some(status) => println!("    {company}: {role:?}  (the one kept now implies {status})"),
            None => println!("    {company}: {role:?}"),
        }
    }
    println!("\n  {} pending proposal(s) left for you: tracked, but this one is further along", report.ahead_of_tracker.len());
    for (company, role, tracked, implied) in &report.ahead_of_tracker {
        println!("    {company}: {role:?}  tracked at {tracked}, the email implies {implied}");
    }
    println!("\n  {} tracked application title(s) corrected", report.applications.len());
    for (company, old, new) in &report.applications {
        println!("    {company}: {old:?}\n      -> {new:?}");
    }
    Ok(())
}

async fn untracked_cli(pool: &SqlitePool, args: &[String]) -> Result<()> {
    let user_id = only_user(pool).await?;
    let rows = pending(pool, &user_id).await?;

    let (verb, target) = match args {
        [] => {
            if rows.is_empty() {
                println!("Nothing awaiting review.");
                return Ok(());
            }
            println!("{} proposal(s) awaiting review:\n", rows.len());
            for (_, company, title, status) in &rows {
                println!(
                    "  {company:<14} {:<40} would be added as {status}",
                    title.as_deref().unwrap_or("(role not named in the email)")
                );
            }
            println!("\n  accept with: inbox untracked accept <company>   (or `all`)");
            return Ok(());
        }
        // By id, for the case the company form refuses: two proposals at one employer, one of
        // them a duplicate. The same `decide` the page's Reject button calls.
        [verb, id] if verb == "reject-id" => {
            return match decide(pool, &user_id, id, false).await? {
                Decision::Rejected => {
                    println!("rejected {id}");
                    Ok(())
                }
                Decision::NotFound => bail!("no pending proposal {id}"),
                Decision::Created(_) => bail!("rejecting {id} created an application, which it never should"),
            };
        }
        [verb, target] if verb == "accept" || verb == "reject" => (verb.as_str(), target.as_str()),
        _ => {
            print!("{USAGE}");
            return Ok(());
        }
    };
    let accept = verb == "accept";

    // `all` is spelled out rather than implied by omitting the target: a bare `accept` that
    // took everything would be one typo away from tracking eleven companies you meant to read
    // first.
    let chosen: Vec<_> = if target.eq_ignore_ascii_case("all") {
        rows.clone()
    } else {
        let wanted = company_key(target);
        rows.iter()
            .filter(|(_, company, _, _)| company_key(company) == wanted)
            .cloned()
            .collect()
    };

    if chosen.is_empty() {
        bail!(
            "no pending proposal matches {target:?}. Run `inbox untracked` to see the list."
        );
    }

    for (id, company, _, status) in &chosen {
        match decide(pool, &user_id, id, accept).await? {
            Decision::Created(app_id) => {
                println!("tracked   {company} as {status}  ({app_id})");
            }
            Decision::Rejected => println!("not mine  {company}"),
            // Only reachable if something reviewed it between the list and the loop.
            Decision::NotFound => println!("skipped   {company} — already reviewed"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Phrasings `backfill-status` could not read a role from on 2026-09-30, paraphrased. Each
    /// returned `None` before the markers and cleanup it names were added.
    #[test]
    fn roles_read_from_the_phrasings_that_used_to_yield_nothing() {
        let cases = [
            // "your interest in <Role>" — only "interest in the" was known.
            (
                "Thank you for your interest in Software Development Engineer Intern, ROBOTICS - \
                 2027 (ID: 10529525). If you have completed",
                "Software Development Engineer Intern, ROBOTICS - 2027",
            ),
            // A company possessive in front of the role.
            (
                "Thank you for your interest in Hudson River Trading&#39;s Summer 2027 Algorithm \
                 Development Internship! Please see below",
                "Summer 2027 Algorithm Development Internship",
            ),
            // A bracket the 200-character preview cut open.
            (
                "We&#39;ve successfully received your application for the following position: \
                 Software Engineering Internship (Summer 2",
                "Software Engineering Internship",
            ),
        ];
        for (snippet, expected) in cases {
            assert_eq!(
                role_from(Some("Thank you"), Some(snippet)).as_deref(),
                Some(expected),
                "{snippet}"
            );
        }
    }

    /// Titles that reached the tracker or the review queue on 2026-11-02 with the rest of their
    /// sentence attached, or with the word that introduced them still in front. Paraphrased
    /// shapes, not the mail itself. Each returned the garbled title before this fix.
    #[test]
    fn a_role_ends_where_its_sentence_moves_on() {
        let cases = [
            (
                "Thanks for your interest! Your application for the Compiler Engineering Intern - \
                 Summer 2027 role has been received and our team will review it",
                "Compiler Engineering Intern - Summer 2027",
            ),
            (
                "Your application for Summer 2027 Intern - Software Engineering has been Received",
                "Summer 2027 Intern - Software Engineering",
            ),
            (
                "Thank you for your application for Summer Intern - Software Engineering and it is \
                 being reviewed by our recruiting team",
                "Summer Intern - Software Engineering",
            ),
            (
                "Thanks for your application for the Software Engineer Intern / Basketball \
                 Operations position with the Oklahoma City Thunder. We will",
                "Software Engineer Intern / Basketball Operations",
            ),
            (
                "Thank you for your interest in TetraMem Inc and the US 2026 Software - Machine \
                 Learning Intern position",
                "US 2026 Software - Machine Learning Intern",
            ),
            (
                "Your application for the position of Systems Software Development Engineer \
                 Intern, Annapurna Labs - 2027. Please finish it",
                "Systems Software Development Engineer Intern, Annapurna Labs - 2027",
            ),
            (
                "Please complete the assessment for your application for position 130043 - \
                 QA/Test Engineer Intern-Poughkeepsie, NY",
                "QA/Test Engineer Intern-Poughkeepsie, NY",
            ),
            (
                "Thanks for your application for the Institutional Trader Intern (2027 Start - \
                 Chicago) role here at Optiver.",
                "Institutional Trader Intern (2027 Start - Chicago)",
            ),
        ];
        for (snippet, expected) in cases {
            assert_eq!(role_from(Some("Thank you"), Some(snippet)).as_deref(), Some(expected), "{snippet}");
        }
    }

    /// The new cuts must not eat a role that merely contains their words.
    #[test]
    fn a_role_containing_and_or_it_keeps_them() {
        assert_eq!(
            role_from(None, Some("Your application for Information Systems and IT Intern - Summer 2027.")).as_deref(),
            Some("Information Systems and IT Intern - Summer 2027")
        );
        // "and the" after a marker that is not about interest is part of the role.
        assert_eq!(
            role_from(None, Some("Your application for Research and the Arts Intern.")).as_deref(),
            Some("Research and the Arts Intern")
        );
    }

    /// The broad new markers are safe only because of the "intern" gate. These must stay `None`:
    /// the first names a company, the second a role that does not say it is an internship, and
    /// the third a position at a company rather than a role.
    #[test]
    fn a_broad_marker_still_refuses_what_is_not_an_internship_role() {
        for snippet in [
            "Thank you for your interest in Two Sigma! We have received your application",
            "Thank you for applying for the Full Stack Product (Web + Services) Req ID (200042195).",
            "Thank you for submitting your application for a position at Roblox! Please click",
        ] {
            assert_eq!(role_from(Some("Thank you"), Some(snippet)), None, "{snippet}");
        }
    }

    /// A term is part of the role; a requisition id is not. Tesla's rejection lost "(Fall 2026)"
    /// to a rule that stripped any parenthetical with a digit in it, and with it the exact match
    /// against the tracked title.
    #[test]
    fn a_term_in_brackets_survives_and_a_requisition_id_does_not() {
        assert_eq!(
            role_from(
                None,
                Some("your interest in the Integration Engineer Intern (Fall 2026) position at Tesla.")
            ),
            Some("Integration Engineer Intern (Fall 2026)".to_string())
        );
        assert_eq!(
            role_from(None, Some("your application for Firmware Engineering INTERN (Job number: 200051878).")),
            Some("Firmware Engineering INTERN".to_string())
        );
        assert!(has_requisition_id("Job number: 200051878"));
        assert!(!has_requisition_id("Summer 2027"));
    }

    /// Requisition residue is not part of the role, in either of its live shapes.
    #[test]
    fn requisition_residue_is_trimmed_from_the_end() {
        assert_eq!(
            role_from(None, Some("your application for Data Engineering Intern Req ID (99887766).")),
            Some("Data Engineering Intern".to_string())
        );
        assert_eq!(
            role_from(None, Some("your application for Software Engineering Intern Req #2.")),
            Some("Software Engineering Intern".to_string())
        );
    }

    /// A tracked title carries the posting's requisition parenthetical and the same role read
    /// out of an email does not. The key has to erase that difference or the two never meet —
    /// seven Microsoft rejections whose role matched character for character were reported as
    /// matching no application until it did.
    #[test]
    fn a_requisition_parenthetical_does_not_make_two_keys_of_one_role() {
        let from_posting = role_key(Some(
            "Software Engineer: Data Platform/Analytics Intern Opportunities for University \
             Students, Redmond (Job number: 200042200)",
        ));
        let from_email = role_key(Some(
            "Software Engineer: Data Platform/Analytics Intern Opportunities for University \
             Students, Redmond",
        ));
        assert_eq!(from_posting, from_email);
        assert!(!from_posting.is_empty());
    }

    /// The strip is conditional on an id actually being present, so a role that ends in one of
    /// the label words keeps it.
    #[test]
    fn a_role_that_merely_ends_in_a_label_word_keeps_it() {
        assert_eq!(role_key(Some("Summer Analyst, Job")), "summer analyst job");
        assert_eq!(role_key(Some("Reference Data Engineer")), "reference data engineer");
    }

    /// Microsoft's rejections put the role in the snippet, and eight of them share one subject.
    /// If the role does not come back out, `match_application` falls through to company-only
    /// and every one of them lands on whichever Microsoft application it sees first — which is
    /// what `inbox backfill-status` found on 2026-09-23: three rejections for three different
    /// roles proposed against the same application.
    #[test]
    fn three_roles_in_three_snippets_are_three_different_roles() {
        let snippets = [
            "\u{feff} Hi, Thank you for taking the time to submit your application for Software \
             Engineer: Data Platform/Analytics Intern Opportunities for University Students, \
             Redmond (Job number: 200042200). We",
            "\u{feff} Hi, Thank you for taking the time to submit your application for Software \
             Engineer: Fullstack Product (Web + Services) Intern Opportunities for University \
             Students, (Job number: 200042195). We",
            "\u{feff} Hi, Thank you for taking the time to submit your application for Software \
             Engineer: Cloud &amp; Distributed Backend Intern Opportunities for University Stud",
        ];
        let roles: Vec<Option<String>> = snippets
            .iter()
            .map(|snippet| role_from(Some("Thank you for your application!"), Some(snippet)))
            .collect();

        for (snippet, role) in snippets.iter().zip(&roles) {
            assert!(role.is_some(), "no role extracted from: {snippet}");
        }
        let keys: Vec<String> = roles.iter().map(|r| role_key(r.as_deref())).collect();
        assert!(keys.iter().all(|k| !k.is_empty()), "empty role key in {keys:?}");
        assert_ne!(keys[0], keys[1], "{keys:?}");
        assert_ne!(keys[0], keys[2], "{keys:?}");
        assert_ne!(keys[1], keys[2], "{keys:?}");
    }

    #[test]
    fn the_generic_word_the_corpus_already_refuses_is_not_a_company() {
        // Not hypothetical: `internship` was one of the twelve companies the live mailbox
        // resolved to on 2026-09-04, and company-aliases.json already refuses it by name.
        assert!(!is_proposable_company("internship"));
        assert!(!is_proposable_company("Internship List"));
        assert!(!is_proposable_company("careers"));
    }

    #[test]
    fn an_ats_that_sent_the_mail_is_not_the_employer() {
        // `no-reply@ashbyhq.com` sends for hundreds of companies. If the guesser falls back to
        // the ATS, the proposal would name the wrong employer with total confidence.
        assert!(!is_proposable_company("greenhouse"));
        assert!(!is_proposable_company("ashby"));
        assert!(!is_proposable_company("workday"));
    }

    #[test]
    fn real_employers_from_the_live_mailbox_survive_the_filter() {
        // The eleven genuine companies the probe found, so a future tightening of the
        // stop-list has to notice what it costs.
        for company in [
            "stripe", "adobe", "amazon", "microsoft", "jump trading", "imc trading",
            "epic games", "igs energy", "phonic", "whatnot", "salesforce",
        ] {
            assert!(is_proposable_company(company), "{company} must survive");
        }
    }

    #[test]
    fn parsing_debris_is_not_a_company() {
        assert!(!is_proposable_company(""));
        assert!(!is_proposable_company("x"));
        assert!(!is_proposable_company("2027"));
        assert!(!is_proposable_company("   "));
    }

    #[test]
    fn an_email_can_never_create_an_application_that_already_ended() {
        assert!(creatable_status(ApplicationStatus::Applied));
        assert!(creatable_status(ApplicationStatus::Oa));
        assert!(creatable_status(ApplicationStatus::Interview));
        // Both presuppose a history this row does not have.
        assert!(!creatable_status(ApplicationStatus::Offer));
        assert!(!creatable_status(ApplicationStatus::Rejected));
    }

    #[test]
    fn a_subject_that_names_no_role_yields_no_title() {
        // The common case, and the reason `title` is nullable. Half the confirmations in the
        // live mailbox look exactly like this.
        assert_eq!(title_from_subject(Some("Thank you for applying to Stripe!")), None);
        assert_eq!(title_from_subject(Some("We received your Stripe Application")), None);
        assert_eq!(title_from_subject(Some("Thanks for Applying to Adobe")), None);
        assert_eq!(title_from_subject(None), None);
    }

    #[test]
    fn a_subject_that_does_name_a_role_yields_it() {
        // Real subjects from the live mailbox.
        assert_eq!(
            title_from_subject(Some(
                "Workiva Careers: Application for Summer 2027 Intern - Software Engineering"
            )),
            Some("Summer 2027 Intern - Software Engineering".to_string())
        );
        assert_eq!(
            title_from_subject(Some("Thank you for applying to the Gameplay Programmer Intern role")),
            Some("Gameplay Programmer Intern".to_string())
        );
    }

    #[test]
    fn the_display_name_is_cosmetic_and_the_key_is_not() {
        // Nothing keys off the display name; `company_key` is computed from the raw guess, so
        // neither the corpus lookup nor the title-case fallback can change which collide.
        assert_eq!(title_case("jump trading"), "Jump Trading");
        assert_eq!(company_key("jump trading"), company_key("Jump Trading"));
    }

    // ---- the database half ----

    use sqlx::SqlitePool;

    async fn seed_user(pool: &SqlitePool) -> String {
        sqlx::query("INSERT INTO users (id, email, created_at) VALUES ('u1','a@b.c','2026-09-04')")
            .execute(pool)
            .await
            .expect("user");
        "u1".to_string()
    }

    async fn seed_verdict(pool: &SqlitePool, id: &str) -> String {
        sqlx::query(
            "INSERT INTO email_messages
                (id, user_id, gmail_message_id, gmail_thread_id, from_address, subject,
                 received_at, created_at)
             VALUES (?1, 'u1', ?1, 't', 'no-reply@stripe.com', 'We received your Stripe Application',
                     '2026-09-04T00:00:00+00:00', '2026-09-04T00:00:00+00:00')",
        )
        .bind(id)
        .execute(pool)
        .await
        .expect("message");
        sqlx::query(
            "INSERT INTO email_verdicts
                (id, message_id, category, confidence, classifier, evidence, created_at)
             VALUES (?1, ?1, 'confirmation', 0.8, 'rules', 'marker', '2026-09-04T00:00:00+00:00')",
        )
        .bind(id)
        .execute(pool)
        .await
        .expect("verdict");
        id.to_string()
    }

    /// A stored message and its verdict, with the text the title is read from.
    async fn seed_mail(pool: &SqlitePool, id: &str, snippet: &str) -> String {
        seed_verdict(pool, id).await;
        // From an ATS mailbox that names no employer, so the repair's company re-guess keeps the
        // company each test stores rather than reading one off this fixture's sender.
        sqlx::query(
            "UPDATE email_messages SET subject = 'Thank you', snippet = ?2, from_address = 'no-reply@greenhouse.io'
              WHERE id = ?1",
        )
            .bind(id)
            .bind(snippet)
            .execute(pool)
            .await
            .expect("snippet");
        id.to_string()
    }

    /// A pending proposal as the old parser wrote it: the stored title and its key.
    async fn seed_stored_proposal(pool: &SqlitePool, id: &str, verdict: &str, company: &str, title: &str) {
        sqlx::query(
            "INSERT INTO application_proposals
                 (id, user_id, verdict_id, company_key, company_name, title, role_key,
                  implied_status, created_at)
             VALUES (?1, 'u1', ?2, ?3, ?4, ?5, ?6, 'applied', '2026-11-02T00:00:00+00:00')",
        )
        .bind(id)
        .bind(verdict)
        .bind(company_key(company))
        .bind(company)
        .bind(title)
        .bind(role_key(Some(title)))
        .execute(pool)
        .await
        .expect("proposal");
    }

    async fn seed_application(pool: &SqlitePool, id: &str, company: &str, title: &str) {
        sqlx::query(
            "INSERT INTO internship_applications
                 (id, user_id, company_name, title, url, snapshot_json, snapshot_at, status,
                  applied_at, status_changed_at, created_at, updated_at)
             VALUES (?1, 'u1', ?2, ?3, 'https://example.com', '{}', '2026-10-01', 'applied',
                     '2026-10-01', '2026-10-01', '2026-10-01', '2026-10-01')",
        )
        .bind(id)
        .bind(company)
        .bind(title)
        .execute(pool)
        .await
        .expect("application");
    }

    async fn proposal_titles(pool: &SqlitePool) -> Vec<(String, Option<String>, String)> {
        sqlx::query_as("SELECT id, title, role_key FROM application_proposals ORDER BY id")
            .fetch_all(pool)
            .await
            .expect("titles")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retitle_corrects_a_pending_title_and_the_key_it_is_matched_on(pool: SqlitePool) {
        seed_user(&pool).await;
        let verdict = seed_mail(&pool, "m1", "Your application for the Compiler Engineering Intern - Summer 2027 role has been received.").await;
        seed_stored_proposal(&pool, "p1", &verdict, "NVIDIA", "Compiler Engineering Intern - Summer 2027 role has been received").await;

        let report = retitle(&pool, Utc::now(), true).await.expect("retitle");
        assert_eq!(report.retitled.len(), 1);
        let rows = proposal_titles(&pool).await;
        assert_eq!(rows[0].1.as_deref(), Some("Compiler Engineering Intern - Summer 2027"));
        assert_eq!(rows[0].2, role_key(Some("Compiler Engineering Intern - Summer 2027")), "the key moves with the title");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retitle_removes_a_proposal_its_corrected_role_shows_was_already_tracked(pool: SqlitePool) {
        // The Thunder case: tracked all along, proposed only because the title ran on.
        seed_user(&pool).await;
        seed_application(&pool, "a1", "Oklahoma City Thunder", "Software Engineer Intern / Basketball Operations").await;
        let verdict = seed_mail(&pool, "m1", "Thanks for your application for the Software Engineer Intern / Basketball Operations position with the Oklahoma City Thunder.").await;
        seed_stored_proposal(&pool, "p1", &verdict, "Oklahoma City Thunder", "Software Engineer Intern / Basketball Operations position with the Oklahoma City Thunder").await;

        let report = retitle(&pool, Utc::now(), true).await.expect("retitle");
        assert_eq!(report.already_tracked.len(), 1);
        assert!(proposal_titles(&pool).await.is_empty(), "no question left about an application you track");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retitle_removes_a_proposal_that_becomes_a_duplicate_and_keeps_the_other(pool: SqlitePool) {
        // TetraMem: one email read the role cleanly, another with the employer in front.
        seed_user(&pool).await;
        let clean = seed_mail(&pool, "m1", "Your application for US 2026 Software - Machine Learning Intern.").await;
        seed_stored_proposal(&pool, "p1", &clean, "TetraMem", "US 2026 Software - Machine Learning Intern").await;
        let garbled = seed_mail(&pool, "m2", "Thank you for your interest in TetraMem Inc and the US 2026 Software - Machine Learning Intern position.").await;
        seed_stored_proposal(&pool, "p2", &garbled, "TetraMem", "TetraMem Inc and the US 2026 Software - Machine Learning Intern").await;

        let report = retitle(&pool, Utc::now(), true).await.expect("retitle");
        assert_eq!(report.duplicates.len(), 1);
        let rows = proposal_titles(&pool).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "p1", "the one that was already right stays");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_removed_duplicate_never_takes_the_further_status_with_it(pool: SqlitePool) {
        // IBM: the confirmation proposed `applied`, the assessment reminder `oa`, one role.
        seed_user(&pool).await;
        let confirmation = seed_mail(&pool, "m1", "Your application for QA Engineer Intern - Poughkeepsie - 130043.").await;
        seed_stored_proposal(&pool, "p1", &confirmation, "IBM", "QA Engineer Intern - Poughkeepsie - 130043").await;
        let assessment = seed_mail(&pool, "m2", "Please complete the assessment for your application for position 130043 - QA Engineer Intern - Poughkeepsie.").await;
        seed_stored_proposal(&pool, "p2", &assessment, "IBM", "position 130043 - QA Engineer Intern - Poughkeepsie").await;
        sqlx::query("UPDATE application_proposals SET implied_status = 'oa' WHERE id = 'p2'")
            .execute(&pool)
            .await
            .expect("oa");

        retitle(&pool, Utc::now(), true).await.expect("retitle");
        let rows: Vec<(String, String, String)> =
            sqlx::query_as("SELECT id, implied_status, verdict_id FROM application_proposals")
                .fetch_all(&pool)
                .await
                .expect("rows");
        assert_eq!(rows, [("p1".to_string(), "oa".to_string(), "m2".to_string())], "one proposal, at oa, citing the OA email");

        // And settled: its title is read from the email it now cites, so a second run has
        // nothing left to correct.
        let again = retitle(&pool, Utc::now(), true).await.expect("second run");
        assert!(again.retitled.is_empty() && again.duplicates.is_empty(), "{again:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_proposal_ahead_of_its_tracked_application_is_left_alone(pool: SqlitePool) {
        // Tracked at `applied`; the email says OA. That is news, not a duplicate.
        seed_user(&pool).await;
        seed_application(&pool, "a1", "Workiva", "Summer 2027 Intern - Software Engineering").await;
        let verdict = seed_mail(&pool, "m1", "Your application for Summer 2027 Intern - Software Engineering has been Received").await;
        seed_stored_proposal(&pool, "p1", &verdict, "Workiva", "Summer 2027 Intern - Software Engineering has been Received").await;
        sqlx::query("UPDATE application_proposals SET implied_status = 'oa'")
            .execute(&pool)
            .await
            .expect("oa");

        let report = retitle(&pool, Utc::now(), true).await.expect("retitle");
        assert_eq!(report.ahead_of_tracker.len(), 1);
        assert_eq!(proposal_titles(&pool).await.len(), 1, "not removed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retitle_renames_a_recruiter_to_the_employer_and_merges_what_that_reveals(pool: SqlitePool) {
        // "Aaron Simonds" was the recruiter; the domain says Nutanix, and a Nutanix proposal for
        // the same programme already existed.
        seed_user(&pool).await;
        let snippet = "Thank you for applying to our Summer 2027 Internship Program at Nutanix.";
        let existing = seed_mail(&pool, "m1", snippet).await;
        seed_stored_proposal(&pool, "p1", &existing, "Nutanix", "Summer 2027 Internship Program").await;
        let recruiter = seed_mail(&pool, "m2", snippet).await;
        sqlx::query("UPDATE email_messages SET from_address = 'Aaron Simonds <notification@jobvite.nutanix.com>' WHERE id = 'm2'")
            .execute(&pool)
            .await
            .expect("sender");
        sqlx::query("UPDATE email_messages SET from_address = 'Nutanix Recruiting <notification@jobvite.nutanix.com>' WHERE id = 'm1'")
            .execute(&pool)
            .await
            .expect("sender");
        seed_stored_proposal(&pool, "p2", &recruiter, "Aaron Simonds", "Summer 2027 Internship Program").await;

        let report = retitle(&pool, Utc::now(), true).await.expect("retitle");
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, company_key FROM application_proposals")
            .fetch_all(&pool)
            .await
            .expect("rows");
        assert_eq!(rows, [("p1".to_string(), "nutanix".to_string())], "{report:?}");
        assert_eq!(report.duplicates.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retitle_is_a_dry_run_unless_applied(pool: SqlitePool) {
        seed_user(&pool).await;
        let verdict = seed_mail(&pool, "m1", "Your application for Summer 2027 Intern - Software Engineering has been Received").await;
        seed_stored_proposal(&pool, "p1", &verdict, "Workiva", "Summer 2027 Intern - Software Engineering has been Received").await;
        let before = proposal_titles(&pool).await;

        let report = retitle(&pool, Utc::now(), false).await.expect("dry run");
        assert_eq!(report.retitled.len(), 1, "the dry run reports what it would do");
        assert_eq!(proposal_titles(&pool).await, before, "and writes none of it");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retitle_never_touches_a_reviewed_proposal(pool: SqlitePool) {
        // A reviewed proposal is an answer you gave; its title is part of what you answered.
        seed_user(&pool).await;
        let verdict = seed_mail(&pool, "m1", "Your application for the Compiler Engineering Intern role has been received.").await;
        seed_stored_proposal(&pool, "p1", &verdict, "NVIDIA", "Compiler Engineering Intern role has been received").await;
        sqlx::query("UPDATE application_proposals SET reviewed_at = '2026-11-01', accepted = 0")
            .execute(&pool)
            .await
            .expect("review");
        let before = proposal_titles(&pool).await;

        let report = retitle(&pool, Utc::now(), true).await.expect("retitle");
        assert!(report.retitled.is_empty());
        assert_eq!(proposal_titles(&pool).await, before);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn retitle_fixes_a_garbled_tracked_title_and_leaves_a_posting_title_alone(pool: SqlitePool) {
        seed_user(&pool).await;
        seed_application(&pool, "a1", "Workiva", "Summer 2027 Intern - Software Engineering has been Received").await;
        seed_application(&pool, "a2", "Qualcomm", "position Machine Learning Engineering Internship - Summer 2027 - 3095746").await;
        // From a posting. The parser would cut it at " at " if it were ever re-parsed.
        seed_application(&pool, "a3", "Scale AI", "Software Engineer Intern at Scale - Summer 2027").await;

        let report = retitle(&pool, Utc::now(), true).await.expect("retitle");
        let titles: Vec<(String, String)> =
            sqlx::query_as("SELECT id, title FROM internship_applications ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("titles");
        assert_eq!(
            titles,
            [
                ("a1".to_string(), "Summer 2027 Intern - Software Engineering".to_string()),
                ("a2".to_string(), "Machine Learning Engineering Internship - Summer 2027 - 3095746".to_string()),
                ("a3".to_string(), "Software Engineer Intern at Scale - Summer 2027".to_string()),
            ]
        );
        assert_eq!(report.applications.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn five_confirmations_from_one_company_ask_once(pool: SqlitePool) {
        // The dedup that makes the queue readable. Without it a month of Stripe autoresponders
        // is a month of identical review items.
        let user = seed_user(&pool).await;
        let now = Utc::now();

        for n in 0..5 {
            let verdict = seed_verdict(&pool, &format!("m{n}")).await;
            propose(&pool, &user, &verdict, "stripe", None, ApplicationStatus::Applied, now)
                .await
                .expect("propose");
        }

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM application_proposals")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 1, "one company, one question");
    }

    #[test]
    fn a_trailing_requisition_id_does_not_make_a_second_application() {
        // Tesla sent one opening twice, once with the id appended. Keeping it in the key
        // proposed the same application as two.
        assert_eq!(
            role_key(Some("Integration Engineer, Access Control Systems (Fall 2026), 277192")),
            role_key(Some("Integration Engineer, Access Control Systems (Fall 2026)"))
        );
        // A short number is part of the role, not an id: "Summer 2027" must not be stripped.
        assert_ne!(role_key(Some("SWE Intern 2027")), role_key(Some("SWE Intern")));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn different_roles_at_one_company_are_different_applications(pool: SqlitePool) {
        // The reported bug: nine Microsoft confirmations for nine roles produced one proposal,
        // so the tracker read 14 when it should have read far more.
        let user = seed_user(&pool).await;
        let now = Utc::now();
        for (n, role) in ["SWE Intern: AI/ML", "SWE Intern: Security", "Firmware Intern"]
            .iter()
            .enumerate()
        {
            let verdict = seed_verdict(&pool, &format!("m{n}")).await;
            assert!(
                propose(&pool, &user, &verdict, "microsoft", Some(role), ApplicationStatus::Applied, now)
                    .await
                    .expect("propose")
            );
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM application_proposals")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 3, "three roles, three applications");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_roleless_email_never_duplicates_a_company_already_proposed(pool: SqlitePool) {
        // Half the autoresponders name no job. Keying those as their own application would
        // propose a duplicate for every one — the failure the original per-company key was
        // written to prevent, and still real.
        let user = seed_user(&pool).await;
        let now = Utc::now();
        let first = seed_verdict(&pool, "m1").await;
        propose(&pool, &user, &first, "stripe", Some("SWE Intern"), ApplicationStatus::Applied, now)
            .await
            .expect("propose");

        let second = seed_verdict(&pool, "m2").await;
        assert!(
            !propose(&pool, &user, &second, "stripe", None, ApplicationStatus::Applied, now)
                .await
                .expect("propose"),
            "a roleless email is not evidence of a second application"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn learning_the_role_upgrades_the_proposal_rather_than_adding_one(pool: SqlitePool) {
        // Two emails about one opening, one naming the job. Seen on the first real run for
        // Adobe, Datadog, Two Sigma and Vercel.
        let user = seed_user(&pool).await;
        let now = Utc::now();
        let first = seed_verdict(&pool, "m1").await;
        propose(&pool, &user, &first, "datadog", None, ApplicationStatus::Applied, now)
            .await
            .expect("propose");

        let second = seed_verdict(&pool, "m2").await;
        assert!(
            propose(&pool, &user, &second, "datadog", Some("SWE Intern (Summer)"), ApplicationStatus::Applied, now)
                .await
                .expect("propose")
        );

        let rows: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT role_key, title FROM application_proposals")
                .fetch_all(&pool)
                .await
                .expect("rows");
        assert_eq!(rows.len(), 1, "still one application, now with its role");
        assert_eq!(rows[0].1.as_deref(), Some("SWE Intern (Summer)"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_rejected_company_is_never_asked_about_again(pool: SqlitePool) {
        // Rejecting means "I did not apply there". The next of five confirmations must not
        // re-ask, which is why `reviewed_at` is deliberately absent from the unique key.
        let user = seed_user(&pool).await;
        let now = Utc::now();

        let first = seed_verdict(&pool, "m1").await;
        assert!(
            propose(&pool, &user, &first, "stripe", None, ApplicationStatus::Applied, now)
                .await
                .expect("propose")
        );

        sqlx::query("UPDATE application_proposals SET reviewed_at = ?1, accepted = 0")
            .bind(now.to_rfc3339())
            .execute(&pool)
            .await
            .expect("reject it");

        let second = seed_verdict(&pool, "m2").await;
        assert!(
            !propose(&pool, &user, &second, "stripe", None, ApplicationStatus::Applied, now)
                .await
                .expect("propose"),
            "a question already answered must not be re-asked"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_accepted_application_is_dated_from_its_mail_not_from_the_click(pool: SqlitePool) {
        // `applied_at` is the cohort key for the monthly breakdown, the left edge of every
        // time-to-first-response measurement, and what the analytics date window filters on.
        // Stamping the moment of acceptance put eleven applications made across five days into
        // one cohort and shortened every response time by however long the row sat unreviewed.
        let user = seed_user(&pool).await;
        let verdict = seed_verdict(&pool, "m1").await;
        propose(&pool, &user, &verdict, "stripe", None, ApplicationStatus::Applied, Utc::now())
            .await
            .expect("propose");
        let id: String = sqlx::query_scalar("SELECT id FROM application_proposals")
            .fetch_one(&pool)
            .await
            .expect("id");

        assert!(matches!(
            decide(&pool, &user, &id, true).await.expect("accept"),
            Decision::Created(_)
        ));

        // `seed_verdict` dates the message 2026-09-04, and the click is happening now.
        let (applied_at, created_at): (String, String) =
            sqlx::query_as("SELECT applied_at, created_at FROM internship_applications")
                .fetch_one(&pool)
                .await
                .expect("row");
        assert!(
            applied_at.starts_with("2026-09-04"),
            "applied_at must come from the email, got {applied_at}"
        );
        assert!(
            !created_at.starts_with("2026-09-04"),
            "created_at is when we recorded it, which is a different fact"
        );

        // And the event agrees, or the fold reports a response before its application.
        let at: String = sqlx::query_scalar("SELECT at FROM application_events")
            .fetch_one(&pool)
            .await
            .expect("event");
        assert!(at.starts_with("2026-09-04"), "event must sit with the application, got {at}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_terminal_status_can_never_create_an_application(pool: SqlitePool) {
        // An offer from a company you never applied to is a classifier error every time.
        let user = seed_user(&pool).await;
        let verdict = seed_verdict(&pool, "m1").await;

        for status in [ApplicationStatus::Offer, ApplicationStatus::Rejected] {
            assert!(
                !propose(&pool, &user, &verdict, "stripe", None, status, Utc::now())
                    .await
                    .expect("propose")
            );
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM application_proposals")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_database_refuses_a_terminal_status_too(pool: SqlitePool) {
        // The CHECK, not just the Rust guard. Two independent statements of the same rule,
        // because the guard is the one a future caller can forget.
        seed_user(&pool).await;
        let verdict = seed_verdict(&pool, "m1").await;
        let result = sqlx::query(
            "INSERT INTO application_proposals
                (id, user_id, verdict_id, company_key, company_name, implied_status, created_at)
             VALUES ('p1','u1',?1,'stripe','Stripe','offer','2026-09-04')",
        )
        .bind(&verdict)
        .execute(&pool)
        .await;
        assert!(result.is_err(), "the CHECK must refuse a terminal status");
    }
}
