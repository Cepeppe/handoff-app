//! The Stop-hook decision (§7.5, SRV-10..13, F-10, ADPT-08, DD-25).
//!
//! The hook connects with role `hook`, gets at most 2 s, and is answered with a decision:
//! continue, or block once with the reason. What decides it is the state of the handoffs
//! of that session — an active handoff with an undelivered question or screenshot and no
//! attached call blocks once (DD-25, OI-10 accepted in T-001 D4) — together with the
//! per-session counter that keeps a run from being blocked twice for the same thing.
//!
//! Which session the hook belongs to is **not** decided here: `sessions::Registry::bind_hook`
//! answers that at `hello` (SRV-17, SRV-18), and the dispatch keeps its answer until the
//! `hook.stop` arrives. This module takes the binding as given and decides what to say.

pub mod decide;

pub use decide::{decide, Decision, Item, MAX_ITEMS};
