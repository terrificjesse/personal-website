//! The Gmail agent (Phase 8a–8d). See `apps/hunt-extension/CLAUDE.md`.
//!
//! Reads a burner Gmail, works out which emails are about which application, and advances the
//! application's status. **8a is the read-only half**: connect, sync, record. It writes to our
//! own tables and to nothing else — no Gmail labels, no status changes, no notifications.
//!
//! # The order this is built in is the point
//!
//! *Classification earns write access; it does not start with it.* 8a syncs with a stub
//! classifier and no writes. 8b classifies and matches, still without writing. Only 8c gets to
//! touch a label or a tracker row. A pipeline that can relabel your inbox before anyone has
//! measured whether it classifies correctly is one nobody can safely evaluate.
//!
//! # Layout
//!
//! - [`oauth`] — connecting the account, and keeping the access token fresh.
//! - [`gmail`] — the Gmail API surface actually used. Read-only in this phase, by construction.
//! - [`labels`] — the ONLY module that modifies a mailbox. Adds labels; never removes,
//!   never archives, never touches a disregarded message.
//! - [`sync`] — the pass: fetch, record, count. Owns `inbox_runs`.
//! - [`classify`] — the rules layer.
//! - [`advance`] — matching, and what an email may do to a status. Rules 2 and 3, pure.
//!
//! # Email is untrusted content — rule 1
//!
//! Everything fetched here is written by someone else, and it flows toward a model that sits
//! upstream of a token which will eventually be able to relabel a mailbox. Nothing in this
//! module follows an instruction found in an email, and nothing fetches a URL found in one.
//! When the classifier arrives in 8b it is a pure function — email in, a constrained enum out,
//! no tools — and every write happens in Rust outside the model call.

pub mod advance;
pub mod classify;
pub mod gmail;
pub mod labels;
pub mod oauth;
pub mod sync;
