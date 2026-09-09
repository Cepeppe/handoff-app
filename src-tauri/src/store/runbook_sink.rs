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
//! - [`Requests`] is the queue of §7.7: which request a `handoff.open` answers (OPEN-08),
//!   which one the user relinked it to (FM-20), the one they gave up on (OPEN-04), and the
//!   request that asks an agent to come back to a handoff the user picked up in the overlay
//!   (RESP-07, FM-31). The overlay can
//!   resume a parked handoff at any time; the agent that opened it may be in the middle of
//!   something else or gone entirely, so the request is queued and delivered by the
//!   clipboard fast path or by the Stop hook.
//!
//! Both have a no-op implementation here, which is what the store is built with when a test
//! is not about them.
//!
//! Both take the store's own `&Db`: neither owns a connection (`requests::queue` says why),
//! a linking that happened on another connection could not be part of the transition that
//! caused it, and the runbook writer needs the diary of questions and replies that §7.4
//! keeps no field for (§4.5.1).

use crate::log::Db;
use crate::requests::queue::OpenLink;
use crate::runbooks::RunbookProposal;

use super::handoff::{FinalState, Handoff};

/// Told that a handoff reached a final state (§7.12, RUN-01, RUN-09).
///
/// It is called **after** the transition has been persisted, so what it reads is what a
/// restart would read. It reports no failure: a runbook that cannot be written must not undo
/// a handoff that is finished (PRIN-10), and the writer logs its own.
///
/// What it may hand back is a [`RunbookProposal`] — the third row of the §7.12 table, a
/// rewrite the user has to accept — because that one has to be kept "in `state_json` until
/// decided" and the store is the only writer of that column.
pub trait RunbookSink: Send {
    /// `handoff` has just reached `final_state`; `db` is the store's connection, for the
    /// diary the handoff record does not keep.
    fn on_finalised(
        &self,
        db: &Db,
        handoff: &Handoff,
        final_state: FinalState,
    ) -> Option<RunbookProposal>;

    /// The user accepted a proposal: rewrite the file it names (RUN-09, §7.12 row 3).
    ///
    /// The one method of this trait that reports its failure, and it has to: the store
    /// clears the question only when the file was written, so a proposal the disk refused
    /// is still on screen and still answerable. The message is the writer's own, for the
    /// window to show.
    ///
    /// # Errors
    ///
    /// Whatever stopped the write: the document did not validate, the certain detector
    /// matched it, or the file could not be replaced. Nothing is written in the first two.
    fn accept(&self, proposal: &RunbookProposal) -> std::result::Result<(), String>;
}

/// The sink of a store nobody is writing runbooks for: every test that is not about
/// runbooks.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoRunbookSink;

impl RunbookSink for NoRunbookSink {
    fn on_finalised(
        &self,
        _db: &Db,
        _handoff: &Handoff,
        _final_state: FinalState,
    ) -> Option<RunbookProposal> {
        None
    }

    fn accept(&self, _proposal: &RunbookProposal) -> std::result::Result<(), String> {
        // A store with no writer behind it makes no proposals either, so this is only
        // reachable from a `state_json` written by a run that had one. Saying so beats
        // reporting a success nobody performed.
        Err("this store writes no runbooks".to_owned())
    }
}

/// The user-request queue of §7.7, as the store reaches it.
///
/// Everything but [`Requests::link_on_open`] returns nothing on purpose: a queue entry that
/// cannot be written must not undo a handoff that was (PRIN-10, FM-28), so the queue logs
/// its own failures. `link_on_open` is asked *before* the transition, so its answer is part
/// of the record the transition writes and a failure there is simply "nothing to link".
pub trait Requests: Send {
    /// Which queued request this `handoff.open` answers (OPEN-08, DD-13). `request_id` is
    /// what the agent quoted, after the store has checked it against the handoffs it holds.
    fn link_on_open(&self, db: &Db, session: Option<&str>, request_id: Option<&str>) -> OpenLink;

    /// `handoff_id` answers `request_id`, and no longer answers whatever it did before
    /// (OPEN-08, FM-20).
    fn linked(&self, db: &Db, handoff_id: &str, request_id: &str);

    /// The user picked `handoff_id` up again. `session_ref` is the session that opened it,
    /// when it is known; the queue decides whether it can still be reached.
    fn request_resume(&self, db: &Db, handoff_id: &str, session_ref: Option<&str>);

    /// A call attached to `handoff_id`: an agent came back, so a resume request about it has
    /// been answered (FM-31).
    fn resumed(&self, db: &Db, handoff_id: &str);

    /// The user abandoned a tab that was still waiting for its spec, so the request it grew
    /// from is over: nothing should ask an agent for that spec again (§7.7, OPEN-06).
    ///
    /// `request_id` is the handoff's own id, which for a user-opened request is the queue
    /// entry's id as well (DD-13). A handoff with no request behind it — every agent-opened
    /// one — simply finds nothing to close.
    fn abandoned(&self, db: &Db, request_id: &str);
}

/// The queue of a store with no user-request queue behind it.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoRequests;

impl Requests for NoRequests {
    fn link_on_open(
        &self,
        _db: &Db,
        _session: Option<&str>,
        _request_id: Option<&str>,
    ) -> OpenLink {
        OpenLink::None
    }

    fn linked(&self, _db: &Db, _handoff_id: &str, _request_id: &str) {}

    fn request_resume(&self, _db: &Db, _handoff_id: &str, _session_ref: Option<&str>) {}

    fn resumed(&self, _db: &Db, _handoff_id: &str) {}

    fn abandoned(&self, _db: &Db, _request_id: &str) {}
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A sink that remembers what it was told, for the tests that care.
    #[derive(Debug, Default)]
    pub(crate) struct RecordingSink {
        pub(crate) finalised: Mutex<Vec<(String, FinalState)>>,
        /// The runbook ids of the proposals the user accepted.
        pub(crate) accepted: Mutex<Vec<String>>,
        /// Whether [`RunbookSink::accept`] should report a failed write.
        pub(crate) refuses: AtomicBool,
    }

    impl RunbookSink for Arc<RecordingSink> {
        fn on_finalised(
            &self,
            _db: &Db,
            handoff: &Handoff,
            final_state: FinalState,
        ) -> Option<RunbookProposal> {
            self.finalised
                .lock()
                .expect("the recording sink is not poisoned")
                .push((handoff.id.clone(), final_state));
            None
        }

        fn accept(&self, proposal: &RunbookProposal) -> std::result::Result<(), String> {
            self.accepted
                .lock()
                .expect("the recording sink is not poisoned")
                .push(proposal.runbook_id.clone());
            if self.refuses.load(Ordering::Relaxed) {
                return Err("the disk refused it".to_owned());
            }
            Ok(())
        }
    }

    /// A queue that only remembers what it was told.
    #[derive(Debug, Default)]
    pub(crate) struct CountingRequests {
        /// How many resume requests were queued (FM-31).
        pub(crate) requested: AtomicUsize,
        /// The ids of the requests the user gave up on (OPEN-04).
        pub(crate) abandoned: Mutex<Vec<String>>,
    }

    impl Requests for Arc<CountingRequests> {
        fn link_on_open(
            &self,
            _db: &Db,
            _session: Option<&str>,
            _request_id: Option<&str>,
        ) -> OpenLink {
            OpenLink::None
        }

        fn linked(&self, _db: &Db, _handoff_id: &str, _request_id: &str) {}

        fn request_resume(&self, _db: &Db, _handoff_id: &str, _session_ref: Option<&str>) {
            self.requested.fetch_add(1, Ordering::Relaxed);
        }

        fn resumed(&self, _db: &Db, _handoff_id: &str) {}

        fn abandoned(&self, _db: &Db, request_id: &str) {
            self.abandoned
                .lock()
                .expect("the recording queue is not poisoned")
                .push(request_id.to_owned());
        }
    }
}
