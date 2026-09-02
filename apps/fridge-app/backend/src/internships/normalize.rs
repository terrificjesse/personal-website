//! The QC / normalization pass: one [`RawPosting`] in, exactly one [`QcOutcome`] out.
//!
//! Source adapters hand over strings. This module turns them into the typed fields that
//! ranking, dedup and the SQL filters actually read — pay as a comparable quantity, term as a
//! season and a year, location as parts plus a three-state remote flag, eligibility as a range
//! of graduation years — and decides which rows do not belong in the corpus at all.
//!
//! # The two invariants this module exists to hold
//!
//! **1. Absent is not zero.** A posting with no salary produces `pay: None`, never
//! `Some(PayRange { min: 0.0, .. })`. [`Location::is_remote`] stays `None` unless something in
//! the record positively says remote or positively says onsite — `None` is *unknown*, a third
//! state, and never a synonym for `Some(false)`. `posted_at` is never invented: if the source
//! stated no date, or stated one we could not parse, it is `None` and the runner backfills it
//! with `posted_at_is_estimated = 1`.
//!
//! **2. Every rejected row is visible.** [`normalize`] is total: every input returns one of
//! the three [`QcOutcome`] variants and there is no path that returns nothing, so
//! `fetched = accepted + filtered + rejected` holds by construction. The runner writes the
//! non-accepted ones to `posting_rejects` with their `raw_json`, so a reject can be diagnosed
//! rather than merely counted.
//!
//! # `Filtered` vs `Rejected` is load-bearing
//!
//! - [`QcOutcome::Filtered`] — the row was **correctly excluded**. Not an internship, not
//!   software, term long past. Expected in bulk. A source returning thousands of these is
//!   healthy, and the count is not a health signal.
//! - [`QcOutcome::Rejected`] — the row **should have been usable and wasn't**. Every one is a
//!   potential bug in an adapter or in this module. Summed into one number with the filtered
//!   count, that defect is invisible, which is the whole reason the two are separate.
//!
//! Only fields that identify the listing at all can reject it. See
//! [the note on unparseable pay](#why-unparseable-pay-is-not-a-rejection).
//!
//! # Reason codes
//!
//! Stable, machine-readable, and matched by SQL in the run-health panel — not prose. Changing
//! one is a breaking change to `posting_rejects.reason`.
//!
//! ## Rejected — a defect, investigate every one
//!
//! | code | field | meaning |
//! |---|---|---|
//! | [`REASON_MISSING_SOURCE`] | `source` | adapter emitted a row with no source name |
//! | [`REASON_MISSING_EXTERNAL_ID`] | `external_id` | no stable per-source handle, so the row cannot be re-recognized next run |
//! | [`REASON_MISSING_URL`] | `url` | nothing to link to |
//! | [`REASON_INVALID_URL`] | `url` | present but not `http://` or `https://` |
//! | [`REASON_MISSING_COMPANY`] | `company` | blank company |
//! | [`REASON_UNNORMALIZABLE_COMPANY`] | `company` | non-blank but [`company_key`] reduces it to nothing (e.g. `"###"`) |
//! | [`REASON_MISSING_TITLE`] | `title` | blank title — also the input every classifier below reads |
//!
//! ## Filtered — correct exclusion, expected in bulk
//!
//! | code | meaning |
//! |---|---|
//! | [`REASON_NOT_AN_INTERNSHIP`] | title/term carry no intern, co-op or apprenticeship signal |
//! | [`REASON_NOT_SOFTWARE`] | title is not a software role (see [`is_software_role`]) |
//! | [`REASON_WRONG_TERM`] | a stated term year outside [`TERM_YEAR_LOOKBACK`]/[`TERM_YEAR_LOOKAHEAD`] of `now` |
//!
//! ## Reserved and deliberately never emitted
//!
//! [`REASON_UNPARSEABLE_PAY`] exists as a constant because `posting_rejects` documents it as
//! an example, and because naming it is the clearest way to record that **this module does not
//! emit it**.
//!
//! ### Why unparseable pay is not a rejection
//!
//! Pay is not part of the listing's identity. A posting whose compensation string reads
//! `"Competitive"` is a perfectly good posting that we happen to know nothing about the pay
//! of, and it is indistinguishable, to every consumer downstream, from the overwhelming
//! majority of postings that carry no compensation field at all. Rejecting it would discard a
//! real job over a field the ranking already has to treat as unknown — and worse, it would
//! bury a genuine adapter defect (a missing company, a malformed record) under a pile of rows
//! whose only sin is that their employer declined to publish a number. So an unparseable pay
//! string yields `pay: None` with `pay_raw` preserved, which keeps *"we could not parse it"*
//! distinct from *"there was not one"* without costing us the row.
//!
//! # Thresholds
//!
//! Every continuous cutoff here is a named constant, and the tests assert on the boundary
//! values themselves rather than on values comfortably either side. Phase 4 lost every recipe
//! rated exactly 4 stars to a `>` that should have been a `>=`; that is the failure this
//! convention is against.
//!
//! # Out of scope
//!
//! [`company_key`] is a **deterministic** normalizer — lowercase, strip legal suffixes,
//! collapse punctuation. It is not fuzzy matching. Fuzzy company/title matching for dedup is
//! reserved: it is the NLP learning area's shape, and the owner has not decided whether
//! `src/nlp.rs` covers it. Do not add it here.
//!
//! # Class-year parsing is deliberately thin — this is not unfinished work
//!
//! [`parse_class_years`] reads a literal four-digit graduation year or nothing at all. Class
//! words (`"rising senior"`, `"junior"`) produce **no bounds**. That is a decision, not a gap,
//! and `docs/INTERNSHIP_SCRAPING.md` § B is the reason: *"Sponsorship and class-year
//! eligibility are effectively unavailable. Do not design ranking inputs that require them."*
//! Every ATS is `N` for class year in the field-availability matrix; Simplify's `degrees[]` is
//! degree *level*, not graduation year, and is empty on 22% of rows (§ B fn 14). Class year is
//! no longer a ranking input — it survives only as an optional hard filter.
//!
//! An earlier version resolved class words against the academic year containing `now`. It was
//! removed because the two readings of "rising senior" — relative to the collection date, or
//! relative to the internship's own term — differ by a year and are equally defensible. On a
//! field this sparse a coin-flip inference buys almost nothing, and on the rare occasion it
//! fires it shifts a user's hard filter by a year without saying so. **If you are about to add
//! class-word inference back, read § B first.**
//!
//! # Assumption ledger
//!
//! What this module guesses about source formats, and which guesses the research doc has since
//! settled. Kept here rather than in a commit message because the next person to touch a
//! parser needs it.
//!
//! **Settled by `docs/INTERNSHIP_SCRAPING.md`:**
//!
//! - *Lever's `createdAt` is epoch milliseconds* — confirmed, § B fn 6. [`parse_timestamp`]
//!   handles 13-digit epochs.
//! - *A magnitude heuristic is needed for pay* — confirmed and stronger than assumed. Greenhouse
//!   has **no interval field**: hourly and annual share `min_cents` (§ A.1, § B fn 1). See
//!   [`MIN_UNAMBIGUOUS_ANNUAL_USD`] and [`MAX_PLAUSIBLE_HOURLY_USD`].
//!
//!   **Corrected 2026-08-22.** The heuristic was one-sided: it could confirm "annual" above
//!   $50,000 but never infer "hourly" below it, so it returned `None` for every rate Greenhouse
//!   actually publishes for internships. Greenhouse contributed zero pay and no test failed,
//!   because the vendored fixture holds only full-time salaries (135k–295k) — all above the
//!   threshold. An hourly band now exists; the ambiguous middle is still left alone.
//! - *Ashby is the only source with an unambiguous interval* (§ B fn 7, `"1 YEAR"` / `"1 MONTH"`
//!   / `"NONE"`). An explicit period therefore always beats the magnitude heuristic, and that
//!   ordering is pinned by test rather than left to reading order.
//! - *Class-year data barely exists* — § B. Resolved by deleting the inference, above.
//!
//! **Still guesses — no source of truth found:**
//!
//! - **What an adapter actually puts in [`RawPosting::pay_raw`] is unspecified anywhere.** The
//!   doc describes source *JSON fields*, not the string an adapter stringifies them into. Every
//!   pay format below is inferred from how humans write compensation, not from observed adapter
//!   output. This is the largest remaining risk in this module.
//! - **No currency marker yields `None`.** `"45-55 hourly"` does not parse, because [`PayRange`]
//!   has nowhere to record that USD was assumed. Conservative; it will drop real pay from
//!   US-centric sources that omit the `$`.
//! - **A bare city leaves `is_remote` as `None`, not `Some(false)`.** A one-constant flip if the
//!   owner would rather a named office count as onsite evidence.
//! - **Term is read from the title and term field only, never the description** — descriptions
//!   carry stray years.
//! - **`"Engineering Intern"` with no discipline named counts as software.** Over-filtering
//!   loses a job invisibly; over-accepting is visible noise.
//! - **Multi-location strings use their first segment**, and an unrecognized second comma-part
//!   becomes a *region*, not a country.
//! - **`03/04/2027` is refused, not parsed.** § B gives no basis to choose day-first over
//!   month-first, and a silent 50% error rate on deadlines is worse than an unknown one.

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Utc};

use crate::internships::models::{
    ClassYearRange, Location, NormalizedPosting, PayPeriod, PayRange, QcOutcome, RawPosting, Season,
};

// ------------------------------------------------------------------------------------------
// Reason codes
// ------------------------------------------------------------------------------------------

/// The adapter emitted a row with no source name. A runner bug, not a data problem.
pub const REASON_MISSING_SOURCE: &str = "missing_source";
/// No per-source identifier, so this listing cannot be recognized as the same one next run.
pub const REASON_MISSING_EXTERNAL_ID: &str = "missing_external_id";
/// No URL at all.
pub const REASON_MISSING_URL: &str = "missing_url";
/// A URL that is not `http://` or `https://` — a relative path or a `javascript:` handler
/// renders as a dead link in the tab, which is worse than not showing the posting.
pub const REASON_INVALID_URL: &str = "invalid_url";
/// Blank company.
pub const REASON_MISSING_COMPANY: &str = "missing_company";
/// Non-blank company that [`company_key`] reduces to nothing, so it can never join
/// `company_signals`.
pub const REASON_UNNORMALIZABLE_COMPANY: &str = "unnormalizable_company";
/// Blank title. Also the input every classifier reads, which is why identity checks run first.
pub const REASON_MISSING_TITLE: &str = "missing_title";

/// Not an intern, co-op or apprenticeship posting.
pub const REASON_NOT_AN_INTERNSHIP: &str = "not_an_internship";
/// Not a software role.
pub const REASON_NOT_SOFTWARE: &str = "not_software";
/// The posting states a term year we are not collecting for.
pub const REASON_WRONG_TERM: &str = "wrong_term";

/// **Never emitted by this module.** See
/// [the module doc](self#why-unparseable-pay-is-not-a-rejection).
pub const REASON_UNPARSEABLE_PAY: &str = "unparseable_pay";

// ------------------------------------------------------------------------------------------
// Named thresholds
// ------------------------------------------------------------------------------------------

/// A pay figure at or below this is not treated as pay.
///
/// Zero is the value at stake, and it is *not* a bargain-priced internship: sources use `0`
/// as a placeholder for "not disclosed" far more often than a real SWE internship pays
/// nothing. Reading that placeholder as "pays nothing" is exactly the absent-is-zero failure
/// this phase exists to prevent, and the cost of the strict reading is only that a genuinely
/// unpaid posting shows as unknown pay with its `pay_raw` intact.
pub const MIN_MEANINGFUL_PAY: f64 = 0.0;

/// The point above which a bare amount with no stated period can only be annual.
///
/// [`PayRange`] cannot be half-constructed, so an amount whose period we cannot discern is not
/// a pay range at all. Magnitude is the one exception, and `docs/INTERNSHIP_SCRAPING.md` § A.1
/// says a heuristic here is unavoidable rather than optional: Greenhouse's `min_cents` /
/// `max_cents` **carry no interval field at all**, so hourly and annual arrive in the same
/// field with nothing distinguishing them (32 ranges under $1,000 — necessarily hourly —
/// against 1,533 annual, across the sampled boards).
///
/// The doc is equally blunt that the heuristic "is least reliable exactly where internships
/// live, since a monthly intern stipend and a low annual salary can land in the same band."
/// So this threshold is set to clear the monthly band outright rather than to split it: the
/// largest monthly intern figure the research records is Ramp's `{"interval":"1 MONTH",
/// "minValue":11700}` (§ A.1, Ashby), and $50,000 is four times that.
///
/// **Amended 2026-08-22.** This used to say "nothing below this line is guessed — it returns
/// `None`". That was the defect: returning `None` below the line discarded every hourly and
/// monthly figure from the one source that most needs the heuristic. Below this threshold the
/// band is now split by [`MAX_PLAUSIBLE_HOURLY_USD`] instead of abandoned. `pay_raw` is still
/// retained either way, so the doc's "decide late" instruction still holds — the raw string
/// remains the record.
///
/// **An explicit period always wins over this.** Ashby is the only source with an unambiguous
/// interval (§ B fn 7), and [`detect_period`] runs first precisely so that precision is never
/// diluted to accommodate the sources that lack it.
pub const MIN_UNAMBIGUOUS_ANNUAL_USD: f64 = 50_000.0;

/// The top of the band a bare amount is read as an **hourly** rate.
///
/// Added 2026-08-22 to fix a defect that made Greenhouse pay structurally undiscoverable.
/// Greenhouse has no interval field, so its adapter emits `"USD 45.00"` for a $45.00/hr
/// internship — and this function used to return `None` for *every* bare amount below
/// [`MIN_UNAMBIGUOUS_ANNUAL_USD`]. The heuristic could confirm "annual" and could never infer
/// "hourly", so it discarded every internship rate Greenhouse publishes while accepting the
/// full-time salaries that QC then filters out as not-internships. Net effect: the largest
/// single source of boards contributed exactly zero pay, and nothing failed.
///
/// $200 sits in a genuinely empty band. Intern hourly rates run roughly $15–$120, with the
/// highest quant rates near $150; monthly stipends start around $2,000 and the largest the
/// research records is Ramp's $11,700 (§ A.1). So nothing plausible lives between $200 and
/// $2,000, and the boundary is placed at the hourly end of that gap deliberately: reading a
/// monthly figure as hourly would multiply a posting's apparent pay ~170x and float it to the
/// top of every pay-sorted list, whereas the opposite error merely buries it. A sub-$200
/// monthly stipend — which would be needed to trigger the loud error — is not a real thing.
///
/// **Only the hourly band was added.** The range between this constant and
/// [`MIN_UNAMBIGUOUS_ANNUAL_USD`] still yields `None`: it is the band the research doc names
/// as least reliable, and the likelier period flips inside it — $11,700 reads as a monthly
/// stipend, $45,000 as a low annual salary. One guess cannot be right at both ends, so
/// nothing is guessed there. Monthly stipends therefore remain unparsed when a source omits
/// the interval; Ashby and Lever both state theirs, and Greenhouse internships are paid
/// hourly, so the residue is small and visible in `pay_raw`.
///
/// **This is still a guess**, and it is confined to bare amounts. An explicit period always
/// wins (`detect_period` runs first), so Ashby's and Lever's stated intervals are untouched.
pub const MAX_PLAUSIBLE_HOURLY_USD: f64 = 200.0;

/// How many years before `now` a stated term year may fall before the posting is stale.
/// Boundary: with `now` in 2026, a `Summer 2025` posting is kept and `Summer 2024` is filtered.
pub const TERM_YEAR_LOOKBACK: i64 = 1;
/// How many years after `now` a stated term year may fall. Postings for two cycles out are
/// normal in the autumn; three is the outer edge of plausible.
pub const TERM_YEAR_LOOKAHEAD: i64 = 3;

/// How many years before `now` a literal graduation year is believable. Its only job is to
/// keep a stray four-digit number (a requisition id, a founding year) out of the range.
pub const CLASS_YEAR_LOOKBACK: i64 = 1;
/// How many years after `now`. Six reaches a first-year student named by a posting today.
pub const CLASS_YEAR_LOOKAHEAD: i64 = 6;

/// A term or graduation year is written with exactly four digits. `45` in `"$45/hr"` is not a
/// year, and neither is a two-digit month.
const YEAR_DIGIT_COUNT: usize = 4;

// ------------------------------------------------------------------------------------------
// Vocabulary
// ------------------------------------------------------------------------------------------

/// Legal-form suffixes stripped from the tail of a company name by [`company_key`].
const LEGAL_SUFFIXES: &[&str] = &[
    "inc",
    "incorporated",
    "llc",
    "l l c",
    "ltd",
    "limited",
    "corp",
    "corporation",
    "co",
    "company",
    "gmbh",
    "plc",
    "lp",
    "llp",
    "ag",
    "sa",
    "sas",
    "bv",
    "nv",
    "ab",
    "oy",
    "as",
    "pty",
    "pte",
    "spa",
    "srl",
    "kk",
];

/// Phrases that mark a posting as an internship. Matched against the space-padded
/// [`phrase`] form, so `"internal tools"` never matches `" intern "`.
const INTERNSHIP_PHRASES: &[&str] = &[
    " intern ",
    " interns ",
    " internship ",
    " internships ",
    " interning ",
    " co op ",
    " coop ",
    " apprentice ",
    " apprenticeship ",
    " industrial placement ",
    " placement student ",
    " trainee ",
];

/// Unambiguous software signals. Any one of these is enough on its own.
const SOFTWARE_PHRASES: &[&str] = &[
    " software ",
    " swe ",
    " sde ",
    " developer ",
    " programmer ",
    " programming ",
    " computer science ",
    " frontend ",
    " front end ",
    " backend ",
    " back end ",
    " fullstack ",
    " full stack ",
    " web development ",
    " web developer ",
    " devops ",
    " site reliability ",
    " sre ",
    " machine learning ",
    " compiler ",
    " distributed systems ",
    " ios ",
    " android ",
    " cybersecurity ",
    " security engineering ",
    " platform engineering ",
    " infrastructure engineering ",
];

/// Weak signals: an engineering role that is software *unless* a non-software discipline is
/// named. `"Engineering Intern"` at a tech company is far more often software than not, and
/// over-filtering loses a real posting where over-accepting only adds noise the user can see.
const ENGINEERING_PHRASES: &[&str] = &[" engineer ", " engineers ", " engineering "];

/// Disciplines that disqualify the weak [`ENGINEERING_PHRASES`] branch.
const NON_SOFTWARE_DISCIPLINES: &[&str] = &[
    " mechanical ",
    " chemical ",
    " civil ",
    " electrical ",
    " industrial ",
    " aerospace ",
    " aeronautical ",
    " biomedical ",
    " petroleum ",
    " structural ",
    " environmental ",
    " manufacturing ",
    " materials ",
    " nuclear ",
    " sales ",
    " field ",
    " hardware ",
    " packaging ",
    " mining ",
    " geotechnical ",
    " automotive ",
    " marine ",
    " agricultural ",
];

/// Tokens naming an hourly period.
const HOUR_TOKENS: &[&str] = &["hour", "hours", "hourly", "hr", "hrs", "h"];
/// Tokens naming a monthly period.
const MONTH_TOKENS: &[&str] = &["month", "months", "monthly", "mo", "mos", "mth", "mnth"];
/// Tokens naming an annual period.
const YEAR_TOKENS: &[&str] = &[
    "year", "years", "yearly", "annual", "annually", "annum", "yr", "yrs",
];
/// Periods [`PayPeriod`] cannot express. Finding one of these first must yield `None`, never a
/// silent promotion to the nearest representable period — `"$2,000/week"` read as monthly is
/// off by a factor of four and looks entirely plausible.
const UNSUPPORTED_PERIOD_TOKENS: &[&str] = &[
    "week",
    "weeks",
    "weekly",
    "wk",
    "wks",
    "biweekly",
    "fortnightly",
    "day",
    "days",
    "daily",
    "semester",
    "quarter",
    "sprint",
];

/// ISO 4217 codes we recognize when spelled out. Lowercase, because matching happens on the
/// [`phrase`] form.
const ISO_CURRENCY_CODES: &[&str] = &[
    "usd", "cad", "gbp", "eur", "inr", "aud", "sgd", "jpy", "chf", "sek", "nzd", "mxn", "brl",
    "pln", "ils", "hkd", "cny", "krw", "zar", "dkk", "nok",
];

/// Words that say the role is remote, onsite, or hybrid. Stripped from a location string
/// before its parts are read, so `"Hybrid — Seattle, WA"` still yields Seattle.
const MODALITY_WORDS: &[&str] = &[
    "remote",
    "remotely",
    "hybrid",
    "onsite",
    "on-site",
    "in-office",
    "wfh",
    "virtual",
    "distributed",
];

/// US state and territory codes, used to tell `"San Francisco, CA"` (city, region) from
/// `"London, United Kingdom"` (city, country).
const US_STATE_CODES: &[&str] = &[
    "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DE", "FL", "GA", "HI", "ID", "IL", "IN", "IA", "KS",
    "KY", "LA", "ME", "MD", "MA", "MI", "MN", "MS", "MO", "MT", "NE", "NV", "NH", "NJ", "NM", "NY",
    "NC", "ND", "OH", "OK", "OR", "PA", "RI", "SC", "SD", "TN", "TX", "UT", "VT", "VA", "WA", "WV",
    "WI", "WY", "DC", "PR", "VI", "GU",
];

/// `(spelling as it appears after [`phrase`], canonical name)`. Deliberately small: an unknown
/// second comma-part is read as a *region*, not a country, so a missing entry here understates
/// what we know instead of asserting something false.
const COUNTRY_ALIASES: &[(&str, &str)] = &[
    ("us", "United States"),
    ("usa", "United States"),
    ("united states", "United States"),
    ("united states of america", "United States"),
    ("america", "United States"),
    ("uk", "United Kingdom"),
    ("gb", "United Kingdom"),
    ("great britain", "United Kingdom"),
    ("england", "United Kingdom"),
    ("scotland", "United Kingdom"),
    ("wales", "United Kingdom"),
    ("united kingdom", "United Kingdom"),
    ("canada", "Canada"),
    ("india", "India"),
    ("ireland", "Ireland"),
    ("germany", "Germany"),
    ("deutschland", "Germany"),
    ("france", "France"),
    ("spain", "Spain"),
    ("italy", "Italy"),
    ("netherlands", "Netherlands"),
    ("poland", "Poland"),
    ("sweden", "Sweden"),
    ("norway", "Norway"),
    ("denmark", "Denmark"),
    ("finland", "Finland"),
    ("switzerland", "Switzerland"),
    ("austria", "Austria"),
    ("portugal", "Portugal"),
    ("israel", "Israel"),
    ("japan", "Japan"),
    ("china", "China"),
    ("singapore", "Singapore"),
    ("australia", "Australia"),
    ("new zealand", "New Zealand"),
    ("brazil", "Brazil"),
    ("mexico", "Mexico"),
    ("south korea", "South Korea"),
    ("taiwan", "Taiwan"),
    ("south africa", "South Africa"),
    ("romania", "Romania"),
    ("czech republic", "Czech Republic"),
    ("hungary", "Hungary"),
];

/// Cues that make a description sentence worth reading for graduation years.
///
/// Deliberately excludes bare `"senior"` and `"junior"`: `"Senior Software Engineer"` appears
/// in plenty of descriptions and would inject a class-year restriction that the posting never
/// stated. A false restriction hides eligible postings, which is strictly worse than no
/// restriction — [`ClassYearRange::admits`] already reads an unstated restriction as
/// admitting everyone.
const CLASS_YEAR_CUES: &[&str] = &["class of", "graduat", "rising", "expected degree"];

// ------------------------------------------------------------------------------------------
// The entry point
// ------------------------------------------------------------------------------------------

/// A parsed hiring term. Both halves are independently optional: `"Summer Internship"` states
/// a season and no year, `"2027 Internship"` the reverse.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Term {
    pub season: Option<Season>,
    pub year: Option<i64>,
}

/// Normalize one raw posting into exactly one outcome.
///
/// `now` is a parameter rather than an internal `Utc::now()` so that the term window and the
/// class-year arithmetic are deterministic under test.
///
/// Order is deliberate: **identity checks reject first, classification filters second.** Every
/// classifier below reads the title, so a row with no title cannot be classified at all — it
/// would be filtered as "not an internship" on the strength of an empty string, and an adapter
/// defect would be recorded as a healthy exclusion.
pub fn normalize(raw: &RawPosting, now: DateTime<Utc>) -> QcOutcome {
    // --- identity: the only fields whose absence may reject the row ---
    let Some(source) = non_empty(&raw.source) else {
        return reject(REASON_MISSING_SOURCE, "source", None);
    };
    let Some(external_id) = non_empty(&raw.external_id) else {
        return reject(REASON_MISSING_EXTERNAL_ID, "external_id", None);
    };
    let Some(url) = non_empty(&raw.url) else {
        return reject(REASON_MISSING_URL, "url", None);
    };
    if !is_http_url(&url) {
        return reject(REASON_INVALID_URL, "url", Some(url));
    }
    let Some(company_name) = non_empty(&raw.company) else {
        return reject(REASON_MISSING_COMPANY, "company", None);
    };
    let company_key = company_key(&company_name);
    if company_key.is_empty() {
        return reject(REASON_UNNORMALIZABLE_COMPANY, "company", Some(company_name));
    }
    let Some(title) = non_empty(&raw.title) else {
        return reject(REASON_MISSING_TITLE, "title", None);
    };

    // --- classification: correct exclusions, expected in bulk ---
    //
    // The internship signal is read from the title and the term field together, because some
    // sources put "Internship" in an employment-type field rather than the title. The software
    // signal is read from the title alone: descriptions mention the word "software" at
    // companies that are not hiring software interns.
    let term_raw = non_empty_opt(raw.term_raw.as_deref());
    let internship_text = match &term_raw {
        Some(term) => format!("{title} {term}"),
        None => title.clone(),
    };
    if !is_internship_role(&internship_text) {
        return filter(REASON_NOT_AN_INTERNSHIP, Some(title));
    }
    if !is_software_role(&title) {
        return filter(REASON_NOT_SOFTWARE, Some(title));
    }

    // --- term ---
    let mut term = parse_term(&title, now);
    if let Some(text) = &term_raw {
        let from_field = parse_term(text, now);
        // The dedicated field wins where it says anything; the title fills the rest in.
        term = Term {
            season: from_field.season.or(term.season),
            year: from_field.year.or(term.year),
        };
    }
    if let Some(year) = term.year {
        let (earliest, latest) = plausible_term_years(now);
        if year < earliest || year > latest {
            return filter(
                REASON_WRONG_TERM,
                Some(format!("term year {year} outside {earliest}..={latest}")),
            );
        }
    }

    // --- everything below here may be unknown without costing the row ---
    let mut location = parse_location(raw.location_raw.as_deref(), raw.remote_hint);
    // A one-way inference only. A title saying "(Remote)" is positive evidence of remoteness;
    // a title that says nothing is not evidence of onsite, so this never writes `Some(false)`.
    if location.is_remote.is_none() && phrase(&title).contains(" remote ") {
        location.is_remote = Some(true);
    }

    let pay_raw = non_empty_opt(raw.pay_raw.as_deref());
    let pay = pay_raw.as_deref().and_then(parse_pay);

    let class_text = non_empty_opt(raw.class_year_raw.as_deref())
        .or_else(|| raw.description.as_deref().and_then(class_year_context));
    let class_years = match class_text {
        Some(text) => parse_class_years(&text, now),
        None => ClassYearRange::default(),
    };

    // Never invented. An unparseable date is `None` and the runner backfills it with
    // `posted_at_is_estimated = 1`, which keeps a date we guessed distinct from one the
    // source stated.
    let posted_at = raw.posted_at_raw.as_deref().and_then(parse_timestamp);
    let deadline = raw.deadline_raw.as_deref().and_then(parse_timestamp);

    QcOutcome::Accepted(Box::new(NormalizedPosting {
        source,
        external_id,
        url,
        company_name,
        company_key,
        title,
        term_season: term.season,
        term_year: term.year,
        location,
        pay,
        pay_raw,
        class_years,
        posted_at,
        deadline,
        raw_json: raw.raw_json.clone(),
    }))
}

fn reject(reason: &str, field: &str, detail: Option<String>) -> QcOutcome {
    QcOutcome::Rejected {
        reason: reason.to_string(),
        field: Some(field.to_string()),
        detail,
    }
}

fn filter(reason: &str, detail: Option<String>) -> QcOutcome {
    QcOutcome::Filtered {
        reason: reason.to_string(),
        detail,
    }
}

// ------------------------------------------------------------------------------------------
// Classification
// ------------------------------------------------------------------------------------------

/// Whether the text names an internship, co-op or apprenticeship.
pub fn is_internship_role(text: &str) -> bool {
    let padded = phrase(text);
    INTERNSHIP_PHRASES.iter().any(|p| padded.contains(p))
}

/// Whether the text names a software role.
///
/// Two branches: an unambiguous software word, or a bare engineering word with no competing
/// discipline named. The second branch deliberately errs toward inclusion — a wrongly filtered
/// posting is a real job the user never sees, while a wrongly accepted one is visible noise
/// they can judge for themselves.
pub fn is_software_role(text: &str) -> bool {
    let padded = phrase(text);
    if SOFTWARE_PHRASES.iter().any(|p| padded.contains(p)) {
        return true;
    }
    let engineering = ENGINEERING_PHRASES.iter().any(|p| padded.contains(p));
    let other_discipline = NON_SOFTWARE_DISCIPLINES.iter().any(|p| padded.contains(p));
    engineering && !other_discipline
}

// ------------------------------------------------------------------------------------------
// Company key
// ------------------------------------------------------------------------------------------

/// Deterministic company-name normalization for `NormalizedPosting::company_key`.
///
/// Lowercases, replaces every non-alphanumeric character with a space, collapses runs of
/// whitespace, and strips legal-form suffixes from the **tail only** — so `"Google LLC"`,
/// `"Google, Inc."` and `"google"` all key to `"google"`, while `"Inc Magazine"` keeps its
/// leading `"inc"`.
///
/// A name made entirely of suffixes (`"Inc."`) keeps them rather than reducing to nothing:
/// an empty key can never join `company_signals`, so the caller rejects on it and stripping
/// into emptiness would turn a usable-if-odd name into a lost row.
///
/// This is **not** fuzzy matching, and must not become it. `"Google"` and `"Google Cloud"` key
/// differently here, on purpose.
/// **Expects text that has already been through [`non_empty`]**, which is where HTML
/// character references are decoded (audit finding F3). Passing raw source text here yields a
/// key built from the literal escape — `"Ben &amp; Jerry&#39;s"` becomes `"ben amp jerry 39 s"`.
///
/// Decoding here as well would mean decoding twice, turning `&amp;lt;` into `<`; the single
/// pass is the point. `normalize` is the only production caller and it satisfies this.
pub fn company_key(company: &str) -> String {
    let mut tokens: Vec<String> = phrase(company)
        .split_whitespace()
        .map(str::to_string)
        .collect();

    while tokens.len() > 1 {
        let last = tokens[tokens.len() - 1].as_str();
        if LEGAL_SUFFIXES.contains(&last) {
            tokens.pop();
        } else {
            break;
        }
    }

    tokens.join(" ")
}

// ------------------------------------------------------------------------------------------
// Pay
// ------------------------------------------------------------------------------------------

/// What the period scan found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeriodSignal {
    Known(PayPeriod),
    /// A real period that [`PayPeriod`] cannot express — weekly, daily, per-semester.
    Unsupported,
    Absent,
}

/// Parse a compensation string into a comparable quantity, or `None`.
///
/// `None` is the common and correct answer. It is returned when:
///
/// - there is no number (`"Competitive"`, `"DOE"`, `""`);
/// - there is no currency marker, because [`PayRange`] carries a currency and inventing `USD`
///   would silently mis-scale every non-US posting;
/// - the period is weekly, daily or per-semester, which [`PayPeriod`] cannot express;
/// - the period is unstated and the amount is below [`MIN_UNAMBIGUOUS_ANNUAL_USD`];
/// - the amount is at or below [`MIN_MEANINGFUL_PAY`];
/// - the range runs backwards, which is a parse bug rather than data.
///
/// In every one of those cases the caller keeps the posting and preserves `pay_raw`.
pub fn parse_pay(text: &str) -> Option<PayRange> {
    let lower = text.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();

    let currency = detect_currency(&lower)?;

    let start = first_amount_index(&chars);
    // `first_amount_index` returns 0 when no currency symbol precedes the amount, so it is
    // not necessarily the digit's index — `read_number` scans forward itself. The sign check
    // needs the actual digit position, or `"-20/hr"` looks at index 0 and finds nothing.
    let digit_start = chars[start..]
        .iter()
        .position(|c| c.is_ascii_digit())
        .map(|offset| start + offset)?;
    if is_negated(&chars, digit_start) {
        // Audit finding F9. The sign used to be skipped along with the currency symbol, so
        // `"$-20/hr"` parsed as a cheerful $20/hr. A negative wage is not a wage — it is a
        // malformed field — and inventing a positive number from it is worse than declining
        // to read it, because the invented figure feeds the highest-weighted ranking input.
        // The raw text is still kept in `pay_raw`.
        return None;
    }
    let (min, after_min) = read_number(&chars, start)?;
    if min <= MIN_MEANINGFUL_PAY {
        return None;
    }

    let max = read_range_max(&chars, after_min);
    if max.is_some_and(|value| value < min) {
        return None;
    }

    let period = match detect_period(&lower) {
        PeriodSignal::Known(period) => period,
        PeriodSignal::Unsupported => return None,
        PeriodSignal::Absent => {
            // Only the top of the range can settle it: "$40,000 - $80,000" is annual on the
            // strength of the 80, and keying on the bottom would throw the pair away.
            let deciding = max.unwrap_or(min);
            if deciding >= MIN_UNAMBIGUOUS_ANNUAL_USD {
                PayPeriod::Year
            } else if deciding <= MAX_PLAUSIBLE_HOURLY_USD {
                PayPeriod::Hour
            } else {
                // Still `None` between the two bands, and deliberately so. This is the range
                // the research doc calls least reliable — "a monthly intern stipend and a low
                // annual salary can land in the same band" — and it flips partway through:
                // $11,700 is a plausible monthly stipend and an implausible salary, while
                // $45,000 is the reverse. Guessing one period for the whole range would be
                // right at one end and badly wrong at the other, so the amount is kept in
                // `pay_raw` and no period is invented.
                return None;
            }
        }
    };

    // Audit finding F10. Applied after the period is resolved, because "1,000,000" is
    // nonsense per hour and unremarkable per year — the ceiling only means anything once the
    // unit is known. Checked against the top of the range, since that is the largest claim.
    if exceeds_credible_pay(max.unwrap_or(min), period, &currency) {
        return None;
    }

    Some(PayRange {
        min,
        max,
        currency,
        period,
    })
}

/// Whether a `-` immediately precedes the amount, allowing one currency symbol between.
///
/// Deliberately does **not** skip whitespace: `"$45 - 55"` is a range whose separator must not
/// be read as a sign. Only the first amount is inspected — a `-` before the *second* number is
/// the range separator by construction, and `read_range_max` owns it.
fn is_negated(chars: &[char], start: usize) -> bool {
    let mut i = start;
    // Step back over the digits' immediate neighbour, and one currency symbol if present.
    for _ in 0..2 {
        if i == 0 {
            return false;
        }
        match chars[i - 1] {
            '-' | '\u{2212}' => return true,
            c if is_currency_symbol(c) => i -= 1,
            _ => return false,
        }
    }
    false
}

/// Whether a figure is too large to be a real offer at this period.
///
/// These are **data-error detectors, not judgements about generosity** — set far above any
/// real internship so that only unit confusion trips them. The failure they exist to catch is
/// a raw source value reaching the parser unscaled: `min_cents` of 4,500,000 read as
/// `$4,500,000/hr` rather than `$45,000.00`.
///
/// Note these sit well above [`MAX_PLAUSIBLE_HOURLY_USD`], which is a different question: that
/// one decides whether a *bare* figure is hourly, this one decides whether a figure with a
/// known period is believable at all. A stated `$300/hr` is unusual but real, and must survive.
///
/// Scoped to USD. The thresholds are USD-denominated and a corpus-wide currency does not make
/// them universal — ¥5,000/hour is an ordinary Japanese wage and must not be refused by a
/// dollar ceiling. Every source here quotes USD (§ B), so nothing currently reaches the
/// permissive branch.
pub const MAX_CREDIBLE_HOURLY_USD: f64 = 1_000.0;
pub const MAX_CREDIBLE_MONTHLY_USD: f64 = 100_000.0;
pub const MAX_CREDIBLE_ANNUAL_USD: f64 = 2_000_000.0;

fn exceeds_credible_pay(amount: f64, period: PayPeriod, currency: &str) -> bool {
    if !currency.eq_ignore_ascii_case("USD") {
        return false;
    }
    let ceiling = match period {
        PayPeriod::Hour => MAX_CREDIBLE_HOURLY_USD,
        PayPeriod::Month => MAX_CREDIBLE_MONTHLY_USD,
        PayPeriod::Year => MAX_CREDIBLE_ANNUAL_USD,
    };
    amount > ceiling
}

fn is_currency_symbol(c: char) -> bool {
    matches!(c, '$' | '£' | '€' | '¥' | '₹')
}

/// An explicit ISO code beats a bare symbol, so `"CAD $45/hr"` is not read as US dollars.
fn detect_currency(lower: &str) -> Option<String> {
    let padded = phrase(lower);
    for token in padded.split_whitespace() {
        if ISO_CURRENCY_CODES.contains(&token) {
            return Some(token.to_uppercase());
        }
    }

    // Longest prefix first: "ca$" contains "a$".
    for (marker, code) in [
        ("ca$", "CAD"),
        ("c$", "CAD"),
        ("nz$", "NZD"),
        ("a$", "AUD"),
        ("us$", "USD"),
    ] {
        if lower.contains(marker) {
            return Some(code.to_string());
        }
    }

    for (symbol, code) in [
        ('$', "USD"),
        ('£', "GBP"),
        ('€', "EUR"),
        ('¥', "JPY"),
        ('₹', "INR"),
    ] {
        if lower.contains(symbol) {
            return Some(code.to_string());
        }
    }

    None
}

/// The **first** period word wins, scanning left to right, so `"$45/hour (40 hours/week)"`
/// reads as hourly rather than tripping over the trailing "week".
fn detect_period(lower: &str) -> PeriodSignal {
    let padded = phrase(lower);
    for token in padded.split_whitespace() {
        if HOUR_TOKENS.contains(&token) {
            return PeriodSignal::Known(PayPeriod::Hour);
        }
        if MONTH_TOKENS.contains(&token) {
            return PeriodSignal::Known(PayPeriod::Month);
        }
        if YEAR_TOKENS.contains(&token) {
            return PeriodSignal::Known(PayPeriod::Year);
        }
        if UNSUPPORTED_PERIOD_TOKENS.contains(&token) {
            return PeriodSignal::Unsupported;
        }
    }
    PeriodSignal::Absent
}

/// Where to start reading the pay figure.
///
/// A currency symbol introduces the number that matters, which is how `"40 hours/week at
/// $45/hr"` yields 45 and not 40. With no symbol anywhere, start at the first digit.
fn first_amount_index(chars: &[char]) -> usize {
    for (i, c) in chars.iter().enumerate() {
        if !is_currency_symbol(*c) {
            continue;
        }
        let mut j = i + 1;
        while j < chars.len() && chars[j].is_whitespace() {
            j += 1;
        }
        if j < chars.len() && chars[j].is_ascii_digit() {
            return j;
        }
    }
    0
}

/// Read the first number at or after `from`, returning its value and the index just past it.
///
/// Handles thousands separators (`8,000`), one decimal point (`45.50`), and a `k` suffix
/// (`120k`). A comma or point not followed by a digit ends the number instead of joining it.
fn read_number(chars: &[char], from: usize) -> Option<(f64, usize)> {
    let mut start = from;
    while start < chars.len() && !chars[start].is_ascii_digit() {
        start += 1;
    }
    if start >= chars.len() {
        return None;
    }

    let mut digits = String::new();
    let mut seen_point = false;
    let mut end = start;
    while end < chars.len() {
        let c = chars[end];
        let next_is_digit = chars.get(end + 1).is_some_and(char::is_ascii_digit);
        if c.is_ascii_digit() {
            digits.push(c);
            end += 1;
        } else if c == ',' && next_is_digit {
            end += 1;
        } else if c == '.' && next_is_digit && !seen_point {
            seen_point = true;
            digits.push('.');
            end += 1;
        } else {
            break;
        }
    }

    let mut value: f64 = digits.parse().ok()?;
    let k_suffix =
        chars.get(end) == Some(&'k') && !chars.get(end + 1).is_some_and(|c| c.is_alphanumeric());
    if k_suffix {
        value *= 1000.0;
        end += 1;
    }

    Some((value, end))
}

/// Read the upper bound of a range, if the text between the two numbers actually says "range".
///
/// Requiring a separator is what stops `"$45/hour, 40 hours per week"` from being read as a
/// backwards 45–40 range. Anything other than whitespace, a currency symbol, a dash, or the
/// words "to"/"and"/"up" ends the range.
fn read_range_max(chars: &[char], from: usize) -> Option<f64> {
    let mut i = from;
    let mut saw_separator = false;

    while i < chars.len() {
        let c = chars[i];

        if c.is_ascii_digit() {
            if !saw_separator {
                return None;
            }
            return read_number(chars, i).map(|(value, _)| value);
        }

        if matches!(c, '-' | '\u{2013}' | '\u{2014}' | '~') {
            saw_separator = true;
            i += 1;
            continue;
        }

        if c.is_whitespace() || is_currency_symbol(c) {
            i += 1;
            continue;
        }

        if c.is_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if word == "to" || word == "and" {
                saw_separator = true;
                continue;
            }
            if word == "up" {
                continue;
            }
            return None;
        }

        return None;
    }

    None
}

// ------------------------------------------------------------------------------------------
// Term
// ------------------------------------------------------------------------------------------

/// A word or a run of digits. Terms are read positionally, so adjacency has to survive
/// tokenizing — `"Summer2027"` must produce two tokens, not one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TermToken {
    Word(String),
    /// Value and how many digits it was written with.
    Number(i64, usize),
}

/// Parse a season and a year out of a title or a term field.
///
/// A four-digit number counts as the term year when it is **adjacent to a season word**
/// (`"Summer 2027"`, `"2027 Summer Internship"`) regardless of how far off it is — that is a
/// stated term, and the caller filters it as [`REASON_WRONG_TERM`] if it is stale. Otherwise a
/// loose number is only believed inside the plausible window, so a requisition id like
/// `"Req 2019"` is ignored rather than mistaken for a term and filtered away.
pub fn parse_term(text: &str, now: DateTime<Utc>) -> Term {
    let tokens = term_tokens(text);

    let mut season = None;
    let mut adjacent_year = None;
    for (i, token) in tokens.iter().enumerate() {
        let TermToken::Word(word) = token else {
            continue;
        };
        let Some(found) = season_from_word(word) else {
            continue;
        };
        if season.is_none() {
            season = Some(found);
        }
        if adjacent_year.is_none() {
            adjacent_year = neighbour_year(&tokens, i);
        }
    }

    let year = adjacent_year.or_else(|| {
        let (earliest, latest) = plausible_term_years(now);
        tokens.iter().find_map(|token| match token {
            TermToken::Number(value, YEAR_DIGIT_COUNT)
                if *value >= earliest && *value <= latest =>
            {
                Some(*value)
            }
            _ => None,
        })
    });

    Term { season, year }
}

/// The inclusive term-year window, derived from `now` rather than hardcoded so the collector
/// does not need a yearly edit.
pub fn plausible_term_years(now: DateTime<Utc>) -> (i64, i64) {
    let year = i64::from(now.year());
    (year - TERM_YEAR_LOOKBACK, year + TERM_YEAR_LOOKAHEAD)
}

fn season_from_word(word: &str) -> Option<Season> {
    match word {
        "summer" => Some(Season::Summer),
        "fall" | "autumn" => Some(Season::Fall),
        "winter" => Some(Season::Winter),
        "spring" => Some(Season::Spring),
        _ => None,
    }
}

/// A four-digit number immediately after, or immediately before, the token at `i`.
fn neighbour_year(tokens: &[TermToken], i: usize) -> Option<i64> {
    let four_digit = |token: Option<&TermToken>| match token {
        Some(TermToken::Number(value, YEAR_DIGIT_COUNT)) => Some(*value),
        _ => None,
    };
    four_digit(tokens.get(i + 1))
        .or_else(|| four_digit(i.checked_sub(1).and_then(|p| tokens.get(p))))
}

fn term_tokens(text: &str) -> Vec<TermToken> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        if chars[i].is_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect::<String>().to_lowercase();
            out.push(TermToken::Word(word));
        } else if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let digits: String = chars[start..i].iter().collect();
            if let Ok(value) = digits.parse::<i64>() {
                out.push(TermToken::Number(value, digits.len()));
            }
        } else {
            i += 1;
        }
    }

    out
}

// ------------------------------------------------------------------------------------------
// Location
// ------------------------------------------------------------------------------------------

/// Parse a location string into parts plus a three-state remote flag.
///
/// `raw` is always preserved verbatim, so a parse that produced nothing is inspectable rather
/// than merely empty.
///
/// **`is_remote` stays `None` unless something says otherwise.** A structured `remote_hint`
/// from the source wins; failing that, a modality word in the string decides — "remote" gives
/// `Some(true)`, "hybrid"/"onsite"/"in-office" give `Some(false)` because all three require
/// physical presence at a named place. **A bare city name yields `None`, not `Some(false)`.**
/// Plenty of postings name an office and are remote-eligible without saying so, and asserting
/// onsite for all of them is precisely the "a remote filter quietly excludes postings that may
/// well be remote" failure that migration `0012` makes the column nullable to avoid.
///
/// Multi-location strings (`"New York, NY; Seattle, WA"`) contribute their **first** segment
/// to the city/region/country fields; the whole string survives in `raw`, and the remote scan
/// runs over all of it so `"San Francisco, CA or Remote"` is still remote.
pub fn parse_location(raw: Option<&str>, remote_hint: Option<bool>) -> Location {
    let Some(text) = non_empty_opt(raw) else {
        return Location {
            raw: None,
            is_remote: remote_hint,
            ..Location::default()
        };
    };

    let is_remote = remote_hint.or_else(|| detect_modality(&text));

    let segment = text.split([';', '|']).next().unwrap_or(&text);
    let cleaned = strip_modality_words(segment);
    let parts: Vec<String> = cleaned
        .split(',')
        .map(clean_part)
        .filter(|part| !part.is_empty())
        .collect();

    let (city, region, country) = match parts.len() {
        0 => (None, None, None),
        1 => match country_alias(&parts[0]) {
            Some(country) => (None, None, Some(country)),
            None => (Some(parts[0].clone()), None, None),
        },
        2 => {
            if is_us_state_code(&parts[1]) {
                (
                    Some(parts[0].clone()),
                    Some(parts[1].to_uppercase()),
                    Some("United States".to_string()),
                )
            } else if let Some(country) = country_alias(&parts[1]) {
                (Some(parts[0].clone()), None, Some(country))
            } else {
                // Not a state we know and not a country we know. "Bengaluru, Karnataka" is
                // city-and-region far more often than city-and-country, and a wrong region is
                // a smaller lie than a wrong country.
                (Some(parts[0].clone()), Some(parts[1].clone()), None)
            }
        }
        _ => {
            let last = parts[parts.len() - 1].clone();
            let country = country_alias(&last).unwrap_or(last);
            let region = if is_us_state_code(&parts[1]) {
                parts[1].to_uppercase()
            } else {
                parts[1].clone()
            };
            (Some(parts[0].clone()), Some(region), Some(country))
        }
    };

    Location {
        raw: Some(collapse_whitespace(&text)),
        city,
        region,
        country,
        is_remote,
    }
}

/// Remote beats hybrid when both appear: a posting advertising both is one you can do
/// remotely.
fn detect_modality(text: &str) -> Option<bool> {
    let padded = phrase(text);
    if padded.contains(" remote ")
        || padded.contains(" remotely ")
        || padded.contains(" wfh ")
        || padded.contains(" work from home ")
    {
        return Some(true);
    }
    if padded.contains(" hybrid ")
        || padded.contains(" onsite ")
        || padded.contains(" on site ")
        || padded.contains(" in office ")
        || padded.contains(" in person ")
    {
        return Some(false);
    }
    None
}

/// Drop modality words and separator-only tokens, keeping everything else spelled as the
/// source spelled it. `"Hybrid — Seattle, WA"` becomes `"Seattle, WA"`.
fn strip_modality_words(segment: &str) -> String {
    segment
        .split_whitespace()
        .filter(|token| {
            let bare: String = token
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-')
                .collect::<String>()
                .to_lowercase();
            let bare = bare.trim_matches('-');
            !bare.is_empty() && !MODALITY_WORDS.contains(&bare)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Trim the punctuation that survives stripping — brackets, dashes, stray colons.
fn clean_part(part: &str) -> String {
    collapse_whitespace(part.trim_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '(' | ')' | '[' | ']' | '{' | '}' | '-' | '\u{2013}' | '\u{2014}' | ':' | '/' | '.'
            )
    }))
}

fn is_us_state_code(part: &str) -> bool {
    let bare: String = part.chars().filter(char::is_ascii_alphabetic).collect();
    bare.len() == 2 && US_STATE_CODES.contains(&bare.to_uppercase().as_str())
}

fn country_alias(part: &str) -> Option<String> {
    let key = phrase(part);
    let key = key.trim();
    COUNTRY_ALIASES
        .iter()
        .find(|(alias, _)| *alias == key)
        .map(|(_, canonical)| (*canonical).to_string())
}

// ------------------------------------------------------------------------------------------
// Class years
// ------------------------------------------------------------------------------------------

/// Parse graduation-year eligibility into the range that filters in SQL.
///
/// **Deliberately thin — do not "finish" it.** `docs/INTERNSHIP_SCRAPING.md` § B: *"Sponsorship
/// and class-year eligibility are effectively unavailable. Do not design ranking inputs that
/// require them."* No source carries a graduation year. Simplify's `degrees[]` is degree
/// *level*, not year, and is empty on 22% of rows (§ B fn 14); every ATS is `N` in the
/// field-availability matrix. Class year is no longer a ranking input at all — it survives
/// only as an optional hard filter, which fires on a literal year or not at all.
///
/// So the only evidence accepted is a **literal four-digit graduation year** inside the
/// plausible window: `"class of 2027"`, `"graduating in 2027"`, `"2026-2027 graduates"`.
/// `min` and `max` are the smallest and largest found, so the range can never run backwards —
/// which is also the CHECK migration `0012` enforces on the way in.
///
/// Class *words* — `"rising senior"`, `"junior"`, `"sophomore"` — produce **no bounds**. An
/// earlier version resolved them against the academic year containing `now`, but "rising
/// senior" anchored to the collection date and "rising senior" anchored to the internship's
/// own term differ by a year and are equally defensible readings. A coin-flip inference buys
/// almost nothing on a field that is near-always absent, and silently shifts a real filter by
/// a year on the rare occasion it fires. Emitting no restriction is the honest answer, and
/// [`ClassYearRange::admits`] already reads an unstated range as admitting everyone.
///
/// `raw` is preserved even when nothing parses, so an eligibility line we failed to read stays
/// inspectable rather than merely absent.
///
/// This function can never reject or filter a posting; see [`normalize`].
pub fn parse_class_years(text: &str, now: DateTime<Utc>) -> ClassYearRange {
    let trimmed = collapse_whitespace(text);
    if trimmed.is_empty() {
        return ClassYearRange::default();
    }

    let padded = phrase(&trimmed);
    let mut years: Vec<i64> = Vec::new();

    let (earliest, latest) = plausible_class_years(now);
    for token in padded.split_whitespace() {
        if token.len() != YEAR_DIGIT_COUNT || !token.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Ok(year) = token.parse::<i64>()
            && year >= earliest
            && year <= latest
        {
            years.push(year);
        }
    }

    years.sort_unstable();
    let min = years.first().copied();
    let max = years.last().copied();

    ClassYearRange {
        min,
        max,
        raw: Some(trimmed),
    }
}

/// The inclusive window of believable graduation years.
pub fn plausible_class_years(now: DateTime<Utc>) -> (i64, i64) {
    let year = i64::from(now.year());
    (year - CLASS_YEAR_LOOKBACK, year + CLASS_YEAR_LOOKAHEAD)
}

/// Pull the sentences of a description that actually talk about graduation, so the year in
/// "founded in 2019" never becomes an eligibility bound.
fn class_year_context(description: &str) -> Option<String> {
    let mut kept: Vec<&str> = Vec::new();
    for segment in description.split(['\n', '.', ';', '!', '?']) {
        let padded = phrase(segment);
        if CLASS_YEAR_CUES.iter().any(|cue| padded.contains(cue)) {
            kept.push(segment.trim());
        }
    }
    if kept.is_empty() {
        return None;
    }
    non_empty_opt(Some(kept.join(" ").as_str()))
}

// ------------------------------------------------------------------------------------------
// Dates
// ------------------------------------------------------------------------------------------

/// Parse a timestamp string, or `None`.
///
/// `None` is never an invented date. The runner backfills a missing `posted_at` from first
/// sighting and marks it estimated, which is what keeps a date we guessed distinguishable from
/// one the source stated.
///
/// Formats: RFC 3339, `YYYY-MM-DDTHH:MM:SS`, `YYYY-MM-DD HH:MM:SS`, `YYYY-MM-DD`,
/// `YYYY/MM/DD`, and all-digit Unix epochs in seconds (10 digits) or milliseconds (13).
/// Ambiguous numeric forms like `03/04/2027` are deliberately **not** parsed — day-first and
/// month-first are indistinguishable and a silent 50% error rate on deadlines is worse than
/// an unknown one.
pub fn parse_timestamp(text: &str) -> Option<DateTime<Utc>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(parsed.with_timezone(&Utc));
    }

    for format in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, format) {
            return Some(naive.and_utc());
        }
    }

    for format in ["%Y-%m-%d", "%Y/%m/%d"] {
        if let Ok(date) = NaiveDate::parse_from_str(trimmed, format) {
            return date.and_hms_opt(0, 0, 0).map(|naive| naive.and_utc());
        }
    }

    if trimmed.chars().all(|c| c.is_ascii_digit())
        && let Ok(value) = trimmed.parse::<i64>()
    {
        return match trimmed.len() {
            10 => DateTime::from_timestamp(value, 0),
            13 => DateTime::from_timestamp_millis(value),
            _ => None,
        };
    }

    None
}

// ------------------------------------------------------------------------------------------
// Small shared helpers
// ------------------------------------------------------------------------------------------

/// Lowercase, non-alphanumeric characters collapsed to single spaces, padded with one space at
/// each end.
///
/// The padding is the point: matching `" intern "` against the result cannot hit `"internal"`
/// or `"international"`, which substring matching on the raw text would.
fn phrase(text: &str) -> String {
    let mut out = String::from(" ");
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    if !out.ends_with(' ') {
        out.push(' ');
    }
    out
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn non_empty(text: &str) -> Option<String> {
    let trimmed = collapse_whitespace(&decode_html_entities(text));
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Named entities worth decoding, chosen for what actually appears in job feeds.
///
/// Deliberately not the full ~2,000-entry HTML5 table: that would be a dependency for a long
/// tail this corpus does not contain, and anything missing here is still covered by the
/// numeric forms below, which is what most encoders emit for accented characters anyway.
const NAMED_ENTITIES: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", ' '),
    ("ndash", '-'),
    ("mdash", '-'),
    ("lsquo", '\''),
    ("rsquo", '\''),
    ("ldquo", '"'),
    ("rdquo", '"'),
    ("hellip", '.'),
    ("middot", '.'),
    ("bull", '.'),
    ("reg", ' '),
    ("copy", ' '),
    ("trade", ' '),
    ("eacute", 'e'),
    ("egrave", 'e'),
    ("uuml", 'u'),
    ("ouml", 'o'),
    ("auml", 'a'),
    ("ccedil", 'c'),
    ("ntilde", 'n'),
];

/// Decode HTML character references in source text.
///
/// # Why this exists
///
/// Audit finding F3. Several feeds deliver HTML-escaped text, and nothing decoded it — so
/// `"Ben &amp; Jerry&#39;s"` reached [`company_key`] as the literal characters and normalized
/// to `"ben amp jerry 39 s"`, while the same company from a clean source normalized to
/// `"ben jerry s"`. `company_key` is the identity that dedup's fallback key, `company_signals`
/// and the prestige alias table all join on, so one company silently became two — with split
/// postings, split prestige, and an alias table that could not match either.
///
/// # Placement
///
/// Called from [`non_empty`], the single funnel every text field passes through, so the
/// decoded form is what identity, display, classification and every sub-parser all see. Doing
/// it in `company_key` alone would have fixed the key and left `company_name` rendering as
/// `Ben &amp; Jerry's` in the UI.
///
/// Note the ordering this relies on: `is_http_url` validates the URL **after** `non_empty`,
/// so a reference is decoded before the scheme is checked rather than after. Decoding after
/// validation would be the classic bypass.
///
/// # Single pass, deliberately
///
/// `&amp;lt;` decodes to `&lt;` and stops, not to `<`. Repeated decoding is how escaped markup
/// becomes live markup; one pass is the standard defence. Nothing downstream renders this text
/// as HTML today — the UI is React interpolation throughout — but that is a property of the
/// current frontend, not something this function should depend on.
///
/// Unrecognized or malformed references are left exactly as written. `&notanentity;` stays
/// literal, which is no worse than the previous behaviour for that input and is visible.
pub fn decode_html_entities(text: &str) -> String {
    // Overwhelmingly the common case; skip the work and the allocation entirely.
    if !text.contains('&') {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'&' {
            // Copy the character whole — indexing by byte would split a multi-byte char.
            let ch = text[index..].chars().next().expect("index is a char boundary");
            out.push(ch);
            index += ch.len_utf8();
            continue;
        }

        // A reference is `&…;` with a short body. The cap stops a stray `&` in prose from
        // scanning the rest of the string looking for a semicolon that belongs to something
        // else entirely.
        const MAX_ENTITY_BODY: usize = 12;
        // Walk the cap back to a character boundary before slicing.
        //
        // `index + 1 + MAX_ENTITY_BODY` is byte arithmetic, and there is no reason for the
        // byte it lands on to begin a character — slicing a `&str` there panics. A real
        // Simplify title did exactly that on 2026-08-30: an `&` in
        // "Materials Planning & Logistics – Development Programs" put the cap at byte 41,
        // which is inside the en-dash occupying 40..43. The panic killed the whole collection
        // task: two sources never ran, the run never finished, and the manual trigger returned
        // an empty body.
        //
        // Entity bodies are ASCII, so anything multi-byte inside the window is already proof
        // this is not an entity. Shrinking the window can only ever cost a match that was
        // never going to happen.
        let mut limit = (index + 1 + MAX_ENTITY_BODY).min(bytes.len());
        while limit > index + 1 && !text.is_char_boundary(limit) {
            limit -= 1;
        }
        let semicolon = text[index + 1..limit].find(';').map(|at| index + 1 + at);

        match semicolon.and_then(|end| decode_reference(&text[index + 1..end]).map(|ch| (ch, end)))
        {
            Some((decoded, end)) => {
                out.push(decoded);
                index = end + 1;
            }
            // Not a reference we recognize: emit the `&` and carry on from the next byte.
            None => {
                out.push('&');
                index += 1;
            }
        }
    }

    out
}

/// The body of one reference, without the `&` or the `;`.
fn decode_reference(body: &str) -> Option<char> {
    if body.is_empty() {
        return None;
    }

    if let Some(digits) = body.strip_prefix('#') {
        let value = match digits.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => digits.parse::<u32>().ok()?,
        };
        // `from_u32` rejects surrogates and out-of-range values, so a hostile `&#xD800;`
        // cannot produce an invalid `char`.
        return char::from_u32(value);
    }

    // Named references are case-sensitive in HTML5; matching case-insensitively would decode
    // `&AMP;`, which no encoder emits, so keep it exact.
    NAMED_ENTITIES
        .iter()
        .find(|(name, _)| *name == body)
        .map(|(_, ch)| *ch)
}

fn non_empty_opt(text: Option<&str>) -> Option<String> {
    text.and_then(non_empty)
}

fn is_http_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact string that panicked a live collection on 2026-08-30.
    ///
    /// An `&` followed within twelve bytes by a multi-byte character: the entity-scan cap
    /// lands mid-character and slicing there panics, taking the whole collection task with it.
    /// Real data found this; no fixture had an en-dash near an ampersand.
    #[test]
    fn an_ampersand_near_a_multibyte_character_does_not_panic() {
        let title = "Manager, Materials Planning & Logistics – Development Programs (R5252)";
        assert_eq!(decode_html_entities(title), title);
    }

    /// The same shape at every offset, since which byte the cap lands on depends on where the
    /// ampersand sits. One passing example proves only that one offset is safe.
    #[test]
    fn an_ampersand_at_any_distance_from_a_multibyte_character_is_safe() {
        for gap in 0..16 {
            let text = format!("A & {}– dash", "x".repeat(gap));
            let decoded = decode_html_entities(&text);
            assert!(decoded.contains('–'), "lost the dash at gap {gap}");
        }
        // And the same for characters wider than three bytes.
        for gap in 0..16 {
            let text = format!("A & {}🙂 emoji", "x".repeat(gap));
            assert!(decode_html_entities(&text).contains('🙂'), "lost the emoji at gap {gap}");
        }
    }

    /// The fix must not cost a real entity that happens to sit near one.
    #[test]
    fn a_real_entity_still_decodes_next_to_a_multibyte_character() {
        assert_eq!(decode_html_entities("Ben &amp; Jerry — ice cream"), "Ben & Jerry — ice cream");
        assert_eq!(decode_html_entities("&amp;–"), "&–");
    }


    // --------------------------------------------------------------------------------------
    // Fixtures
    // --------------------------------------------------------------------------------------

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(year, month, day)
            .expect("valid date")
            .and_hms_opt(12, 0, 0)
            .expect("valid time")
            .and_utc()
    }

    /// 20 August 2026 — inside the academic year ending June 2027, so it exercises the
    /// inside the 2026-27 academic year. Term window: 2025..=2029.
    fn now() -> DateTime<Utc> {
        at(2026, 8, 20)
    }

    /// A posting that passes every identity check and both classifiers, so a test can change
    /// exactly one thing and know that thing is what moved the outcome.
    fn raw_posting(title: &str) -> RawPosting {
        RawPosting {
            source: "greenhouse".to_string(),
            external_id: "ext-1".to_string(),
            url: "https://example.com/jobs/1".to_string(),
            company: "Acme Inc.".to_string(),
            title: title.to_string(),
            location_raw: None,
            pay_raw: None,
            term_raw: None,
            class_year_raw: None,
            posted_at_raw: None,
            deadline_raw: None,
            description: None,
            remote_hint: None,
            raw_json: "{\"id\":1}".to_string(),
        }
    }

    fn expect_accepted(outcome: QcOutcome) -> NormalizedPosting {
        match outcome {
            QcOutcome::Accepted(posting) => *posting,
            other => panic!("expected Accepted, got {other:?}"),
        }
    }

    /// Returns the reason code, so callers assert on the *specific* code rather than merely
    /// on "it wasn't accepted".
    fn expect_filtered(outcome: QcOutcome) -> String {
        match outcome {
            QcOutcome::Filtered { reason, .. } => reason,
            other => panic!("expected Filtered, got {other:?}"),
        }
    }

    /// Returns `(reason, field)`.
    fn expect_rejected(outcome: QcOutcome) -> (String, Option<String>) {
        match outcome {
            QcOutcome::Rejected { reason, field, .. } => (reason, field),
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    fn usd(min: f64, max: Option<f64>, period: PayPeriod) -> PayRange {
        PayRange {
            min,
            max,
            currency: "USD".to_string(),
            period,
        }
    }

    // --------------------------------------------------------------------------------------
    // Pay — the formats sources actually emit
    // --------------------------------------------------------------------------------------

    #[test]
    fn an_hourly_range_with_symbols_parses() {
        assert_eq!(
            parse_pay("$45 - $55 / hour"),
            Some(usd(45.0, Some(55.0), PayPeriod::Hour))
        );
    }

    #[test]
    fn a_single_hourly_figure_has_no_upper_bound() {
        // `max: None` is "the source quoted one figure", distinct from a range whose top
        // happens to equal its bottom.
        assert_eq!(parse_pay("$45/hr"), Some(usd(45.0, None, PayPeriod::Hour)));
    }

    #[test]
    fn an_iso_code_and_a_word_period_parse_without_any_symbol() {
        assert_eq!(
            parse_pay("45-55 USD hourly"),
            Some(usd(45.0, Some(55.0), PayPeriod::Hour))
        );
    }

    #[test]
    fn a_monthly_stipend_keeps_its_thousands_separator_out_of_the_number() {
        assert_eq!(
            parse_pay("$8,000/month"),
            Some(usd(8_000.0, None, PayPeriod::Month))
        );
    }

    #[test]
    fn a_k_suffix_scales_to_thousands() {
        assert_eq!(
            parse_pay("$120k - $150k per year"),
            Some(usd(120_000.0, Some(150_000.0), PayPeriod::Year))
        );
    }

    #[test]
    fn between_x_and_y_reads_as_a_range() {
        assert_eq!(
            parse_pay("between $45 and $55 an hour"),
            Some(usd(45.0, Some(55.0), PayPeriod::Hour))
        );
    }

    #[test]
    fn an_explicit_iso_code_beats_a_bare_dollar_sign() {
        // Otherwise a Canadian posting silently ranks against US dollars.
        let parsed = parse_pay("CAD $45/hr").expect("a code and an amount are both present");
        assert_eq!(parsed.currency, "CAD");
    }

    // --- pay: the cases that must produce None ---

    #[test]
    fn competitive_is_not_a_pay_range() {
        assert_eq!(parse_pay("Competitive"), None);
    }

    #[test]
    fn doe_is_not_a_pay_range() {
        assert_eq!(parse_pay("DOE"), None);
    }

    #[test]
    fn an_empty_pay_string_is_not_a_pay_range() {
        assert_eq!(parse_pay(""), None);
    }

    #[test]
    fn an_amount_with_no_currency_marker_is_not_a_pay_range() {
        // Defaulting to USD would mis-scale every non-US posting, and `PayRange` has nowhere
        // to record that the currency was assumed.
        assert_eq!(parse_pay("45-55 hourly"), None);
    }

    #[test]
    fn a_weekly_figure_is_refused_rather_than_rescaled() {
        // `PayPeriod` has no Week. Promoting this to Month is a factor-of-four error that
        // looks entirely plausible in the ranked list.
        assert_eq!(parse_pay("$2,000/week"), None);
        assert_eq!(parse_pay("$400 daily"), None);
        assert_eq!(parse_pay("$9,000 per semester"), None);
    }

    #[test]
    fn a_backwards_range_is_a_parse_bug_not_data() {
        assert_eq!(parse_pay("$55 - $45/hour"), None);
    }

    #[test]
    fn a_stated_zero_is_not_read_as_pay() {
        // Sources use 0 as a "not disclosed" placeholder far more often than a SWE internship
        // genuinely pays nothing, and absent-read-as-zero is this phase's named failure.
        assert_eq!(parse_pay("$0/hour"), None);
        assert_eq!(parse_pay("$0 - $0 per year"), None);
    }

    #[test]
    fn the_smallest_amount_above_zero_is_still_pay() {
        // The companion to the test above: the cutoff is `> MIN_MEANINGFUL_PAY`, so anything
        // strictly above it survives. Without this the zero rule could be an accident of a
        // parser that rejects every small number.
        assert_eq!(
            parse_pay("$0.01/hour"),
            Some(usd(0.01, None, PayPeriod::Hour))
        );
    }

    // --- pay: the magnitude threshold, tested on the boundary itself ---

    #[test]
    fn a_bare_amount_at_the_annual_threshold_is_annual() {
        let boundary = format!("${MIN_UNAMBIGUOUS_ANNUAL_USD:.0}");
        assert_eq!(
            parse_pay(&boundary),
            Some(usd(MIN_UNAMBIGUOUS_ANNUAL_USD, None, PayPeriod::Year)),
            "the threshold itself must be inclusive"
        );
    }

    // --- pay sanity bounds, added 2026-08-22 (audit findings F9, F10) ---

    #[test]
    fn a_negative_amount_is_refused_rather_than_made_positive() {
        // The defect: the sign was skipped with the currency symbol, so this parsed as a
        // cheerful $20/hr. Inventing a positive figure from a malformed field is worse than
        // reading nothing, because pay is the highest-weighted ranking input.
        assert_eq!(parse_pay("$-20/hr"), None);
        assert_eq!(parse_pay("-$20/hr"), None);
        assert_eq!(parse_pay("-20/hr"), None);
    }

    #[test]
    fn a_range_separator_is_not_mistaken_for_a_minus_sign() {
        // The reason `is_negated` refuses to skip whitespace. Breaking this would silently
        // discard every hyphenated pay range in the corpus, which is most of them.
        assert_eq!(
            parse_pay("$45 - 55 per hour"),
            Some(usd(45.0, Some(55.0), PayPeriod::Hour))
        );
        assert_eq!(
            parse_pay("$45-55/hr"),
            Some(usd(45.0, Some(55.0), PayPeriod::Hour))
        );
    }

    #[test]
    fn a_dash_before_the_amount_is_not_a_minus_sign() {
        // The case `is_negated`'s refusal to skip whitespace actually protects, and the one a
        // hyphen-separated title makes routine. Caught by mutation testing: without this, a
        // version of `is_negated` that skipped whitespace passed every other test while
        // silently discarding pay from any posting whose text put a dash before the figure.
        assert_eq!(
            parse_pay("Software Engineer Intern - $45/hr"),
            Some(usd(45.0, None, PayPeriod::Hour))
        );
        assert_eq!(
            parse_pay("Summer 2027 \u{2014} $45.00 per hour"),
            Some(usd(45.0, None, PayPeriod::Hour))
        );
    }

    #[test]
    fn an_incredible_figure_is_refused_at_each_period() {
        // Unit confusion, the failure these exist to catch: a raw source value arriving
        // unscaled and being read at face value.
        assert_eq!(parse_pay("$1,000,000/hr"), None);
        assert_eq!(parse_pay("$5,000,000 per month"), None);
        assert_eq!(parse_pay("$50,000,000 per year"), None);
    }

    #[test]
    fn the_credible_ceilings_are_inclusive_at_their_boundary() {
        // On the boundary itself. These sit far above `MAX_PLAUSIBLE_HOURLY_USD`, which
        // answers a different question — whether a *bare* figure is hourly.
        assert_eq!(
            parse_pay(&format!("${MAX_CREDIBLE_HOURLY_USD:.0}/hr")),
            Some(usd(MAX_CREDIBLE_HOURLY_USD, None, PayPeriod::Hour)),
            "the ceiling itself must still be accepted"
        );
        assert_eq!(
            parse_pay(&format!("${:.0}/hr", MAX_CREDIBLE_HOURLY_USD + 1.0)),
            None
        );
    }

    #[test]
    fn an_unusual_but_real_hourly_rate_still_survives() {
        // The ceiling must not become a judgement about generosity. $300/hr is high and real;
        // only unit confusion should trip this.
        assert_eq!(
            parse_pay("$300 per hour"),
            Some(usd(300.0, None, PayPeriod::Hour))
        );
    }

    #[test]
    fn the_credible_ceiling_does_not_apply_to_other_currencies() {
        // The thresholds are USD-denominated. 5,000 yen per hour is an ordinary wage and must
        // not be refused by a dollar ceiling.
        assert_eq!(
            parse_pay("JPY 5000 per hour"),
            Some(PayRange {
                min: 5000.0,
                max: None,
                currency: "JPY".to_string(),
                period: PayPeriod::Hour,
            })
        );
    }

    // --- HTML entity decoding, added 2026-08-22 (audit finding F3) ---

    #[test]
    fn an_entity_encoded_company_has_the_same_identity_as_a_plain_one() {
        // The defect: `&amp;` normalized to the word "amp" and `&#39;` to "39", so one company
        // arriving from two feeds became two companies with split postings and split prestige.
        //
        // Asserted through `normalize` rather than through `company_key` directly, because the
        // decoding lives at the `non_empty` funnel — deliberately one site, so the text is
        // decoded exactly once. See `company_key`'s own note on its precondition.
        let identity = |company: &str| {
            let mut raw = raw_posting("Software Engineer Intern");
            raw.company = company.to_string();
            let QcOutcome::Accepted(posting) = normalize(&raw, now()) else {
                panic!("{company} should normalize cleanly");
            };
            (posting.company_key.clone(), posting.company_name.clone())
        };

        assert_eq!(identity("Ben &amp; Jerry&#39;s"), identity("Ben & Jerry's"));
        assert_eq!(identity("Procter &amp; Gamble"), identity("Procter & Gamble"));

        // And the display name is decoded too — fixing only the key would have left the UI
        // rendering the literal text "Ben &amp; Jerry's".
        assert_eq!(identity("Ben &amp; Jerry&#39;s").1, "Ben & Jerry's");
    }

    #[test]
    fn numeric_and_hex_references_both_decode() {
        assert_eq!(decode_html_entities("caf&#233;"), "café");
        assert_eq!(decode_html_entities("caf&#xE9;"), "café");
        assert_eq!(decode_html_entities("caf&#Xe9;"), "café");
    }

    #[test]
    fn decoding_is_a_single_pass() {
        // `&amp;lt;` must become `&lt;`, never `<`. Repeated decoding is how escaped markup
        // turns back into live markup.
        assert_eq!(decode_html_entities("&amp;lt;script&amp;gt;"), "&lt;script&gt;");
    }

    #[test]
    fn a_reference_is_decoded_before_the_url_scheme_is_checked() {
        // Ordering matters more than the decoding here: `non_empty` runs before `is_http_url`,
        // so a reference cannot smuggle a scheme past validation by being decoded afterwards.
        let mut raw = raw_posting("Software Engineer Intern");
        raw.url = "&#106;avascript:alert(1)".to_string();
        let (reason, _) = expect_rejected(normalize(&raw, now()));
        assert_eq!(reason, REASON_INVALID_URL);
    }

    #[test]
    fn malformed_references_are_left_alone_rather_than_panicking() {
        for input in [
            "&", "&;", "&#;", "&#x;", "&#xZZ;", "&#99999999999;", "&#xD800;", "&notreal;",
            "a & b", "&amp", "&&amp;;", "100% &amp; rising", "&#", "&#x",
        ] {
            let _ = decode_html_entities(input);
        }
        // A bare ampersand in prose survives untouched.
        assert_eq!(decode_html_entities("Ben & Jerry"), "Ben & Jerry");
        // An unrecognized name stays literal rather than being silently eaten.
        assert_eq!(decode_html_entities("&notreal;"), "&notreal;");
        // A surrogate code point is not a `char`; it must not decode.
        assert_eq!(decode_html_entities("&#xD800;"), "&#xD800;");
    }

    #[test]
    fn multi_byte_text_around_a_reference_survives_intact() {
        // The decoder walks bytes, so a multi-byte character adjacent to a reference is the
        // case that would corrupt output or panic if the indexing were wrong.
        assert_eq!(decode_html_entities("工程師 &amp; 実習"), "工程師 & 実習");
        assert_eq!(decode_html_entities("🚀&amp;🚀"), "🚀&🚀");
        assert_eq!(decode_html_entities("café&#39;s 🚀"), "café's 🚀");
        // And with no reference at all, the fast path must be byte-exact.
        assert_eq!(decode_html_entities("工程師實習生 🚀"), "工程師實習生 🚀");
    }

    #[test]
    fn a_long_run_of_text_after_a_stray_ampersand_is_not_swallowed() {
        // Without a length cap the scan would run to a distant semicolon and delete everything
        // between, which is far worse than leaving the `&` alone.
        let input = "R&D at Acme; we build things";
        assert_eq!(decode_html_entities(input), input);
    }

    // --- pay: the hourly band, added 2026-08-22 (audit finding F1) ---

    #[test]
    fn a_bare_hourly_rate_is_read_as_hourly() {
        // The defect: this returned `None`, so every Greenhouse internship rate was discarded.
        assert_eq!(parse_pay("USD 45.00"), Some(usd(45.0, None, PayPeriod::Hour)));
    }

    #[test]
    fn a_bare_hourly_range_is_read_as_hourly() {
        // Greenhouse's exact output shape for a range: `"{currency} {min:.2} - {max:.2}"`.
        assert_eq!(
            parse_pay("USD 45.00 - 55.00"),
            Some(usd(45.0, Some(55.0), PayPeriod::Hour))
        );
    }

    #[test]
    fn the_hourly_band_is_inclusive_at_its_own_boundary() {
        // On the threshold, not either side of it — the repo has already lost a whole rating
        // band to a `>` that should have been `>=`.
        let at = format!("${MAX_PLAUSIBLE_HOURLY_USD:.0}");
        assert_eq!(
            parse_pay(&at),
            Some(usd(MAX_PLAUSIBLE_HOURLY_USD, None, PayPeriod::Hour)),
            "the boundary itself must be hourly"
        );
    }

    #[test]
    fn just_above_the_hourly_band_is_ambiguous_rather_than_guessed() {
        // Not `Month`: this is the band where the likelier period flips, so nothing is
        // inferred. Pinning it stops someone "completing" the heuristic without reading why.
        let above = format!("${:.0}", MAX_PLAUSIBLE_HOURLY_USD + 1.0);
        assert_eq!(parse_pay(&above), None);
    }

    #[test]
    fn an_explicit_period_still_beats_the_hourly_band() {
        // Ashby and Lever state their intervals; the new band must not override them.
        assert_eq!(
            parse_pay("USD 150.00 per year"),
            Some(usd(150.0, None, PayPeriod::Year))
        );
        assert_eq!(
            parse_pay("USD 150.00 per month"),
            Some(usd(150.0, None, PayPeriod::Month))
        );
    }

    #[test]
    fn a_bare_amount_just_below_the_annual_threshold_has_no_discernible_period() {
        let below = format!("${:.0}", MIN_UNAMBIGUOUS_ANNUAL_USD - 1.0);
        assert_eq!(parse_pay(&below), None);
    }

    #[test]
    fn a_bare_six_figure_amount_is_annual() {
        assert_eq!(
            parse_pay("$120,000"),
            Some(usd(120_000.0, None, PayPeriod::Year))
        );
    }

    #[test]
    fn the_top_of_a_bare_range_settles_the_period() {
        // Keying on the bottom would throw this pair away for being below the threshold.
        assert_eq!(
            parse_pay("$40,000 - $80,000"),
            Some(usd(40_000.0, Some(80_000.0), PayPeriod::Year))
        );
    }

    #[test]
    fn an_explicit_period_always_beats_the_magnitude_heuristic() {
        // Ashby is the only source with an unambiguous interval (§ B fn 7). Its precision must
        // not be diluted to accommodate the sources that lack one, so a stated period wins
        // even where magnitude alone would have decided otherwise — in both directions.
        // $60,000 rather than the $120,000 this used to use: any figure above
        // `MIN_UNAMBIGUOUS_ANNUAL_USD` demonstrates the precedence, and $120,000 *per month*
        // now trips `MAX_CREDIBLE_MONTHLY_USD` (audit finding F10) — a separate guard whose
        // job is catching unit errors. Restoring the old value fails for that reason, not
        // because period precedence broke.
        assert_eq!(
            parse_pay("$60,000 per month"),
            Some(usd(60_000.0, None, PayPeriod::Month)),
            "magnitude would have said annual; the source said monthly"
        );
        assert_eq!(
            parse_pay("$30,000 per year"),
            Some(usd(30_000.0, None, PayPeriod::Year)),
            "below the annual threshold, but the period was stated outright"
        );
    }

    #[test]
    fn ashbys_interval_vocabulary_parses() {
        // "1 YEAR" / "1 MONTH" / "NONE" (§ A.1). The leading 1 must not be read as the amount.
        assert_eq!(
            parse_pay("USD 150000 1 YEAR"),
            Some(usd(150_000.0, None, PayPeriod::Year))
        );
        assert_eq!(
            parse_pay("USD 11700 1 MONTH"),
            Some(usd(11_700.0, None, PayPeriod::Month))
        );
        assert_eq!(
            parse_pay("USD 11700 NONE"),
            None,
            "\"NONE\" is not a period, and the amount is inside the ambiguous band"
        );
    }

    #[test]
    fn the_largest_recorded_monthly_intern_stipend_is_not_read_as_a_salary() {
        // Ramp's intern posting carried {"interval":"1 MONTH","minValue":11700} (§ A.1). If an
        // adapter loses the interval, the bare figure must NOT be inferred as annual — that is
        // exactly the "a monthly intern stipend and a low annual salary can land in the same
        // band" trap the research doc names, and it is why the threshold clears the band
        // rather than splitting it.
        assert_eq!(parse_pay("$11,700"), None);
        assert_eq!(
            parse_pay("$11,700/month"),
            Some(usd(11_700.0, None, PayPeriod::Month))
        );
    }

    // --- pay: heuristics that stop a duration becoming the rate ---

    #[test]
    fn a_leading_duration_does_not_become_the_rate() {
        assert_eq!(
            parse_pay("40 hours/week at $45/hr"),
            Some(usd(45.0, None, PayPeriod::Hour))
        );
    }

    #[test]
    fn a_trailing_duration_does_not_become_the_top_of_a_range() {
        // Without the range-separator requirement this reads as 45–40, which is backwards and
        // would silently drop the pay entirely.
        assert_eq!(
            parse_pay("$45/hour, 40 hours per week"),
            Some(usd(45.0, None, PayPeriod::Hour))
        );
    }

    // --------------------------------------------------------------------------------------
    // Term
    // --------------------------------------------------------------------------------------

    #[test]
    fn a_season_and_year_parse_in_either_order() {
        assert_eq!(
            parse_term("Summer 2027", now()),
            Term {
                season: Some(Season::Summer),
                year: Some(2027)
            }
        );
        assert_eq!(
            parse_term("2027 Summer Internship", now()),
            Term {
                season: Some(Season::Summer),
                year: Some(2027)
            }
        );
    }

    #[test]
    fn a_winter_coop_parses_despite_the_hyphen() {
        assert_eq!(
            parse_term("Winter 2026 Co-op", now()),
            Term {
                season: Some(Season::Winter),
                year: Some(2026)
            }
        );
    }

    #[test]
    fn fall_and_autumn_are_the_same_season() {
        assert_eq!(parse_term("Autumn 2026", now()).season, Some(Season::Fall));
        assert_eq!(parse_term("Fall 2026", now()).season, Some(Season::Fall));
        assert_eq!(
            parse_term("Spring 2027", now()).season,
            Some(Season::Spring)
        );
    }

    #[test]
    fn a_season_with_no_year_is_half_a_term() {
        assert_eq!(
            parse_term("Summer Internship", now()),
            Term {
                season: Some(Season::Summer),
                year: None
            }
        );
    }

    #[test]
    fn a_title_stating_neither_produces_an_empty_term() {
        // The must-fail case for this parser: a plain title has no term at all, and inventing
        // one from the collection date is explicitly forbidden by `models.rs`.
        assert_eq!(
            parse_term("Software Engineer Intern", now()),
            Term::default()
        );
    }

    #[test]
    fn a_loose_number_outside_the_window_is_not_a_term_year() {
        // A requisition id must not become a term and get the posting filtered as stale.
        assert_eq!(
            parse_term("Software Engineer Intern Req 2019", now()).year,
            None
        );
    }

    #[test]
    fn a_loose_number_inside_the_window_is_a_term_year() {
        assert_eq!(
            parse_term("Software Engineer Intern 2027", now()).year,
            Some(2027)
        );
    }

    #[test]
    fn a_stale_year_next_to_a_season_is_still_read_as_a_stated_term() {
        // It has to be read before it can be filtered — see the wrong_term tests below.
        assert_eq!(parse_term("Summer 2024", now()).year, Some(2024));
    }

    #[test]
    fn a_two_digit_number_is_never_a_year() {
        assert_eq!(parse_term("Summer 27 Internship", now()).year, None);
    }

    // --------------------------------------------------------------------------------------
    // Term window — the boundary values themselves
    // --------------------------------------------------------------------------------------

    #[test]
    fn the_term_window_is_derived_from_now() {
        assert_eq!(plausible_term_years(now()), (2025, 2029));
    }

    #[test]
    fn the_earliest_acceptable_term_year_is_kept() {
        let posting = expect_accepted(normalize(
            &raw_posting("Software Engineer Intern, Summer 2025"),
            now(),
        ));
        assert_eq!(posting.term_year, Some(2025));
    }

    #[test]
    fn one_year_before_the_earliest_is_filtered_as_the_wrong_term() {
        let reason = expect_filtered(normalize(
            &raw_posting("Software Engineer Intern, Summer 2024"),
            now(),
        ));
        assert_eq!(reason, REASON_WRONG_TERM);
    }

    #[test]
    fn the_latest_acceptable_term_year_is_kept() {
        let posting = expect_accepted(normalize(
            &raw_posting("Software Engineer Intern, Summer 2029"),
            now(),
        ));
        assert_eq!(posting.term_year, Some(2029));
    }

    #[test]
    fn one_year_after_the_latest_is_filtered_as_the_wrong_term() {
        let reason = expect_filtered(normalize(
            &raw_posting("Software Engineer Intern, Summer 2030"),
            now(),
        ));
        assert_eq!(reason, REASON_WRONG_TERM);
    }

    #[test]
    fn a_dedicated_term_field_fills_in_what_the_title_omits() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.term_raw = Some("Summer 2027".to_string());
        let posting = expect_accepted(normalize(&raw, now()));
        assert_eq!(posting.term_season, Some(Season::Summer));
        assert_eq!(posting.term_year, Some(2027));
    }

    // --------------------------------------------------------------------------------------
    // Location
    // --------------------------------------------------------------------------------------

    #[test]
    fn remote_is_remote_and_names_no_city() {
        let location = parse_location(Some("Remote"), None);
        assert_eq!(location.is_remote, Some(true));
        assert_eq!(location.city, None);
        assert_eq!(location.raw.as_deref(), Some("Remote"));
    }

    #[test]
    fn remote_with_a_country_keeps_the_country() {
        let location = parse_location(Some("Remote (US)"), None);
        assert_eq!(location.is_remote, Some(true));
        assert_eq!(location.country.as_deref(), Some("United States"));
        assert_eq!(location.city, None);
    }

    #[test]
    fn a_us_city_gets_its_state_and_country() {
        let location = parse_location(Some("San Francisco, CA"), None);
        assert_eq!(location.city.as_deref(), Some("San Francisco"));
        assert_eq!(location.region.as_deref(), Some("CA"));
        assert_eq!(location.country.as_deref(), Some("United States"));
    }

    #[test]
    fn an_international_city_gets_its_country() {
        let location = parse_location(Some("London, United Kingdom"), None);
        assert_eq!(location.city.as_deref(), Some("London"));
        assert_eq!(location.country.as_deref(), Some("United Kingdom"));
        assert_eq!(location.region, None);
    }

    #[test]
    fn hybrid_is_not_remote_and_still_yields_its_city() {
        let location = parse_location(Some("Hybrid — Seattle, WA"), None);
        assert_eq!(
            location.is_remote,
            Some(false),
            "hybrid requires physical presence at a named place"
        );
        assert_eq!(location.city.as_deref(), Some("Seattle"));
        assert_eq!(location.region.as_deref(), Some("WA"));
    }

    #[test]
    fn a_bare_city_leaves_remoteness_unknown() {
        // THE THIRD STATE. `None` here is not `Some(false)`: plenty of postings name an office
        // and are remote-eligible without saying so, and asserting onsite for all of them is
        // what makes a remote filter quietly drop them.
        assert_eq!(
            parse_location(Some("San Francisco, CA"), None).is_remote,
            None
        );
        assert_eq!(
            parse_location(Some("London, United Kingdom"), None).is_remote,
            None
        );
    }

    #[test]
    fn an_explicit_onsite_word_is_positive_evidence_of_onsite() {
        // The companion to the test above: `Some(false)` must still be reachable, or the
        // "bare city is unknown" rule could be an accident of a function that never says
        // onsite at all.
        assert_eq!(
            parse_location(Some("Onsite - Austin, TX"), None).is_remote,
            Some(false)
        );
    }

    #[test]
    fn a_structured_hint_from_the_source_beats_the_string() {
        assert_eq!(
            parse_location(Some("San Francisco, CA"), Some(true)).is_remote,
            Some(true)
        );
    }

    #[test]
    fn no_location_string_at_all_is_all_unknown() {
        let location = parse_location(None, None);
        assert_eq!(location, Location::default());
        assert_eq!(location.is_remote, None);
    }

    #[test]
    fn a_hint_survives_a_missing_location_string() {
        let location = parse_location(None, Some(false));
        assert_eq!(location.raw, None);
        assert_eq!(location.is_remote, Some(false));
    }

    #[test]
    fn a_multi_location_string_uses_its_first_segment_and_keeps_the_whole_raw() {
        let location = parse_location(Some("New York, NY; San Francisco, CA"), None);
        assert_eq!(location.city.as_deref(), Some("New York"));
        assert_eq!(location.region.as_deref(), Some("NY"));
        assert_eq!(
            location.raw.as_deref(),
            Some("New York, NY; San Francisco, CA"),
            "the segments we did not use must stay inspectable"
        );
    }

    #[test]
    fn remote_anywhere_in_a_multi_location_string_still_counts() {
        let location = parse_location(Some("San Francisco, CA or Remote"), None);
        assert_eq!(location.is_remote, Some(true));
        assert_eq!(location.city.as_deref(), Some("San Francisco"));
    }

    #[test]
    fn an_unrecognized_second_part_is_a_region_not_a_country() {
        // A wrong region is a smaller lie than a wrong country, and country filters matter more.
        let location = parse_location(Some("Bengaluru, Karnataka"), None);
        assert_eq!(location.city.as_deref(), Some("Bengaluru"));
        assert_eq!(location.region.as_deref(), Some("Karnataka"));
        assert_eq!(location.country, None);
    }

    #[test]
    fn a_three_part_location_reads_city_region_country() {
        let location = parse_location(Some("Seattle, WA, United States"), None);
        assert_eq!(location.city.as_deref(), Some("Seattle"));
        assert_eq!(location.region.as_deref(), Some("WA"));
        assert_eq!(location.country.as_deref(), Some("United States"));
    }

    #[test]
    fn a_remote_title_infers_remoteness_but_never_onsite() {
        let mut raw = raw_posting("Software Engineer Intern (Remote)");
        raw.location_raw = Some("San Francisco, CA".to_string());
        assert_eq!(
            expect_accepted(normalize(&raw, now())).location.is_remote,
            Some(true)
        );

        let plain = raw_posting("Software Engineer Intern");
        assert_eq!(
            expect_accepted(normalize(&plain, now())).location.is_remote,
            None,
            "a title that says nothing is not evidence of onsite"
        );
    }

    // --------------------------------------------------------------------------------------
    // Class years
    // --------------------------------------------------------------------------------------

    #[test]
    fn the_class_year_window_is_derived_from_now() {
        assert_eq!(plausible_class_years(now()), (2025, 2032));
    }

    #[test]
    fn a_class_word_alone_states_no_graduation_year() {
        // The behaviour change from the INTERNSHIP_SCRAPING.md § B review. "rising senior"
        // anchored to the collection date and to the internship's own term differ by a year
        // and are equally defensible, so we infer neither. No bounds means no restriction,
        // which `admits` already reads as admitting everyone.
        for text in [
            "rising senior",
            "senior",
            "open to juniors and seniors",
            "sophomore",
            "must be a rising freshman or sophomore",
        ] {
            let range = parse_class_years(text, now());
            assert_eq!(range.min, None, "{text} must not produce a lower bound");
            assert_eq!(range.max, None, "{text} must not produce an upper bound");
            assert!(range.admits(2026), "{text} must admit everyone");
            assert!(range.admits(2031), "{text} must admit everyone");
            assert_eq!(
                range.raw.as_deref(),
                Some(text),
                "the text stays inspectable even though it produced no bounds"
            );
        }
    }

    #[test]
    fn a_literal_year_beside_a_class_word_is_still_read() {
        // The companion to the test above: dropping class-word inference must not have
        // disabled the explicit path that runs through the same string.
        let range = parse_class_years("rising seniors, class of 2027", now());
        assert_eq!(range.min, Some(2027));
        assert_eq!(range.max, Some(2027));
    }

    #[test]
    fn a_hyphenated_span_of_graduation_years_is_a_range() {
        let range = parse_class_years("2026-2027 graduates", now());
        assert_eq!(range.min, Some(2026));
        assert_eq!(range.max, Some(2027));
    }

    #[test]
    fn graduating_in_a_year_pins_both_bounds() {
        let range = parse_class_years("graduating in 2027", now());
        assert_eq!(range.min, Some(2027));
        assert_eq!(range.max, Some(2027));
    }

    #[test]
    fn two_stated_years_become_a_range() {
        let range = parse_class_years("class of 2026 or 2027", now());
        assert_eq!(range.min, Some(2026));
        assert_eq!(range.max, Some(2027));
    }

    #[test]
    fn a_span_of_graduation_dates_becomes_a_range() {
        let range = parse_class_years("Dec 2026 – June 2027 graduates", now());
        assert_eq!(range.min, Some(2026));
        assert_eq!(range.max, Some(2027));
    }

    #[test]
    fn a_range_never_runs_backwards_however_it_was_written() {
        // Migration 0012 CHECKs this; building it by construction means the insert cannot fail.
        let range = parse_class_years("class of 2027 or 2026", now());
        assert_eq!(range.min, Some(2026));
        assert_eq!(range.max, Some(2027));
    }

    #[test]
    fn eligibility_text_with_no_years_restricts_nobody() {
        // The must-fail case for this parser. An unstated restriction is not a restriction,
        // but the text is still preserved so the miss is inspectable.
        let range = parse_class_years("Open to all majors", now());
        assert_eq!(range.min, None);
        assert_eq!(range.max, None);
        assert_eq!(range.raw.as_deref(), Some("Open to all majors"));
        assert!(range.admits(2024));
        assert!(range.admits(2031));
    }

    #[test]
    fn class_year_text_can_never_reject_or_filter_a_posting() {
        // Class year is an optional hard filter, never an identity field, so no eligibility
        // string — absent, empty, unreadable, or actively strange — may cost us the row.
        for class_year_raw in [
            None,
            Some(""),
            Some("rising senior"),
            Some("Open to all majors"),
            Some("###"),
            Some("class of 1899"),
            Some("graduating at some point, probably"),
        ] {
            let mut raw = raw_posting("Software Engineer Intern");
            raw.class_year_raw = class_year_raw.map(str::to_string);
            let outcome = normalize(&raw, now());
            assert!(
                matches!(outcome, QcOutcome::Accepted(_)),
                "{class_year_raw:?} must not change the outcome, got {outcome:?}"
            );
            let posting = expect_accepted(outcome);
            assert_eq!(posting.class_years.min, None, "{class_year_raw:?}");
            assert_eq!(posting.class_years.max, None, "{class_year_raw:?}");
        }
    }

    #[test]
    fn the_earliest_believable_graduation_year_is_kept() {
        assert_eq!(parse_class_years("class of 2025", now()).min, Some(2025));
    }

    #[test]
    fn one_year_before_the_earliest_believable_is_ignored() {
        let range = parse_class_years("class of 2024", now());
        assert_eq!(
            range.min, None,
            "2024 is outside the window and is not a class year"
        );
        assert_eq!(range.raw.as_deref(), Some("class of 2024"));
    }

    #[test]
    fn the_latest_believable_graduation_year_is_kept() {
        assert_eq!(parse_class_years("class of 2032", now()).min, Some(2032));
        assert_eq!(parse_class_years("class of 2033", now()).min, None);
    }

    // --------------------------------------------------------------------------------------
    // Class years from a description
    // --------------------------------------------------------------------------------------

    #[test]
    fn a_cue_bearing_sentence_supplies_class_years_when_the_field_is_absent() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.description =
            Some("Acme was founded in 2019. You must be graduating in 2027.".to_string());
        let posting = expect_accepted(normalize(&raw, now()));
        assert_eq!(posting.class_years.min, Some(2027));
        assert_eq!(
            posting.class_years.max,
            Some(2027),
            "the founding year is in a sentence with no eligibility cue and must not widen the range"
        );
    }

    #[test]
    fn a_description_with_no_eligibility_cue_restricts_nobody() {
        // Non-vacuous guard on the cue list: "Senior Software Engineer" is a seniority level,
        // not a class year, and a stray founding year is not a graduation year.
        let mut raw = raw_posting("Software Engineer Intern");
        raw.description = Some(
            "You will report to a Senior Software Engineer. Acme was founded in 2019.".to_string(),
        );
        let posting = expect_accepted(normalize(&raw, now()));
        assert_eq!(posting.class_years, ClassYearRange::default());
        assert!(posting.class_years.admits(2027));
    }

    #[test]
    fn the_dedicated_field_wins_over_the_description() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.class_year_raw = Some("class of 2028".to_string());
        raw.description = Some("Ideally graduating in 2026.".to_string());
        let posting = expect_accepted(normalize(&raw, now()));
        assert_eq!(posting.class_years.min, Some(2028));
        assert_eq!(posting.class_years.max, Some(2028));
    }

    // --------------------------------------------------------------------------------------
    // Company key
    // --------------------------------------------------------------------------------------

    #[test]
    fn legal_suffixes_are_stripped_from_the_tail() {
        assert_eq!(company_key("Google LLC"), "google");
        assert_eq!(company_key("Acme, Inc."), "acme");
        assert_eq!(company_key("Example Ltd"), "example");
        assert_eq!(company_key("Contoso Corp."), "contoso");
        assert_eq!(company_key("Beispiel GmbH"), "beispiel");
        assert_eq!(company_key("Widgets plc"), "widgets");
        assert_eq!(company_key("Acme Co Ltd"), "acme");
    }

    #[test]
    fn case_and_punctuation_and_spacing_all_collapse() {
        assert_eq!(company_key("  GOOGLE  "), "google");
        assert_eq!(company_key("Ben & Jerry's"), "ben jerry s");
        assert_eq!(company_key("Google"), company_key("google, inc."));
    }

    #[test]
    fn a_suffix_word_that_leads_the_name_survives() {
        assert_eq!(company_key("Inc Magazine"), "inc magazine");
    }

    #[test]
    fn a_name_made_only_of_a_suffix_keeps_it() {
        // Stripping into emptiness would turn an odd-but-usable name into a rejected row.
        assert_eq!(company_key("Inc."), "inc");
    }

    #[test]
    fn a_company_of_pure_punctuation_has_no_key() {
        assert_eq!(company_key("###"), "");
        assert_eq!(company_key("   "), "");
    }

    #[test]
    fn the_company_key_is_deterministic_not_fuzzy() {
        // Fuzzy company matching is a reserved NLP decision. If this ever starts passing,
        // someone has put fuzzy matching where it does not belong.
        assert_ne!(company_key("Google"), company_key("Google Cloud"));
        assert_ne!(company_key("Meta"), company_key("Meta Platforms"));
    }

    // --------------------------------------------------------------------------------------
    // Rejected — identity failures, every one a potential bug
    // --------------------------------------------------------------------------------------

    #[test]
    fn a_blank_company_is_rejected_not_filtered() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.company = "   ".to_string();
        let (reason, field) = expect_rejected(normalize(&raw, now()));
        assert_eq!(reason, REASON_MISSING_COMPANY);
        assert_eq!(field.as_deref(), Some("company"));
    }

    #[test]
    fn a_company_that_normalizes_to_nothing_is_rejected() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.company = "###".to_string();
        let (reason, field) = expect_rejected(normalize(&raw, now()));
        assert_eq!(reason, REASON_UNNORMALIZABLE_COMPANY);
        assert_eq!(field.as_deref(), Some("company"));
    }

    #[test]
    fn a_blank_title_is_rejected() {
        let mut raw = raw_posting("");
        raw.title = String::new();
        let (reason, field) = expect_rejected(normalize(&raw, now()));
        assert_eq!(reason, REASON_MISSING_TITLE);
        assert_eq!(field.as_deref(), Some("title"));
    }

    #[test]
    fn a_missing_url_is_rejected() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.url = String::new();
        assert_eq!(
            expect_rejected(normalize(&raw, now())).0,
            REASON_MISSING_URL
        );
    }

    #[test]
    fn a_non_http_url_is_rejected() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.url = "/jobs/1".to_string();
        let (reason, field) = expect_rejected(normalize(&raw, now()));
        assert_eq!(reason, REASON_INVALID_URL);
        assert_eq!(field.as_deref(), Some("url"));
    }

    #[test]
    fn an_uppercase_scheme_is_still_a_valid_url() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.url = "HTTPS://example.com/jobs/1".to_string();
        expect_accepted(normalize(&raw, now()));
    }

    #[test]
    fn a_missing_external_id_is_rejected() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.external_id = String::new();
        assert_eq!(
            expect_rejected(normalize(&raw, now())).0,
            REASON_MISSING_EXTERNAL_ID
        );
    }

    #[test]
    fn a_missing_source_is_rejected() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.source = String::new();
        assert_eq!(
            expect_rejected(normalize(&raw, now())).0,
            REASON_MISSING_SOURCE
        );
    }

    #[test]
    fn identity_is_checked_before_classification() {
        // A row with no title cannot be classified, so it must not be recorded as a healthy
        // "not an internship" exclusion — that would hide an adapter defect in the bulk count.
        let mut raw = raw_posting("");
        raw.title = String::new();
        raw.company = String::new();
        let (reason, _) = expect_rejected(normalize(&raw, now()));
        assert_eq!(reason, REASON_MISSING_COMPANY, "company is checked first");
    }

    // --------------------------------------------------------------------------------------
    // Filtered — correct exclusions
    // --------------------------------------------------------------------------------------

    #[test]
    fn a_full_time_role_is_filtered_as_not_an_internship() {
        let reason = expect_filtered(normalize(&raw_posting("Senior Software Engineer"), now()));
        assert_eq!(reason, REASON_NOT_AN_INTERNSHIP);
    }

    #[test]
    fn internal_is_not_intern() {
        // The space-padded phrase match earns its keep here: substring matching on the raw
        // title would accept this, and "international" too.
        let reason = expect_filtered(normalize(&raw_posting("Internal Tools Engineer"), now()));
        assert_eq!(reason, REASON_NOT_AN_INTERNSHIP);
        assert!(!is_internship_role("International Software Engineer"));
    }

    #[test]
    fn a_non_software_internship_is_filtered_as_not_software() {
        let reason = expect_filtered(normalize(&raw_posting("Marketing Intern"), now()));
        assert_eq!(reason, REASON_NOT_SOFTWARE);
        assert_eq!(
            expect_filtered(normalize(&raw_posting("Data Science Intern"), now())),
            REASON_NOT_SOFTWARE
        );
    }

    #[test]
    fn another_engineering_discipline_is_filtered_as_not_software() {
        assert_eq!(
            expect_filtered(normalize(
                &raw_posting("Mechanical Engineering Intern"),
                now()
            )),
            REASON_NOT_SOFTWARE
        );
        assert_eq!(
            expect_filtered(normalize(
                &raw_posting("Electrical Engineering Co-op"),
                now()
            )),
            REASON_NOT_SOFTWARE
        );
    }

    #[test]
    fn a_bare_engineering_internship_is_accepted() {
        // The weak branch errs toward inclusion on purpose: a filtered posting is a job the
        // user never sees, while an accepted one is visible noise they can judge.
        expect_accepted(normalize(&raw_posting("Engineering Intern"), now()));
    }

    #[test]
    fn the_usual_software_titles_are_all_accepted() {
        for title in [
            "Software Engineer Intern",
            "Backend Engineering Co-op",
            "iOS Developer Intern",
            "Site Reliability Engineering Internship",
            "Machine Learning Intern",
            "SWE Intern",
        ] {
            expect_accepted(normalize(&raw_posting(title), now()));
        }
    }

    #[test]
    fn a_term_field_can_supply_the_internship_signal() {
        // Some sources carry "Internship" in an employment-type field, not the title.
        let mut raw = raw_posting("Software Engineer, Platform");
        raw.term_raw = Some("Summer 2027 Internship".to_string());
        expect_accepted(normalize(&raw, now()));
    }

    // --------------------------------------------------------------------------------------
    // Absent is not zero
    // --------------------------------------------------------------------------------------

    #[test]
    fn an_absent_salary_is_unknown_never_zero() {
        let posting = expect_accepted(normalize(&raw_posting("Software Engineer Intern"), now()));
        assert_eq!(posting.pay, None);
        assert_eq!(posting.pay_raw, None);
    }

    #[test]
    fn an_unparseable_salary_keeps_the_posting_and_the_raw_string() {
        // Pay is not identity. Rejecting over it would discard a real job AND bury genuine
        // adapter defects under rows whose only sin is an undisclosed number.
        let mut raw = raw_posting("Software Engineer Intern");
        raw.pay_raw = Some("Competitive".to_string());
        let posting = expect_accepted(normalize(&raw, now()));
        assert_eq!(posting.pay, None);
        assert_eq!(
            posting.pay_raw.as_deref(),
            Some("Competitive"),
            "'we could not parse it' must stay distinct from 'there was not one'"
        );
    }

    #[test]
    fn a_parseable_salary_is_carried_through_with_its_raw_string() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.pay_raw = Some("$45 - $55 / hour".to_string());
        let posting = expect_accepted(normalize(&raw, now()));
        assert_eq!(posting.pay, Some(usd(45.0, Some(55.0), PayPeriod::Hour)));
        assert_eq!(posting.pay_raw.as_deref(), Some("$45 - $55 / hour"));
    }

    #[test]
    fn an_unstated_posted_date_is_never_invented() {
        let posting = expect_accepted(normalize(&raw_posting("Software Engineer Intern"), now()));
        assert_eq!(posting.posted_at, None);
        assert_eq!(posting.deadline, None);
    }

    #[test]
    fn an_unparseable_posted_date_is_never_invented() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.posted_at_raw = Some("last Tuesday".to_string());
        assert_eq!(expect_accepted(normalize(&raw, now())).posted_at, None);
    }

    // --------------------------------------------------------------------------------------
    // Dates
    // --------------------------------------------------------------------------------------

    #[test]
    fn the_supported_date_formats_all_parse_to_the_same_instant() {
        let reference = parse_timestamp("2026-08-01T00:00:00Z").expect("rfc3339 parses");
        assert_eq!(parse_timestamp("2026-08-01"), Some(reference));
        assert_eq!(parse_timestamp("2026/08/01"), Some(reference));
        assert_eq!(parse_timestamp("2026-08-01T00:00:00"), Some(reference));
        assert_eq!(parse_timestamp("2026-08-01 00:00:00"), Some(reference));

        let seconds = reference.timestamp().to_string();
        assert_eq!(
            seconds.len(),
            10,
            "the ten-digit epoch branch must be exercised"
        );
        assert_eq!(parse_timestamp(&seconds), Some(reference));

        let millis = reference.timestamp_millis().to_string();
        assert_eq!(
            millis.len(),
            13,
            "the thirteen-digit epoch branch must be exercised"
        );
        assert_eq!(parse_timestamp(&millis), Some(reference));
    }

    #[test]
    fn an_offset_timestamp_is_converted_to_utc() {
        assert_eq!(
            parse_timestamp("2026-08-01T00:00:00-04:00"),
            parse_timestamp("2026-08-01T04:00:00Z")
        );
    }

    #[test]
    fn unparseable_and_ambiguous_dates_both_yield_none() {
        assert_eq!(parse_timestamp(""), None);
        assert_eq!(parse_timestamp("last Tuesday"), None);
        assert_eq!(
            parse_timestamp("03/04/2027"),
            None,
            "day-first and month-first are indistinguishable; a 50% silent error rate on \
             deadlines is worse than an unknown one"
        );
    }

    #[test]
    fn stated_dates_reach_the_normalized_posting() {
        let mut raw = raw_posting("Software Engineer Intern");
        raw.posted_at_raw = Some("2026-08-01T00:00:00Z".to_string());
        raw.deadline_raw = Some("2026-11-30".to_string());
        let posting = expect_accepted(normalize(&raw, now()));
        assert_eq!(posting.posted_at, parse_timestamp("2026-08-01T00:00:00Z"));
        assert_eq!(posting.deadline, parse_timestamp("2026-11-30"));
    }

    #[test]
    fn a_past_deadline_is_not_qcs_business() {
        // Expiry is a separate stage that owns the tombstone, so that an applied posting can
        // survive it. Filtering here would delete the row before the tracker ever saw it.
        let mut raw = raw_posting("Software Engineer Intern");
        raw.deadline_raw = Some("2020-01-01".to_string());
        expect_accepted(normalize(&raw, now()));
    }

    // --------------------------------------------------------------------------------------
    // Identity passthrough
    // --------------------------------------------------------------------------------------

    #[test]
    fn source_identity_and_the_raw_record_survive_normalization() {
        let raw = raw_posting("  Software   Engineer Intern  ");
        let posting = expect_accepted(normalize(&raw, now()));
        assert_eq!(posting.source, "greenhouse");
        assert_eq!(posting.external_id, "ext-1");
        assert_eq!(posting.url, "https://example.com/jobs/1");
        assert_eq!(posting.company_name, "Acme Inc.");
        assert_eq!(posting.company_key, "acme");
        assert_eq!(
            posting.title, "Software Engineer Intern",
            "whitespace collapses, spelling does not change"
        );
        assert_eq!(
            posting.raw_json, "{\"id\":1}",
            "a reject with no payload tells you nothing about what went wrong"
        );
    }

    // --------------------------------------------------------------------------------------
    // The conservation invariant
    // --------------------------------------------------------------------------------------

    #[test]
    fn every_raw_posting_produces_exactly_one_outcome() {
        // `fetched = accepted + filtered + rejected`. There is no fourth, silent path, which
        // is the whole reason a scraper that discards half its input cannot look healthy here.
        let mut batch = vec![
            raw_posting("Software Engineer Intern, Summer 2027"),
            raw_posting("Backend Engineering Co-op, Winter 2026"),
            raw_posting("Senior Software Engineer"),
            raw_posting("Marketing Intern"),
            raw_posting("Software Engineer Intern, Summer 2024"),
        ];

        let mut no_company = raw_posting("Software Engineer Intern");
        no_company.company = String::new();
        batch.push(no_company);

        let mut no_title = raw_posting("Software Engineer Intern");
        no_title.title = String::new();
        batch.push(no_title);

        let mut bad_url = raw_posting("Software Engineer Intern");
        bad_url.url = "not-a-url".to_string();
        batch.push(bad_url);

        let fetched = batch.len();
        let mut accepted = 0;
        let mut filtered = 0;
        let mut rejected = 0;
        for raw in &batch {
            match normalize(raw, now()) {
                QcOutcome::Accepted(_) => accepted += 1,
                QcOutcome::Filtered { .. } => filtered += 1,
                QcOutcome::Rejected { .. } => rejected += 1,
            }
        }

        assert_eq!(fetched, 8);
        assert_eq!(accepted, 2);
        assert_eq!(filtered, 3);
        assert_eq!(rejected, 3);
        assert_eq!(
            fetched,
            accepted + filtered + rejected,
            "fetched = accepted + filtered + rejected"
        );

        // Non-vacuity: a function that accepted everything, or rejected everything, would
        // still satisfy the sum above. All three buckets have to be reachable.
        assert!(accepted > 0 && filtered > 0 && rejected > 0);
    }

    #[test]
    fn filtered_and_rejected_are_never_the_same_bucket() {
        // The split is load-bearing: thousands of filtered rows are healthy, while a handful
        // of rejected ones is a defect. Summed into one number the defect is invisible.
        let healthy_exclusion = normalize(&raw_posting("Mechanical Engineering Intern"), now());
        assert!(matches!(healthy_exclusion, QcOutcome::Filtered { .. }));

        let mut defect = raw_posting("Software Engineer Intern");
        defect.company = String::new();
        assert!(matches!(
            normalize(&defect, now()),
            QcOutcome::Rejected { .. }
        ));
    }

    #[test]
    fn reason_codes_are_stable_snake_case_identifiers() {
        // They are matched by SQL in the run-health panel, so a prose reason silently breaks
        // the grouping rather than failing loudly.
        for code in [
            REASON_MISSING_SOURCE,
            REASON_MISSING_EXTERNAL_ID,
            REASON_MISSING_URL,
            REASON_INVALID_URL,
            REASON_MISSING_COMPANY,
            REASON_UNNORMALIZABLE_COMPANY,
            REASON_MISSING_TITLE,
            REASON_NOT_AN_INTERNSHIP,
            REASON_NOT_SOFTWARE,
            REASON_WRONG_TERM,
            REASON_UNPARSEABLE_PAY,
        ] {
            assert!(!code.is_empty());
            assert!(
                code.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{code} is not a machine-readable code"
            );
        }
    }
}
