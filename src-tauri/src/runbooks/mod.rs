//! The runbook writer (§7.12, RUN-01..10, DD-18).
//!
//! The app writes `~/.handoff/runbooks/*.json`; the server only reads them (§3.4). What is
//! written is the sequence actually executed, with the values replaced by placeholders —
//! a value never travels into a runbook file, which is the failure this module exists to
//! prevent.
// TASK: T-044
