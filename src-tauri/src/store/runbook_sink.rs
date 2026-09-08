//! The two things the store hands to modules that do not exist yet.
//!
//! The store is the single owner of handoff state and it stays that way by talking to the
//! rest of the app through traits rather than by calling into it: `lib.rs` explains why —
//! `cargo test` has to exercise the whole state machine with no webview, and the e2e
//! automation channel plays the user by substituting fakes for exactly these seams.
//!
//! - [`RunbookSink`] is told that a handoff reached a final state. Two of the five save or
//!   refresh a runbook (RUN-01, §7.12); the other three are told anyway, because "this run
//!   failed and no correction followed" is also a fact the writer acts on (RUN-09).
//! - [`ResumeRequests`] is asked to bring an agent back to a handoff the user picked up in
//!   the overlay (RESP-07, FM-31). The overlay can resume a parked handoff at any time; the
//!   agent that opened it may be in the middle of something else or gone entirely, so the
//!   request is queued and delivered by the clipboard fast path or by the Stop hook.
//!
//! Both have a no-op implementation here, which is what the store is built with until the
//! modules that fill them land.
// TASK: T-044 — the runbook writer implements `RunbookSink`.
// TASK: T-035 — the user-request queue implements `ResumeRequests`.

use super::handoff::{FinalState, Handoff};

/// Told that a handoff reached a final state (§7.12, RUN-01, RUN-09).
///
/// It is called **after** the transition has been persisted, so what it reads is what a
/// restart would read. It returns nothing: a runbook that cannot be written must not undo a
/// handoff that is finished (PRIN-10), and the writer reports its own failures.
pub trait RunbookSink: Send {
    /// `handoff` has just reached `final_state`.
    fn on_finalised(&self, handoff: &Handoff, final_state: FinalState);
}

/// The sink of a store nobody is writing runbooks for: every test that is not about
/// runbooks, and every build before T-044.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoRunbookSink;

impl RunbookSink for NoRunbookSink {
    fn on_finalised(&self, _handoff: &Handoff, _final_state: FinalState) {}
}

/// Asked to get an agent back to a handoff the user resumed from the overlay (RESP-07,
/// FM-31, §7.7).
pub trait ResumeRequests: Send {
    /// The user picked `handoff_id` up again. `session_ref` is the session that opened it,
    /// when it is known; the queue decides whether it can still be reached.
    fn request_resume(&self, handoff_id: &str, session_ref: Option<&str>);
}

/// The queue of a store with no user-request queue behind it.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoResumeRequests;

impl ResumeRequests for NoResumeRequests {
    fn request_resume(&self, _handoff_id: &str, _session_ref: Option<&str>) {}
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A sink that remembers what it was told, for the tests that care.
    #[derive(Debug, Default)]
    pub(crate) struct RecordingSink {
        pub(crate) finalised: Mutex<Vec<(String, FinalState)>>,
    }

    impl RunbookSink for Arc<RecordingSink> {
        fn on_finalised(&self, handoff: &Handoff, final_state: FinalState) {
            self.finalised
                .lock()
                .expect("the recording sink is not poisoned")
                .push((handoff.id.clone(), final_state));
        }
    }

    /// A queue that only counts.
    #[derive(Debug, Default)]
    pub(crate) struct CountingResumes {
        pub(crate) requested: AtomicUsize,
    }

    impl ResumeRequests for Arc<CountingResumes> {
        fn request_resume(&self, _handoff_id: &str, _session_ref: Option<&str>) {
            self.requested.fetch_add(1, Ordering::Relaxed);
        }
    }
}
