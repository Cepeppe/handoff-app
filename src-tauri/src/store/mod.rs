//! The handoff store and its state machine (§7.4, §8.1, DD-11, DD-12, NFR-12).
//!
//! Every transition is written through to SQLite before it is acknowledged, so an app
//! restart, an interrupted call, a dead server or a detached session all resume from the
//! same state (NFR-12). The queue of undelivered events, the deferral counters, the rounds
//! and the timers of the verifying states live here as well.
//!
//! No `AppHandle` reaches this module: what the UI needs is emitted through the traits of
//! `ui_bridge`, and what the channel needs is returned as data.
//!
//! # Reading order
//!
//! - [`handoff`] is the record of §7.4: what a handoff *is*, and the small questions the
//!   machine keeps asking it.
//! - [`outcome`] builds the object of §4.3 that an agent reads.
//! - [`actor`] is the machine: one owner, one connection, one transition at a time, each of
//!   them a transaction (`log::transitions`). [`actor::Store`] is the synchronous core, so
//!   the whole of §8.1 can be exercised with no runtime and no channel;
//!   [`actor::StoreHandle`] is the tokio task around it.
//! - [`runbook_sink`] holds the two seams to the rest of the app: the runbook writer and
//!   the user-request queue.
//! - [`watch`] is the third seam: the window is told which tab changed, so that it re-reads
//!   this store rather than keeping a copy of it.
//!
//! # The two rules that shape it
//!
//! - **The true spec never reaches the database.** `log::handoffs::upsert` masks it
//!   (LOG-02) and sweeps the same literals out of `state_json`, so what a restart reads
//!   back is the masked spec. The copy button and the outcome's `context` need the true one
//!   (DET-04), so it is held in memory for the life of the process and nowhere else.
//! - **A failed write changes nothing** (FM-28). Every transition is applied to a copy,
//!   persisted, and only then put back: an error leaves the in-memory handoff exactly as it
//!   was and comes back to the caller as [`Refusal::Persistence`].

pub mod actor;
pub mod handoff;
pub mod outcome;
pub mod runbook_sink;
pub mod watch;

pub use actor::{
    spawn, Command, Delivery, HandoffSnapshot, OpenAccepted, OpenParams, Reply, ResumeSnapshot,
    RoundSummary, Store, StoreHandle, UserAction, VerifyAccepted,
};
pub use handoff::{
    AttachedCall, Call, Cursor, FinalState, Handoff, Opener, PendingKind, PendingQuestion, Queued,
    Round, ScreenshotPayload,
};
pub use runbook_sink::{NoRequests, NoRunbookSink, Requests, RunbookSink};
pub use watch::{HandoffsObserver, NoWatchers};

use crate::format::channel::ChannelErrorCode;
use crate::log::{HandoffState, StoreError};

/// Why the store refused what it was asked.
///
/// Five of the seven map onto the application errors of §6.3, which is how a peer learns
/// what went wrong; the other two do not, deliberately:
///
/// - [`Refusal::NotActive`] answers a **user** action offered in a state that cannot take
///   it. The overlay decides which buttons a tab shows (§8.4), so this is a defect of the
///   view rather than something an agent can be told about, and the protocol has no code
///   for it.
/// - [`Refusal::Persistence`] is FM-28: the disk refused the transition, the in-memory
///   state is unchanged and the action can be retried once there is room. §6.3 numbers no
///   code for it either, so what a peer is told is the dispatch's decision (T-034).
#[derive(Debug, thiserror::Error)]
pub enum Refusal {
    /// No handoff with that id (`not_found`).
    #[error("no handoff with that id")]
    NotFound,
    /// A reply with no pending question and no replacement steps (`not_waiting`, FM-32).
    #[error("the handoff is not waiting for a reply")]
    NotWaiting,
    /// Replacement steps, or a report, on a handoff that is closed (`final`).
    #[error("the handoff is closed")]
    Final,
    /// `handoff.verify` on a spec that asked for no verification (`no_verify_in_spec`).
    #[error("the spec asked for no verification")]
    NoVerifyInSpec,
    /// A continue naming value keys the spec does not declare (`unknown_value_key`).
    #[error("the spec declares no such value")]
    UnknownValueKey {
        /// The names that are not in the spec's `values`. Never empty.
        keys: Vec<String>,
    },
    /// A user action that this state does not offer (§8.4). Not a protocol error.
    #[error("the handoff is {} and does not take that action", .state.as_str())]
    NotActive {
        /// Where the handoff actually is.
        state: HandoffState,
    },
    /// The database refused the transition (FM-28).
    #[error(transparent)]
    Persistence(#[from] StoreError),
}

impl Refusal {
    /// The application error of §6.3 this is, when it is one.
    #[must_use]
    pub fn code(&self) -> Option<ChannelErrorCode> {
        match self {
            Self::NotFound => Some(ChannelErrorCode::NotFound),
            Self::NotWaiting => Some(ChannelErrorCode::NotWaiting),
            Self::Final => Some(ChannelErrorCode::Final),
            Self::NoVerifyInSpec => Some(ChannelErrorCode::NoVerifyInSpec),
            Self::UnknownValueKey { .. } => Some(ChannelErrorCode::UnknownValueKey),
            Self::NotActive { .. } | Self::Persistence(_) => None,
        }
    }
}

/// The result of every operation of the store.
pub type Result<T> = std::result::Result<T, Refusal>;
