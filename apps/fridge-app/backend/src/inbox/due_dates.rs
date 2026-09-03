//! Finding a due date in an email (Phase 11f).
//!
//! # Rule 1 is the whole shape of this
//!
//! Email is untrusted content. Everything here is a **pure function over text**: it fetches no
//! URL, writes to no mailbox, touches no calendar, and — importantly — **never advances an
//! application's status**. A date parsed out of a stranger's marketing copy raises a
//! notification and nothing else. That is the entire blast radius, by construction.
//!
//! # What it has to work with, which is less than you would expect
//!
//! `src/inbox/gmail.rs` fetches `format=metadata` on purpose — *"it is a burner account, but it
//! is still someone's mail"* — so **the body is never transferred at all**. This sees the
//! subject and Gmail's ~200-character snippet, and nothing else. Deadlines usually live further
//! down the body than that, so recall here is low by construction rather than by defect. The
//! design does not change if bodies are ever fetched; only the input does.
//!
//! # Extraction is cue-anchored
//!
//! A bare date in an email is usually **not** a deadline: it is a meeting time, a copyright
//! year, a "posted on", a message id with slashes in it. So a date only counts when a cue word
//! — *due*, *deadline*, *expires*, *by*, *within*, *complete*, *closes* — appears close before
//! it. This trades recall for precision deliberately: a missed deadline costs one notification
//! that would have been useful, while a false one trains you to ignore the channel that exists
//! for the alert you cannot miss.
//!
//! # How a bare date becomes an instant — the decision that can cost an interview
//!
//! Emails write "due September 12", not "due 2026-09-12T23:59:59-07:00". A bare date is
//! therefore resolved to **00:00 UTC of that day — the START of it**, and the alert sweep fires
//! ahead of that.
//!
//! It is deliberately the earliest reading, not the most likely one. The most likely reading is
//! end-of-day in the sender's timezone, which is up to 31 hours later; taking it would mean an
//! alert that arrives after a deadline that had already passed in the reader's timezone. **A
//! day early is a nuisance. A day late is the failure this feature exists to prevent.**
//!
//! The residual risk is stated rather than hidden: for a sender at UTC+13 or beyond, start-of-day
//! local is earlier than start-of-day UTC. That is not a US internship hunt, and the fix if it
//! ever matters is to subtract a fixed slack, not to guess a timezone.

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};

/// A due date found in one message, with the words that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub due_at: DateTime<Utc>,
    /// Verbatim, so a wrong date is arguable rather than merely wrong.
    pub source_text: String,
}

/// Words that make a following date a deadline rather than a mention.
const CUES: &[&str] = &[
    "due", "deadline", "expire", "expires", "expiring", "by", "before", "within", "complete",
    "completed", "submit", "respond", "reply", "closes", "closing", "ends", "no later than",
];

/// How much text may sit between a cue and the date it governs.
///
/// Wide enough for "please complete the assessment by September 12", narrow enough that a cue
/// in one sentence does not reach a date in the next.
const CUE_WINDOW: usize = 48;

const MONTHS: &[(&str, u32)] = &[
    ("january", 1), ("february", 2), ("march", 3), ("april", 4), ("may", 5), ("june", 6),
    ("july", 7), ("august", 8), ("september", 9), ("october", 10), ("november", 11),
    ("december", 12), ("jan", 1), ("feb", 2), ("mar", 3), ("apr", 4), ("jun", 6), ("jul", 7),
    ("aug", 8), ("sep", 9), ("sept", 9), ("oct", 10), ("nov", 11), ("dec", 12),
];

const WEEKDAYS: &[(&str, u32)] = &[
    ("monday", 0), ("tuesday", 1), ("wednesday", 2), ("thursday", 3), ("friday", 4),
    ("saturday", 5), ("sunday", 6),
];

/// The earliest due date this message appears to carry, if any.
///
/// **Earliest, not first.** Two candidates in one message are two readings of the same
/// deadline far more often than they are two deadlines, and the earlier one is the safe one.
pub fn extract(
    subject: Option<&str>,
    snippet: Option<&str>,
    received_at: DateTime<Utc>,
) -> Option<Extracted> {
    let text = [subject.unwrap_or_default(), snippet.unwrap_or_default()].join(" · ");
    let haystack = normalize(&text);

    let mut best: Option<Extracted> = None;
    for candidate in candidates(&haystack, received_at) {
        // A deadline in the past relative to the email that announced it is a
        // misparse, not a deadline.
        if candidate.due_at < received_at {
            continue;
        }
        if best.as_ref().is_none_or(|found| candidate.due_at < found.due_at) {
            best = Some(candidate);
        }
    }
    best
}

/// Lowercased, with HTML entities and punctuation that splits words turned into spaces.
///
/// Gmail snippets arrive HTML-escaped (`&#39;`, `&amp;`), which is how "we&#39;re" ends up in
/// the corpus. Left alone, the escapes glue words together and a cue stops being a word.
fn normalize(text: &str) -> String {
    let replaced = text
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&nbsp;", " ")
        .to_lowercase();
    replaced
        .chars()
        .map(|c| if c == ',' || c == '\n' || c == '\t' || c == '—' { ' ' } else { c })
        .collect()
}

/// Every date-shaped thing in the text that has a cue in front of it.
///
/// Walks word starts rather than using a regex crate: the patterns below are small and each
/// one wants different follow-on tokens, and no new dependency is worth a matcher this size.
fn candidates(haystack: &str, received_at: DateTime<Utc>) -> Vec<Extracted> {
    let mut found = Vec::new();
    let mut position = 0usize;

    for word in haystack.split(' ') {
        // `position` counts BYTES and every split is on an ASCII space, so it always lands on
        // a character boundary even when the snippet carries an em dash or an accent.
        if !word.is_empty() && cued(haystack, position) {
            let tail = &haystack[position..];
            if let Some(extracted) = relative(tail, received_at)
                .or_else(|| month_name(tail, received_at))
                .or_else(|| numeric(tail, received_at))
                .or_else(|| weekday(tail, received_at))
            {
                found.push(extracted);
            }
        }
        position += word.len() + 1;
    }

    found
}

/// Whether a cue word appears within [`CUE_WINDOW`] bytes before `position`.
fn cued(haystack: &str, position: usize) -> bool {
    let mut start = position.saturating_sub(CUE_WINDOW);
    while start < position && !haystack.is_char_boundary(start) {
        start += 1;
    }
    let window = &haystack[start..position];

    // "no later than" is the only multi-word cue, so it is checked as a phrase; the rest must
    // match whole words, or "by" would fire inside "byte" and "ends" inside "recommends".
    window.contains("no later than")
        || window
            .split(|c: char| !c.is_ascii_alphabetic())
            .any(|token| CUES.contains(&token))
}

/// "within 5 days", "in 48 hours".
fn relative(tail: &str, received_at: DateTime<Utc>) -> Option<Extracted> {
    let mut parts = tail.split_whitespace();
    let number: i64 = parts.next()?.parse().ok()?;
    if number <= 0 || number > 365 {
        return None;
    }
    let unit = parts.next()?.trim_end_matches('.');
    let delta = match unit {
        "day" | "days" => Duration::days(number),
        "hour" | "hours" | "hrs" | "hr" => Duration::hours(number),
        "week" | "weeks" => Duration::weeks(number),
        _ => return None,
    };
    Some(Extracted {
        due_at: received_at + delta,
        source_text: format!("{number} {unit}"),
    })
}

/// "september 12", "sep 12 2026", "12 september".
fn month_name(tail: &str, received_at: DateTime<Utc>) -> Option<Extracted> {
    let mut parts = tail.split_whitespace();
    let first = parts.next()?.trim_end_matches('.');
    let second = parts.next()?;

    let (month, day_text) = match MONTHS.iter().find(|(name, _)| *name == first) {
        Some((_, month)) => (*month, second),
        None => {
            let day_first: u32 = first.parse().ok()?;
            let month = MONTHS
                .iter()
                .find(|(name, _)| *name == second.trim_end_matches('.'))
                .map(|(_, month)| *month)?;
            return build(month, day_first, parts.next(), received_at, &format!("{first} {second}"));
        }
    };

    let day: u32 = day_text
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .ok()?;
    build(month, day, parts.next(), received_at, &format!("{first} {day_text}"))
}

/// "2026-09-12", and "9/12/2026" — **month first**, because this is a US internship hunt and
/// every source in `INTERNSHIP_SCRAPING.md` is a US board. A day-first sender is misread by up
/// to 11 days, always in the earlier direction, which is the safe one.
fn numeric(tail: &str, received_at: DateTime<Utc>) -> Option<Extracted> {
    let token = tail.split_whitespace().next()?.trim_end_matches('.');

    if let Some((year, rest)) = token.split_once('-') {
        let (month, day) = rest.split_once('-')?;
        let date = NaiveDate::from_ymd_opt(year.parse().ok()?, month.parse().ok()?, day.parse().ok()?)?;
        return Some(Extracted {
            due_at: start_of_day(date),
            source_text: token.to_string(),
        });
    }

    let (month, rest) = token.split_once('/')?;
    let (day, year) = match rest.split_once('/') {
        Some((day, year)) => (day, Some(year)),
        None => (rest, None),
    };
    build(month.parse().ok()?, day.parse().ok()?, year, received_at, token)
}

/// "by friday" — the next one at or after the email.
fn weekday(tail: &str, received_at: DateTime<Utc>) -> Option<Extracted> {
    let token = tail.split_whitespace().next()?.trim_end_matches('.');
    let (_, target) = WEEKDAYS.iter().find(|(name, _)| *name == token)?;
    let today = received_at.weekday().num_days_from_monday();
    let ahead = (*target + 7 - today) % 7;
    let days = if ahead == 0 { 7 } else { i64::from(ahead) };
    Some(Extracted {
        due_at: start_of_day((received_at + Duration::days(days)).date_naive()),
        source_text: token.to_string(),
    })
}

/// A month and day, with an optional year. Without a year, the next occurrence.
fn build(
    month: u32,
    day: u32,
    year: Option<&str>,
    received_at: DateTime<Utc>,
    source_text: &str,
) -> Option<Extracted> {
    let year = match year.and_then(|raw| raw.trim_end_matches('.').parse::<i32>().ok()) {
        Some(explicit) if (2000..=2100).contains(&explicit) => explicit,
        Some(short) if (0..=99).contains(&short) => 2000 + short,
        _ => {
            let this_year = NaiveDate::from_ymd_opt(received_at.year(), month, day)?;
            if start_of_day(this_year) >= received_at {
                received_at.year()
            } else {
                received_at.year() + 1
            }
        }
    };
    Some(Extracted {
        due_at: start_of_day(NaiveDate::from_ymd_opt(year, month, day)?),
        source_text: source_text.to_string(),
    })
}

/// The START of the day, in UTC. See the module doc: this is the early reading, on purpose.
fn start_of_day(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight exists"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received() -> DateTime<Utc> {
        // A Wednesday, so the weekday cases have an unambiguous "next".
        Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap()
    }

    fn due(subject: &str, snippet: &str) -> Option<DateTime<Utc>> {
        extract(Some(subject), Some(snippet), received()).map(|found| found.due_at)
    }

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    // ---- what it should find ----

    #[test]
    fn a_named_date_after_a_cue() {
        assert_eq!(due("", "please complete the assessment by September 12"), Some(at(2026, 9, 12)));
        assert_eq!(due("", "submit before sep 30 2026"), Some(at(2026, 9, 30)));
        assert_eq!(due("", "due 12 october"), Some(at(2026, 10, 12)));
    }

    #[test]
    fn a_relative_offset_is_anchored_on_the_email() {
        assert_eq!(due("", "complete within 5 days"), Some(received() + Duration::days(5)));
        assert_eq!(due("", "the link expires in 48 hours"), Some(received() + Duration::hours(48)));
    }

    #[test]
    fn an_iso_or_us_numeric_date() {
        assert_eq!(due("", "due 2026-09-12"), Some(at(2026, 9, 12)));
        // Month first: every source in this project is a US board.
        assert_eq!(due("", "deadline 9/12/2026"), Some(at(2026, 9, 12)));
    }

    #[test]
    fn a_weekday_means_the_next_one() {
        // Received on a Wednesday.
        assert_eq!(due("", "please respond by friday"), Some(at(2026, 9, 4)));
        assert_eq!(due("", "reply by wednesday"), Some(at(2026, 9, 9)), "not today");
    }

    #[test]
    fn a_bare_date_without_a_year_rolls_forward_rather_than_backwards() {
        // January is behind September, so it means next January.
        assert_eq!(due("", "due january 15"), Some(at(2027, 1, 15)));
    }

    #[test]
    fn the_earliest_candidate_wins() {
        // Two readings of one deadline far more often than two deadlines.
        assert_eq!(
            due("", "complete by september 20 · interview scheduled by september 12"),
            Some(at(2026, 9, 12))
        );
    }

    #[test]
    fn html_escapes_do_not_hide_a_cue() {
        // Gmail snippets arrive escaped; this is the real shape from the burner corpus.
        assert_eq!(due("", "we&#39;re thrilled &amp; the test is due september 12"), Some(at(2026, 9, 12)));
    }

    // ---- what it must NOT find, which is the half that matters ----

    #[test]
    fn a_date_with_no_cue_is_not_a_deadline() {
        // A meeting time, a "posted on", a copyright year, a signature.
        assert_eq!(due("", "our next town hall is september 12"), None);
        assert_eq!(due("", "copyright 2026 acme corp"), None);
        assert_eq!(due("Interview scheduled", "we met on 9/12/2026"), None);
    }

    #[test]
    fn a_cue_inside_a_longer_word_does_not_count() {
        // "by" in "byte", "ends" in "recommends", "due" in "duel".
        assert_eq!(due("", "the byte 12 september field"), None);
        assert_eq!(due("", "everyone recommends 5 days of preparation"), None);
    }

    #[test]
    fn a_cue_too_far_from_the_date_does_not_reach_it() {
        let far = format!("due {} september 12", "filler ".repeat(12));
        assert_eq!(due("", &far), None);
    }

    #[test]
    fn a_date_before_the_email_is_a_misparse_not_a_deadline() {
        assert_eq!(due("", "due september 1 2026"), None, "the day before it arrived");
    }

    #[test]
    fn nonsense_numbers_are_not_dates() {
        assert_eq!(due("", "due 99/99/2026"), None);
        assert_eq!(due("", "complete within 0 days"), None);
        assert_eq!(due("", "expires in 900 days"), None);
    }

    #[test]
    fn nothing_at_all_is_the_normal_answer() {
        assert_eq!(due("Your application", "thanks for applying, we will be in touch"), None);
        assert_eq!(extract(None, None, received()), None);
    }

    /// The one real pressing email in the burner corpus, verbatim.
    ///
    /// It carries no date, and the honest result is `None`. Pinned so that a future widening of
    /// the patterns has to look this in the face rather than quietly making it match something.
    #[test]
    fn the_real_roblox_assessment_email_yields_nothing() {
        let subject = "[Action Required] Your Roblox Assessments Invitation";
        let snippet = "Roblox Assessments Invitation Hi Jesse, We&#39;re thrilled to invite you \
                       to the next step of the recruiting process — the assessments! Our hiring \
                       assessments are a mix of technical and non-technical";

        assert_eq!(extract(Some(subject), Some(snippet), received()), None);
    }

    #[test]
    fn the_source_text_is_kept_so_a_wrong_date_is_arguable() {
        let found = extract(None, Some("please finish by september 12"), received()).unwrap();
        assert_eq!(found.source_text, "september 12");
    }

}
