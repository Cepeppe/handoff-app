//! The session registry (§7.5, SRV-17..20, DD-22).
//!
//! One record per connected server: connection, PID chain, working directory, the resolved
//! capability row, and the `session_id` bound to it once an agent claims it. The ancestor
//! chain is what lets a Stop hook find the session it belongs to, since the hook is not a
//! direct child of the agent (measured in T-023, A-11).
//!
//! # The two files
//!
//! - [`process_table`] — the machine's processes, and the chain completed from them. The
//!   part DD-22 moves to the app because a hook cannot afford to walk its own tree on
//!   Windows.
//! - [`registry`] — the sessions of this run, their write-through to `log::sessions`, and
//!   [`registry::Registry::bind_hook`], which is where SRV-17 and SRV-18 are decided.
//!
//! What the registry does **not** do is answer a hook: the decision of §7.5 — which
//! handoffs of the bound session are worth blocking for, and the once-per-item counter —
//! is `hook::decide` (T-035). This module only says *which session*.

pub mod process_table;
pub mod registry;

pub use process_table::{complete_chain, ProcessTable, SyntheticProcessTable, SystemProcessTable};
pub use registry::{HookBinding, NoObserver, Registry, Session, SessionsObserver};
