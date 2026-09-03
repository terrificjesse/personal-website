//! Source adapters: one trait, a registry, and the orchestration that runs them.
//!
//! Adding a source means writing one file here and adding one line to [`registry`]. Nothing
//! else in the pipeline changes — [`collect_all`] iterates the registry and knows nothing about
//! any particular source.
//!
//! # Per-source isolation is the architecture, not the error handling
//!
//! One source being blocked, rate-limited, reshaped, or returning garbage must never fail the
//! run or reduce what the other sources produced. Three mechanisms enforce that, in increasing
//! order of paranoia:
//!
//! 1. [`Source::fetch`] cannot return an error. Its return type is [`SourceFetch`], which is
//!    *always* a recorded outcome. There is no `?` for an adapter to propagate out of, so
//!    "this source failed" and "this source succeeded" travel through the same channel and the
//!    runner treats them as equally normal.
//! 2. Every adapter runs in its own [`tokio::task`]. A **panic** unwinds into that task's
//!    `JoinHandle` and comes back as an `Err`, which [`collect_all`] records as
//!    [`SourceOutcome::Failed`]. A panicking adapter therefore cannot take down the run — which
//!    it could if this were a plain `join!` of futures on one task, however carefully each
//!    adapter were written.
//! 3. Nothing here touches the database. [`collect_all`] returns values; the coordinator
//!    persists them. A source cannot corrupt state it has no handle to.
//!
//! # Getting the outcome right is the most consequential thing in this module
//!
//! [`SourceOutcome`] is what stands between a broken fetch and a mass expiry — see
//! `expiry.rs`'s module doc, and `docs/INTERNSHIP_SCRAPING.md` § D.3. Only
//! [`SourceOutcome::Success`] can ever expire postings, so **`Success` is a claim that the
//! enumeration was complete**, not a claim that nothing went wrong.
//!
//! | outcome | the claim it makes | what may conclude from absence |
//! |---|---|---|
//! | `Success` | I enumerated **everything** this source currently offers | absence is evidence of closure |
//! | `Partial` | I got some of it and stopped | nothing |
//! | `Failed` | I got nothing usable | nothing |
//! | `Skipped` | I deliberately did not fetch | nothing |
//!
//! Three rules follow, and each is pinned by a test:
//!
//! - **A multi-board source is `Success` only if every board succeeded.** 484 good boards and
//!   one 500 is `Partial`: the postings on the failed board are not gone, they are unobserved.
//! - **A budget that truncates the work is `Partial`**, even though every fetch it did make
//!   succeeded. Stopping early on purpose is still stopping early.
//! - **A truncated feed can never be `Success`.** WeWorkRemotely publishes the 25 most recent
//!   items and nothing older, so a complete enumeration is not available at any price and
//!   absence from it means nothing at all.
//!
//! # Scopes: the same rule at a finer grain
//!
//! Those three rules are about one verdict for a whole source, and for a source that is one
//! endpoint that is the right shape. Greenhouse is 485 endpoints under one name, and the first
//! rule then means a single unreachable board disqualifies the other 484 — which on the
//! 2026-09-02 uncapped run is exactly what happened.
//!
//! A source may therefore also report [`ScopeRun`]s: per-scope verdicts, attached with
//! [`SourceFetch::with_scopes`], where a scope is a sub-unit it can enumerate completely on its
//! own. The source-level outcome is unchanged and still means what it always meant — scopes are
//! additive, and a source that reports none behaves exactly as it did before.
//!
//! **All three board sources report scopes as of 12r** — Greenhouse (485 boards), Lever (157)
//! and Ashby (297). Each needed only its adapter half; the rest of the mechanism was already
//! source-agnostic, which is the claim 12i made and 12r is the test of it. What remains
//! unscoped is every source that genuinely is one endpoint: Simplify, vanshb03,
//! weworkremotely, and the three best-effort adapters.

pub mod ashby;
pub mod best_effort;
pub mod greenhouse;
pub mod lever;
pub mod rss;
pub mod simplify;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Deserialize;

use super::expiry::SourceRunResult;
use super::http::PoliteClient;
use super::models::{RawPosting, ScopeRun, SourceOutcome};

// ------------------------------------------------------------------------------------------
// The result of one source's run
// ------------------------------------------------------------------------------------------

/// What one adapter produced.
///
/// The fields are private and the only way in is one of the four constructors, so the
/// invariant migration `0012` states in prose — *"`error` is non-NULL whenever outcome is
/// 'failed' or 'skipped'"* — cannot be violated by a struct literal that forgot to set it. A
/// failure with no stated reason is indistinguishable from a success in a health panel, which
/// is the failure mode the whole run-record exists to prevent.
#[derive(Debug, Clone)]
pub struct SourceFetch {
    outcome: SourceOutcome,
    postings: Vec<RawPosting>,
    closed_external_ids: Vec<String>,
    error: Option<String>,
    scopes: Vec<ScopeRun>,
}

impl SourceFetch {
    /// The adapter enumerated **everything** the source currently offers.
    ///
    /// Only call this when that is literally true. It is the one outcome that grants the run
    /// permission to expire postings, so an optimistic `Success` on an incomplete fetch is how
    /// this phase loses data.
    pub fn success(postings: Vec<RawPosting>) -> Self {
        SourceFetch {
            outcome: SourceOutcome::Success,
            postings,
            closed_external_ids: Vec::new(),
            error: None,
            scopes: Vec::new(),
        }
    }

    /// The adapter got part of the way and stopped. The postings are real; absence proves
    /// nothing. `reason` says *where* it stopped, because "partial" with no boundary is not a
    /// diagnosable state.
    pub fn partial(postings: Vec<RawPosting>, reason: impl Into<String>) -> Self {
        SourceFetch {
            outcome: SourceOutcome::Partial,
            postings,
            closed_external_ids: Vec::new(),
            error: Some(reason.into()),
            scopes: Vec::new(),
        }
    }

    /// Nothing usable: blocked, rate-limited, reshaped, unparseable, timed out.
    pub fn failed(reason: impl Into<String>) -> Self {
        SourceFetch {
            outcome: SourceOutcome::Failed,
            postings: Vec::new(),
            closed_external_ids: Vec::new(),
            error: Some(reason.into()),
            scopes: Vec::new(),
        }
    }

    /// Deliberately not fetched — `robots.txt` disallowed it, or it is switched off. A correct
    /// outcome, and the health panel must not paint it as a failure.
    pub fn skipped(reason: impl Into<String>) -> Self {
        SourceFetch {
            outcome: SourceOutcome::Skipped,
            postings: Vec::new(),
            closed_external_ids: Vec::new(),
            error: Some(reason.into()),
            scopes: Vec::new(),
        }
    }

    /// Attach ids this source states outright are closed.
    ///
    /// `docs/INTERNSHIP_SCRAPING.md` § D.1: an explicit `active: false` is the strongest
    /// closure evidence available anywhere, and it is far better than waiting for three
    /// consecutive misses. [`RawPosting`] has no field for it — it describes a *live* listing —
    /// so it travels beside the postings rather than inside them, and the coordinator may
    /// expire these with `ExpiryReason::SourceMarkedClosed`.
    pub fn with_closed_ids(mut self, ids: Vec<String>) -> Self {
        self.closed_external_ids = ids;
        self
    }

    /// Declare what each of this source's scopes did — see [`ScopeRun`].
    ///
    /// Additive: the overall [`SourceOutcome`] is unchanged and still means what it always
    /// meant. A source that reports scopes is saying "my verdict is answerable at a finer
    /// grain than one row", and `expiry::settle_source_run` will then advance disappearance
    /// counters for the completed scopes even when the source as a whole is `Partial`.
    ///
    /// **The postings and the scopes must come from the same pass.** A scope reported
    /// `Completed` whose ids are missing from `external_ids` gets its whole board incremented
    /// and nothing reset, which is the one way this mechanism loses data. Build both from one
    /// parse, never from two.
    pub fn with_scopes(mut self, scopes: Vec<ScopeRun>) -> Self {
        self.scopes = scopes;
        self
    }

    pub fn outcome(&self) -> SourceOutcome {
        self.outcome
    }

    pub fn postings(&self) -> &[RawPosting] {
        &self.postings
    }

    pub fn closed_external_ids(&self) -> &[String] {
        &self.closed_external_ids
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn scopes(&self) -> &[ScopeRun] {
        &self.scopes
    }
}

/// One source's contribution to a run: the record the coordinator persists, plus the payload
/// it persists alongside it.
///
/// Separate from [`SourceRunResult`] because that type is the *database's* view and carries
/// only counts. This carries the rows too, since nothing here writes them.
#[derive(Debug, Clone)]
pub struct SourceRunOutput {
    /// Hand this to `expiry::settle_source_run`. **`accepted`/`filtered`/`rejected` are 0** —
    /// QC has not run yet, and this module does not run it. The coordinator fills them in
    /// after `normalize::normalize`, which is what makes
    /// `fetched = accepted + filtered + rejected` hold.
    pub result: SourceRunResult,
    /// Everything the source returned, unfiltered. Adapters deliberately do **not** drop
    /// non-internship rows: `filtered` is a health signal only if the denominator is honest,
    /// and six adapters each re-implementing the title-matching trap from
    /// `docs/INTERNSHIP_SCRAPING.md` § C is six places to get it wrong instead of one.
    pub postings: Vec<RawPosting>,
    /// Ids this source says are closed outright. See [`SourceFetch::with_closed_ids`].
    pub closed_external_ids: Vec<String>,
}

// ------------------------------------------------------------------------------------------
// The trait
// ------------------------------------------------------------------------------------------

/// A boxed future. Spelled out rather than pulled from `futures`, which is not a dependency.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One collectable source.
///
/// `fetch` returns [`SourceFetch`] rather than `Result`, which is the isolation rule expressed
/// in the type system: an adapter has no way to signal failure except by *recording* it, so
/// there is no path by which one source's problem becomes the run's problem.
///
/// Boxed futures rather than `async fn` because the registry stores `Arc<dyn Source>` and an
/// `async fn` in a trait is not dyn-compatible.
pub trait Source: Send + Sync {
    /// Stable identifier, written to `source_runs.source` and to every [`RawPosting::source`]
    /// this adapter emits. **Changing it orphans every sighting the old name accumulated**, so
    /// treat it as a database key rather than a label.
    fn name(&self) -> &str;

    /// One line for the run-health panel: what this source is and what to expect from it.
    fn description(&self) -> &str;

    fn fetch<'a>(&'a self, ctx: &'a SourceContext) -> BoxFuture<'a, SourceFetch>;
}

// ------------------------------------------------------------------------------------------
// Context
// ------------------------------------------------------------------------------------------

/// Everything an adapter is allowed to reach.
///
/// Note what is **not** here: no database pool, no `reqwest::Client`, no writable state. An
/// adapter can fetch politely and return values, and that is the whole of its authority.
#[derive(Debug, Clone)]
pub struct SourceContext {
    pub http: PoliteClient,
    /// Which ATS boards to poll. Stable across runs on purpose — see [`BoardDirectory`].
    pub boards: BoardDirectory,
    /// Ceiling on boards fetched per multi-board source in one run.
    ///
    /// Defaults to no cap. A cap makes the run finish sooner and makes the enumeration
    /// incomplete, so a capped source reports [`SourceOutcome::Partial`] **forever**.
    ///
    /// What that costs changed with migration 0026. For an unscoped source it is still total:
    /// `Partial` advances no counters, so a capped Lever or Ashby can never expire anything.
    /// For a **scoped** source it is now proportional — the boards inside the cap are genuinely
    /// complete enumerations and do advance counters; the boards beyond it have no verdict and
    /// are left alone. A capped Greenhouse therefore expires within the slice it polled, which
    /// is correct, and is worth knowing before setting a cap and assuming nothing can move.
    pub max_boards_per_run: usize,
    /// Sources switched off by configuration. They still produce a `source_runs` row, with
    /// [`SourceOutcome::Skipped`] — a source that vanishes from the health panel looks like a
    /// source nobody noticed breaking.
    pub disabled_sources: Vec<String>,
    /// Specific Handshake `/public/jobs/{id}` URLs to enrich this run.
    ///
    /// Handshake is **never swept** — see `best_effort`'s module doc — so it fetches exactly
    /// what is listed here and nothing else. Empty is the normal case and yields
    /// [`SourceOutcome::Skipped`]. A non-empty run yields `Partial`, never `Success`, because
    /// fetching known URLs is not an enumeration of the source.
    pub handshake_urls: Vec<String>,
}

impl SourceContext {
    pub fn new(http: PoliteClient) -> Self {
        SourceContext {
            http,
            boards: BoardDirectory::vendored(),
            max_boards_per_run: usize::MAX,
            disabled_sources: Vec::new(),
            handshake_urls: Vec::new(),
        }
    }

    pub fn is_disabled(&self, name: &str) -> bool {
        self.disabled_sources.iter().any(|source| source == name)
    }
}

// ------------------------------------------------------------------------------------------
// The board directory
// ------------------------------------------------------------------------------------------

/// Which (ATS, board slug) pairs to poll, keyed by ATS name.
///
/// You do not guess a board slug — `AECOM2`, `3SBusinessCorporationInc1` — you harvest it.
/// `docs/INTERNSHIP_SCRAPING.md` § A.2: Simplify's `url` field *is* a board-slug directory,
/// and extracting it costs nothing because the file is already downloaded.
/// [`simplify::extract_board_slugs`] does the extraction; `data/internships/board-slugs.json`
/// is a committed snapshot of its output so the very first run has boards to poll.
///
/// **The list must be stable across runs.** Dropping a slug stops polling its board, every
/// posting on it goes unobserved, and after the miss threshold they all expire together —
/// indistinguishable from the board genuinely closing. Grow this file; do not prune it on the
/// strength of one quiet run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct BoardDirectory {
    by_ats: BTreeMap<String, Vec<String>>,
}

/// The committed snapshot. See `data/internships/README.md` for provenance.
const VENDORED_BOARDS: &str = include_str!("../../../data/internships/board-slugs.json");

impl BoardDirectory {
    /// The committed snapshot, or an empty directory if it will not parse.
    ///
    /// Deliberately not a panic: a malformed data file must degrade the ATS sources to
    /// "nothing to poll" (which they report honestly) rather than prevent the process from
    /// starting and take the fridge app down with it.
    pub fn vendored() -> Self {
        match serde_json::from_str(VENDORED_BOARDS) {
            Ok(directory) => directory,
            Err(error) => {
                eprintln!(
                    "internships: data/internships/board-slugs.json did not parse ({error}); \
                     ATS sources will have no boards to poll"
                );
                BoardDirectory::default()
            }
        }
    }

    pub fn from_map(by_ats: BTreeMap<String, Vec<String>>) -> Self {
        BoardDirectory { by_ats }
    }

    /// Slugs for one ATS, in a stable order.
    pub fn slugs(&self, ats: &str) -> &[String] {
        self.by_ats.get(ats).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn is_empty(&self) -> bool {
        self.by_ats.values().all(Vec::is_empty)
    }

    /// Merge freshly discovered slugs in. Additive only — see the type's doc.
    pub fn merge(&mut self, other: &BoardDirectory) {
        for (ats, slugs) in &other.by_ats {
            let existing = self.by_ats.entry(ats.clone()).or_default();
            for slug in slugs {
                if !existing.contains(slug) {
                    existing.push(slug.clone());
                }
            }
            existing.sort();
        }
    }
}

// ------------------------------------------------------------------------------------------
// The registry
// ------------------------------------------------------------------------------------------

/// Every source, in the collection order `docs/INTERNSHIP_SCRAPING.md` § G recommends:
/// coverage-per-effort first, expensive and thin last.
///
/// Adding a source is one line here plus one file in this directory. [`collect_all`] does not
/// change, and neither does the coordinator.
pub fn registry() -> Vec<Arc<dyn Source>> {
    vec![
        // 1 — one conditional GET, ~1,900 active listings, term + degree + an explicit
        //     `active` closure flag. Nothing else is close on effort-to-coverage.
        Arc::new(simplify::SimplifySource::simplify_jobs()),
        // 3 — best pay data anywhere (explicit interval) and `employmentType == "Intern"`.
        Arc::new(ashby::AshbySource::new()),
        // 4 — largest slug count after Workday; pay via `pay_transparency=true`.
        Arc::new(greenhouse::GreenhouseSource::new()),
        // 5 — structured `salaryRange`; whole board per request.
        Arc::new(lever::LeverSource::new()),
        // 6 — 29% URL overlap with Simplify, so ~285 unique listings, and MIT-licensed.
        Arc::new(simplify::SimplifySource::vanshb03()),
        // 8 — cheap, but truncated to 25 items, so it can never be a complete enumeration.
        Arc::new(rss::RssSource::we_work_remotely()),
        // — not built, and the reason is recorded rather than left looking like a bug.
        Arc::new(best_effort::BestEffortSource::linkedin()),
        Arc::new(best_effort::BestEffortSource::indeed()),
        Arc::new(best_effort::BestEffortSource::handshake()),
    ]
}

// ------------------------------------------------------------------------------------------
// Orchestration
// ------------------------------------------------------------------------------------------

/// Run every source with per-source isolation and return what each produced.
///
/// **Pure with respect to the database.** No `sqlx`, no pool, no writes. The coordinator takes
/// these values and persists them, because everything that touches the database is where this
/// phase's data-loss bugs live and concentrating it in one owner is the point.
///
/// Sources run concurrently. Politeness is not sacrificed to that: the per-host rate limiter
/// inside [`PoliteClient`] is shared by every source, so two adapters that happen to hit the
/// same host still queue behind each other, while adapters on different hosts genuinely
/// overlap.
///
/// Results come back in registry order regardless of which source finished first, so two
/// identical runs produce identical output.
pub async fn collect_all(sources: Vec<Arc<dyn Source>>, ctx: Arc<SourceContext>) -> Vec<SourceRunOutput> {
    let mut receiver = collect_streaming(sources, ctx);
    let mut indexed = Vec::new();
    while let Some(item) = receiver.recv().await {
        indexed.push(item);
    }
    // Restore registry order. Completion order is whatever the network decided, and two
    // identical runs must produce identical output.
    indexed.sort_by_key(|(index, _)| *index);
    indexed.into_iter().map(|(_, output)| output).collect()
}

/// Like [`collect_all`], but hands each source's result over **the moment that source
/// finishes** rather than after the slowest one does.
///
/// This is what the coordinator uses, and the reason is latency, not elegance. An uncapped run
/// polls ~2,084 boards and takes on the order of half an hour; batching meant the database
/// stayed empty for that entire time and then gained everything at once. Simplify alone
/// answers in seconds and supplies most of the corpus, so streaming turns "nothing for thirty
/// minutes" into "most of it almost immediately". It also means one slow or hanging source
/// cannot delay every other source's data from landing.
///
/// The item is `(registry_index, output)` so [`collect_all`] can restore deterministic order;
/// callers that persist as they go can ignore the index.
///
/// **Every source still produces exactly one item**, panics included — the guarantee the
/// run-health panel depends on. Each source is awaited inside a supervising task that converts
/// a panicked `JoinHandle` into a recorded failure before sending, so an adapter that panics
/// cannot simply go missing from the channel.
pub fn collect_streaming(
    sources: Vec<Arc<dyn Source>>,
    ctx: Arc<SourceContext>,
) -> tokio::sync::mpsc::Receiver<(usize, SourceRunOutput)> {
    // Capacity is the whole registry, so no adapter ever blocks on a slow consumer.
    let (sender, receiver) = tokio::sync::mpsc::channel(sources.len().max(1));

    for (index, source) in sources.into_iter().enumerate() {
        let ctx = Arc::clone(&ctx);
        let sender = sender.clone();
        let name = source.name().to_string();
        let task_name = name.clone();

        tokio::task::spawn(async move {
            // Inner task so a panicking adapter lands in a `JoinHandle` rather than taking
            // this supervisor — and therefore its `send` — down with it.
            let inner = tokio::task::spawn(async move {
                if ctx.is_disabled(&task_name) {
                    return SourceFetch::skipped("disabled by configuration");
                }
                source.fetch(&ctx).await
            });

            let fetch = match inner.await {
                Ok(fetch) => fetch,
                Err(join_error) => {
                    // A panicking adapter is a bug, and the run keeps going anyway. Recorded
                    // as a failure so it is visible, and *not* as a success so it cannot
                    // expire anything on its way out.
                    let detail = panic_detail(join_error);
                    eprintln!("internships: source {name} panicked: {detail}");
                    SourceFetch::failed(format!("adapter panicked: {detail}"))
                }
            };

            // A send failure means the coordinator went away; nothing useful to do about it.
            let _ = sender.send((index, into_output(&name, fetch))).await;
        });
    }

    receiver
}

/// Turn one adapter's [`SourceFetch`] into the record the coordinator persists.
fn into_output(name: &str, fetch: SourceFetch) -> SourceRunOutput {
    let outcome = fetch.outcome();
    let fetched = fetch.postings().len() as i64;

    // Populated **only** on `Success`, and used only by the unscoped settle path.
    // `expiry::settle_source_run` documents this field as meaningful only then, and leaving it
    // empty otherwise means that if a future edit ever let a partial run advance counters, the
    // blanket-increment-then-reset would find no ids to reset and expire everything — a loud,
    // immediate failure instead of a silent wrong answer. The postings themselves are still
    // returned, so nothing is lost.
    //
    // Migration 0026 is that future edit, arriving on purpose: a scoped source's partial run
    // now does advance counters. It does not weaken this, because the scoped path never reads
    // this field. It reads `ScopeRun::external_ids` instead, and the equivalent safety net
    // there is structural rather than incidental — a scope's verdict and its ids are built in
    // the same pass, so "completed with no ids" is not a state an adapter reaches by
    // forgetting something. See `ScopeRun` and `greenhouse::fetch`.
    let seen_external_ids = if outcome == SourceOutcome::Success {
        fetch
            .postings()
            .iter()
            .map(|posting| posting.external_id.clone())
            .collect()
    } else {
        Vec::new()
    };

    let scopes = fetch.scopes().to_vec();

    let result = SourceRunResult {
        source: name.to_string(),
        outcome,
        seen_external_ids,
        scopes,
        fetched,
        // QC has not run. The coordinator sets these three after `normalize::normalize`.
        accepted: 0,
        filtered: 0,
        rejected: 0,
        error: fetch.error().map(str::to_string),
    };

    SourceRunOutput {
        result,
        postings: fetch.postings.clone(),
        closed_external_ids: fetch.closed_external_ids.clone(),
    }
}

fn panic_detail(error: tokio::task::JoinError) -> String {
    if error.is_cancelled() {
        return "task cancelled".to_string();
    }
    match error.try_into_panic() {
        Ok(payload) => {
            if let Some(message) = payload.downcast_ref::<&str>() {
                (*message).to_string()
            } else if let Some(message) = payload.downcast_ref::<String>() {
                message.clone()
            } else {
                "panicked with a non-string payload".to_string()
            }
        }
        Err(error) => error.to_string(),
    }
}

// ------------------------------------------------------------------------------------------
// Shared helpers for adapters
// ------------------------------------------------------------------------------------------

/// Read the first present, non-blank string among several JSON keys.
///
/// Adapters use this rather than a rigid `#[derive(Deserialize)]` struct for two reasons.
/// First, `docs/INTERNSHIP_SCRAPING.md` establishes what each source *provides* without always
/// naming the key (it records that Greenhouse has a posted date, not which of `updated_at` and
/// `first_published` carries it), and probing a short list is honest where inventing one name
/// is not. Second, a struct that fails to deserialize loses the whole board over one reshaped
/// field, whereas a probe loses one field and keeps the posting.
pub(crate) fn first_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        match value.get(key) {
            Some(serde_json::Value::String(text)) if !text.trim().is_empty() => {
                return Some(text.trim().to_string());
            }
            Some(serde_json::Value::Number(number)) => return Some(number.to_string()),
            _ => {}
        }
    }
    None
}

/// Read an identifier that may be a JSON string or a JSON number.
///
/// Greenhouse ids are numbers (`8403127002`); Lever and Ashby ids are uuid strings. Both are
/// `external_id`, which is TEXT.
pub(crate) fn id_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    first_string(value, keys)
}

/// Join non-blank strings for a `location_raw` field.
pub(crate) fn join_locations(parts: impl IntoIterator<Item = String>) -> Option<String> {
    let joined: Vec<String> = parts
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect();
    if joined.is_empty() {
        None
    } else {
        Some(joined.join("; "))
    }
}

// ------------------------------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in adapter whose outcome the test dictates.
    struct Fake {
        name: String,
        behaviour: Behaviour,
    }

    enum Behaviour {
        Yields(usize),
        Fails,
        Panics,
    }

    impl Source for Fake {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "test double"
        }
        fn fetch<'a>(&'a self, _ctx: &'a SourceContext) -> BoxFuture<'a, SourceFetch> {
            Box::pin(async move {
                match self.behaviour {
                    Behaviour::Yields(count) => SourceFetch::success(
                        (0..count).map(|i| posting(&self.name, i)).collect(),
                    ),
                    Behaviour::Fails => SourceFetch::failed("blocked by the test"),
                    Behaviour::Panics => panic!("this adapter is broken"),
                }
            })
        }
    }

    fn posting(source: &str, index: usize) -> RawPosting {
        RawPosting {
            source: source.to_string(),
            external_id: format!("{source}-{index}"),
            url: format!("https://example.test/{source}/{index}"),
            company: "Acme".to_string(),
            title: "Software Engineer Intern".to_string(),
            location_raw: None,
            pay_raw: None,
            term_raw: None,
            class_year_raw: None,
            posted_at_raw: None,
            deadline_raw: None,
            description: None,
            remote_hint: None,
            raw_json: "{}".to_string(),
        }
    }

    fn test_context() -> Arc<SourceContext> {
        let http = PoliteClient::with_host_delay(std::time::Duration::ZERO)
            .expect("the client builds");
        Arc::new(SourceContext::new(http))
    }

    // ---- isolation ----

    #[tokio::test]
    async fn one_failing_source_does_not_reduce_what_the_others_produced() {
        let sources: Vec<Arc<dyn Source>> = vec![
            Arc::new(Fake { name: "good".into(), behaviour: Behaviour::Yields(3) }),
            Arc::new(Fake { name: "bad".into(), behaviour: Behaviour::Fails }),
            Arc::new(Fake { name: "also_good".into(), behaviour: Behaviour::Yields(2) }),
        ];

        let outputs = collect_all(sources, test_context()).await;

        assert_eq!(outputs.len(), 3);
        assert_eq!(outputs[0].result.outcome, SourceOutcome::Success);
        assert_eq!(outputs[0].postings.len(), 3);
        assert_eq!(outputs[1].result.outcome, SourceOutcome::Failed);
        assert_eq!(outputs[2].result.outcome, SourceOutcome::Success);
        assert_eq!(outputs[2].postings.len(), 2);
    }

    #[tokio::test]
    async fn a_panicking_adapter_is_recorded_as_a_failure_and_the_run_continues() {
        // Isolation enforced structurally rather than by convention: no amount of care in an
        // adapter can guarantee it never panics, so the runner survives one that does.
        let sources: Vec<Arc<dyn Source>> = vec![
            Arc::new(Fake { name: "exploding".into(), behaviour: Behaviour::Panics }),
            Arc::new(Fake { name: "fine".into(), behaviour: Behaviour::Yields(4) }),
        ];

        let outputs = collect_all(sources, test_context()).await;

        assert_eq!(outputs[0].result.outcome, SourceOutcome::Failed);
        assert!(
            outputs[0]
                .result
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("panicked"),
            "the panic must be legible in the run record, not merely counted"
        );
        assert_eq!(outputs[1].result.outcome, SourceOutcome::Success);
        assert_eq!(outputs[1].postings.len(), 4);
    }

    #[tokio::test]
    async fn a_disabled_source_still_produces_a_run_record() {
        // A source that simply vanishes from the health panel looks like a source nobody
        // noticed breaking.
        let http = PoliteClient::with_host_delay(std::time::Duration::ZERO).expect("builds");
        let mut ctx = SourceContext::new(http);
        ctx.disabled_sources = vec!["off".to_string()];

        let sources: Vec<Arc<dyn Source>> =
            vec![Arc::new(Fake { name: "off".into(), behaviour: Behaviour::Yields(9) })];
        let outputs = collect_all(sources, Arc::new(ctx)).await;

        assert_eq!(outputs[0].result.outcome, SourceOutcome::Skipped);
        assert_eq!(outputs[0].result.fetched, 0);
        assert!(outputs[0].result.error.is_some());
    }

    #[tokio::test]
    async fn results_come_back_in_registry_order_not_completion_order() {
        let sources: Vec<Arc<dyn Source>> = vec![
            Arc::new(Fake { name: "a".into(), behaviour: Behaviour::Yields(1) }),
            Arc::new(Fake { name: "b".into(), behaviour: Behaviour::Yields(1) }),
            Arc::new(Fake { name: "c".into(), behaviour: Behaviour::Yields(1) }),
        ];
        let outputs = collect_all(sources, test_context()).await;
        let names: Vec<&str> = outputs.iter().map(|o| o.result.source.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    // ---- the outcome contract ----

    #[test]
    fn only_a_successful_run_reports_the_ids_it_saw() {
        // `settle_source_run` resets `consecutive_misses` for exactly these ids after a blanket
        // increment. Reporting them from a partial run would say "everything I did not reach
        // is missing", which is the phase's named data-loss bug.
        let partial = SourceFetch::partial(vec![posting("s", 0)], "stopped at board 4 of 30");
        let output = into_output("s", partial);
        assert!(output.result.seen_external_ids.is_empty());
        assert_eq!(output.result.fetched, 1, "the posting itself is still kept");
        assert_eq!(output.postings.len(), 1);

        let success = SourceFetch::success(vec![posting("s", 0)]);
        let output = into_output("s", success);
        assert_eq!(output.result.seen_external_ids, vec!["s-0".to_string()]);
    }

    #[test]
    fn a_failure_always_carries_a_reason() {
        // Migration 0012: "Non-NULL whenever outcome is 'failed' or 'skipped'". The
        // constructors are the only way to build a `SourceFetch`, so this holds by
        // construction rather than by review.
        assert!(SourceFetch::failed("x").error().is_some());
        assert!(SourceFetch::skipped("y").error().is_some());
        assert!(SourceFetch::partial(vec![], "z").error().is_some());
        assert!(SourceFetch::success(vec![]).error().is_none());
    }

    #[test]
    fn explicitly_closed_ids_travel_beside_the_postings() {
        let fetch = SourceFetch::success(vec![posting("simplify", 0)])
            .with_closed_ids(vec!["dead-1".to_string(), "dead-2".to_string()]);
        let output = into_output("simplify", fetch);
        assert_eq!(output.closed_external_ids.len(), 2);
        assert_eq!(
            output.result.fetched, 1,
            "closed ids are not fetched postings and must not inflate the count"
        );
    }

    #[test]
    fn qc_counts_start_at_zero_because_qc_has_not_run() {
        let output = into_output("s", SourceFetch::success(vec![posting("s", 0)]));
        assert_eq!(output.result.accepted, 0);
        assert_eq!(output.result.filtered, 0);
        assert_eq!(output.result.rejected, 0);
    }

    // ---- the registry ----

    #[test]
    fn every_registered_source_has_a_distinct_name() {
        // `source_runs` is UNIQUE (run_id, source): two sources sharing a name is a database
        // error at runtime and a silently merged sighting history before that.
        let registry = registry();
        let mut names: Vec<&str> = registry.iter().map(|s| s.name()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "source names must be unique");
    }

    #[test]
    fn every_registered_source_covers_a_class_from_the_research_doc() {
        let registry = registry();
        let names: Vec<&str> = registry.iter().map(|s| s.name()).collect();
        for expected in [
            // ATS public JSON APIs
            "greenhouse",
            "lever",
            "ashby",
            // GitHub internship-list repos
            "simplify",
            "vanshb03",
            // RSS / JSON feeds
            "weworkremotely",
            // best effort
            "linkedin",
            "indeed",
            "handshake",
        ] {
            assert!(names.contains(&expected), "{expected} is missing from the registry");
        }
    }

    // ---- the board directory ----

    #[test]
    fn the_vendored_board_directory_parses_and_is_not_empty() {
        let boards = BoardDirectory::vendored();
        assert!(!boards.is_empty());
        // The counts the research doc measured, allowing for the file having moved on. If
        // these collapse, the extraction or the snapshot is broken.
        assert!(boards.slugs("greenhouse").len() > 300, "greenhouse slugs look wrong");
        assert!(boards.slugs("ashby").len() > 200, "ashby slugs look wrong");
        assert!(boards.slugs("lever").len() > 100, "lever slugs look wrong");
    }

    #[test]
    fn merging_a_directory_only_ever_adds() {
        // Dropping a slug stops polling its board, and every posting on it then expires
        // together after the miss threshold — indistinguishable from a real closure.
        let mut base = BoardDirectory::from_map(BTreeMap::from([(
            "greenhouse".to_string(),
            vec!["airtable".to_string()],
        )]));
        let discovered = BoardDirectory::from_map(BTreeMap::from([(
            "greenhouse".to_string(),
            vec!["anthropic".to_string()],
        )]));
        base.merge(&discovered);
        assert_eq!(
            base.slugs("greenhouse"),
            ["airtable".to_string(), "anthropic".to_string()]
        );
    }

    #[test]
    fn an_unknown_ats_has_no_slugs_rather_than_panicking() {
        assert!(BoardDirectory::vendored().slugs("no-such-ats").is_empty());
    }

    // ---- the structural rule ----

    #[test]
    fn adapters_do_not_build_their_own_http_client() {
        // Politeness is a property of the process, not of a request: the per-host limiter only
        // limits anything if every request queues behind the same one, and robots.txt only
        // constrains anything if every request consults the same cache. A second client
        // anywhere in this directory silently opts out of both while looking like code that
        // opted in — so this is checked rather than trusted.
        //
        // `include_str!` reads these at compile time, so the check needs no filesystem and
        // runs in CI exactly as it runs here.
        let adapters: &[(&str, &str)] = &[
            ("simplify.rs", include_str!("simplify.rs")),
            ("greenhouse.rs", include_str!("greenhouse.rs")),
            ("lever.rs", include_str!("lever.rs")),
            ("ashby.rs", include_str!("ashby.rs")),
            ("rss.rs", include_str!("rss.rs")),
            ("best_effort.rs", include_str!("best_effort.rs")),
        ];

        // Spelled in pieces so this test's own source does not match the needle it looks for.
        let needle = concat!("req", "west");
        for (file, source) in adapters {
            assert!(
                !source.contains(needle),
                "{file} names the HTTP client crate directly; every fetch must go through \
                 internships::http::PoliteClient"
            );
        }
    }

    // ---- helpers ----

    #[test]
    fn a_probe_skips_absent_and_blank_keys() {
        let value = serde_json::json!({ "a": "", "b": "   ", "c": "found" });
        assert_eq!(first_string(&value, &["a", "b", "c"]), Some("found".to_string()));
        assert_eq!(first_string(&value, &["missing"]), None);
    }

    #[test]
    fn a_numeric_id_reads_as_text() {
        // Greenhouse ids are JSON numbers; `external_id` is TEXT.
        let value = serde_json::json!({ "id": 8403127002u64 });
        assert_eq!(id_string(&value, &["id"]), Some("8403127002".to_string()));
    }

    #[test]
    fn joining_locations_drops_blanks_and_yields_none_when_empty() {
        assert_eq!(
            join_locations(vec!["Chicago, IL".to_string(), "  ".to_string(), "NYC".to_string()]),
            Some("Chicago, IL; NYC".to_string())
        );
        assert_eq!(join_locations(vec!["".to_string()]), None);
    }
}
