//! Runbooks (§4.5, §7.12, RUN-01..10, DD-18).
//!
//! The app writes `~/.handoff/runbooks/*.json`; the server only reads them (§3.4). What is
//! written is the sequence actually executed, with the values replaced by placeholders — a
//! value never travels into a runbook file, which is the failure the writer exists to
//! prevent.
//!
//! [`matching`] is here, because the rule of §4.5.3 is shared with the server and is
//! checked against the same fixtures. The writer, the placeholder substitution and the
//! update proposals are not.
// TASK: T-044 — runbook writer, placeholder substitution, update proposals, failed marks.

pub mod matching;
