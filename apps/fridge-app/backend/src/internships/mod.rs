//! The internship tab (Phase 7): collect open SWE internship postings, normalize and dedup
//! them, rank them, track applications, and drop postings once they close.
//!
//! Not a Learning Mode area — including the ranking. See `docs/PLAN.md` § Phase 7.
//!
//! # Layout
//!
//! - [`models`] — the shared type contract between every stage below. Read it first.
//! - [`normalize`] — the QC pass. Every raw posting yields exactly one outcome, so
//!   `fetched = accepted + filtered + rejected` holds and nothing is silently dropped.
//! - [`prestige`] — the company prestige signal: curated tiers over a derived fallback.
//! - [`alerts`] — which newly collected postings are worth a desktop notification. Phase 8e;
//!   it reads `prestige`'s curated tiers and writes `hunt_events`. See `crate::hunt`.
//! - [`rank`] — hard filters and the composite ranking. Its module doc carries the
//!   per-input absent-data policy table; read that before changing any weight.
//! - [`http`] — the shared polite-fetch layer. Every adapter goes through it.
//! - [`sources`] — one trait per source, plus the isolated runner.
//! - [`store`] — translation between SQLite rows and the typed models.
//! - [`collector`] — the coordinator: fetch, QC, dedup, persist, settle, sweep.
//! - [`dedup`] — the merge key. Exact-only by design; fuzzy matching is a reserved seam.
//! - [`expiry`] — the disappearance rule and the sweep. Read its module doc before
//!   touching anything about expiry; the split between its two functions is a safety
//!   property, not an organizational preference.
//!
//! # The two rules that shape everything here
//!
//! 1. **Absent is not zero.** Most sources carry no salary. A posting with unknown pay must
//!    rank as unknown, not as free labour. [`models`] encodes this in the type system and
//!    migration `0012` in CHECK constraints.
//! 2. **Disappearance is not closure.** A source that is blocked, rate-limited or reshaped
//!    makes its postings look closed. Only a run that *actually succeeded* may count a
//!    posting as having vanished, and that rule lives at exactly one write site — see
//!    `posting_sightings.consecutive_misses` in migration `0012`.

pub mod alerts;
pub mod application_events;
pub mod collector;
pub mod dedup;
pub mod expiry;
pub mod http;
pub mod models;
pub mod normalize;
pub mod prestige;
pub mod rank;
pub mod sources;
pub mod store;
