//! The handoff store and its state machine (§7.4, §8.1, DD-11, DD-12, NFR-12).
//!
//! Every transition is written through to SQLite before it is acknowledged, so an app
//! restart, an interrupted call, a dead server or a detached session all resume from the
//! same state (NFR-12). The queue of undelivered events, the deferral counters, the rounds
//! and the timers of the verifying states live here as well.
//!
//! No `AppHandle` reaches this module: what the UI needs is emitted through the traits of
//! `ui_bridge`, and what the channel needs is returned as data.
// TASK: T-030 (schema, migrations, repositories), T-033 (state machine, outcomes, timers)
