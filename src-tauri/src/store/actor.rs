//! The single owner of handoff state (§7.4, §8.1, DD-11, DD-12, NFR-12).
//!
//! # Two layers, on purpose
//!
//! [`Store`] is the machine, and it is **synchronous**: it owns the map of handoffs and the
//! one database connection, applies a transition, writes it through and answers. Everything
//! §8.1 says can be exercised against it with no runtime, no channel and no window, which
//! is what makes the property suite of §11.2 possible at all.
//!
//! [`StoreHandle`] is the tokio task around it (DD-11): one command channel in, one stream
//! of [`Delivery`] out, and the verification timer of VER-06 kept by the same `select!` that
//! reads the commands. Nothing else may hold a `Store`, which is what makes "the single
//! owner" true rather than aspirational.
//!
//! # Every transition is applied to a copy first (FM-28)
//!
//! A handoff is cloned, the transition is applied to the clone, the clone is persisted, and
//! only a successful write puts it back in the map. A disk that refuses leaves the
//! in-memory handoff exactly as the user last saw it and the failure comes back as
//! [`super::Refusal::Persistence`]; nothing is lost silently and the action can be retried.
//!
//! # Where an outcome goes
//!
//! An interrupting action builds an outcome (§7.4). If a call is attached it becomes a
//! [`Delivery`] and the call detaches; otherwise it is pushed onto `undelivered`, which
//! `handoff.resume` pops oldest first (DD-12, §5.7). Deliveries are collected in an outbox
//! and drained by the actor, so there is exactly one place in the app that sends
//! `handoff.event` — including the ones a timer or a disconnect produced, which no request
//! is waiting for.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use crate::format::channel::DetachReason;
use crate::format::outcome::{Outcome, OutcomeStatus, ResumedFrom, SecretTreated, VerifyReport};
use crate::format::spec::{HandoffSpec, HandoffStep};
use crate::ids;
use crate::log::events::{EventKind, EventRow};
use crate::log::handoffs::HandoffRow;
use crate::log::rounds::RoundRow;
use crate::log::sends::{SendKind, SendRow};
use crate::log::transitions::{commit as commit_transition, Transition};
use crate::log::{handoffs, user_requests, Db, HandoffState, StoreError, Timestamp};
use crate::requests::queue::OpenLink;

use super::handoff::{
    Call, Cursor, FinalState, Handoff, Opener, PendingKind, PendingQuestion, Queued, Round,
    ScreenshotPayload, VERIFYING_TIMEOUT_MS,
};
use super::outcome::{already_delivered, build};
use super::runbook_sink::{NoRequests, NoRunbookSink, Requests, RunbookSink};
use super::watch::{HandoffsObserver, NoWatchers};
use super::{Refusal, Result};

/// How many commands may wait for the actor before a sender is made to wait.
///
/// Deep enough that a burst of user clicks and a reconnecting session never block each
/// other, shallow enough that a wedged store shows up as backpressure rather than as
/// unbounded memory.
const COMMAND_QUEUE: usize = 64;

/// An outcome that has to reach a waiting call (§6.3 `handoff.event`).
///
/// The dispatch sends it; the store never touches a socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// The connection the call arrived on.
    pub conn_id: u64,
    /// The call to resolve.
    pub call_id: String,
    /// Which handoff.
    pub handoff_id: String,
    /// What happened.
    pub outcome: Outcome,
    /// The pixels of a screenshot, when the user sent one as an image (§6.6).
    pub image: Option<String>,
}

/// What a `handoff.open` carries (§6.3).
#[derive(Debug, Clone)]
pub struct OpenParams {
    /// The spec, exactly as the agent sent it (DET-04).
    pub spec: HandoffSpec,
    /// What the certain detector matched at ingress.
    pub secret_treated: Vec<SecretTreated>,
    /// The id of a user-opened request; the handoff then takes it (DD-13).
    pub request_id: Option<String>,
    /// The session that opened it.
    pub opener: Opener,
    /// The call that will wait on it.
    pub call: Call,
}

/// The answer to a `handoff.open`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAccepted {
    /// The id the app assigned, or the request's id it took over.
    pub handoff_id: String,
    /// The opening session, when this call comes from another one. Always absent on an
    /// open: the call that opens a handoff is by definition the first one on it.
    pub resumed_from: Option<ResumedFrom>,
}

/// The snapshot a `handoff.resume` is answered with (§5.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSnapshot {
    /// Where the handoff stands (§8.1).
    pub state: HandoffState,
    /// The final outcome, or the oldest queued one; absent when the call attached and waits.
    pub outcome: Option<Outcome>,
    /// The pixels that belong beside a queued screenshot (§6.6).
    pub image: Option<String>,
    /// The opening session, when this call comes from a different one (TOOL-08).
    pub resumed_from: Option<ResumedFrom>,
}

/// The answer to a `handoff.verify` (§4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyAccepted {
    /// The final outcome the reporting agent gets back.
    pub outcome: Outcome,
}

/// One closed round, as the History section of §7.6 shows it.
///
/// Only rounds the cursor has left: the current one is the flat `steps` / `confirmed` /
/// `skipped` / `notes` of [`HandoffSnapshot`], so nothing is carried twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundSummary {
    /// 1-based round number, as the counter printed it.
    pub no: u32,
    /// The steps of that round.
    pub steps: Vec<HandoffStep>,
    /// 1-based indices the user confirmed.
    pub confirmed: Vec<u32>,
    /// 1-based indices the user skipped.
    pub skipped: Vec<u32>,
    /// What the user wrote on its steps.
    pub notes: Vec<crate::format::outcome::OutcomeNote>,
    /// What the agent reported about it (VER-05, VER-10).
    pub verify: Option<VerifyReport>,
}

/// What an agent answered, and on which step (RESP-04, TOOL-04).
///
/// Read from the diary rather than from the record: a reply is an `events` row of kind
/// `reply`, because §7.4 keeps no field for it — the tool result carried it to the agent's
/// caller and the overlay shows it on the step it referred to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// The round it was written in.
    pub round: u32,
    /// The 1-based step it answered.
    pub step: u32,
    /// What the agent wrote.
    pub text: String,
    /// When it arrived.
    pub at: Timestamp,
}

/// What the user asked, and on which step (RESP-04, §7.6).
///
/// The other half of [`Reply`], read from the same diary: an `events` row of kind `ask`.
/// §7.6 shows it twice — beside the "waiting for the reply" banner while the agent owes an
/// answer, and in the History of a closed round — and a round trip that printed the answer
/// without the question would be a conversation with one voice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// The round it was asked in.
    pub round: u32,
    /// The 1-based step it was raised on.
    pub step: u32,
    /// What the user wrote, as it was sent — already redacted (§7.10).
    pub text: String,
    /// When it was asked.
    pub at: Timestamp,
}

/// One handoff's whole question-and-answer trail, read in a single pass (§7.6).
///
/// Both halves come out of `events` and both are wanted by the same repaint, so they are
/// read together: a second call would be a second scan of the same rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exchanges {
    /// What the user asked, oldest first.
    pub questions: Vec<Question>,
    /// What the agents answered, oldest first.
    pub replies: Vec<Reply>,
}

/// What a view needs to draw one tab (§7.6, §8.4).
///
/// A projection, not the record: the store stays the only owner, and a view that held a
/// [`Handoff`] would be a second one.
///
/// Everything here is a clone of what is already in memory, because the list is rebuilt on
/// every Stop hook as well as on every repaint (`hook::decide`) and a query per handoff
/// there would spend a real-time budget on the window's behalf. The two things a view needs
/// and this does not carry are read on their own: the question-and-answer trail
/// ([`Store::exchanges`], one query for one tab) and whether the session is still
/// connected, which belongs to the registry (SRV-21).
///
/// **`values` are the true values.** §5.5 sends the app the spec unmasked so that the copy
/// button can copy what the user has to paste (DET-04); `secret_treated` says which of them
/// the window must show as `••••••`. Masking is `ui_bridge::view`'s, and it is the only
/// thing that crosses into the webview: this type is not serialisable on purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffSnapshot {
    /// `hf_` + 10 characters.
    pub id: String,
    /// Where it stands (§8.1).
    pub state: HandoffState,
    /// The current round (VER-09).
    pub round: u32,
    /// The 1-based step the counter shows.
    pub step_index: u32,
    /// How many steps the round has.
    pub step_total: u32,
    /// The steps of the current round.
    pub steps: Vec<HandoffStep>,
    /// 1-based indices confirmed in this round.
    pub confirmed: Vec<u32>,
    /// 1-based indices skipped in this round (RESP-03).
    pub skipped: Vec<u32>,
    /// The notes of this round (RESP-03).
    pub notes: Vec<crate::format::outcome::OutcomeNote>,
    /// 0, 1 or 2.
    pub deferral_count: u32,
    /// Set while the agent owes a reply (§8.4 "Question sent").
    pub pending_question: Option<PendingQuestion>,
    /// How many outcomes wait for the next resume (DD-12).
    pub undelivered: usize,
    /// Whether a call is listening right now (§8.4 "Guiding, agent away").
    pub call_attached: bool,
    /// The session that opened it.
    pub session_ref: Option<String>,
    /// Agent and project of the opening session, which is what the tab is labelled with
    /// (OPEN-02, MULTI-01).
    pub opener_label: Option<ResumedFrom>,
    /// The project folder of the opening session; `secrets` destinations are relative to it
    /// (SEC-02).
    pub project_dir: Option<String>,
    /// The spec's goal, once there is a spec.
    pub goal: Option<String>,
    /// The spec's `where`.
    pub location: Option<String>,
    /// The spec's own starting point, when it has one (GUIDE-03).
    pub url: Option<String>,
    /// BCP-47 tag of the spec's texts; the steps are shown in the language the agent wrote
    /// them, whatever the UI language is (GUIDE-06).
    pub lang: Option<String>,
    /// The spec's values, **true** (DET-04). See the note on this type.
    pub values: indexmap::IndexMap<String, crate::format::spec::SpecValue>,
    /// Variable name to destination file (SEC-02). Names only; never a value (PRIN-03).
    pub secrets: Option<indexmap::IndexMap<String, String>>,
    /// What the certain detector matched at ingress: which locations are masked, and with
    /// which family (DET-04, §5.9).
    pub secret_treated: Vec<SecretTreated>,
    /// The rounds the cursor has left, oldest first (VER-09).
    pub history: Vec<RoundSummary>,
    /// What the agent reported about the last round (VER-05, §8.4 "declared by agent").
    pub verify_report: Option<VerifyReport>,
    /// What the agent will check (VER-04).
    pub verify: Option<String>,
    /// The user's own words, when it grew from a request.
    pub request_text: Option<String>,
    /// The request it answers, when the link is not the id itself (FM-20).
    pub linked_request_id: Option<String>,
    /// The opening session, when the current call comes from another one.
    pub resumed_from: Option<ResumedFrom>,
    /// The outcome a final state produced.
    pub final_outcome: Option<Outcome>,
    /// A final outcome nobody collected for seven days (SRV-23, FM-27).
    pub orphan: bool,
    /// When it opened.
    pub created_at: Timestamp,
    /// When it closed.
    pub closed_at: Option<Timestamp>,
}

/// Who is going to read the outcome a finalisation produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handover {
    /// Nothing is waiting for it in particular: it goes to the attached call, or onto the
    /// queue the next resume reads (DD-12).
    QueueIfNobodyListens,
    /// The request that caused it carries it back — `handoff.verify` answers with
    /// `{outcome}` (§4.4). A call that happens to be attached is still released, so it does
    /// not sit there until its heartbeat; but there is nobody to queue for, and queueing
    /// would hand the same agent the same outcome a second time at its next resume.
    AnsweredByItsOwnRequest,
}

/// What one transition produces beside the handoff row.
///
/// The deliveries are in here rather than straight in the store's outbox because a
/// transition that fails to persist must tell nobody: FM-28 asks for the in-memory state to
/// survive a refused write, and an outcome already on its way to an agent would be the one
/// piece of it that could not be taken back.
#[derive(Debug, Default)]
struct Journal {
    rounds: Vec<RoundRow>,
    events: Vec<EventRow>,
    sends: Vec<SendRow>,
    deliveries: Vec<Delivery>,
}

impl Journal {
    fn event(
        &mut self,
        handoff: &Handoff,
        at: &Timestamp,
        kind: EventKind,
        step_index: Option<u32>,
        payload: Option<serde_json::Value>,
    ) {
        self.events.push(EventRow {
            id: 0,
            handoff_id: handoff.id.clone(),
            round: Some(i64::from(handoff.cursor.round)),
            at: at.clone(),
            kind,
            step_index: step_index.map(i64::from),
            payload_json: payload.map(|value| value.to_string()),
        });
    }

    fn round(&mut self, handoff: &Handoff, round: &Round) -> Result<()> {
        self.rounds.push(round_row(&handoff.id, round)?);
        Ok(())
    }
}

/// The machine: one map of handoffs, one connection, one transition at a time.
pub struct Store {
    db: Db,
    handoffs: BTreeMap<String, Handoff>,
    runbooks: Box<dyn RunbookSink>,
    requests: Box<dyn Requests>,
    watchers: Box<dyn HandoffsObserver>,
    outbox: Vec<Delivery>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Store")
            .field("handoffs", &self.handoffs.len())
            .field("outbox", &self.outbox.len())
            .finish_non_exhaustive()
    }
}

impl Store {
    /// A store with nothing in it. Tests, and nothing else: the app always [`Store::open`]s.
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self {
            db,
            handoffs: BTreeMap::new(),
            runbooks: Box::new(NoRunbookSink),
            requests: Box::new(NoRequests),
            watchers: Box::new(NoWatchers),
            outbox: Vec::new(),
        }
    }

    /// The same store, telling `watchers` which tab changed (§7.6).
    ///
    /// A builder rather than a fourth constructor parameter: watching is the window's
    /// business, and every test and every headless build wants the store without it.
    #[must_use]
    pub fn watched_by(mut self, watchers: Box<dyn HandoffsObserver>) -> Self {
        self.watchers = watchers;
        self
    }

    /// The store as the database left it (§7.2, NFR-12, FM-13).
    ///
    /// Not called `open`: `handoff.open` is one of the operations below, and one name for
    /// the constructor and for a transition would read as the same thing twice.
    ///
    /// Both the open handoffs and the closed ones are read back: a resume of a concluded
    /// handoff has to answer with the same final outcome and `already_delivered` (TOOL-07),
    /// and the orphan list of SRV-23 is drawn from the closed ones that nobody collected.
    /// No handoff comes back with an attached call — a call is a connection, and none of
    /// them survived.
    ///
    /// # Errors
    ///
    /// [`Refusal::Persistence`] when the rows cannot be read; a `state_json` that no longer
    /// parses is one of those.
    pub fn load(
        db: Db,
        runbooks: Box<dyn RunbookSink>,
        requests: Box<dyn Requests>,
    ) -> Result<Self> {
        let mut handoffs = BTreeMap::new();
        for row in handoffs::list_active(&db)?
            .into_iter()
            .chain(handoffs::list_final(&db)?)
        {
            let restored = restore(&row)?;
            handoffs.insert(restored.id.clone(), restored);
        }
        Ok(Self {
            db,
            handoffs,
            runbooks,
            requests,
            watchers: Box::new(NoWatchers),
            outbox: Vec::new(),
        })
    }

    /// The outcomes waiting to be sent, and clears them.
    pub fn take_deliveries(&mut self) -> Vec<Delivery> {
        std::mem::take(&mut self.outbox)
    }

    /// One handoff, if the store knows it.
    #[must_use]
    pub fn get(&self, handoff_id: &str) -> Option<&Handoff> {
        self.handoffs.get(handoff_id)
    }

    /// One tab (§7.6).
    #[must_use]
    pub fn snapshot(&self, handoff_id: &str, now: &Timestamp) -> Option<HandoffSnapshot> {
        self.handoffs
            .get(handoff_id)
            .map(|handoff| snapshot_of(handoff, now))
    }

    /// Every handoff the overlay may show, oldest first (§7.6, §8.4).
    #[must_use]
    pub fn list_for_ui(&self, now: &Timestamp) -> Vec<HandoffSnapshot> {
        let mut all: Vec<&Handoff> = self.handoffs.values().collect();
        all.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        all.into_iter()
            .map(|handoff| snapshot_of(handoff, now))
            .collect()
    }

    // ------------------------------------------------------------------- user actions

    /// The user typed a request: the tab of OPEN-04, waiting for a spec that has not been
    /// asked for yet (§7.7, DD-13).
    ///
    /// `id` is the queue entry's, because the handoff *becomes* it: one tab keeps one id
    /// from "waiting for spec" to its final state, which is what lets an agent quote
    /// `request_id` and have [`Store::open`] adopt this very row. The queue entry is written
    /// by `requests::Queue::create` before this is called, so an id that reaches here is
    /// already in the queue; nothing is linked yet, because nothing has answered it.
    ///
    /// An id the store already knows is refused rather than overwritten: the only way to
    /// reach that is a caller that minted the same id twice, and a live handoff is not
    /// something a new request may replace.
    ///
    /// # Errors
    ///
    /// [`Refusal::NotActive`] when a handoff of that id already exists,
    /// [`Refusal::Persistence`] when the write fails.
    pub fn open_request(
        &mut self,
        id: &str,
        request_text: &str,
        opener: Option<&Opener>,
        now: &Timestamp,
    ) -> Result<()> {
        if let Some(known) = self.handoffs.get(id) {
            return Err(Refusal::NotActive { state: known.state });
        }
        let draft = Handoff::awaiting_spec(id.to_owned(), request_text.to_owned(), opener, now);
        let mut journal = Journal::default();
        journal.event(
            &draft,
            now,
            EventKind::State,
            None,
            Some(json!({"state": HandoffState::AwaitingSpec.as_str()})),
        );
        self.commit(draft, journal)
    }

    // ------------------------------------------------------------------ agent actions

    /// `handoff.open`: a new handoff, or the spec a user request was waiting for (DD-13,
    /// OPEN-08).
    ///
    /// # Errors
    ///
    /// [`Refusal::Persistence`] when the write fails.
    pub fn open(&mut self, params: OpenParams, now: &Timestamp) -> Result<OpenAccepted> {
        let OpenParams {
            spec,
            secret_treated,
            request_id,
            opener,
            call,
        } = params;

        // DD-13: the handoff takes the request's id, so one tab keeps one id from "waiting
        // for spec" to its final state. An id that is not a `hf_` id, or one that already
        // names a handoff which is past waiting for a spec, is not adopted: the agent quoted
        // something that is not a request, and answering by overwriting a live handoff would
        // lose the one it named.
        let adopted = request_id
            .filter(|id| ids::HANDOFF_ID_RE.is_match(id))
            .filter(|id| self.handoffs.get(id).is_none_or(is_waiting_for_a_spec));
        let id = adopted.clone().unwrap_or_else(ids::new_handoff_id);

        // OPEN-08: with an id, the queue confirms the request the handoff now *is*; without
        // one, it names the oldest request the session left open, which this handoff answers
        // without taking its id. Either way the queue decides, and it is the same rule the
        // **Change** control of FM-20 applies afterwards.
        let link =
            self.requests
                .link_on_open(&self.db, Some(&opener.session_ref), adopted.as_deref());

        let waiting = self.handoffs.get(&id);
        let mut draft = Handoff::opened(id.clone(), spec, secret_treated, &opener, now);
        if let Some(waiting) = waiting {
            draft.created_at = waiting.created_at.clone();
            draft.request_text = waiting.request_text.clone();
            draft.linked_request_id = waiting.linked_request_id.clone();
        }
        match &link {
            OpenLink::Adopted(request) => {
                // The tab keeps the instant the user opened it, not the one the agent
                // answered it: §7.7 shows one tab from "waiting for spec" onwards.
                draft.created_at = request.created_at.clone();
                draft.request_text = Some(request.text.clone());
            }
            OpenLink::Oldest(request) => {
                draft.linked_request_id = Some(request.id.clone());
                draft.request_text = Some(request.text.clone());
            }
            OpenLink::None => {}
        }
        draft.attached_call = Some(call.attached());

        let mut journal = Journal::default();
        let round = draft.rounds[0].clone();
        journal.round(&draft, &round)?;
        journal.event(
            &draft,
            now,
            EventKind::Attach,
            None,
            Some(json!({"call": &call.call_id})),
        );
        journal.event(
            &draft,
            now,
            EventKind::State,
            None,
            Some(json!({"state": HandoffState::Active.as_str()})),
        );
        self.commit(draft, journal)?;

        // The request row learns which handoff answered it (OPEN-08); it is a note about
        // where the request went, so a queue that refuses the write is not a reason to
        // refuse the handoff. After the commit, so that a refused transition leaves a
        // request open rather than pointing at a handoff that does not exist.
        if let Some(request) = link.request() {
            self.requests.linked(&self.db, &id, &request.id);
        }

        Ok(OpenAccepted {
            handoff_id: id,
            resumed_from: None,
        })
    }

    /// `handoff.continue`: the agent answered, and may replace the remaining steps (TOOL-04,
    /// VER-08, VER-09).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`], [`Refusal::UnknownValueKey`], [`Refusal::NotWaiting`],
    /// [`Refusal::Final`] or [`Refusal::Persistence`].
    pub fn continue_handoff(
        &mut self,
        handoff_id: &str,
        call: &Call,
        reply: &str,
        replacement_steps: Option<Vec<HandoffStep>>,
        now: &Timestamp,
    ) -> Result<()> {
        let mut draft = self.draft(handoff_id)?;

        if let Some(steps) = replacement_steps.as_ref() {
            let unknown = undeclared_value_keys(&draft, steps);
            if !unknown.is_empty() {
                return Err(Refusal::UnknownValueKey { keys: unknown });
            }
            match draft.state {
                // VER-08: a failed verification is left by an agent action on the same id.
                HandoffState::Active | HandoffState::Failed => {}
                state if state.is_final() => return Err(Refusal::Final),
                _ => return Err(Refusal::NotWaiting),
            }
        } else if draft.pending_question.is_none() {
            // FM-32: a reply with nothing pending and no replacement steps.
            return Err(Refusal::NotWaiting);
        }

        let mut journal = Journal::default();
        let step = draft.cursor.step_index;
        draft.pending_question = None;
        draft.attached_call = Some(call.attached());
        journal.event(
            &draft,
            now,
            EventKind::Reply,
            Some(step),
            Some(json!({"text": reply})),
        );
        journal.event(
            &draft,
            now,
            EventKind::Attach,
            None,
            Some(json!({"call": &call.call_id})),
        );

        if let Some(steps) = replacement_steps {
            open_correction_round(&mut draft, steps, now, &mut journal)?;
        }

        self.commit(draft, journal)
    }

    /// `handoff.resume`: a call re-attaches, or collects what is waiting (§5.7, TOOL-07,
    /// TOOL-08).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`] or [`Refusal::Persistence`].
    pub fn resume(
        &mut self,
        handoff_id: &str,
        call: &Call,
        now: &Timestamp,
    ) -> Result<ResumeSnapshot> {
        let mut draft = self.draft(handoff_id)?;
        let mut journal = Journal::default();
        journal.event(
            &draft,
            now,
            EventKind::Resume,
            None,
            Some(json!({"call": &call.call_id})),
        );

        // The call that was listening is told the handoff moved on (FM-25). Its outcome is
        // built first, while `resumed_from` still describes *its* call and not the new one.
        let transfer = draft.attached_call.take();
        if let Some(previous) = transfer.filter(|previous| previous.call_id != call.call_id) {
            let outcome = build(&draft, OutcomeStatus::TransferredToOtherSession, None, None);
            journal.deliveries.push(Delivery {
                conn_id: previous.conn_id,
                call_id: previous.call_id.clone(),
                handoff_id: draft.id.clone(),
                outcome,
                image: None,
            });
            journal.event(
                &draft,
                now,
                EventKind::Detach,
                None,
                Some(json!({"call": &previous.call_id, "reason": "transferred"})),
            );
        }

        // TOOL-08: the handoff belongs to the user, so any session may resume it, and the
        // outcome says where it came from when the sessions differ.
        let from_another_session = draft.session_ref.is_some()
            && call.session_ref.is_some()
            && draft.session_ref != call.session_ref;
        draft.resumed_from = if from_another_session {
            draft.opener_label.clone()
        } else {
            None
        };
        let resumed_from = draft.resumed_from.clone();

        let snapshot = if draft.is_final() && draft.final_outcome.is_some() {
            // TOOL-07: the identical final outcome, and the second reader is told so.
            let seen = draft.delivered_at.is_some();
            let stored = draft.final_outcome.clone().expect("checked just above");
            let mut outcome = if seen {
                already_delivered(&stored)
            } else {
                stored
            };
            outcome.resumed_from = resumed_from.clone();
            if !seen {
                draft.delivered_at = Some(now.clone());
            }
            ResumeSnapshot {
                state: draft.state,
                outcome: Some(outcome),
                image: None,
                resumed_from: resumed_from.clone(),
            }
        } else if let Some(queued) = draft.undelivered.pop_front() {
            // DD-12: oldest first, so a question and the screenshot that followed it arrive
            // in the order the user produced them.
            let mut outcome = queued.outcome;
            outcome.resumed_from = resumed_from.clone();
            ResumeSnapshot {
                state: draft.state,
                outcome: Some(outcome),
                image: queued.image,
                resumed_from: resumed_from.clone(),
            }
        } else {
            // §8.1, `deferred --> active: agent resumes`. Only here, and only from
            // `deferred`: a resume that pops a queued outcome is the agent *reading* what
            // happened, not coming back to the work, and `parked` is left by the user alone
            // (the parked instruction tells the agent not to resume at all).
            if draft.state == HandoffState::Deferred {
                draft.state = HandoffState::Active;
                journal.event(
                    &draft,
                    now,
                    EventKind::State,
                    None,
                    Some(json!({"state": HandoffState::Active.as_str(), "by": "resume"})),
                );
            }
            draft.attached_call = Some(call.attached());
            journal.event(
                &draft,
                now,
                EventKind::Attach,
                None,
                Some(json!({"call": &call.call_id})),
            );
            ResumeSnapshot {
                state: draft.state,
                outcome: None,
                image: None,
                resumed_from: resumed_from.clone(),
            }
        };

        let attached = snapshot.outcome.is_none();
        self.commit(draft, journal)?;
        // FM-31: a call is listening again, so the resume request the user queued from the
        // overlay has been answered. Only where the call *attached* — the branches above
        // hand back an outcome the agent is reading, which is not coming back to the work.
        if attached {
            self.requests.resumed(&self.db, handoff_id);
        }
        Ok(snapshot)
    }

    /// `handoff.verify`: the agent reports what it checked (§4.4, VER-01, VER-03, DD-16).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`], [`Refusal::NoVerifyInSpec`], [`Refusal::NotWaiting`],
    /// [`Refusal::Final`] or [`Refusal::Persistence`].
    pub fn verify(
        &mut self,
        handoff_id: &str,
        ok: Option<bool>,
        detail: Option<String>,
        now: &Timestamp,
    ) -> Result<VerifyAccepted> {
        let mut draft = self.draft(handoff_id)?;
        if draft
            .spec
            .as_ref()
            .and_then(|spec| spec.verify.as_ref())
            .is_none()
        {
            return Err(Refusal::NoVerifyInSpec);
        }

        // DD-16: a report that arrives after the timeout or the disconnect is accepted while
        // the handoff is younger than seven days, and recorded as late.
        let late = match draft.state {
            HandoffState::AwaitingVerification => false,
            HandoffState::NotVerified if draft.accepts_a_late_report(now) => true,
            state if state.is_final() => return Err(Refusal::Final),
            _ => return Err(Refusal::NotWaiting),
        };

        let report = VerifyReport {
            ok,
            detail,
            reported_at: now.to_string(),
            late,
        };
        let status = match ok {
            Some(true) => OutcomeStatus::Verified,
            Some(false) => OutcomeStatus::Failed,
            None => OutcomeStatus::NotVerified,
        };

        let mut journal = Journal::default();
        if let Some(round) = draft.current_round_mut() {
            round.verify = Some(report.clone());
            if round.ended_at.is_none() {
                round.ended_at = Some(now.clone());
            }
            let round = round.clone();
            journal.round(&draft, &round)?;
        }
        let outcome = self.finalise(
            &mut draft,
            status,
            None,
            Handover::AnsweredByItsOwnRequest,
            now,
            &mut journal,
        )?;
        self.commit(draft, journal)?;
        Ok(VerifyAccepted { outcome })
    }

    /// `handoff.detach_call`: the call stopped waiting, and the tab may say so (DD-24).
    ///
    /// A detach naming a call that is not the attached one is not an error: the server sent
    /// it about a call this side had already let go of, which is the normal race between a
    /// heartbeat and an outcome.
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`] or [`Refusal::Persistence`].
    pub fn detach_call(
        &mut self,
        handoff_id: &str,
        call_id: &str,
        reason: DetachReason,
        now: &Timestamp,
    ) -> Result<()> {
        let mut draft = self.draft(handoff_id)?;
        let matches = draft
            .attached_call
            .as_ref()
            .is_some_and(|call| call.call_id == call_id);
        if !matches {
            return Ok(());
        }
        draft.attached_call = None;
        let mut journal = Journal::default();
        let reason = match reason {
            DetachReason::Heartbeat => "heartbeat",
            DetachReason::Cancelled => "cancelled",
        };
        journal.event(
            &draft,
            now,
            EventKind::Detach,
            None,
            Some(json!({"call": call_id, "reason": reason})),
        );
        self.commit(draft, journal)
    }

    // ------------------------------------------------------------------- user actions

    /// The user marked the step done and moved on (RESP-01, RESP-03).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`], [`Refusal::NotActive`] or [`Refusal::Persistence`].
    pub fn confirm(&mut self, handoff_id: &str, now: &Timestamp) -> Result<()> {
        self.mark_step(handoff_id, EventKind::Confirm, now)
    }

    /// The user skipped the step (RESP-01, RESP-03).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`], [`Refusal::NotActive`] or [`Refusal::Persistence`].
    pub fn skip(&mut self, handoff_id: &str, now: &Timestamp) -> Result<()> {
        self.mark_step(handoff_id, EventKind::Skip, now)
    }

    /// The user annotated the step (RESP-02, RESP-03).
    ///
    /// A blank note is nothing to record and nothing to report: the outcome schema requires
    /// a note to carry text, and an empty one would be an outcome no agent could read.
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`], [`Refusal::NotActive`] or [`Refusal::Persistence`].
    pub fn note(&mut self, handoff_id: &str, text: &str, now: &Timestamp) -> Result<()> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(());
        }
        let mut draft = self.active(handoff_id)?;
        let step = draft.cursor.step_index;
        let mut journal = Journal::default();
        if let Some(round) = draft.current_round_mut() {
            round.notes.push(crate::format::outcome::OutcomeNote {
                step,
                text: text.to_owned(),
                at: now.to_string(),
            });
        }
        journal.event(
            &draft,
            now,
            EventKind::Note,
            Some(step),
            Some(json!({"text": text})),
        );
        self.commit(draft, journal)
    }

    /// The user asked the agent something (RESP-04, TOOL-03).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`], [`Refusal::NotActive`] or [`Refusal::Persistence`].
    pub fn ask(&mut self, handoff_id: &str, text: &str, now: &Timestamp) -> Result<()> {
        let mut draft = self.active(handoff_id)?;
        let step = draft.cursor.step_index;
        draft.pending_question = Some(PendingQuestion {
            kind: PendingKind::Question,
            step,
            at: now.clone(),
        });

        let mut journal = Journal::default();
        journal.event(&draft, now, EventKind::Ask, Some(step), None);
        journal.sends.push(SendRow {
            id: 0,
            handoff_id: draft.id.clone(),
            at: now.clone(),
            kind: SendKind::Question,
            text_as_sent: Some(text.to_owned()),
            image_sha256: None,
            image_w: None,
            image_h: None,
            redaction_boxes_json: None,
            ocr_engine: None,
            patterns_version: None,
        });

        let outcome = build(&draft, OutcomeStatus::Question, Some(text.to_owned()), None);
        hand_over(&mut draft, outcome, None, now, &mut journal);
        self.commit(draft, journal)
    }

    /// The user sent what they see (CAP-01, PREV-01, LOG-03).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`], [`Refusal::NotActive`] or [`Refusal::Persistence`].
    pub fn screenshot(
        &mut self,
        handoff_id: &str,
        payload: &ScreenshotPayload,
        now: &Timestamp,
    ) -> Result<()> {
        let mut draft = self.active(handoff_id)?;
        let step = draft.cursor.step_index;
        draft.pending_question = Some(PendingQuestion {
            kind: PendingKind::Screenshot,
            step,
            at: now.clone(),
        });

        let image_mode = matches!(payload.mode, crate::format::outcome::ScreenshotMode::Image);
        let mut journal = Journal::default();
        journal.event(&draft, now, EventKind::Screenshot, Some(step), None);
        journal.sends.push(SendRow {
            id: 0,
            handoff_id: draft.id.clone(),
            at: now.clone(),
            kind: if image_mode {
                SendKind::ScreenshotImage
            } else {
                SendKind::ScreenshotText
            },
            // What actually left: the extracted text in text mode, the comment beside the
            // picture in image mode. The pixels never (LOG-03) — only their hash.
            text_as_sent: if image_mode {
                payload.comment.clone()
            } else {
                payload.text.clone()
            },
            image_sha256: payload.image_sha256.clone(),
            image_w: Some(i64::from(payload.width)),
            image_h: Some(i64::from(payload.height)),
            redaction_boxes_json: payload.redaction_boxes_json.clone(),
            ocr_engine: payload.ocr_engine.clone(),
            patterns_version: payload.patterns_version.clone(),
        });

        let outcome = build(
            &draft,
            OutcomeStatus::Screenshot,
            payload.comment.clone(),
            Some(payload),
        );
        let image = if image_mode {
            payload.image_base64.clone()
        } else {
            None
        };
        hand_over(&mut draft, outcome, image, now, &mut journal);
        self.commit(draft, journal)
    }

    /// The user deferred: once the agent comes back to it, twice it waits in the overlay
    /// (RESP-05, RESP-07).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`], [`Refusal::NotActive`] or [`Refusal::Persistence`].
    pub fn defer(
        &mut self,
        handoff_id: &str,
        reason: Option<String>,
        now: &Timestamp,
    ) -> Result<()> {
        let mut draft = self.draft(handoff_id)?;
        if !matches!(draft.state, HandoffState::Active | HandoffState::Deferred) {
            return Err(Refusal::NotActive { state: draft.state });
        }
        draft.deferral_count = (draft.deferral_count + 1).min(2);
        draft.pending_question = None;
        let (state, status) = if draft.deferral_count >= 2 {
            (HandoffState::Parked, OutcomeStatus::Parked)
        } else {
            (HandoffState::Deferred, OutcomeStatus::Deferred)
        };
        draft.state = state;

        let mut journal = Journal::default();
        journal.event(
            &draft,
            now,
            EventKind::Defer,
            Some(draft.cursor.step_index),
            Some(json!({"count": draft.deferral_count})),
        );
        journal.event(
            &draft,
            now,
            EventKind::State,
            None,
            Some(json!({"state": state.as_str()})),
        );
        journal
            .sends
            .push(text_send(&draft, SendKind::Defer, reason.clone(), now));

        let outcome = build(&draft, status, reason, None);
        hand_over(&mut draft, outcome, None, now, &mut journal);
        self.commit(draft, journal)
    }

    /// The user abandoned the handoff (RESP-08).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`], [`Refusal::NotActive`] or [`Refusal::Persistence`].
    pub fn abandon(
        &mut self,
        handoff_id: &str,
        reason: Option<String>,
        now: &Timestamp,
    ) -> Result<()> {
        let mut draft = self.draft(handoff_id)?;
        if draft.is_final() {
            return Err(Refusal::NotActive { state: draft.state });
        }
        // The user gave up on a tab that never got its spec, so the request behind it is
        // over too (§7.7). Without this the queue would keep it open: the Stop hook would go
        // on asking the agent for a spec at every end of turn (OPEN-06), and the session's
        // next handoff would be linked to it (OPEN-08) — both of them the opposite of what
        // pressing Abandon said. Every other state's request was answered long ago.
        let gave_up_on_the_request = draft.state == HandoffState::AwaitingSpec;
        let mut journal = Journal::default();
        journal.event(
            &draft,
            now,
            EventKind::Abandon,
            Some(draft.cursor.step_index),
            None,
        );
        journal
            .sends
            .push(text_send(&draft, SendKind::Abandon, reason.clone(), now));
        self.finalise(
            &mut draft,
            OutcomeStatus::Abandoned,
            reason,
            Handover::QueueIfNobodyListens,
            now,
            &mut journal,
        )?;
        self.commit(draft, journal)?;
        // After the commit, like every other note the queue takes about a transition: a
        // request row that cannot be closed is a log line, never a reason to leave the
        // handoff open (PRIN-10, FM-28).
        if gave_up_on_the_request {
            self.requests.abandoned(&self.db, handoff_id);
        }
        Ok(())
    }

    /// "Done" on the last step: the agent verifies, or the user's word is the result
    /// (RESP-09, VER-04, PRIN-08).
    ///
    /// The overlay only offers it on the last step (§8.4); the store accepts it wherever the
    /// cursor stands and closes the round there, because a machine that refused would turn a
    /// view mistake into a handoff nobody can finish.
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`], [`Refusal::NotActive`] or [`Refusal::Persistence`].
    pub fn done(&mut self, handoff_id: &str, now: &Timestamp) -> Result<()> {
        let mut draft = self.active(handoff_id)?;
        let step = draft.cursor.step_index;
        let mut journal = Journal::default();
        if let Some(round) = draft.current_round_mut() {
            if !round.confirmed.contains(&step) && !round.skipped.contains(&step) {
                round.confirmed.push(step);
            }
            round.ended_at = Some(now.clone());
            let round = round.clone();
            journal.round(&draft, &round)?;
        }
        journal.event(&draft, now, EventKind::Confirm, Some(step), None);

        let has_verify = draft
            .spec
            .as_ref()
            .and_then(|spec| spec.verify.as_ref())
            .is_some();
        if has_verify {
            draft.state = HandoffState::AwaitingVerification;
            draft.verifying_since = Some(now.clone());
            draft.pending_question = None;
            journal.event(
                &draft,
                now,
                EventKind::State,
                None,
                Some(json!({"state": HandoffState::AwaitingVerification.as_str()})),
            );
            let outcome = build(&draft, OutcomeStatus::AwaitingVerification, None, None);
            hand_over(&mut draft, outcome, None, now, &mut journal);
        } else {
            self.finalise(
                &mut draft,
                OutcomeStatus::ConfirmedByUser,
                None,
                Handover::QueueIfNobodyListens,
                now,
                &mut journal,
            )?;
        }
        self.commit(draft, journal)
    }

    /// The user picked a deferred or parked handoff up again (RESP-07, FM-31).
    ///
    /// The handoff becomes active at once; getting the agent back to it is a request on the
    /// queue, delivered by the clipboard fast path or by the Stop hook.
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`], [`Refusal::NotActive`] or [`Refusal::Persistence`].
    pub fn resume_from_overlay(&mut self, handoff_id: &str, now: &Timestamp) -> Result<()> {
        let mut draft = self.draft(handoff_id)?;
        if !matches!(draft.state, HandoffState::Deferred | HandoffState::Parked) {
            return Err(Refusal::NotActive { state: draft.state });
        }
        draft.state = HandoffState::Active;
        let mut journal = Journal::default();
        journal.event(
            &draft,
            now,
            EventKind::State,
            None,
            Some(json!({"state": HandoffState::Active.as_str(), "by": "user"})),
        );
        let session_ref = draft.session_ref.clone();
        let id = draft.id.clone();
        self.commit(draft, journal)?;
        self.requests
            .request_resume(&self.db, &id, session_ref.as_deref());
        Ok(())
    }

    /// The user closed an outcome nobody came back for (SRV-23, FM-27).
    ///
    /// The app never closes an orphan on its own, so this is the only way a final outcome
    /// leaves the list without an agent taking it. It is recorded as delivered at the moment
    /// the user closed it: the orphan rule is the query "final, undelivered and old", and
    /// there is no third column to write.
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`], [`Refusal::NotActive`] or [`Refusal::Persistence`].
    pub fn close_orphan(&mut self, handoff_id: &str, now: &Timestamp) -> Result<()> {
        let mut draft = self.draft(handoff_id)?;
        if !draft.is_final() || draft.delivered_at.is_some() {
            return Err(Refusal::NotActive { state: draft.state });
        }
        draft.delivered_at = Some(now.clone());
        let mut journal = Journal::default();
        journal.event(
            &draft,
            now,
            EventKind::State,
            None,
            Some(json!({"state": draft.state.as_str(), "closed_by": "user"})),
        );
        self.commit(draft, journal)
    }

    /// The user corrected which request this handoff answers (FM-20, §12.4).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`] or [`Refusal::Persistence`].
    pub fn relink(&mut self, handoff_id: &str, request_id: &str, now: &Timestamp) -> Result<()> {
        let mut draft = self.draft(handoff_id)?;
        let request = user_requests::get(&self.db, request_id)?;
        draft.linked_request_id = Some(request_id.to_owned());
        if let Some(request) = request.as_ref() {
            draft.request_text = Some(request.text.clone());
        }
        let mut journal = Journal::default();
        journal.event(
            &draft,
            now,
            EventKind::State,
            None,
            Some(json!({"linked_request_id": request_id})),
        );
        let id = draft.id.clone();
        self.commit(draft, journal)?;
        if request.is_some() {
            // The queue also re-opens the request this handoff was answering: the one-click
            // correction of §12.4 must be undoable, and a request consumed by the wrong
            // handoff would otherwise be gone (`requests::queue::relink`).
            self.requests.linked(&self.db, &id, request_id);
        }
        Ok(())
    }

    // ----------------------------------------------------------------- system events

    /// A session's server went away (§8.3, FM-08, SRV-21, SRV-22, VER-06).
    ///
    /// Its calls stop waiting and a handoff that was waiting for its verification becomes
    /// `not_verified` at once — pessimistic on purpose, and reversible by a late report
    /// (DD-16). No other handoff state changes: a disconnection is a banner (§8.4).
    ///
    /// # Errors
    ///
    /// [`Refusal::Persistence`] when a write fails. The remaining handoffs are still
    /// processed, so one bad row does not strand the others.
    pub fn session_disconnected(&mut self, session_ref: &str, now: &Timestamp) -> Result<()> {
        let affected: Vec<String> = self
            .handoffs
            .values()
            .filter(|handoff| handoff.belongs_to(session_ref))
            .map(|handoff| handoff.id.clone())
            .collect();

        let mut first_error = None;
        for id in affected {
            if let Err(error) = self.disconnect_one(&id, session_ref, now) {
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// The thirty minutes of VER-06 ran out (§8.1, FM-08).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotFound`] or [`Refusal::Persistence`].
    pub fn verifying_timeout(&mut self, handoff_id: &str, now: &Timestamp) -> Result<()> {
        let mut draft = self.draft(handoff_id)?;
        if draft.state != HandoffState::AwaitingVerification || !draft.verifying_expired(now) {
            // The report arrived, or the window was restarted by a correction round.
            return Ok(());
        }
        let mut journal = Journal::default();
        self.finalise(
            &mut draft,
            OutcomeStatus::NotVerified,
            None,
            Handover::QueueIfNobodyListens,
            now,
            &mut journal,
        )?;
        self.commit(draft, journal)
    }

    /// The handoffs whose verification window has run out at `now`.
    #[must_use]
    pub fn expired_verifying(&self, now: &Timestamp) -> Vec<String> {
        self.handoffs
            .values()
            .filter(|handoff| {
                handoff.state == HandoffState::AwaitingVerification
                    && handoff.verifying_expired(now)
            })
            .map(|handoff| handoff.id.clone())
            .collect()
    }

    /// How long until the earliest verification window runs out.
    #[must_use]
    pub fn next_verifying_deadline(&self, now: &Timestamp) -> Option<Duration> {
        self.handoffs
            .values()
            .filter(|handoff| handoff.state == HandoffState::AwaitingVerification)
            .filter_map(|handoff| handoff.verifying_since.as_ref())
            .map(|since| {
                let elapsed = now.millis().saturating_sub(since.millis());
                u64::try_from(VERIFYING_TIMEOUT_MS.saturating_sub(elapsed)).unwrap_or(0)
            })
            .min()
            .map(Duration::from_millis)
    }

    // ------------------------------------------------------------------------ innards

    /// A copy of one handoff to apply a transition to (FM-28).
    fn draft(&self, handoff_id: &str) -> Result<Handoff> {
        self.handoffs
            .get(handoff_id)
            .cloned()
            .ok_or(Refusal::NotFound)
    }

    /// The same, refusing anything the user cannot act on right now.
    fn active(&self, handoff_id: &str) -> Result<Handoff> {
        let draft = self.draft(handoff_id)?;
        if draft.state == HandoffState::Active {
            Ok(draft)
        } else {
            Err(Refusal::NotActive { state: draft.state })
        }
    }

    fn mark_step(&mut self, handoff_id: &str, kind: EventKind, now: &Timestamp) -> Result<()> {
        let mut draft = self.active(handoff_id)?;
        let step = draft.cursor.step_index;
        let mut journal = Journal::default();
        if let Some(round) = draft.current_round_mut() {
            let list = match kind {
                EventKind::Skip => &mut round.skipped,
                _ => &mut round.confirmed,
            };
            if !list.contains(&step) {
                list.push(step);
            }
        }
        journal.event(&draft, now, kind, Some(step), None);
        draft.advance();
        self.commit(draft, journal)
    }

    /// Takes a handoff to a final state (§8.1, VER-01), and tells the runbook writer.
    fn finalise(
        &mut self,
        draft: &mut Handoff,
        status: OutcomeStatus,
        user_text: Option<String>,
        handover: Handover,
        now: &Timestamp,
        journal: &mut Journal,
    ) -> Result<Outcome> {
        let state = match status {
            OutcomeStatus::Verified => HandoffState::Verified,
            OutcomeStatus::Failed => HandoffState::Failed,
            OutcomeStatus::NotVerified => HandoffState::NotVerified,
            OutcomeStatus::ConfirmedByUser => HandoffState::ConfirmedByUser,
            OutcomeStatus::Abandoned => HandoffState::Abandoned,
            // `finalise` is private and every caller passes one of the five above; a sixth
            // would be a defect here, and leaving the handoff where it is beats inventing a
            // final state for it.
            _ => return Err(Refusal::NotActive { state: draft.state }),
        };
        draft.state = state;
        draft.closed_at = Some(now.clone());
        draft.verifying_since = None;
        draft.pending_question = None;
        if let Some(round) = draft.current_round_mut() {
            if round.ended_at.is_none() {
                round.ended_at = Some(now.clone());
            }
            let round = round.clone();
            journal.round(draft, &round)?;
        }
        journal.event(
            draft,
            now,
            EventKind::State,
            None,
            Some(json!({"state": state.as_str()})),
        );

        let outcome = build(draft, status, user_text, None);
        draft.final_outcome = Some(outcome.clone());
        match handover {
            Handover::QueueIfNobodyListens => {
                hand_over(draft, outcome.clone(), None, now, journal);
            }
            Handover::AnsweredByItsOwnRequest => {
                if draft.attached_call.is_some() {
                    hand_over(draft, outcome.clone(), None, now, journal);
                }
                draft.delivered_at = Some(now.clone());
            }
        }
        Ok(outcome)
    }

    fn disconnect_one(&mut self, id: &str, session_ref: &str, now: &Timestamp) -> Result<()> {
        let mut draft = self.draft(id)?;
        let mut journal = Journal::default();
        let mut touched = false;

        if let Some(call) = draft
            .attached_call
            .as_ref()
            .filter(|call| call.session_ref.as_deref() == Some(session_ref))
            .cloned()
        {
            draft.attached_call = None;
            journal.event(
                &draft,
                now,
                EventKind::Detach,
                None,
                Some(json!({"call": &call.call_id, "reason": "disconnected"})),
            );
            touched = true;
        }

        // VER-06: the session that owed the report is gone, so the honest answer is "not
        // verified" until one arrives late (DD-16).
        if draft.state == HandoffState::AwaitingVerification {
            self.finalise(
                &mut draft,
                OutcomeStatus::NotVerified,
                None,
                Handover::QueueIfNobodyListens,
                now,
                &mut journal,
            )?;
            touched = true;
        }

        if touched {
            self.commit(draft, journal)?;
        }
        Ok(())
    }

    /// Persists a transition and, only then, puts the handoff back (FM-28).
    fn commit(&mut self, draft: Handoff, journal: Journal) -> Result<()> {
        let was_final = self
            .handoffs
            .get(&draft.id)
            .is_some_and(super::handoff::Handoff::is_final);
        let row = row_of(&draft)?;
        commit_transition(
            &self.db,
            &Transition::of(&row)
                .with_rounds(&journal.rounds)
                .with_events(&journal.events)
                .with_sends(&journal.sends),
        )?;

        let final_state = draft.final_state().filter(|_| !was_final);
        let id = draft.id.clone();
        self.handoffs.insert(id.clone(), draft);
        self.outbox.extend(journal.deliveries);
        if let Some(final_state) = final_state {
            let handoff = self.handoffs.get(&id).expect("just inserted");
            self.runbooks.on_finalised(handoff, final_state);
        }
        // Last, and only here: a window told about a transition the disk refused would draw
        // a tab that does not exist (FM-28). Every path that changes a handoff comes
        // through this function, so the event fires once per transition and no caller has
        // to remember it.
        self.watchers.handoff_changed(&id);
        Ok(())
    }

    /// What was asked and what was answered on this handoff, oldest first (RESP-04,
    /// TOOL-04).
    ///
    /// Read from the diary rather than from the record, which keeps neither: §7.4 has no
    /// field for a question or a reply, because the question left in a tool result and the
    /// answer came back in the next call, and §7.6 shows both again on the step they
    /// referred to. One pass for one tab, which is why it is not part of
    /// [`HandoffSnapshot`].
    ///
    /// The two halves live in two tables, and each is where it is for a reason: a reply is
    /// an `events` row carrying its text, while a question is an `events` row (which has
    /// the round and the step) **and** a `sends` row (which has the text as it was sent,
    /// after the redaction of §7.10, and which is what LOG-02 asks to be able to show).
    /// `Store::ask` writes exactly one of each in one transaction, so the *n*-th of one is
    /// the *n*-th of the other.
    ///
    /// # Errors
    ///
    /// [`Refusal::Persistence`] when the rows cannot be read.
    pub fn exchanges(&self, handoff_id: &str) -> Result<Exchanges> {
        let events = crate::log::events::list_for_handoff(&self.db, handoff_id)?;
        let sends = crate::log::sends::list_for_handoff(&self.db, handoff_id)?;

        let replies = events
            .iter()
            .filter(|row| row.kind == EventKind::Reply)
            .filter_map(|row| {
                Some(Reply {
                    round: round_of(row),
                    step: step_of(row),
                    text: reply_text(row)?,
                    at: row.at.clone(),
                })
            })
            .collect();

        let asked = events.iter().filter(|row| row.kind == EventKind::Ask);
        let sent = sends
            .iter()
            .filter(|row| row.kind == crate::log::sends::SendKind::Question);
        let questions = asked
            .zip(sent)
            .filter_map(|(event, send)| {
                Some(Question {
                    round: round_of(event),
                    step: step_of(event),
                    text: send.text_as_sent.clone()?,
                    at: event.at.clone(),
                })
            })
            .collect();

        Ok(Exchanges { questions, replies })
    }

    /// The **true** value `key` names, as the copy button and the ten-second reveal need it
    /// (DET-04, GUIDE-02).
    ///
    /// One entry for a single value, one per item for an array, so that "copy the whole" and
    /// "copy this item" are the same call with and without an index. Empty when the handoff
    /// has no spec yet or declares no such value; there is no error to report, because the
    /// window drew the chip from the same spec.
    #[must_use]
    pub fn value(&self, handoff_id: &str, key: &str) -> Vec<String> {
        let Some(spec) = self
            .handoffs
            .get(handoff_id)
            .and_then(|it| it.spec.as_ref())
        else {
            return Vec::new();
        };
        match spec.values.get(key) {
            Some(crate::format::spec::SpecValue::One(value)) => vec![value.clone()],
            Some(crate::format::spec::SpecValue::Many(items)) => items.clone(),
            None => Vec::new(),
        }
    }
}

/// Delivers an outcome to the attached call, or queues it (§7.4, DD-12).
///
/// The call detaches either way: it has its answer. What has no listener is pushed onto
/// `undelivered`, which is where a resume looks first.
fn hand_over(
    draft: &mut Handoff,
    outcome: Outcome,
    image: Option<String>,
    now: &Timestamp,
    journal: &mut Journal,
) {
    match draft.attached_call.take() {
        Some(call) => {
            journal.event(
                draft,
                now,
                EventKind::Detach,
                None,
                Some(json!({"call": &call.call_id, "reason": "outcome"})),
            );
            if outcome.is_final {
                draft.delivered_at = Some(now.clone());
            }
            journal.deliveries.push(Delivery {
                conn_id: call.conn_id,
                call_id: call.call_id,
                handoff_id: draft.id.clone(),
                outcome,
                image,
            });
        }
        None => draft.undelivered.push_back(Queued { outcome, image }),
    }
}

/// Closes the current round and opens the next one over `steps` (§7.4, VER-09).
fn open_correction_round(
    draft: &mut Handoff,
    steps: Vec<HandoffStep>,
    now: &Timestamp,
    journal: &mut Journal,
) -> Result<()> {
    if let Some(round) = draft.current_round_mut() {
        round.ended_at = Some(now.clone());
        let round = round.clone();
        journal.round(draft, &round)?;
    }
    let no = draft.rounds.last().map_or(1, |round| round.no) + 1;
    let round = Round::new(no, steps, now);
    journal.round(draft, &round)?;
    draft.rounds.push(round);
    draft.cursor = Cursor {
        round: no,
        step_index: 1,
    };
    draft.state = HandoffState::Active;
    draft.closed_at = None;
    draft.delivered_at = None;
    draft.final_outcome = None;
    draft.verifying_since = None;
    journal.event(
        draft,
        now,
        EventKind::Replace,
        None,
        Some(json!({"round": no})),
    );
    journal.event(
        draft,
        now,
        EventKind::State,
        None,
        Some(json!({"state": HandoffState::Active.as_str()})),
    );
    Ok(())
}

/// The round an `events` row belongs to, defaulting to the first (§7.11 allows a null).
fn round_of(row: &EventRow) -> u32 {
    u32::try_from(row.round.unwrap_or(1)).unwrap_or(1)
}

/// The 1-based step an `events` row is about, defaulting to the first.
fn step_of(row: &EventRow) -> u32 {
    u32::try_from(row.step_index.unwrap_or(1)).unwrap_or(1)
}

/// The words an agent's `reply` row carries, when it carries any.
fn reply_text(row: &EventRow) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_str(row.payload_json.as_deref()?).ok()?;
    payload
        .get("text")
        .and_then(serde_json::Value::as_str)
        .map(std::borrow::ToOwned::to_owned)
}

/// Whether a handoff is still waiting for the spec that will fill it (OPEN-04, DD-13).
fn is_waiting_for_a_spec(handoff: &Handoff) -> bool {
    handoff.state == HandoffState::AwaitingSpec || handoff.spec.is_none()
}

/// The value names a replacement step cites that the spec never declared (`unknown_value_key`).
fn undeclared_value_keys(handoff: &Handoff, steps: &[HandoffStep]) -> Vec<String> {
    let Some(spec) = handoff.spec.as_ref() else {
        return Vec::new();
    };
    let mut unknown: Vec<String> = Vec::new();
    for name in steps
        .iter()
        .filter_map(|step| step.values.as_ref())
        .flatten()
    {
        if !spec.values.contains_key(name) && !unknown.contains(name) {
            unknown.push(name.clone());
        }
    }
    unknown
}

/// A `sends` row for the three actions that carry a text the user typed.
fn text_send(handoff: &Handoff, kind: SendKind, text: Option<String>, at: &Timestamp) -> SendRow {
    SendRow {
        id: 0,
        handoff_id: handoff.id.clone(),
        at: at.clone(),
        kind,
        text_as_sent: text,
        image_sha256: None,
        image_w: None,
        image_h: None,
        redaction_boxes_json: None,
        ocr_engine: None,
        patterns_version: None,
    }
}

/// The handoff as its row, with the state serialised into `state_json`.
fn row_of(handoff: &Handoff) -> Result<HandoffRow> {
    let state_json = serde_json::to_string(handoff).map_err(|error| {
        Refusal::Persistence(StoreError::of("writing the handoff state", error))
    })?;
    let resumed_from_json = handoff
        .resumed_from
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| {
            Refusal::Persistence(StoreError::of("writing the handoff state", error))
        })?;
    Ok(HandoffRow {
        id: handoff.id.clone(),
        created_at: handoff.created_at.clone(),
        closed_at: handoff.closed_at.clone(),
        session_ref: handoff.session_ref.clone(),
        agent_id: handoff.agent_id.clone(),
        client_name: handoff.client_name.clone(),
        project_dir: handoff.project_dir.clone(),
        request_text: handoff.request_text.clone(),
        state: handoff.state,
        final_state: handoff.final_state().map(FinalState::state),
        spec: handoff.spec.clone(),
        state_json,
        delivered_at: handoff.delivered_at.clone(),
        resumed_from_json,
        lang: handoff.lang.clone(),
    })
}

/// One round as its row.
fn round_row(handoff_id: &str, round: &Round) -> Result<RoundRow> {
    let steps_json = serde_json::to_string(&round.steps)
        .map_err(|error| Refusal::Persistence(StoreError::of("writing a round", error)))?;
    Ok(RoundRow {
        handoff_id: handoff_id.to_owned(),
        no: i64::from(round.no),
        steps_json,
        started_at: round.started_at.clone(),
        ended_at: round.ended_at.clone(),
        verify_ok: round.verify.as_ref().and_then(|report| report.ok),
        verify_detail: round
            .verify
            .as_ref()
            .and_then(|report| report.detail.clone()),
        verify_reported_at: round
            .verify
            .as_ref()
            .map(|report| Timestamp::parse(&report.reported_at))
            .transpose()?,
        verify_late: round.verify.as_ref().is_some_and(|report| report.late),
    })
}

/// The handoff a row describes, with the spec taken from its own column.
///
/// What comes back is the **masked** spec (LOG-02): the true one lived in memory and did
/// not survive the process. Everything a restarted app does with it — showing the steps,
/// building a `context`, writing a runbook — therefore shows the mask where a value was
/// treated as a secret, which is the same thing the user was already being shown.
fn restore(row: &HandoffRow) -> Result<Handoff> {
    let mut handoff: Handoff = serde_json::from_str(&row.state_json).map_err(|error| {
        Refusal::Persistence(StoreError::of("reading the handoff state", error))
    })?;
    handoff.id.clone_from(&row.id);
    handoff.spec.clone_from(&row.spec);
    handoff.attached_call = None;
    Ok(handoff)
}

/// The projection a view draws one tab from.
fn snapshot_of(handoff: &Handoff, now: &Timestamp) -> HandoffSnapshot {
    let round = handoff.current_round();
    HandoffSnapshot {
        id: handoff.id.clone(),
        state: handoff.state,
        round: handoff.cursor.round,
        step_index: handoff.cursor.step_index,
        step_total: handoff.step_count(),
        steps: round.map(|round| round.steps.clone()).unwrap_or_default(),
        confirmed: round
            .map(|round| round.confirmed.clone())
            .unwrap_or_default(),
        skipped: round.map(|round| round.skipped.clone()).unwrap_or_default(),
        notes: round.map(|round| round.notes.clone()).unwrap_or_default(),
        deferral_count: handoff.deferral_count,
        pending_question: handoff.pending_question.clone(),
        undelivered: handoff.undelivered.len(),
        call_attached: handoff.attached_call.is_some(),
        session_ref: handoff.session_ref.clone(),
        opener_label: handoff.opener_label.clone(),
        project_dir: handoff.project_dir.clone(),
        goal: handoff.spec.as_ref().map(|spec| spec.goal.clone()),
        location: handoff.spec.as_ref().map(|spec| spec.r#where.clone()),
        url: handoff.spec.as_ref().and_then(|spec| spec.url.clone()),
        lang: handoff.lang.clone(),
        values: handoff
            .spec
            .as_ref()
            .map(|spec| spec.values.clone())
            .unwrap_or_default(),
        secrets: handoff.spec.as_ref().and_then(|spec| spec.secrets.clone()),
        secret_treated: handoff.secret_treated.clone(),
        history: handoff
            .rounds
            .iter()
            .filter(|past| past.no != handoff.cursor.round)
            .map(summary_of)
            .collect(),
        verify_report: handoff
            .last_round()
            .and_then(|last| last.verify.clone())
            .or_else(|| {
                // A late report (DD-16) lands on the round it belongs to, which is not
                // always the last one; the banner still has to say "declared by agent".
                handoff
                    .rounds
                    .iter()
                    .rev()
                    .find_map(|round| round.verify.clone())
            }),
        verify: handoff.spec.as_ref().and_then(|spec| spec.verify.clone()),
        request_text: handoff.request_text.clone(),
        linked_request_id: handoff.linked_request_id.clone(),
        resumed_from: handoff.resumed_from.clone(),
        final_outcome: handoff.final_outcome.clone(),
        orphan: handoff.is_orphan(now),
        created_at: handoff.created_at.clone(),
        closed_at: handoff.closed_at.clone(),
    }
}

/// One round of the history (§7.6).
fn summary_of(round: &Round) -> RoundSummary {
    RoundSummary {
        no: round.no,
        steps: round.steps.clone(),
        confirmed: round.confirmed.clone(),
        skipped: round.skipped.clone(),
        notes: round.notes.clone(),
        verify: round.verify.clone(),
    }
}

// ------------------------------------------------------------------------- the actor

/// One message to the store, with the channel its answer comes back on (DD-11).
///
/// Every variant is one of the operations of [`Store`]; the actor exists to serialise them,
/// not to add behaviour.
#[derive(Debug)]
pub enum Command {
    /// `handoff.open` (§6.3).
    Open(
        Box<OpenParams>,
        Timestamp,
        oneshot::Sender<Result<OpenAccepted>>,
    ),
    /// `handoff.continue` (§6.3).
    Continue {
        /// Which handoff.
        handoff_id: String,
        /// The call that re-attaches.
        call: Call,
        /// What the agent replied.
        reply: String,
        /// New steps for the rest of the round.
        replacement_steps: Option<Vec<HandoffStep>>,
        /// When.
        at: Timestamp,
        /// Where the answer goes.
        reply_to: oneshot::Sender<Result<()>>,
    },
    /// `handoff.resume` (§5.7).
    Resume {
        /// Which handoff.
        handoff_id: String,
        /// The call that attaches.
        call: Call,
        /// When.
        at: Timestamp,
        /// Where the snapshot goes.
        reply_to: oneshot::Sender<Result<ResumeSnapshot>>,
    },
    /// `handoff.verify` (§4.4).
    Verify {
        /// Which handoff.
        handoff_id: String,
        /// `null` when the agent could not verify.
        ok: Option<bool>,
        /// What it found.
        detail: Option<String>,
        /// When.
        at: Timestamp,
        /// Where the outcome goes.
        reply_to: oneshot::Sender<Result<VerifyAccepted>>,
    },
    /// `handoff.detach_call` (DD-24).
    DetachCall {
        /// Which handoff.
        handoff_id: String,
        /// Which call.
        call_id: String,
        /// Why.
        reason: DetachReason,
        /// When.
        at: Timestamp,
        /// Where the acknowledgement goes.
        reply_to: oneshot::Sender<Result<()>>,
    },
    /// The user typed a request in the sheet of §7.7: open the tab that waits for its spec.
    OpenRequest {
        /// The queue entry's id, which the handoff takes over (DD-13).
        id: String,
        /// What the user typed (OPEN-04).
        request_text: String,
        /// The session it was addressed to, when one was selected (OPEN-04a).
        opener: Option<Box<Opener>>,
        /// When.
        at: Timestamp,
        /// Where the answer goes.
        reply_to: oneshot::Sender<Result<()>>,
    },
    /// One of the actions a person takes in the overlay.
    User {
        /// Which one.
        action: UserAction,
        /// Which handoff.
        handoff_id: String,
        /// When.
        at: Timestamp,
        /// Where the answer goes.
        reply_to: oneshot::Sender<Result<()>>,
    },
    /// A session's server went away (§8.3).
    SessionDisconnected {
        /// Which session.
        session_ref: String,
        /// When.
        at: Timestamp,
        /// Where the answer goes.
        reply_to: oneshot::Sender<Result<()>>,
    },
    /// The verification window of VER-06 ran out.
    VerifyingTimeout {
        /// Which handoff.
        handoff_id: String,
        /// When.
        at: Timestamp,
        /// Where the answer goes.
        reply_to: oneshot::Sender<Result<()>>,
    },
    /// One tab (§7.6).
    Snapshot {
        /// Which handoff.
        handoff_id: String,
        /// When, for the orphan flag.
        at: Timestamp,
        /// Where the projection goes.
        reply_to: oneshot::Sender<Option<HandoffSnapshot>>,
    },
    /// Every tab (§7.6).
    ListForUi {
        /// When, for the orphan flags.
        at: Timestamp,
        /// Where the projections go.
        reply_to: oneshot::Sender<Vec<HandoffSnapshot>>,
    },
    /// What was asked and answered on one handoff (§7.6, RESP-04).
    Exchanges {
        /// Which handoff.
        handoff_id: String,
        /// Where they go; empty when the rows cannot be read.
        reply_to: oneshot::Sender<Exchanges>,
    },
    /// The true value behind a chip (DET-04, GUIDE-02).
    Value {
        /// Which handoff.
        handoff_id: String,
        /// Which key of the spec's `values`.
        key: String,
        /// One entry for a single value, one per item for an array.
        reply_to: oneshot::Sender<Vec<String>>,
    },
}

/// What a person did in the overlay (RESP-01..09, SRV-23, FM-20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserAction {
    /// Step done, go to the next (RESP-01).
    Confirm,
    /// Annotate the step (RESP-02).
    Note(String),
    /// Skip the step (RESP-01).
    Skip,
    /// Ask the agent (RESP-04).
    Ask(String),
    /// Send what they see (CAP-01).
    Screenshot(Box<ScreenshotPayload>),
    /// Defer (RESP-05, RESP-07).
    Defer(Option<String>),
    /// Abandon (RESP-08).
    Abandon(Option<String>),
    /// Done on the last step (RESP-09).
    Done,
    /// Pick a parked handoff up again (FM-31).
    ResumeFromOverlay,
    /// Close a final outcome nobody collected (SRV-23).
    CloseOrphan,
    /// Correct which request this handoff answers (FM-20).
    Relink(String),
}

/// How the rest of the app reaches the store: by asking, never by holding it (DD-11).
#[derive(Debug, Clone)]
pub struct StoreHandle {
    commands: mpsc::Sender<Command>,
}

impl StoreHandle {
    /// Sends a command, and gives up quietly when the actor is gone.
    ///
    /// A store that has stopped means the app is shutting down; a caller that treated that
    /// as a failure would put an error in front of a user who is already leaving.
    async fn ask<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<T>) -> Command,
        gone: impl FnOnce() -> T,
    ) -> T {
        let (sender, receiver) = oneshot::channel();
        if self.commands.send(make(sender)).await.is_err() {
            return gone();
        }
        receiver.await.unwrap_or_else(|_| gone())
    }

    /// `handoff.open`.
    pub async fn open(&self, params: OpenParams, at: Timestamp) -> Result<OpenAccepted> {
        self.ask(
            |reply_to| Command::Open(Box::new(params), at, reply_to),
            || Err(Refusal::NotFound),
        )
        .await
    }

    /// The tab a user's request opens, waiting for the spec an agent has not sent yet
    /// (OPEN-04).
    pub async fn open_request(
        &self,
        id: String,
        request_text: String,
        opener: Option<Opener>,
        at: Timestamp,
    ) -> Result<()> {
        self.ask(
            |reply_to| Command::OpenRequest {
                id,
                request_text,
                opener: opener.map(Box::new),
                at,
                reply_to,
            },
            || Err(Refusal::NotFound),
        )
        .await
    }

    /// `handoff.continue`.
    pub async fn continue_handoff(
        &self,
        handoff_id: String,
        call: Call,
        reply: String,
        replacement_steps: Option<Vec<HandoffStep>>,
        at: Timestamp,
    ) -> Result<()> {
        self.ask(
            |reply_to| Command::Continue {
                handoff_id,
                call,
                reply,
                replacement_steps,
                at,
                reply_to,
            },
            || Err(Refusal::NotFound),
        )
        .await
    }

    /// `handoff.resume`.
    pub async fn resume(
        &self,
        handoff_id: String,
        call: Call,
        at: Timestamp,
    ) -> Result<ResumeSnapshot> {
        self.ask(
            |reply_to| Command::Resume {
                handoff_id,
                call,
                at,
                reply_to,
            },
            || Err(Refusal::NotFound),
        )
        .await
    }

    /// `handoff.verify`.
    pub async fn verify(
        &self,
        handoff_id: String,
        ok: Option<bool>,
        detail: Option<String>,
        at: Timestamp,
    ) -> Result<VerifyAccepted> {
        self.ask(
            |reply_to| Command::Verify {
                handoff_id,
                ok,
                detail,
                at,
                reply_to,
            },
            || Err(Refusal::NotFound),
        )
        .await
    }

    /// `handoff.detach_call`.
    pub async fn detach_call(
        &self,
        handoff_id: String,
        call_id: String,
        reason: DetachReason,
        at: Timestamp,
    ) -> Result<()> {
        self.ask(
            |reply_to| Command::DetachCall {
                handoff_id,
                call_id,
                reason,
                at,
                reply_to,
            },
            || Err(Refusal::NotFound),
        )
        .await
    }

    /// One of the actions a person takes in the overlay.
    pub async fn user(&self, handoff_id: String, action: UserAction, at: Timestamp) -> Result<()> {
        self.ask(
            |reply_to| Command::User {
                action,
                handoff_id,
                at,
                reply_to,
            },
            || Err(Refusal::NotFound),
        )
        .await
    }

    /// The verification window of VER-06 ran out. The actor fires this on its own timer;
    /// the method exists for a caller that knows better, such as a test or the startup sweep
    /// of §7.2 over handoffs whose window ran out while the app was not running.
    pub async fn verifying_timeout(&self, handoff_id: String, at: Timestamp) -> Result<()> {
        self.ask(
            |reply_to| Command::VerifyingTimeout {
                handoff_id,
                at,
                reply_to,
            },
            || Err(Refusal::NotFound),
        )
        .await
    }

    /// A session's server went away.
    pub async fn session_disconnected(&self, session_ref: String, at: Timestamp) -> Result<()> {
        self.ask(
            |reply_to| Command::SessionDisconnected {
                session_ref,
                at,
                reply_to,
            },
            || Err(Refusal::NotFound),
        )
        .await
    }

    /// One tab.
    pub async fn snapshot(&self, handoff_id: String, at: Timestamp) -> Option<HandoffSnapshot> {
        self.ask(
            |reply_to| Command::Snapshot {
                handoff_id,
                at,
                reply_to,
            },
            || None,
        )
        .await
    }

    /// Every tab.
    pub async fn list_for_ui(&self, at: Timestamp) -> Vec<HandoffSnapshot> {
        self.ask(|reply_to| Command::ListForUi { at, reply_to }, Vec::new)
            .await
    }

    /// What was asked and what was answered on one handoff, oldest first.
    pub async fn exchanges(&self, handoff_id: String) -> Exchanges {
        self.ask(
            |reply_to| Command::Exchanges {
                handoff_id,
                reply_to,
            },
            Exchanges::default,
        )
        .await
    }

    /// The true value behind a chip, one entry per item.
    pub async fn value(&self, handoff_id: String, key: String) -> Vec<String> {
        self.ask(
            |reply_to| Command::Value {
                handoff_id,
                key,
                reply_to,
            },
            Vec::new,
        )
        .await
    }
}

/// Starts the store's task and returns the handle and the stream of outcomes to send.
///
/// The receiver is the only source of `handoff.event`: outcomes an action produced, the
/// `transferred_to_other_session` of a takeover, and the ones a timer or a disconnect
/// produced that no request is waiting for.
// TASK: T-034 — the dispatch drains the deliveries and sends them on the recorded conn_id.
#[must_use]
pub fn spawn(store: Store) -> (StoreHandle, mpsc::Receiver<Delivery>) {
    let (commands, rx) = mpsc::channel(COMMAND_QUEUE);
    let (deliveries, out) = mpsc::channel(COMMAND_QUEUE);
    tokio::spawn(run(store, rx, deliveries));
    (StoreHandle { commands }, out)
}

/// The actor loop: one command at a time, and the verification timer of VER-06 beside it.
async fn run(
    mut store: Store,
    mut commands: mpsc::Receiver<Command>,
    deliveries: mpsc::Sender<Delivery>,
) {
    loop {
        let wait = store
            .next_verifying_deadline(&Timestamp::now())
            .unwrap_or(Duration::from_secs(3600));
        let timer = tokio::time::sleep(wait);
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { return };
                apply(&mut store, command);
            }
            () = timer => {
                let now = Timestamp::now();
                for id in store.expired_verifying(&now) {
                    if let Err(error) = store.verifying_timeout(&id, &now) {
                        tracing::error!(error = %error, "the verification timeout could not be recorded");
                    }
                }
            }
        }
        for delivery in store.take_deliveries() {
            if deliveries.send(delivery).await.is_err() {
                // Nobody is sending outcomes any more: the channel is down or the app is
                // leaving. The outcomes stay in the handoffs' queues, which is where a
                // resume looks for them (DD-12).
                return;
            }
        }
    }
}

/// Applies one command. A receiver that has gone away is not an error: the caller stopped
/// waiting, and the transition is still the store's to record.
fn apply(store: &mut Store, command: Command) {
    match command {
        Command::Open(params, at, reply_to) => {
            let _ = reply_to.send(store.open(*params, &at));
        }
        Command::OpenRequest {
            id,
            request_text,
            opener,
            at,
            reply_to,
        } => {
            let result = store.open_request(&id, &request_text, opener.as_deref(), &at);
            let _ = reply_to.send(result);
        }
        Command::Continue {
            handoff_id,
            call,
            reply,
            replacement_steps,
            at,
            reply_to,
        } => {
            let result = store.continue_handoff(&handoff_id, &call, &reply, replacement_steps, &at);
            let _ = reply_to.send(result);
        }
        Command::Resume {
            handoff_id,
            call,
            at,
            reply_to,
        } => {
            let _ = reply_to.send(store.resume(&handoff_id, &call, &at));
        }
        Command::Verify {
            handoff_id,
            ok,
            detail,
            at,
            reply_to,
        } => {
            let _ = reply_to.send(store.verify(&handoff_id, ok, detail, &at));
        }
        Command::DetachCall {
            handoff_id,
            call_id,
            reason,
            at,
            reply_to,
        } => {
            let _ = reply_to.send(store.detach_call(&handoff_id, &call_id, reason, &at));
        }
        Command::User {
            action,
            handoff_id,
            at,
            reply_to,
        } => {
            let _ = reply_to.send(apply_user(store, &handoff_id, action, &at));
        }
        Command::SessionDisconnected {
            session_ref,
            at,
            reply_to,
        } => {
            let _ = reply_to.send(store.session_disconnected(&session_ref, &at));
        }
        Command::VerifyingTimeout {
            handoff_id,
            at,
            reply_to,
        } => {
            let _ = reply_to.send(store.verifying_timeout(&handoff_id, &at));
        }
        Command::Snapshot {
            handoff_id,
            at,
            reply_to,
        } => {
            let _ = reply_to.send(store.snapshot(&handoff_id, &at));
        }
        Command::ListForUi { at, reply_to } => {
            let _ = reply_to.send(store.list_for_ui(&at));
        }
        Command::Exchanges {
            handoff_id,
            reply_to,
        } => {
            // A diary that cannot be read costs the tab its questions and its replies and
            // nothing else; the step, the values and the buttons are all in memory.
            let exchanges = store.exchanges(&handoff_id).unwrap_or_else(|error| {
                tracing::warn!(error = %error, handoff_id, "the exchanges could not be read");
                Exchanges::default()
            });
            let _ = reply_to.send(exchanges);
        }
        Command::Value {
            handoff_id,
            key,
            reply_to,
        } => {
            let _ = reply_to.send(store.value(&handoff_id, &key));
        }
    }
}

/// One user action against the store.
pub fn apply_user(
    store: &mut Store,
    handoff_id: &str,
    action: UserAction,
    at: &Timestamp,
) -> Result<()> {
    match action {
        UserAction::Confirm => store.confirm(handoff_id, at),
        UserAction::Note(text) => store.note(handoff_id, &text, at),
        UserAction::Skip => store.skip(handoff_id, at),
        UserAction::Ask(text) => store.ask(handoff_id, &text, at),
        UserAction::Screenshot(payload) => store.screenshot(handoff_id, &payload, at),
        UserAction::Defer(reason) => store.defer(handoff_id, reason, at),
        UserAction::Abandon(reason) => store.abandon(handoff_id, reason, at),
        UserAction::Done => store.done(handoff_id, at),
        UserAction::ResumeFromOverlay => store.resume_from_overlay(handoff_id, at),
        UserAction::CloseOrphan => store.close_orphan(handoff_id, at),
        UserAction::Relink(request_id) => store.relink(handoff_id, &request_id, at),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    use indexmap::IndexMap;

    use super::*;
    use crate::format::outcome::ScreenshotMode;
    use crate::format::spec::SpecValue;
    use crate::log::sessions;
    use crate::log::testing::{at, session};
    use crate::store::runbook_sink::testing::{CountingRequests, RecordingSink};

    const OPENER: &str = "ses_00000001";
    const OTHER: &str = "ses_00000002";

    fn database() -> Db {
        let db = Db::open_in_memory().expect("a database");
        for reference in [OPENER, OTHER] {
            sessions::register(&db, &session(reference)).expect("a session");
        }
        db
    }

    /// A store built as the app builds it, with the real user-request queue behind it: the
    /// linking rules of OPEN-08 and FM-20 are the queue's, and a test store that had none
    /// would pass whatever they did.
    fn store() -> Store {
        Store::load(
            database(),
            Box::new(NoRunbookSink),
            Box::new(Arc::new(crate::requests::Queue::new(Box::new(
                crate::requests::NoRequestObserver,
            )))),
        )
        .expect("an empty store")
    }

    fn watched() -> (Store, Arc<RecordingSink>, Arc<CountingRequests>) {
        let sink = Arc::new(RecordingSink::default());
        let resumes = Arc::new(CountingRequests::default());
        let store = Store::load(
            database(),
            Box::new(Arc::clone(&sink)),
            Box::new(Arc::clone(&resumes)),
        )
        .expect("an empty store");
        (store, sink, resumes)
    }

    fn spec(steps: usize, verify: bool) -> HandoffSpec {
        let mut values = IndexMap::new();
        values.insert(
            "endpoint_url".to_owned(),
            SpecValue::One("https://api.example.test/hook".to_owned()),
        );
        HandoffSpec {
            spec_version: 1,
            goal: "Register the webhook".to_owned(),
            r#where: "Dashboard, then Webhooks".to_owned(),
            url: None,
            why_human: "only a person can log in".to_owned(),
            values,
            secrets: None,
            steps: (1..=steps)
                .map(|index| HandoffStep {
                    text: format!("step {index}"),
                    url: None,
                    values: Some(vec!["endpoint_url".to_owned()]),
                    warning: None,
                })
                .collect(),
            verify: verify.then(|| "the webhook fires".to_owned()),
            lang: Some("en".to_owned()),
        }
    }

    fn opener(session_ref: &str) -> Opener {
        Opener {
            session_ref: session_ref.to_owned(),
            agent_id: Some("claude-code".to_owned()),
            client_name: Some("claude-code".to_owned()),
            project_dir: Some("C:/projects/baton".to_owned()),
            label: ResumedFrom {
                agent: "Claude Code".to_owned(),
                project: "baton".to_owned(),
            },
        }
    }

    fn call(conn_id: u64, call_id: &str, session_ref: &str) -> Call {
        Call {
            conn_id,
            call_id: call_id.to_owned(),
            session_ref: Some(session_ref.to_owned()),
        }
    }

    fn open_with(store: &mut Store, spec: HandoffSpec) -> String {
        store
            .open(
                OpenParams {
                    spec,
                    secret_treated: Vec::new(),
                    request_id: None,
                    opener: opener(OPENER),
                    call: call(1, "call_00000001", OPENER),
                },
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("the open is accepted")
            .handoff_id
    }

    fn screenshot(sha: &str) -> ScreenshotPayload {
        ScreenshotPayload {
            mode: ScreenshotMode::Image,
            text: None,
            image_base64: Some("UE5H".to_owned()),
            image_sha256: Some(sha.to_owned()),
            width: 1600,
            height: 900,
            redactions: 1,
            redaction_boxes_json: Some("[]".to_owned()),
            ocr_engine: Some("vision".to_owned()),
            patterns_version: Some("1".to_owned()),
            comment: Some("is this the right page?".to_owned()),
        }
    }

    // ------------------------------------------------------------------ open, guidance

    #[test]
    fn an_open_creates_an_active_handoff_on_the_first_step_with_the_call_attached() {
        let mut store = store();
        let id = open_with(&mut store, spec(3, true));

        assert!(ids::HANDOFF_ID_RE.is_match(&id), "{id}");
        let snapshot = store
            .snapshot(&id, &at("2026-09-08T11:00:00Z"))
            .expect("a tab");
        assert_eq!(snapshot.state, HandoffState::Active);
        assert_eq!(snapshot.round, 1);
        assert_eq!(snapshot.step_index, 1);
        assert_eq!(snapshot.step_total, 3);
        assert!(snapshot.call_attached);
        assert_eq!(snapshot.undelivered, 0);
        assert!(
            store.take_deliveries().is_empty(),
            "an open answers nothing"
        );
    }

    #[test]
    fn a_user_request_opens_a_tab_that_is_waiting_for_a_spec() {
        // OPEN-04: the tab appears the moment the user presses Enter, before any agent has
        // been asked anything.
        let mut store = store();
        let opened = at("2026-09-08T10:00:00Z");
        store
            .open_request(
                "hf_9p2r4k7m3t",
                "I'm about to create the API key on Stripe",
                Some(&opener(OPENER)),
                &opened,
            )
            .expect("the request opens a tab");

        let snapshot = store
            .snapshot("hf_9p2r4k7m3t", &at("2026-09-08T10:00:01Z"))
            .expect("the tab is there");
        assert_eq!(snapshot.state, HandoffState::AwaitingSpec);
        assert_eq!(
            snapshot.request_text.as_deref(),
            Some("I'm about to create the API key on Stripe")
        );
        assert_eq!(snapshot.session_ref.as_deref(), Some(OPENER));
        assert_eq!(snapshot.created_at, opened);
        // Nothing a spec would have filled: no goal, no steps, nothing to walk.
        assert_eq!(snapshot.goal, None);
        assert_eq!(snapshot.step_total, 0);
        assert!(snapshot.steps.is_empty());
        assert!(!snapshot.call_attached);
    }

    #[test]
    fn a_request_with_no_session_still_opens_its_tab() {
        // OPEN-04a: the sheet opens with nothing registered, and the request waits for the
        // first session that does.
        let mut store = store();
        store
            .open_request(
                "hf_9p2r4k7m3t",
                "book the domain",
                None,
                &at("2026-09-08T10:00:00Z"),
            )
            .expect("the request opens a tab");
        let snapshot = store
            .snapshot("hf_9p2r4k7m3t", &at("2026-09-08T10:00:01Z"))
            .expect("the tab is there");
        assert_eq!(snapshot.session_ref, None);
        assert_eq!(snapshot.opener_label, None);
    }

    #[test]
    fn a_request_never_replaces_a_handoff_that_already_exists() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, false));
        let refused = store
            .open_request(&id, "something else", None, &at("2026-09-08T12:00:00Z"))
            .expect_err("a live handoff is not overwritten");
        assert!(matches!(refused, Refusal::NotActive { .. }), "{refused:?}");
        assert_eq!(
            store
                .snapshot(&id, &at("2026-09-08T12:00:01Z"))
                .expect("still there")
                .goal,
            Some("Register the webhook".to_owned())
        );
    }

    #[test]
    fn the_spec_fills_the_tab_the_user_opened_and_keeps_its_instant() {
        // DD-13, §7.7: one tab from "waiting for spec" to the final state, with the instant
        // the *user* opened it and the words they typed.
        let mut store = store();
        let request_id = "hf_9p2r4k7m3t";
        let opened = at("2026-09-08T10:00:00Z");
        store
            .open_request(
                request_id,
                "create the API key on Stripe",
                Some(&opener(OPENER)),
                &opened,
            )
            .expect("the request opens a tab");

        let accepted = store
            .open(
                OpenParams {
                    spec: spec(2, false),
                    secret_treated: Vec::new(),
                    request_id: Some(request_id.to_owned()),
                    opener: opener(OPENER),
                    call: call(1, "call_00000001", OPENER),
                },
                &at("2026-09-08T10:04:00Z"),
            )
            .expect("the spec is accepted");

        assert_eq!(accepted.handoff_id, request_id);
        let snapshot = store
            .snapshot(request_id, &at("2026-09-08T10:04:01Z"))
            .expect("the same tab");
        assert_eq!(snapshot.state, HandoffState::Active);
        assert_eq!(
            snapshot.created_at, opened,
            "the tab keeps the user's instant"
        );
        assert_eq!(
            snapshot.request_text.as_deref(),
            Some("create the API key on Stripe")
        );
        assert_eq!(snapshot.step_total, 2);
    }

    #[test]
    fn giving_up_on_a_tab_that_never_got_its_spec_closes_the_request() {
        // Without this the Stop hook would go on asking for a spec at every end of turn
        // (OPEN-06) and the session's next handoff would be linked to it (OPEN-08).
        let (mut store, _sink, requests) = watched();
        store
            .open_request(
                "hf_9p2r4k7m3t",
                "create the API key",
                Some(&opener(OPENER)),
                &at("2026-09-08T10:00:00Z"),
            )
            .expect("the request opens a tab");
        store
            .abandon("hf_9p2r4k7m3t", None, &at("2026-09-08T10:02:00Z"))
            .expect("abandon");

        assert_eq!(
            *requests.abandoned.lock().expect("not poisoned"),
            vec!["hf_9p2r4k7m3t".to_owned()]
        );
    }

    #[test]
    fn giving_up_on_a_handoff_that_has_a_spec_touches_no_request() {
        // Its request, if it had one, was answered when the spec arrived; closing it here
        // would close an entry that is already closed.
        let (mut store, _sink, requests) = watched();
        let id = open_with(&mut store, spec(2, false));
        store
            .abandon(
                &id,
                Some("not today".to_owned()),
                &at("2026-09-08T11:30:00Z"),
            )
            .expect("abandon");
        assert!(requests.abandoned.lock().expect("not poisoned").is_empty());
    }

    #[test]
    fn a_spec_carrying_a_request_id_takes_that_id_over() {
        let mut store = store();
        let request_id = "hf_9p2r4k7m3t";
        let accepted = store
            .open(
                OpenParams {
                    spec: spec(1, false),
                    secret_treated: Vec::new(),
                    request_id: Some(request_id.to_owned()),
                    opener: opener(OPENER),
                    call: call(1, "call_00000001", OPENER),
                },
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("the open is accepted");
        assert_eq!(accepted.handoff_id, request_id);
        assert_eq!(accepted.resumed_from, None);
    }

    #[test]
    fn confirm_skip_and_note_change_the_round_and_send_nothing() {
        let mut store = store();
        let id = open_with(&mut store, spec(3, true));
        let now = at("2026-09-08T11:05:00Z");

        store.confirm(&id, &now).expect("confirm");
        store
            .note(&id, "  the button is called Add destination  ", &now)
            .expect("note");
        store
            .note(&id, "   ", &now)
            .expect("a blank note is nothing");
        store.skip(&id, &now).expect("skip");

        let snapshot = store.snapshot(&id, &now).expect("a tab");
        assert_eq!(snapshot.confirmed, vec![1]);
        assert_eq!(snapshot.skipped, vec![2]);
        assert_eq!(snapshot.notes.len(), 1, "the blank note was not recorded");
        assert_eq!(snapshot.notes[0].step, 2);
        assert_eq!(
            snapshot.notes[0].text,
            "the button is called Add destination"
        );
        assert_eq!(snapshot.step_index, 3);
        assert!(
            store.take_deliveries().is_empty(),
            "RESP-03: local actions interrupt nobody"
        );
    }

    // --------------------------------------------------------- interrupting, queueing

    #[test]
    fn an_ask_returns_the_attached_call_and_a_second_one_queues() {
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        let now = at("2026-09-08T11:05:00Z");

        store.ask(&id, "which button?", &now).expect("ask");
        let delivered = store.take_deliveries();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].call_id, "call_00000001");
        assert_eq!(delivered[0].outcome.status, OutcomeStatus::Question);
        assert_eq!(
            delivered[0].outcome.user_text.as_deref(),
            Some("which button?")
        );
        assert!(delivered[0].outcome.context.is_some(), "CTX-01");

        let snapshot = store.snapshot(&id, &now).expect("a tab");
        assert!(
            !snapshot.call_attached,
            "the call detached with its outcome"
        );
        assert!(snapshot.pending_question.is_some());

        // No call is listening now, so the next interruption waits (DD-12).
        store.ask(&id, "and now?", &now).expect("a second ask");
        assert!(store.take_deliveries().is_empty());
        assert_eq!(store.snapshot(&id, &now).expect("a tab").undelivered, 1);
    }

    #[test]
    fn a_screenshot_sends_its_pixels_beside_the_outcome_and_stores_only_a_hash() {
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        let now = at("2026-09-08T11:05:00Z");

        store
            .screenshot(&id, &screenshot(&"a".repeat(64)), &now)
            .expect("screenshot");
        let delivered = store.take_deliveries();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].image.as_deref(), Some("UE5H"));
        let shot = delivered[0]
            .outcome
            .screenshot
            .clone()
            .expect("a screenshot");
        assert!(shot.image_attached);
        assert_eq!(shot.width, 1600);

        let sends = crate::log::sends::list_for_handoff(&store.db, &id).expect("the sends");
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].kind, crate::log::sends::SendKind::ScreenshotImage);
        assert_eq!(
            sends[0].image_sha256.as_deref(),
            Some("a".repeat(64).as_str())
        );
        assert_eq!(
            sends[0].text_as_sent.as_deref(),
            Some("is this the right page?")
        );
    }

    #[test]
    fn a_reply_clears_the_pending_question_and_re_attaches_the_call() {
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        let now = at("2026-09-08T11:05:00Z");
        store.ask(&id, "which button?", &now).expect("ask");
        store.take_deliveries();

        store
            .continue_handoff(
                &id,
                &call(1, "call_00000001", OPENER),
                "the blue one",
                None,
                &now,
            )
            .expect("the reply");
        let snapshot = store.snapshot(&id, &now).expect("a tab");
        assert!(snapshot.pending_question.is_none());
        assert!(
            snapshot.call_attached,
            "T-020: a continue re-attaches the call the app already knows"
        );
    }

    #[test]
    fn the_diary_gives_back_the_question_and_the_answer_on_the_step_they_belong_to() {
        // §7.6 shows a round trip, not half of one: the question the user asked and the
        // answer that came back, both on the step they referred to.
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        let asked = at("2026-09-08T11:05:00Z");
        store.ask(&id, "which button?", &asked).expect("ask");
        store.take_deliveries();
        store
            .continue_handoff(
                &id,
                &call(1, "call_00000001", OPENER),
                "the blue one",
                None,
                &at("2026-09-08T11:06:00Z"),
            )
            .expect("the reply");

        let exchanges = store.exchanges(&id).expect("the diary");
        assert_eq!(exchanges.questions.len(), 1);
        assert_eq!(exchanges.questions[0].text, "which button?");
        assert_eq!(exchanges.questions[0].round, 1);
        assert_eq!(exchanges.questions[0].step, 1);
        assert_eq!(exchanges.questions[0].at, asked);
        assert_eq!(exchanges.replies.len(), 1);
        assert_eq!(exchanges.replies[0].text, "the blue one");
        assert_eq!(exchanges.replies[0].step, 1);
    }

    #[test]
    fn a_handoff_nobody_asked_anything_on_has_an_empty_diary() {
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        assert_eq!(
            store.exchanges(&id).expect("the diary"),
            Exchanges::default()
        );
    }

    #[test]
    fn two_questions_keep_their_own_steps_and_their_own_words() {
        // The `events` row carries the step and the `sends` row the words, so the pairing
        // is what this is about: a second question must not take the first one's step.
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        store
            .ask(&id, "which button?", &at("2026-09-08T11:05:00Z"))
            .expect("the first question");
        store.take_deliveries();
        store
            .continue_handoff(
                &id,
                &call(1, "call_00000001", OPENER),
                "the blue one",
                None,
                &at("2026-09-08T11:06:00Z"),
            )
            .expect("the reply");
        store
            .confirm(&id, &at("2026-09-08T11:07:00Z"))
            .expect("the first step is done");
        store
            .ask(&id, "and now?", &at("2026-09-08T11:08:00Z"))
            .expect("the second question");

        let exchanges = store.exchanges(&id).expect("the diary");
        assert_eq!(
            exchanges
                .questions
                .iter()
                .map(|question| (question.step, question.text.as_str()))
                .collect::<Vec<_>>(),
            vec![(1, "which button?"), (2, "and now?")]
        );
    }

    #[test]
    fn a_reply_with_nothing_pending_and_no_replacement_steps_is_not_waiting() {
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        let error = store
            .continue_handoff(
                &id,
                &call(1, "call_00000001", OPENER),
                "here you go",
                None,
                &at("2026-09-08T11:05:00Z"),
            )
            .expect_err("FM-32");
        assert!(matches!(error, Refusal::NotWaiting));
        assert_eq!(
            error.code(),
            Some(crate::format::channel::ChannelErrorCode::NotWaiting)
        );
    }

    #[test]
    fn replacement_steps_citing_a_value_the_spec_never_declared_are_refused() {
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        let error = store
            .continue_handoff(
                &id,
                &call(1, "call_00000001", OPENER),
                "try this instead",
                Some(vec![HandoffStep {
                    text: "use the other endpoint".to_owned(),
                    url: None,
                    values: Some(vec!["secret_url".to_owned(), "endpoint_url".to_owned()]),
                    warning: None,
                }]),
                &at("2026-09-08T11:05:00Z"),
            )
            .expect_err("unknown_value_key");
        match error {
            Refusal::UnknownValueKey { keys } => assert_eq!(keys, vec!["secret_url".to_owned()]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn replacement_steps_open_the_next_round_with_the_counter_restarted() {
        let mut store = store();
        let id = open_with(&mut store, spec(3, true));
        let now = at("2026-09-08T11:05:00Z");
        store.confirm(&id, &now).expect("confirm");
        store.skip(&id, &now).expect("skip");

        store
            .continue_handoff(
                &id,
                &call(1, "call_00000001", OPENER),
                "start again from the error",
                Some(vec![HandoffStep {
                    text: "open the other page".to_owned(),
                    url: None,
                    values: None,
                    warning: None,
                }]),
                &now,
            )
            .expect("the correction");

        let snapshot = store.snapshot(&id, &now).expect("a tab");
        assert_eq!(snapshot.round, 2, "VER-09");
        assert_eq!(snapshot.step_index, 1);
        assert_eq!(snapshot.step_total, 1);
        assert!(
            snapshot.confirmed.is_empty(),
            "the counters restart per round"
        );
        assert!(snapshot.skipped.is_empty());
        assert_eq!(snapshot.state, HandoffState::Active);
    }

    // ------------------------------------------------------------ defer, park, resume

    #[test]
    fn one_deferral_defers_and_the_second_parks() {
        let (mut store, _sink, resumes) = watched();
        let id = open_with(&mut store, spec(2, true));
        let now = at("2026-09-08T11:05:00Z");

        store
            .defer(&id, Some("later today".to_owned()), &now)
            .expect("defer");
        let delivered = store.take_deliveries();
        assert_eq!(delivered[0].outcome.status, OutcomeStatus::Deferred);
        assert_eq!(delivered[0].outcome.deferral_count, 1);
        assert_eq!(
            store.snapshot(&id, &now).expect("a tab").state,
            HandoffState::Deferred
        );

        store.defer(&id, None, &now).expect("a second deferral");
        let snapshot = store.snapshot(&id, &now).expect("a tab");
        assert_eq!(snapshot.state, HandoffState::Parked);
        assert_eq!(snapshot.deferral_count, 2);

        store
            .resume_from_overlay(&id, &now)
            .expect("the user picks it up");
        assert_eq!(
            store.snapshot(&id, &now).expect("a tab").state,
            HandoffState::Active
        );
        assert_eq!(resumes.requested.load(Ordering::Relaxed), 1, "FM-31");
    }

    #[test]
    fn the_agent_resuming_a_deferred_handoff_makes_it_active_again() {
        // §8.1, `deferred --> active: agent resumes`, and `fixtures/channel/f05-defer-park`
        // answers exactly that resume with `{"state":"active"}`. A `parked` handoff is not
        // touched: its instruction told the agent not to resume, and only the user picks it
        // up (FM-31).
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        let now = at("2026-09-08T11:05:00Z");
        store.defer(&id, None, &now).expect("defer");
        store.take_deliveries();

        let snapshot = store
            .resume(&id, &call(1, "call_00000002", OPENER), &now)
            .expect("the agent comes back");
        assert_eq!(snapshot.state, HandoffState::Active);
        assert!(snapshot.outcome.is_none(), "the call attaches and waits");
        assert_eq!(
            store.snapshot(&id, &now).expect("a tab").state,
            HandoffState::Active
        );

        store.defer(&id, None, &now).expect("a second deferral");
        store.take_deliveries();
        assert_eq!(
            store.snapshot(&id, &now).expect("a tab").state,
            HandoffState::Parked
        );
        let parked = store
            .resume(&id, &call(1, "call_00000003", OPENER), &now)
            .expect("a resume of a parked handoff is still answered");
        assert_eq!(parked.state, HandoffState::Parked);
    }

    #[test]
    fn an_active_handoff_cannot_be_resumed_from_the_overlay() {
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        let error = store
            .resume_from_overlay(&id, &at("2026-09-08T11:05:00Z"))
            .expect_err("nothing to pick up");
        assert!(matches!(error, Refusal::NotActive { state } if state == HandoffState::Active));
        assert_eq!(error.code(), None, "a view mistake is not a protocol error");
    }

    // ---------------------------------------------------------------- done and verify

    #[test]
    fn done_with_a_verify_waits_for_the_agent_and_without_one_is_confirmed_by_the_user() {
        let (mut store, sink, _resumes) = watched();
        let now = at("2026-09-08T11:30:00Z");

        let asked = open_with(&mut store, spec(1, true));
        store.done(&asked, &now).expect("done");
        let delivered = store.take_deliveries();
        assert_eq!(
            delivered[0].outcome.status,
            OutcomeStatus::AwaitingVerification
        );
        assert!(!delivered[0].outcome.is_final);
        assert_eq!(
            store.snapshot(&asked, &now).expect("a tab").state,
            HandoffState::AwaitingVerification
        );
        assert_eq!(
            store.get(&asked).expect("it").verifying_since,
            Some(now.clone())
        );

        let plain = open_with(&mut store, spec(1, false));
        store.done(&plain, &now).expect("done");
        let delivered = store.take_deliveries();
        assert_eq!(delivered[0].outcome.status, OutcomeStatus::ConfirmedByUser);
        assert!(delivered[0].outcome.is_final);
        assert_eq!(
            sink.finalised.lock().expect("the sink").as_slice(),
            [(plain.clone(), FinalState::ConfirmedByUser)],
            "RUN-01: a confirmed handoff is a runbook"
        );
        assert_eq!(store.get(&plain).expect("it").delivered_at, Some(now));
    }

    #[test]
    fn a_report_of_true_false_or_null_takes_the_handoff_to_its_final_state() {
        for (ok, status, state) in [
            (Some(true), OutcomeStatus::Verified, HandoffState::Verified),
            (Some(false), OutcomeStatus::Failed, HandoffState::Failed),
            (None, OutcomeStatus::NotVerified, HandoffState::NotVerified),
        ] {
            let mut store = store();
            let id = open_with(&mut store, spec(1, true));
            let now = at("2026-09-08T11:30:00Z");
            store.done(&id, &now).expect("done");
            store.take_deliveries();

            let accepted = store
                .verify(&id, ok, Some("what I ran".to_owned()), &now)
                .expect("the report");
            assert_eq!(accepted.outcome.status, status);
            assert!(accepted.outcome.is_final);
            let report = accepted.outcome.verify.clone().expect("VER-05");
            assert_eq!(report.ok, ok);
            assert!(!report.late);
            assert_eq!(store.snapshot(&id, &now).expect("a tab").state, state);
        }
    }

    #[test]
    fn a_report_is_carried_by_its_own_request_and_is_not_queued_a_second_time() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, true));
        let now = at("2026-09-08T11:30:00Z");
        store.done(&id, &now).expect("done");
        store.take_deliveries();
        assert!(!store.snapshot(&id, &now).expect("a tab").call_attached);

        store
            .verify(&id, Some(true), Some("it fired".to_owned()), &now)
            .expect("the report");
        let snapshot = store.snapshot(&id, &now).expect("a tab");
        assert_eq!(
            snapshot.undelivered, 0,
            "the reporting agent already has the outcome"
        );
        assert!(store.take_deliveries().is_empty());
        assert_eq!(store.get(&id).expect("it").delivered_at, Some(now.clone()));
        assert!(
            !store
                .snapshot(&id, &at("2026-09-20T11:30:00Z"))
                .expect("a tab")
                .orphan,
            "an outcome the agent was handed is never an orphan"
        );
    }

    #[test]
    fn a_report_still_releases_a_call_that_was_left_waiting_on_the_handoff() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, true));
        let now = at("2026-09-08T11:30:00Z");
        store.done(&id, &now).expect("done");
        store.take_deliveries();
        // A second call attached itself while the agent was verifying.
        store
            .resume(&id, &call(1, "call_00000002", OPENER), &now)
            .expect("resume");
        assert!(store.snapshot(&id, &now).expect("a tab").call_attached);

        store
            .verify(&id, Some(true), None, &now)
            .expect("the report");
        let delivered = store.take_deliveries();
        assert_eq!(
            delivered.len(),
            1,
            "the waiting call is not left to time out"
        );
        assert_eq!(delivered[0].call_id, "call_00000002");
        assert_eq!(delivered[0].outcome.status, OutcomeStatus::Verified);
        assert_eq!(store.snapshot(&id, &now).expect("a tab").undelivered, 0);
    }

    #[test]
    fn a_request_id_that_names_a_live_handoff_is_not_taken_over() {
        let mut store = store();
        let existing = open_with(&mut store, spec(2, true));
        let now = at("2026-09-08T11:05:00Z");
        store.confirm(&existing, &now).expect("confirm");

        let accepted = store
            .open(
                OpenParams {
                    spec: spec(1, false),
                    secret_treated: Vec::new(),
                    request_id: Some(existing.clone()),
                    opener: opener(OPENER),
                    call: call(1, "call_00000002", OPENER),
                },
                &now,
            )
            .expect("the open is still accepted");
        assert_ne!(
            accepted.handoff_id, existing,
            "DD-13 adopts a request, never a handoff that is already being guided"
        );
        let untouched = store.snapshot(&existing, &now).expect("the first tab");
        assert_eq!(untouched.step_index, 2);
        assert_eq!(untouched.step_total, 2);
    }

    #[test]
    fn a_report_on_a_spec_that_asked_for_no_verification_is_refused() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, false));
        let error = store
            .verify(&id, Some(true), None, &at("2026-09-08T11:30:00Z"))
            .expect_err("NO_VERIFY_IN_SPEC");
        assert!(matches!(error, Refusal::NoVerifyInSpec));
    }

    #[test]
    fn verified_is_never_reached_without_a_report() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, true));
        let now = at("2026-09-08T11:30:00Z");
        store.done(&id, &now).expect("done");
        // The way out of `awaiting_verification` that is not a report (VER-02, PRIN-08).
        store
            .session_disconnected(OPENER, &at("2026-09-08T11:35:00Z"))
            .expect("the disconnect");
        assert_eq!(
            store.snapshot(&id, &now).expect("a tab").state,
            HandoffState::NotVerified
        );
        assert!(store
            .get(&id)
            .expect("it")
            .last_round()
            .expect("a round")
            .verify
            .is_none());
    }

    #[test]
    fn the_thirty_minute_window_closes_the_verification_pessimistically() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, true));
        store.done(&id, &at("2026-09-08T11:30:00Z")).expect("done");
        store.take_deliveries();

        let early = at("2026-09-08T11:59:00Z");
        store.verifying_timeout(&id, &early).expect("too early");
        assert_eq!(
            store.snapshot(&id, &early).expect("a tab").state,
            HandoffState::AwaitingVerification
        );

        let late = at("2026-09-08T12:00:00Z");
        store
            .verifying_timeout(&id, &late)
            .expect("the window closed");
        let snapshot = store.snapshot(&id, &late).expect("a tab");
        assert_eq!(snapshot.state, HandoffState::NotVerified);
        assert_eq!(snapshot.undelivered, 1);
    }

    #[test]
    fn a_late_report_is_accepted_for_seven_days_and_recorded_as_late() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, true));
        store.done(&id, &at("2026-09-08T11:30:00Z")).expect("done");
        store
            .verifying_timeout(&id, &at("2026-09-08T12:00:00Z"))
            .expect("the window closed");
        store.take_deliveries();

        let later = at("2026-09-10T09:00:00Z");
        let accepted = store
            .verify(
                &id,
                Some(true),
                Some("it fired after all".to_owned()),
                &later,
            )
            .expect("DD-16");
        assert_eq!(accepted.outcome.status, OutcomeStatus::Verified);
        assert!(accepted.outcome.verify.expect("a report").late);
        assert_eq!(
            store.snapshot(&id, &later).expect("a tab").state,
            HandoffState::Verified
        );
    }

    #[test]
    fn a_report_older_than_seven_days_is_refused() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, true));
        store.done(&id, &at("2026-09-08T11:30:00Z")).expect("done");
        store
            .verifying_timeout(&id, &at("2026-09-08T12:00:00Z"))
            .expect("the window closed");
        let error = store
            .verify(&id, Some(true), None, &at("2026-09-16T12:00:00Z"))
            .expect_err("too late");
        assert!(matches!(error, Refusal::Final));
    }

    #[test]
    fn a_failed_verification_is_left_by_replacement_steps_on_the_same_id() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, true));
        let now = at("2026-09-08T11:30:00Z");
        store.done(&id, &now).expect("done");
        store
            .verify(
                &id,
                Some(false),
                Some("the webhook never fired".to_owned()),
                &now,
            )
            .expect("the report");
        store.take_deliveries();
        assert_eq!(
            store.snapshot(&id, &now).expect("a tab").state,
            HandoffState::Failed
        );

        store
            .continue_handoff(
                &id,
                &call(1, "call_00000002", OPENER),
                "start from the error",
                Some(vec![HandoffStep {
                    text: "delete the endpoint and add it again".to_owned(),
                    url: None,
                    values: None,
                    warning: None,
                }]),
                &now,
            )
            .expect("VER-08");
        let snapshot = store.snapshot(&id, &now).expect("a tab");
        assert_eq!(snapshot.state, HandoffState::Active);
        assert_eq!(snapshot.round, 2);
        assert!(snapshot.final_outcome.is_none());
    }

    #[test]
    fn replacement_steps_on_a_handoff_that_is_really_closed_are_refused() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, false));
        let now = at("2026-09-08T11:30:00Z");
        store.done(&id, &now).expect("done");
        let error = store
            .continue_handoff(
                &id,
                &call(1, "call_00000002", OPENER),
                "one more thing",
                Some(vec![HandoffStep {
                    text: "again".to_owned(),
                    url: None,
                    values: None,
                    warning: None,
                }]),
                &now,
            )
            .expect_err("final");
        assert!(matches!(error, Refusal::Final));
    }

    // ----------------------------------------------------------------- abandon, orphan

    #[test]
    fn abandon_closes_the_handoff_from_every_state_it_is_offered_in() {
        let now = at("2026-09-08T11:30:00Z");
        for prepare in 0_u8..4 {
            let (mut store, sink, _resumes) = watched();
            let id = open_with(&mut store, spec(1, true));
            match prepare {
                1 => store.defer(&id, None, &now).expect("deferred"),
                2 => {
                    store.defer(&id, None, &now).expect("deferred");
                    store.defer(&id, None, &now).expect("parked");
                }
                3 => store.done(&id, &now).expect("awaiting verification"),
                _ => {}
            }
            store.take_deliveries();

            store
                .abandon(&id, Some("not needed any more".to_owned()), &now)
                .expect("abandon");
            assert_eq!(
                store.snapshot(&id, &now).expect("a tab").state,
                HandoffState::Abandoned
            );
            assert_eq!(
                sink.finalised.lock().expect("the sink").len(),
                1,
                "the writer is told once"
            );
        }
    }

    #[test]
    fn an_uncollected_final_outcome_becomes_an_orphan_and_the_user_may_close_it() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, false));
        let now = at("2026-09-08T11:30:00Z");
        store
            .detach_call(&id, "call_00000001", DetachReason::Heartbeat, &now)
            .expect("detach");
        store.done(&id, &now).expect("done");
        assert!(
            store.take_deliveries().is_empty(),
            "nothing was listening, so it queued"
        );

        let later = at("2026-09-16T11:30:00Z");
        assert!(store.snapshot(&id, &later).expect("a tab").orphan, "SRV-23");

        store.close_orphan(&id, &later).expect("the user closes it");
        assert!(!store.snapshot(&id, &later).expect("a tab").orphan);
        assert!(store.close_orphan(&id, &later).is_err(), "only once");
    }

    // ------------------------------------------------------------ resume, transfer, FIFO

    #[test]
    fn a_resume_of_a_concluded_handoff_returns_the_same_outcome_and_says_so_the_second_time() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, false));
        let now = at("2026-09-08T11:30:00Z");
        store
            .detach_call(&id, "call_00000001", DetachReason::Heartbeat, &now)
            .expect("detach");
        store.done(&id, &now).expect("done");

        let first = store
            .resume(&id, &call(1, "call_00000002", OPENER), &now)
            .expect("resume");
        let first_outcome = first.outcome.clone().expect("a final outcome");
        assert!(!first_outcome.already_delivered);
        assert_eq!(first.state, HandoffState::ConfirmedByUser);

        let second = store
            .resume(&id, &call(1, "call_00000003", OPENER), &now)
            .expect("resume again");
        let second_outcome = second.outcome.expect("the same outcome");
        assert!(second_outcome.already_delivered, "TOOL-07");
        assert_eq!(
            Outcome {
                already_delivered: false,
                ..second_outcome
            },
            first_outcome
        );
    }

    #[test]
    fn a_resume_pops_the_oldest_queued_outcome_first() {
        let mut store = store();
        let id = open_with(&mut store, spec(3, true));
        let now = at("2026-09-08T11:05:00Z");
        store
            .detach_call(&id, "call_00000001", DetachReason::Heartbeat, &now)
            .expect("detach");
        store.ask(&id, "first", &now).expect("ask");
        store
            .continue_handoff(&id, &call(1, "call_00000002", OPENER), "answer", None, &now)
            .expect("the reply");
        store
            .detach_call(&id, "call_00000002", DetachReason::Heartbeat, &now)
            .expect("detach");
        store.ask(&id, "second", &now).expect("ask again");
        assert_eq!(store.snapshot(&id, &now).expect("a tab").undelivered, 2);

        let first = store
            .resume(&id, &call(1, "call_00000003", OPENER), &now)
            .expect("resume");
        assert_eq!(
            first.outcome.expect("the oldest").user_text.as_deref(),
            Some("first")
        );
        let second = store
            .resume(&id, &call(1, "call_00000004", OPENER), &now)
            .expect("resume");
        assert_eq!(
            second.outcome.expect("the next").user_text.as_deref(),
            Some("second")
        );
        let third = store
            .resume(&id, &call(1, "call_00000005", OPENER), &now)
            .expect("resume");
        assert!(
            third.outcome.is_none(),
            "nothing left: the call attaches and waits"
        );
        assert!(store.snapshot(&id, &now).expect("a tab").call_attached);
    }

    #[test]
    fn a_resume_from_another_session_transfers_the_waiting_call_and_records_where_it_came_from() {
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        let now = at("2026-09-08T11:05:00Z");

        let snapshot = store
            .resume(&id, &call(9, "call_00000002", OTHER), &now)
            .expect("resume from another session");
        assert!(
            snapshot.outcome.is_none(),
            "the new call attaches and waits"
        );
        assert_eq!(
            snapshot.resumed_from,
            Some(ResumedFrom {
                agent: "Claude Code".to_owned(),
                project: "baton".to_owned()
            }),
            "TOOL-08"
        );

        let transferred = store.take_deliveries();
        assert_eq!(transferred.len(), 1, "FM-25");
        assert_eq!(transferred[0].call_id, "call_00000001");
        assert_eq!(
            transferred[0].outcome.status,
            OutcomeStatus::TransferredToOtherSession
        );
        assert!(!transferred[0].outcome.is_final);
    }

    #[test]
    fn a_detach_of_a_call_that_is_not_the_attached_one_changes_nothing() {
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        let now = at("2026-09-08T11:05:00Z");
        store
            .detach_call(&id, "call_99999999", DetachReason::Cancelled, &now)
            .expect("a stale detach is not an error");
        assert!(store.snapshot(&id, &now).expect("a tab").call_attached);

        store
            .detach_call(&id, "call_00000001", DetachReason::Cancelled, &now)
            .expect("detach");
        let snapshot = store.snapshot(&id, &now).expect("a tab");
        assert!(!snapshot.call_attached, "SRV-22");
        assert_eq!(
            snapshot.state,
            HandoffState::Active,
            "a detached call never changes handoff state"
        );
    }

    #[test]
    fn a_disconnect_detaches_the_calls_of_that_session_and_leaves_the_others_alone() {
        let mut store = store();
        let mine = open_with(&mut store, spec(2, true));
        let theirs = store
            .open(
                OpenParams {
                    spec: spec(2, true),
                    secret_treated: Vec::new(),
                    request_id: None,
                    opener: opener(OTHER),
                    call: call(9, "call_00000002", OTHER),
                },
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("a second handoff")
            .handoff_id;

        let now = at("2026-09-08T11:10:00Z");
        store
            .session_disconnected(OPENER, &now)
            .expect("the disconnect");
        let mine = store.snapshot(&mine, &now).expect("a tab");
        assert!(!mine.call_attached);
        assert_eq!(
            mine.state,
            HandoffState::Active,
            "SRV-22: the overlay is never closed by an agent-side event"
        );
        assert!(store.snapshot(&theirs, &now).expect("a tab").call_attached);
    }

    #[test]
    fn a_relink_points_the_handoff_at_another_request() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, false));
        let now = at("2026-09-08T11:05:00Z");
        let mut request = crate::log::testing::request("hf_9p2r4k7m3t");
        request.text = "the one I actually meant".to_owned();
        user_requests::upsert(&store.db, &request).expect("a request");

        store.relink(&id, "hf_9p2r4k7m3t", &now).expect("FM-20");
        let snapshot = store.snapshot(&id, &now).expect("a tab");
        assert_eq!(snapshot.linked_request_id.as_deref(), Some("hf_9p2r4k7m3t"));
        assert_eq!(
            snapshot.request_text.as_deref(),
            Some("the one I actually meant")
        );
        assert_eq!(
            user_requests::get(&store.db, "hf_9p2r4k7m3t")
                .expect("a read")
                .expect("the request")
                .linked_handoff_id
                .as_deref(),
            Some(id.as_str())
        );
    }

    #[test]
    fn an_open_that_quotes_no_request_answers_the_oldest_one_of_its_session() {
        // OPEN-08: the fast path put the id in the clipboard text, so this is the case where
        // the user typed the request themselves and the agent never quoted it.
        let mut store = store();
        let mut older = crate::log::testing::request("hf_0000000001");
        older.session_ref = Some(OPENER.to_owned());
        older.text = "I'm about to create the API key on Stripe".to_owned();
        older.created_at = at("2026-09-08T10:00:00Z");
        user_requests::upsert(&store.db, &older).expect("a request");
        let mut newer = crate::log::testing::request("hf_0000000002");
        newer.session_ref = Some(OPENER.to_owned());
        newer.created_at = at("2026-09-08T10:30:00Z");
        user_requests::upsert(&store.db, &newer).expect("a second request");

        let id = open_with(&mut store, spec(1, false));
        assert_ne!(
            id, "hf_0000000001",
            "no id is taken over without a request_id"
        );
        let snapshot = store
            .snapshot(&id, &at("2026-09-08T11:00:00Z"))
            .expect("a tab");
        assert_eq!(snapshot.linked_request_id.as_deref(), Some("hf_0000000001"));
        assert_eq!(
            snapshot.request_text.as_deref(),
            Some("I'm about to create the API key on Stripe"),
            "the tab shows the user's own words (§7.7)"
        );
        assert_eq!(
            user_requests::get(&store.db, "hf_0000000001")
                .expect("a read")
                .expect("the request")
                .linked_handoff_id
                .as_deref(),
            Some(id.as_str()),
            "and the request is no longer waiting for a spec"
        );
        assert_eq!(
            user_requests::get(&store.db, "hf_0000000002")
                .expect("a read")
                .expect("the request")
                .linked_handoff_id,
            None,
            "the second one is still open for the next handoff"
        );
    }

    #[test]
    fn an_open_that_quotes_a_request_takes_its_id_and_closes_it() {
        // DD-13, and the other half of OPEN-08: the request the agent quoted stops being in
        // the queue, or the Stop hook would keep asking for a spec that has arrived.
        let mut store = store();
        let mut request = crate::log::testing::request("hf_3n8v5t1q6w");
        request.session_ref = Some(OPENER.to_owned());
        user_requests::upsert(&store.db, &request).expect("a request");

        let id = store
            .open(
                OpenParams {
                    spec: spec(1, false),
                    secret_treated: Vec::new(),
                    request_id: Some("hf_3n8v5t1q6w".to_owned()),
                    opener: opener(OPENER),
                    call: call(1, "call_00000001", OPENER),
                },
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("the open is accepted")
            .handoff_id;
        assert_eq!(id, "hf_3n8v5t1q6w");
        assert_eq!(
            user_requests::get(&store.db, "hf_3n8v5t1q6w")
                .expect("a read")
                .expect("the request")
                .linked_handoff_id
                .as_deref(),
            Some("hf_3n8v5t1q6w")
        );
    }

    #[test]
    fn a_resume_request_is_queued_when_the_user_picks_a_handoff_up_and_closed_when_a_call_comes() {
        // FM-31 and RESP-07, end to end through the store's own queue.
        let mut store = store();
        let id = open_with(&mut store, spec(1, false));
        let now = at("2026-09-08T11:05:00Z");
        store
            .defer(&id, Some("later".to_owned()), &now)
            .expect("the user defers");
        store
            .detach_call(&id, "call_00000001", DetachReason::Cancelled, &now)
            .expect("the call stops waiting");
        store.resume_from_overlay(&id, &now).expect("RESP-07");

        let queued = user_requests::list_open(&store.db, Some(OPENER)).expect("the queue");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].about_handoff_id.as_deref(), Some(id.as_str()));

        store
            .resume(&id, &call(1, "call_00000002", OPENER), &now)
            .expect("the agent comes back");
        assert!(
            user_requests::list_open(&store.db, Some(OPENER))
                .expect("the queue")
                .is_empty(),
            "a call attached, so the request has been answered"
        );
    }

    // ------------------------------------------------------------------ timers, actor

    #[test]
    fn the_store_knows_how_long_the_earliest_verification_window_still_has() {
        let mut store = store();
        let id = open_with(&mut store, spec(1, true));
        assert_eq!(
            store.next_verifying_deadline(&at("2026-09-08T11:00:00Z")),
            None
        );

        store.done(&id, &at("2026-09-08T11:30:00Z")).expect("done");
        assert_eq!(
            store.next_verifying_deadline(&at("2026-09-08T11:40:00Z")),
            Some(std::time::Duration::from_secs(20 * 60))
        );
        assert!(store
            .expired_verifying(&at("2026-09-08T11:40:00Z"))
            .is_empty());

        assert_eq!(
            store.next_verifying_deadline(&at("2026-09-08T12:30:00Z")),
            Some(std::time::Duration::ZERO),
            "a window that ran out asks to be looked at now, never in the past"
        );
        assert_eq!(
            store.expired_verifying(&at("2026-09-08T12:00:00Z")),
            vec![id]
        );
    }

    #[tokio::test]
    async fn the_actor_answers_commands_and_puts_every_outcome_on_one_stream() {
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        let (handle, mut deliveries) = spawn(store);
        let now = at("2026-09-08T11:05:00Z");

        handle
            .user(
                id.clone(),
                UserAction::Ask("which button?".to_owned()),
                now.clone(),
            )
            .await
            .expect("the ask");
        let delivery = deliveries.recv().await.expect("the outcome of the ask");
        assert_eq!(delivery.handoff_id, id);
        assert_eq!(delivery.call_id, "call_00000001");
        assert_eq!(delivery.outcome.status, OutcomeStatus::Question);

        handle
            .continue_handoff(
                id.clone(),
                call(1, "call_00000001", OPENER),
                "the blue one".to_owned(),
                None,
                now.clone(),
            )
            .await
            .expect("the reply");

        let tabs = handle.list_for_ui(now.clone()).await;
        assert_eq!(tabs.len(), 1);
        assert!(tabs[0].pending_question.is_none());
        assert!(handle.snapshot(id, now).await.is_some());
    }

    #[tokio::test]
    async fn a_command_to_a_store_that_has_stopped_is_answered_rather_than_awaited_for_ever() {
        let (handle, deliveries) = spawn(store());
        drop(deliveries);
        // The actor returns as soon as nobody is reading the deliveries, which is what the
        // app looks like while it is shutting down.
        let error = handle
            .user(
                "hf_0000000001".to_owned(),
                UserAction::Confirm,
                at("2026-09-08T11:05:00Z"),
            )
            .await
            .expect_err("there is no handoff and, soon, no actor");
        assert!(matches!(error, Refusal::NotFound));
    }

    // ------------------------------------------------------------- persistence, FM-28

    #[test]
    fn an_unknown_handoff_is_not_found_whatever_is_asked_of_it() {
        let mut store = store();
        let now = at("2026-09-08T11:05:00Z");
        assert!(matches!(
            store.confirm("hf_9p2r4k7m3t", &now),
            Err(Refusal::NotFound)
        ));
        assert!(matches!(
            store.verify("hf_9p2r4k7m3t", Some(true), None, &now),
            Err(Refusal::NotFound)
        ));
        assert!(store.snapshot("hf_9p2r4k7m3t", &now).is_none());
    }

    #[test]
    fn a_write_that_fails_leaves_the_handoff_exactly_as_it_was() {
        let mut store = store();
        let id = open_with(&mut store, spec(2, true));
        let now = at("2026-09-08T11:05:00Z");
        let before = store.snapshot(&id, &now).expect("a tab");

        // The schema refuses a hash that is not 64 characters (LOG-03), so the whole
        // transition is rolled back, the handoff row with it.
        let error = store
            .screenshot(&id, &screenshot("not-a-digest"), &now)
            .expect_err("FM-28");
        assert!(matches!(error, Refusal::Persistence(_)), "{error:?}");

        let after = store.snapshot(&id, &now).expect("a tab");
        assert_eq!(before, after, "the in-memory state is untouched");
        assert!(store.get(&id).expect("it").pending_question.is_none());
        assert!(store.take_deliveries().is_empty());
        assert!(crate::log::sends::list_for_handoff(&store.db, &id)
            .expect("the sends")
            .is_empty());
    }

    #[test]
    fn a_restarted_store_reads_back_what_the_last_one_wrote() {
        let dir = crate::log::testing::tempdir();
        let path = dir.join("handoff.sqlite");
        let now = at("2026-09-08T11:05:00Z");

        let mut expected = {
            let db = Db::open_at(&path).expect("a database");
            for reference in [OPENER, OTHER] {
                sessions::register(&db, &session(reference)).expect("a session");
            }
            let mut store = Store::new(db);
            let guided = open_with(&mut store, spec(3, true));
            store.confirm(&guided, &now).expect("confirm");
            store
                .note(&guided, "the label changed", &now)
                .expect("note");
            store
                .detach_call(&guided, "call_00000001", DetachReason::Heartbeat, &now)
                .expect("detach");
            store.ask(&guided, "which one?", &now).expect("ask");
            let closed = open_with(&mut store, spec(1, false));
            store.done(&closed, &now).expect("done");
            store.take_deliveries();
            store.list_for_ui(&now)
        };

        let db = Db::open_at(&path).expect("the same database");
        let restored = Store::load(db, Box::new(NoRunbookSink), Box::new(NoRequests))
            .expect("the store as it was");
        let mut reloaded = restored.list_for_ui(&now);
        assert_eq!(reloaded.len(), 2);
        // The one difference a restart is allowed to make: no call survives a process
        // (§7.4). Everything else, the queued outcome included, comes back.
        for snapshot in &mut reloaded {
            assert!(!snapshot.call_attached);
        }
        for snapshot in &mut expected {
            snapshot.call_attached = false;
        }
        assert_eq!(reloaded, expected);
        // The two were created in the same millisecond, so the list is ordered by id and
        // the guided one is found by what it carries, not by where it sits.
        assert_eq!(
            reloaded
                .iter()
                .map(|snapshot| snapshot.undelivered)
                .sum::<usize>(),
            1,
            "DD-12 survives a restart"
        );
        assert!(reloaded
            .iter()
            .any(|snapshot| snapshot.pending_question.is_some()));
        crate::log::testing::clean(&dir);
    }
}
