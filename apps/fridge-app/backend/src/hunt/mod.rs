//! The hunt alert channel (Phase 8e). See `apps/hunt-extension/CLAUDE.md`.
//!
//! One table, two producers, one poll endpoint, one notification path. This module owns the
//! table and nothing else: it knows what an event *is*, not what is worth alerting about.
//! That judgement belongs to whoever produces the event —
//! [`crate::internships::alerts`] for postings, `inbox` for email once 8d lands — because
//! only the producer has the context to make it.
//!
//! # Why the table and not each producer holds the dedup state
//!
//! The consumer is a Firefox MV3 background page, which the browser kills and restarts
//! whenever it likes. Everything it remembered is gone on the next wake, so any dedup it
//! keeps in memory re-fires every alert; `browser.storage.local` is only marginally better,
//! being per-profile, clearable, and empty in a fresh one. `hunt_events.acked_at` is
//! therefore the record, and the client's storage is a cache. That is rule 6, and it is the
//! reason this is a table rather than a queue in the extension.

pub mod events;
pub mod profile;
pub mod tokens;
