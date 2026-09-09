//! One handoff, as the store holds it (§7.4, §8.1).
//!
//! The record of §7.4 and nothing else: this file has no database, no channel and no
//! decisions. What may happen to a handoff and in which order is [`super::actor`]; what a
//! handoff *is* — where the cursor stands, which steps were confirmed in this round, what
//! is queued for an agent that is not listening — is here, together with the small
//! questions the machine and the outcome builder keep asking it.
//!
//! # Two fields do not survive a restart, and that is deliberate
//!
//! - **`spec`.** `log::handoffs::upsert` masks the spec before writing it (LOG-02), so what
//!   comes back out of the row is the masked one. The true spec is what the copy button
//!   copies (DET-04) and what the outcome's `context` carries, so it is held in memory and
//!   read back from the row only as the masked shape it was stored as. It is `serde(skip)`
//!   here because the row already has a column for it; keeping a second copy inside
//!   `state_json` would mean masking the same values twice and storing them once too often.
//! - **`attached_call`.** A call is a connection and a promise to answer it. Neither
//!   outlives the process, so a restored handoff has no attached call — its outcomes queue
//!   in [`Handoff::undelivered`] until an agent resumes, which is exactly DD-12.
//!
//! # The pixels
//!
//! [`Queued`] carries the base64 image of a screenshot the user sent while no call was
//! attached, because the published outcome is closed and has nowhere to put it: the bytes
//! travel beside the outcome on `handoff.event` and on the `handoff.resume` snapshot
//! (`DEVIATIONS.md`, T-020). They are `serde(skip)`, so nothing of them reaches
//! `state_json` — LOG-03 allows the log a hash and never a pixel.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::channel::ConnId;
use crate::format::outcome::{
    Outcome, OutcomeNote, ResumedFrom, ScreenshotMode, SecretTreated, VerifyReport,
};
use crate::format::spec::{HandoffSpec, HandoffStep};
use crate::log::handoffs::ORPHAN_AGE_MS;
use crate::log::{HandoffState, Timestamp};
use crate::sessions::Session;

/// How long a handoff waits for the verification report it asked for (§4.1
/// `VERIFYING_TIMEOUT_MS`, VER-06). The same value as the raised tool timeout: after it,
/// the agent's call has expired anyway.
pub const VERIFYING_TIMEOUT_MS: i64 = 1_800_000;

/// Where the user is: the round, and the 1-based step inside it (§7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    /// Which round, 1-based.
    pub round: u32,
    /// Which step of that round, 1-based. It never points past the last step: "done" is a
    /// transition, not a position.
    pub step_index: u32,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            round: 1,
            step_index: 1,
        }
    }
}

/// One pass through a step list (§7.4, VER-09, VER-10).
///
/// The confirmations, the skips and the notes are per round, so a correction round starts
/// with an empty slate and the counter restarts at "1 of n".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Round {
    /// 1-based; replacement steps open round n+1.
    pub no: u32,
    /// The steps of this round.
    pub steps: Vec<HandoffStep>,
    /// When it opened.
    pub started_at: Timestamp,
    /// When it closed; absent while it is the current one.
    pub ended_at: Option<Timestamp>,
    /// 1-based indices the user marked done.
    pub confirmed: Vec<u32>,
    /// 1-based indices the user skipped (RESP-03).
    pub skipped: Vec<u32>,
    /// What the user wrote on a step (RESP-03).
    pub notes: Vec<OutcomeNote>,
    /// What the agent reported about this round (VER-05, VER-10).
    pub verify: Option<VerifyReport>,
}

impl Round {
    /// A fresh round `no` over `steps`.
    #[must_use]
    pub fn new(no: u32, steps: Vec<HandoffStep>, at: &Timestamp) -> Self {
        Self {
            no,
            steps,
            started_at: at.clone(),
            ended_at: None,
            confirmed: Vec::new(),
            skipped: Vec::new(),
            notes: Vec::new(),
            verify: None,
        }
    }

    /// How many steps it has. Never zero: the spec schema requires at least one.
    #[must_use]
    pub fn total(&self) -> u32 {
        u32::try_from(self.steps.len()).unwrap_or(u32::MAX)
    }
}

/// Which kind of interruption is waiting for an answer (§7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingKind {
    /// The user pressed Ask (RESP-04).
    Question,
    /// The user sent a screenshot.
    Screenshot,
}

/// An interruption the agent has not answered yet (§7.4).
///
/// It is what FM-32 asks about: a `reply` with nothing pending and no replacement steps is
/// `not_waiting`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingQuestion {
    /// Question or screenshot.
    pub kind: PendingKind,
    /// The 1-based step it was raised on.
    pub step: u32,
    /// When.
    pub at: Timestamp,
}

/// The blocking call currently waiting on a handoff (§7.4, TOOL-03).
///
/// At most one per handoff, which is the invariant the property suite checks. It is not
/// serialised: see the module documentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedCall {
    /// The connection to answer on.
    pub conn_id: ConnId,
    /// The call to resolve (`call_` + 8 characters, minted by the server).
    pub call_id: String,
    /// The session that call belongs to, when it registered one.
    pub session_ref: Option<String>,
}

/// A call that wants to attach to a handoff: `handoff.open` and `handoff.resume` are the
/// only two messages that carry a new one (the server mints them, §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// The connection it arrived on.
    pub conn_id: ConnId,
    /// The call id.
    pub call_id: String,
    /// The session that owns it.
    pub session_ref: Option<String>,
}

impl Call {
    /// The attached form of this call.
    #[must_use]
    pub fn attached(&self) -> AttachedCall {
        AttachedCall {
            conn_id: self.conn_id,
            call_id: self.call_id.clone(),
            session_ref: self.session_ref.clone(),
        }
    }
}

/// An outcome produced while no call was attached (DD-12).
///
/// FIFO: `handoff.resume` pops the oldest first, so nothing is lost and the order the user
/// produced them in is the order the agent reads them in (§5.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queued {
    /// What to deliver.
    pub outcome: Outcome,
    /// The pixels that belong beside it, when the user sent an image. Never persisted.
    #[serde(skip)]
    pub image: Option<String>,
}

/// The session that opened a handoff, as the store needs it (§7.4, §7.11).
///
/// Four columns of the `handoffs` row and the two labels a `resumed_from` is made of. It is
/// a value rather than a borrow of the registry: the store outlives any single session, and
/// a handoff keeps saying who opened it long after that session is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opener {
    /// `ses_` + 8 characters.
    pub session_ref: String,
    /// The capability-table key the server resolved (§5.6).
    pub agent_id: Option<String>,
    /// `clientInfo.name` of the MCP handshake.
    pub client_name: Option<String>,
    /// The project folder the session was tied to.
    pub project_dir: Option<String>,
    /// Agent and project as a person reads them, for `resumed_from` (TOOL-08, OPEN-02).
    pub label: ResumedFrom,
}

impl From<&Session> for Opener {
    fn from(session: &Session) -> Self {
        Self {
            session_ref: session.session_ref.clone(),
            agent_id: session.agent_id.clone(),
            client_name: session.client.as_ref().map(|client| client.name.clone()),
            project_dir: session.project_dir.clone(),
            label: session.resumed_from(),
        }
    }
}

/// A screenshot the user decided to send (§7.8–§7.10, LOG-03, PREV-01).
///
/// The shape is fixed now because the store has to record it and build an outcome from it;
/// what fills it is the capture and preview pipeline.
// TASK: T-049 — produced by the preview, once capture, OCR and redaction exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenshotPayload {
    /// Whether the user sent the pixels or the extracted text (PREV-04).
    pub mode: ScreenshotMode,
    /// The OCR text as the user edited it, in text mode.
    pub text: Option<String>,
    /// The redacted PNG as base64, in image mode. It reaches the channel and the tool
    /// result, never the database (LOG-03).
    pub image_base64: Option<String>,
    /// 64 lowercase hexadecimal characters of the redacted image, for the log.
    pub image_sha256: Option<String>,
    /// Pixel width of what was captured.
    pub width: u32,
    /// Pixel height of what was captured.
    pub height: u32,
    /// How many regions were burned out before anything left the machine (PRIN-09).
    pub redactions: u32,
    /// The boxes that were burned, as the preview recorded them.
    pub redaction_boxes_json: Option<String>,
    /// Which OCR engine produced the text (§7.9).
    pub ocr_engine: Option<String>,
    /// The version of the certain-secret pattern file that was applied (§4.6).
    pub patterns_version: Option<String>,
    /// What the user wrote beside the screenshot; it becomes `user_text`.
    pub comment: Option<String>,
}

/// One of the five states a handoff ends in (VER-01, §8.1).
///
/// Not a second vocabulary for [`HandoffState`] but a refinement of it: the runbook writer
/// is told *which* final state was reached and must not have to consider the five that are
/// not final, so the signature says so instead of the documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalState {
    /// The agent reported `ok: true` (RUN-01: a runbook is written).
    Verified,
    /// The agent reported `ok: false` (VER-08).
    Failed,
    /// No report, or `ok: null`, or the timeout, or a disconnect (VER-06).
    NotVerified,
    /// Done on the last step of a spec with no `verify` (RUN-01: a runbook is written).
    ConfirmedByUser,
    /// The user abandoned it (RESP-08).
    Abandoned,
}

impl FinalState {
    /// The state of §8.1 this is.
    #[must_use]
    pub fn state(self) -> HandoffState {
        match self {
            Self::Verified => HandoffState::Verified,
            Self::Failed => HandoffState::Failed,
            Self::NotVerified => HandoffState::NotVerified,
            Self::ConfirmedByUser => HandoffState::ConfirmedByUser,
            Self::Abandoned => HandoffState::Abandoned,
        }
    }

    /// The final state `state` is, when it is one.
    #[must_use]
    pub fn of(state: HandoffState) -> Option<Self> {
        match state {
            HandoffState::Verified => Some(Self::Verified),
            HandoffState::Failed => Some(Self::Failed),
            HandoffState::NotVerified => Some(Self::NotVerified),
            HandoffState::ConfirmedByUser => Some(Self::ConfirmedByUser),
            HandoffState::Abandoned => Some(Self::Abandoned),
            _ => None,
        }
    }

    /// Whether reaching this state saves or refreshes a runbook (RUN-01, §7.12).
    #[must_use]
    pub fn writes_a_runbook(self) -> bool {
        matches!(self, Self::Verified | Self::ConfirmedByUser)
    }
}

/// One unit of human work, from open to a final state (§7.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoff {
    /// `hf_` + 10 characters (§4.1). A handoff that grew from a user request carries that
    /// request's id (DD-13).
    pub id: String,
    /// When it (or the request it grew from) was opened.
    pub created_at: Timestamp,
    /// When it reached a final state.
    pub closed_at: Option<Timestamp>,
    /// The session that opened it.
    pub session_ref: Option<String>,
    /// The capability-table key of that session.
    pub agent_id: Option<String>,
    /// `clientInfo.name` of that session.
    pub client_name: Option<String>,
    /// The project folder of that session.
    pub project_dir: Option<String>,
    /// Agent and project of the opening session, as `resumed_from` prints them.
    pub opener_label: Option<ResumedFrom>,
    /// The user's own words, when it grew from a request (OPEN-04).
    pub request_text: Option<String>,
    /// The request this handoff answers, when the link is not the id itself (FM-20, §12.4).
    pub linked_request_id: Option<String>,
    /// BCP-47 tag of the spec's texts.
    pub lang: Option<String>,
    /// The **true** spec. Absent while `awaiting_spec`, and masked after a restart.
    #[serde(skip)]
    pub spec: Option<HandoffSpec>,
    /// What the certain detector matched at ingress (DET-04).
    pub secret_treated: Vec<SecretTreated>,
    /// One entry per round, in order.
    pub rounds: Vec<Round>,
    /// Where the user is.
    pub cursor: Cursor,
    /// 0, 1 or 2; the second deferral parks the handoff (RESP-07).
    pub deferral_count: u32,
    /// Set while an interruption waits for the agent's reply.
    pub pending_question: Option<PendingQuestion>,
    /// Outcomes produced with no call attached, oldest first (DD-12).
    pub undelivered: VecDeque<Queued>,
    /// The call waiting on this handoff, at most one. Not serialised.
    #[serde(skip)]
    pub attached_call: Option<AttachedCall>,
    /// Where it stands in §8.1.
    pub state: HandoffState,
    /// The outcome a final state produced, kept until an agent consumes it (SRV-23).
    pub final_outcome: Option<Outcome>,
    /// When the final outcome reached an agent, or the user closed it by hand.
    pub delivered_at: Option<Timestamp>,
    /// When the handoff entered `awaiting_verification`; the 30-minute timer of VER-06 is
    /// measured from it, and it survives a restart so the timer does too.
    pub verifying_since: Option<Timestamp>,
    /// The opening session, when the current call comes from a different one (TOOL-08).
    pub resumed_from: Option<ResumedFrom>,
}

impl Handoff {
    /// A handoff a `handoff.open` has just created.
    #[must_use]
    pub fn opened(
        id: String,
        spec: HandoffSpec,
        secret_treated: Vec<SecretTreated>,
        opener: &Opener,
        at: &Timestamp,
    ) -> Self {
        let round = Round::new(1, spec.steps.clone(), at);
        Self {
            id,
            created_at: at.clone(),
            closed_at: None,
            session_ref: Some(opener.session_ref.clone()),
            agent_id: opener.agent_id.clone(),
            client_name: opener.client_name.clone(),
            project_dir: opener.project_dir.clone(),
            opener_label: Some(opener.label.clone()),
            request_text: None,
            linked_request_id: None,
            lang: spec.lang.clone(),
            spec: Some(spec),
            secret_treated,
            rounds: vec![round],
            cursor: Cursor::default(),
            deferral_count: 0,
            pending_question: None,
            undelivered: VecDeque::new(),
            attached_call: None,
            state: HandoffState::Active,
            final_outcome: None,
            delivered_at: None,
            verifying_since: None,
            resumed_from: None,
        }
    }

    /// The tab a user's request opens, before any agent has answered it (OPEN-04, DD-13).
    ///
    /// Everything a spec would fill is empty and stays empty until `handoff.open` arrives
    /// with this id: no spec, **no rounds**, no steps to walk. §8.4 gives the row its own
    /// banner and no step view, which is what makes an empty round list a state rather than
    /// a gap — `current_round` answers `None` and every projection built on it is empty.
    ///
    /// `opener` is the session the request was addressed to, and it is optional: OPEN-04a
    /// opens the sheet with no session registered at all, and the tab still has to exist.
    /// The label then falls back to the id (`ui_bridge::view`), until the spec arrives with
    /// the session that produced it.
    #[must_use]
    pub fn awaiting_spec(
        id: String,
        request_text: String,
        opener: Option<&Opener>,
        at: &Timestamp,
    ) -> Self {
        Self {
            id,
            created_at: at.clone(),
            closed_at: None,
            session_ref: opener.map(|opener| opener.session_ref.clone()),
            agent_id: opener.and_then(|opener| opener.agent_id.clone()),
            client_name: opener.and_then(|opener| opener.client_name.clone()),
            project_dir: opener.and_then(|opener| opener.project_dir.clone()),
            opener_label: opener.map(|opener| opener.label.clone()),
            request_text: Some(request_text),
            linked_request_id: None,
            lang: None,
            spec: None,
            secret_treated: Vec::new(),
            rounds: Vec::new(),
            cursor: Cursor::default(),
            deferral_count: 0,
            pending_question: None,
            undelivered: VecDeque::new(),
            attached_call: None,
            state: HandoffState::AwaitingSpec,
            final_outcome: None,
            delivered_at: None,
            verifying_since: None,
            resumed_from: None,
        }
    }

    /// The round the cursor is in.
    ///
    /// A handoff always has at least one round from the moment a spec arrives, and the
    /// cursor is moved only by this module, so the fallback is unreachable; it is written
    /// as a fallback rather than a panic because a corrupted `state_json` must not be able
    /// to take the app down (FM-28).
    #[must_use]
    pub fn current_round(&self) -> Option<&Round> {
        self.rounds
            .iter()
            .find(|round| round.no == self.cursor.round)
    }

    /// The round the cursor is in, mutably.
    pub fn current_round_mut(&mut self) -> Option<&mut Round> {
        let no = self.cursor.round;
        self.rounds.iter_mut().find(|round| round.no == no)
    }

    /// The last round, which is the one a verification belongs to.
    #[must_use]
    pub fn last_round(&self) -> Option<&Round> {
        self.rounds.last()
    }

    /// The step the cursor is on.
    #[must_use]
    pub fn current_step(&self) -> Option<&HandoffStep> {
        let round = self.current_round()?;
        let index = usize::try_from(self.cursor.step_index)
            .ok()?
            .checked_sub(1)?;
        round.steps.get(index)
    }

    /// How many steps the current round has.
    #[must_use]
    pub fn step_count(&self) -> u32 {
        self.current_round().map_or(0, Round::total)
    }

    /// Whether the cursor is on the last step of its round.
    #[must_use]
    pub fn is_last_step(&self) -> bool {
        self.cursor.step_index >= self.step_count()
    }

    /// Moves the cursor to the next step, stopping on the last one.
    pub fn advance(&mut self) {
        if !self.is_last_step() {
            self.cursor.step_index += 1;
        }
    }

    /// Whether a value of the spec was treated as a secret at ingress (DET-04).
    ///
    /// The locations the server reports are display paths (`values.api_key`, §4.7.5), so a
    /// top-level value name is matched against exactly that prefix.
    #[must_use]
    pub fn is_secret_value(&self, name: &str) -> bool {
        let location = format!("values.{name}");
        self.secret_treated
            .iter()
            .any(|treated| treated.location == location)
    }

    /// Whether the handoff has reached one of the five final states.
    #[must_use]
    pub fn is_final(&self) -> bool {
        self.state.is_final()
    }

    /// The final state it reached, when it reached one.
    #[must_use]
    pub fn final_state(&self) -> Option<FinalState> {
        FinalState::of(self.state)
    }

    /// A final outcome nobody has collected for seven days (SRV-23, FM-27).
    ///
    /// Computed, never stored: a handoff becomes an orphan by the passage of time and stops
    /// being one the moment an agent resumes it or the user closes it by hand.
    #[must_use]
    pub fn is_orphan(&self, now: &Timestamp) -> bool {
        let Some(closed_at) = self.closed_at.as_ref() else {
            return false;
        };
        self.final_outcome.is_some()
            && self.delivered_at.is_none()
            && closed_at.millis() <= now.millis().saturating_sub(ORPHAN_AGE_MS)
    }

    /// Whether the verification window of VER-06 has run out at `now`.
    #[must_use]
    pub fn verifying_expired(&self, now: &Timestamp) -> bool {
        self.verifying_since.as_ref().is_some_and(|since| {
            now.millis().saturating_sub(since.millis()) >= VERIFYING_TIMEOUT_MS
        })
    }

    /// Whether a late report is still accepted: within `ORPHAN_AGE_MS` of the opening
    /// (DD-16, "while the handoff is younger than `ORPHAN_AGE_MS`").
    #[must_use]
    pub fn accepts_a_late_report(&self, now: &Timestamp) -> bool {
        now.millis().saturating_sub(self.created_at.millis()) < ORPHAN_AGE_MS
    }

    /// Whether `session_ref` is a session this handoff belongs to: the one that opened it,
    /// or the one whose call is attached to it.
    ///
    /// The same predicate §7.5 uses to decide which handoffs a Stop hook of that session
    /// should be told about, and the one that decides what a disconnect touches (FM-08).
    #[must_use]
    pub fn belongs_to(&self, session_ref: &str) -> bool {
        self.session_ref.as_deref() == Some(session_ref)
            || self
                .attached_call
                .as_ref()
                .and_then(|call| call.session_ref.as_deref())
                == Some(session_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::spec::SpecValue;
    use indexmap::IndexMap;

    fn at(text: &str) -> Timestamp {
        Timestamp::parse(text).expect("rfc 3339")
    }

    pub(super) fn spec(steps: usize) -> HandoffSpec {
        let mut values = IndexMap::new();
        values.insert(
            "endpoint_url".to_owned(),
            SpecValue::One("https://api.example.test/hook".to_owned()),
        );
        HandoffSpec {
            spec_version: 1,
            goal: "Register the webhook".to_owned(),
            r#where: "Dashboard → Webhooks".to_owned(),
            url: None,
            why_human: "only a person can log in".to_owned(),
            values,
            secrets: None,
            steps: (1..=steps)
                .map(|index| HandoffStep {
                    text: format!("step {index}"),
                    url: None,
                    values: None,
                    warning: None,
                })
                .collect(),
            verify: Some("the webhook fires".to_owned()),
            lang: Some("en".to_owned()),
        }
    }

    fn opener() -> Opener {
        Opener {
            session_ref: "ses_00000001".to_owned(),
            agent_id: Some("claude-code".to_owned()),
            client_name: Some("claude-code".to_owned()),
            project_dir: Some("C:\\projects\\baton".to_owned()),
            label: ResumedFrom {
                agent: "Claude Code".to_owned(),
                project: "baton".to_owned(),
            },
        }
    }

    fn handoff() -> Handoff {
        Handoff::opened(
            "hf_0000000001".to_owned(),
            spec(3),
            Vec::new(),
            &opener(),
            &at("2026-09-08T11:00:00Z"),
        )
    }

    #[test]
    fn a_tab_waiting_for_its_spec_has_no_round_to_walk() {
        // OPEN-04: the tab exists before any agent has said anything. Every projection is
        // built from the current round, and there is none, so all of them must be empty
        // rather than wrong.
        let waiting = Handoff::awaiting_spec(
            "hf_0000000001".to_owned(),
            "create the API key on Stripe".to_owned(),
            Some(&opener()),
            &at("2026-09-08T10:00:00Z"),
        );
        assert_eq!(waiting.state, HandoffState::AwaitingSpec);
        assert!(waiting.spec.is_none());
        assert!(waiting.rounds.is_empty());
        assert_eq!(waiting.current_round(), None);
        assert_eq!(waiting.current_step(), None);
        assert_eq!(waiting.step_count(), 0);
        assert_eq!(
            waiting.request_text.as_deref(),
            Some("create the API key on Stripe")
        );
        assert_eq!(waiting.session_ref.as_deref(), Some("ses_00000001"));
        assert!(!waiting.is_final());
    }

    #[test]
    fn a_tab_waiting_for_its_spec_with_no_session_carries_none() {
        // OPEN-04a: the sheet opens with nothing registered.
        let waiting = Handoff::awaiting_spec(
            "hf_0000000001".to_owned(),
            "book the domain".to_owned(),
            None,
            &at("2026-09-08T10:00:00Z"),
        );
        assert_eq!(waiting.session_ref, None);
        assert_eq!(waiting.opener_label, None);
        assert_eq!(waiting.project_dir, None);
    }

    #[test]
    fn a_fresh_handoff_stands_on_the_first_step_of_the_first_round() {
        let handoff = handoff();
        assert_eq!(handoff.cursor, Cursor::default());
        assert_eq!(handoff.step_count(), 3);
        assert_eq!(handoff.current_step().expect("a step").text, "step 1");
        assert!(!handoff.is_last_step());
        assert_eq!(handoff.state, HandoffState::Active);
    }

    #[test]
    fn the_cursor_stops_on_the_last_step() {
        let mut handoff = handoff();
        for _ in 0..10 {
            handoff.advance();
        }
        assert_eq!(handoff.cursor.step_index, 3);
        assert!(handoff.is_last_step());
    }

    #[test]
    fn a_value_is_secret_treated_by_its_display_path() {
        let mut handoff = handoff();
        handoff.secret_treated = vec![SecretTreated {
            location: "values.api_key".to_owned(),
            kind: "api_key".to_owned(),
        }];
        assert!(handoff.is_secret_value("api_key"));
        assert!(!handoff.is_secret_value("endpoint_url"));
        // The prefix has to match exactly: a step's value list is not a location.
        assert!(!handoff.is_secret_value("values.api_key"));
    }

    #[test]
    fn an_orphan_needs_a_final_outcome_nobody_took_and_seven_days() {
        let mut handoff = handoff();
        let now = at("2026-09-20T11:00:00Z");
        assert!(!handoff.is_orphan(&now), "an active handoff is never one");

        handoff.state = HandoffState::Abandoned;
        handoff.closed_at = Some(at("2026-09-08T12:00:00Z"));
        handoff.final_outcome = Some(crate::store::outcome::testing::any_outcome());
        assert!(handoff.is_orphan(&now));

        assert!(
            !handoff.is_orphan(&at("2026-09-10T11:00:00Z")),
            "two days is not seven"
        );

        handoff.delivered_at = Some(at("2026-09-09T11:00:00Z"));
        assert!(
            !handoff.is_orphan(&now),
            "a delivered outcome is not an orphan"
        );
    }

    #[test]
    fn the_verifying_window_is_thirty_minutes_and_a_late_report_seven_days() {
        let mut handoff = handoff();
        handoff.verifying_since = Some(at("2026-09-08T11:00:00Z"));
        assert!(!handoff.verifying_expired(&at("2026-09-08T11:29:59Z")));
        assert!(handoff.verifying_expired(&at("2026-09-08T11:30:00Z")));

        assert!(handoff.accepts_a_late_report(&at("2026-09-15T10:59:59Z")));
        assert!(!handoff.accepts_a_late_report(&at("2026-09-15T11:00:00Z")));
    }

    #[test]
    fn a_handoff_belongs_to_its_opener_and_to_the_session_whose_call_is_attached() {
        let mut handoff = handoff();
        assert!(handoff.belongs_to("ses_00000001"));
        assert!(!handoff.belongs_to("ses_00000002"));

        handoff.attached_call = Some(AttachedCall {
            conn_id: 7,
            call_id: "call_00000001".to_owned(),
            session_ref: Some("ses_00000002".to_owned()),
        });
        assert!(handoff.belongs_to("ses_00000002"));
    }

    #[test]
    fn the_five_final_states_round_trip_and_two_of_them_write_a_runbook() {
        for state in [
            HandoffState::Verified,
            HandoffState::Failed,
            HandoffState::NotVerified,
            HandoffState::ConfirmedByUser,
            HandoffState::Abandoned,
        ] {
            let final_state = FinalState::of(state).expect("a final state");
            assert_eq!(final_state.state(), state);
        }
        assert!(FinalState::of(HandoffState::Active).is_none());
        assert!(FinalState::Verified.writes_a_runbook());
        assert!(FinalState::ConfirmedByUser.writes_a_runbook());
        assert!(!FinalState::Failed.writes_a_runbook());
    }

    #[test]
    fn neither_the_spec_nor_the_attached_call_nor_the_pixels_reach_state_json() {
        let mut handoff = handoff();
        handoff.attached_call = Some(AttachedCall {
            conn_id: 7,
            call_id: "call_00000001".to_owned(),
            session_ref: None,
        });
        handoff.undelivered.push_back(Queued {
            outcome: crate::store::outcome::testing::any_outcome(),
            image: Some("PIXELSPIXELSPIXELS".to_owned()),
        });

        let json = serde_json::to_string(&handoff).expect("serialisable");
        assert!(
            !json.contains("why_human"),
            "the spec is a column, not state"
        );
        assert!(!json.contains("call_00000001"));
        assert!(!json.contains("PIXELSPIXELSPIXELS"));

        let restored: Handoff = serde_json::from_str(&json).expect("deserialisable");
        assert!(restored.spec.is_none());
        assert!(restored.attached_call.is_none());
        assert_eq!(restored.undelivered.len(), 1);
        assert!(restored.undelivered[0].image.is_none());
    }
}
