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

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::internships::models::ApplicationStatus;
use crate::internships::normalize::company_key;

/// Guesses that are never an employer.
///
/// This is a stop-list and it is deliberately short. The guesser's job is to name a company
/// from a sender and a subject line; when it returns a generic word it has failed, and creating
/// a review item for `internship` trains the reader to dismiss the queue without looking.
///
/// `internship` is not hypothetical — `data/internships/company-aliases.json` already refuses
/// it, in as many words: *"Neither is a company. Both are junk company names that survived
/// QC"*. The rest are the shapes an ATS sender produces when no employer name is in the mail.
///
/// **Compared after [`company_key`]**, so casing and punctuation are already gone.
const NOT_COMPANIES: &[&str] = &[
    "internship",
    "internship list",
    "internships",
    "careers",
    "career",
    "recruiting",
    "recruitment",
    "talent",
    "talent acquisition",
    "hiring",
    "jobs",
    "job",
    "no reply",
    "noreply",
    "do not reply",
    "team",
    "university",
    "greenhouse",
    "workday",
    "ashby",
    "lever",
];

/// Whether a guessed company is worth asking a human about.
///
/// Conservative in the direction that costs least: a real company wrongly filtered here is one
/// application you add by hand, while junk that gets through is a queue nobody reads.
pub fn is_proposable_company(company: &str) -> bool {
    let key = company_key(company);
    if key.len() < 2 {
        return false;
    }
    if NOT_COMPANIES.contains(&key.as_str()) {
        return false;
    }
    // A guess that is all digits, or has no letter in it at all, came from parsing debris.
    key.chars().any(|c| c.is_alphabetic())
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

    let inserted = sqlx::query(
        "INSERT INTO application_proposals
             (id, user_id, verdict_id, company_key, company_name, title,
              implied_status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (user_id, company_key) DO NOTHING",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(verdict_id)
    .bind(company_key(company_guess))
    .bind(display_name_for(pool, company_guess).await?)
    .bind(title_guess)
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
pub fn title_from_subject(subject: Option<&str>) -> Option<String> {
    let subject = subject?;
    let lower = subject.to_lowercase();
    // Only the shapes that genuinely carry a role, and only the text after the marker.
    for marker in [
        "your application for ",
        "application for ",
        "applying to the ",
        "application: ",
    ] {
        if let Some(at) = lower.find(marker) {
            let tail = subject[at + marker.len()..].trim();
            let tail = tail
                .trim_end_matches(['!', '.', '?'])
                .split(" at ")
                .next()
                .unwrap_or(tail)
                .trim();
            let tail = tail
                .strip_prefix("the ")
                .or_else(|| tail.strip_prefix("The "))
                .unwrap_or(tail);
            // The subject's own sentence, not part of the role: "... Intern role",
            // "... Position". Trimmed so the tracker shows a title and not a clause.
            let tail = ["role", "Role", "position", "Position"]
                .iter()
                .fold(tail, |acc, suffix| acc.trim_end().trim_end_matches(suffix))
                .trim();
            if tail.len() >= 4 && tail.len() <= 120 && tail.to_lowercase().contains("intern") {
                return Some(tail.to_string());
            }
        }
    }
    None
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
        let applications: Vec<(String, String)> =
            sqlx::query_as("SELECT id, company_name FROM internship_applications WHERE user_id = ?")
                .bind(user_id)
                .fetch_all(pool)
                .await?;
        if super::advance::match_application(verdict.company_guess.as_deref(), &applications)
            .is_some()
        {
            report.already_tracked += 1;
            continue;
        }

        let Some(company) = verdict.company_guess.as_deref().filter(|c| is_proposable_company(c))
        else {
            report.unusable_company += 1;
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
            continue;
        };

        if propose(
            pool,
            user_id,
            &verdict_id,
            company,
            title_from_subject(subject.as_deref()).as_deref(),
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

const USAGE: &str = "\
usage: inbox backfill-untracked

Re-reads stored mail and proposes the applications it implies. Proposes only; creates nothing.
Idempotent — running it twice proposes nothing the second time.
";

pub async fn main(pool: &SqlitePool, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("backfill-untracked") if args.len() == 1 => {}
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
