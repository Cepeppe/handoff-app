//! The view model: one tab, as the window draws it (§7.6, §8.4).
//!
//! It sits between [`crate::store::HandoffSnapshot`] — the store's own projection, which is
//! Rust-only and carries the **true** values — and the webview, which receives this and
//! nothing else. Three rules shape it, and they are the reason it exists as a layer at all:
//!
//! - **A secret-treated value never crosses on its own.** DET-04 shows it as `••••••` with a
//!   local ten-second **Show**; the reveal and the copy are two commands the user has to
//!   press, so the value crosses on demand and once, never as part of every repaint.
//! - **No text is written here.** `src/locales/{en,it}.json` is the single catalogue of the
//!   product (T-028), so what this builds is a *key* and its arguments — `counter.step` with
//!   an index and a total, `banner.verifying` with the spec's own `verify` — and the window
//!   renders them with `t()`. The only strings that leave here are the user's and the
//!   agent's own words, which are never translated (GUIDE-06).
//! - **A button the state does not offer is not drawn.** The store answers a user action it
//!   cannot take with `Refusal::NotActive`, which §7.4 calls a defect of the view; the
//!   `actions` block is that contract, written once, next to the states it reads.
//!
//! What it does not decide is the badge of the tab strip. "Unseen events" is a fact about
//! which tab the user is looking at, and the core does not know that; the window keeps it
//! (`src/overlay/state.svelte.ts`).

use serde::Serialize;

use crate::format::outcome::{ResumedFrom, VerifyReport};
use crate::format::spec::{url_allowed, SpecValue};
use crate::log::{HandoffState, Timestamp};
use crate::store::{Exchanges, HandoffSnapshot, Question, Reply, RoundSummary};

/// What a secret-treated value looks like until the user asks to see it (DET-04, §7.6).
pub const MASK: &str = "••••••";

/// The separator between the agent and the project in a tab label.
///
/// Imported rather than written again: `Session::display_name` prints the same two parts,
/// and the tab and an outcome's `resumed_from` name the same session — a user reading both
/// must not meet two spellings of it.
use crate::sessions::registry::LABEL_SEPARATOR;

/// The row of §8.4 a tab is on.
///
/// Not a second vocabulary for [`HandoffState`]: three of the rows are the `active` state
/// told apart by what is attached to it, and one — `Detached` — is a state of the *session*
/// rather than of the handoff, which is why the session's own connection is an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UiState {
    /// `awaiting_spec`: a user request the agent has not answered yet.
    WaitingForSpec,
    /// `active` with a call attached: the ordinary case, and the only one with no banner.
    Guiding,
    /// `active`, no call, nothing queued: the agent will pick it up at its next resume.
    AgentAway,
    /// `active` with a question or a screenshot the agent has not answered.
    QuestionSent,
    /// Deferred once.
    Deferred,
    /// Deferred twice; it waits for the user (RESP-07).
    Parked,
    /// Done on the last step of a spec that carries a `verify`.
    Verifying,
    /// One of the five final states.
    Final,
    /// Not final, and the session that owns it is gone (SRV-22).
    Detached,
}

/// Where a tab sits in the strip: the ordinary list, or the collapsible "waiting" group
/// §7.6 puts orphans and parked handoffs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TabGroup {
    /// Drawn in the strip.
    Open,
    /// Drawn in the collapsible group.
    Waiting,
}

/// One entry of the tab strip (MULTI-01, OPEN-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabView {
    /// `hf_` + 10 characters.
    pub id: String,
    /// Agent and project, as the tab is labelled.
    pub label: String,
    /// The agent alone, for a layout that wants the two apart.
    pub agent: Option<String>,
    /// The project folder's name alone.
    pub project: Option<String>,
    /// The state of §8.1, by its wire name.
    pub state: &'static str,
    /// The row of §8.4.
    pub ui_state: UiState,
    /// Which part of the strip it belongs to.
    pub group: TabGroup,
    /// The spec's goal, once there is one; the tooltip and the "waiting" group show it.
    pub goal: Option<String>,
    /// A final outcome nobody collected for seven days (SRV-23).
    pub orphan: bool,
    /// Which buttons this tab offers where it is listed.
    ///
    /// The same block the whole view carries, and for the same reason: the "waiting" group
    /// of §7.6 offers **Resume** and **Close it** from the list itself (SRV-23), and a
    /// second rule deciding which of them to draw there would be a second rule to keep in
    /// step with the store.
    pub actions: ActionsView,
    /// When it opened; the strip is ordered by it.
    pub created_at: Timestamp,
}

/// The counter of GUIDE-01, as a key and its numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CounterView {
    /// `counter.step` in the first round, `counter.correction` in a correction round.
    pub key: &'static str,
    /// 1-based step inside the round.
    pub index: u32,
    /// How many steps the round has.
    pub total: u32,
    /// Which round, 1-based.
    pub round: u32,
}

/// A link the user may open with one click, or plain text (GUIDE-03, SPEC-07).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkView {
    /// The URL as the spec wrote it.
    pub href: String,
    /// Whether its scheme is one of the four SPEC-07 allows. A spec's own URLs always are —
    /// the schema refused anything else — so this is what tells an **Open** button from a
    /// line of text the user has to copy.
    pub openable: bool,
}

impl LinkView {
    /// The link `href` is, with its scheme already judged.
    #[must_use]
    pub fn of(href: String) -> Self {
        let openable = url_allowed(&href);
        Self { href, openable }
    }
}

/// One value chip (GUIDE-02, DET-04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueChipView {
    /// The key of the spec's `values`.
    pub name: String,
    /// Whether the certain detector matched it at ingress, so the window shows the mask and
    /// offers **Show** (DET-04).
    pub masked: bool,
    /// The family that matched, when it did: `api_key`, `token`, … never the pattern id.
    pub kind: Option<String>,
    /// Whether the value is a list, which is copyable as a whole and per item (GUIDE-02).
    pub list: bool,
    /// What to draw: the value's items, or one [`MASK`] per item.
    pub items: Vec<String>,
}

/// One entry of the `secrets` list (SEC-01, SEC-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretEntryView {
    /// The variable name.
    pub name: String,
    /// The destination file, as the spec wrote it: relative to the project, or absolute.
    pub file: String,
}

/// The step the user is on (GUIDE-01..04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepView {
    /// The counter.
    pub counter: CounterView,
    /// The step's own text, in the language the agent wrote it (GUIDE-06).
    pub text: String,
    /// What the person should know before acting (GUIDE-04).
    pub warning: Option<String>,
    /// The step's `url`, else the spec's, else nothing (§7.6).
    pub url: Option<LinkView>,
    /// The values this step names, in the order it names them (GUIDE-02).
    pub values: Vec<ValueChipView>,
    /// Whether this step was already confirmed in this round.
    pub confirmed: bool,
    /// Whether it was skipped.
    pub skipped: bool,
    /// What the user wrote on it (RESP-03).
    pub notes: Vec<StepNoteView>,
    /// What the user asked on it (RESP-04); the reply below answers it.
    pub questions: Vec<StepQuestionView>,
    /// What an agent answered on it (RESP-04, TOOL-04).
    pub replies: Vec<StepReplyView>,
    /// Whether it is the last step of the round, which is where **Done** ends the round
    /// rather than advancing (RESP-09).
    pub last: bool,
}

/// A question the user asked, on the step they asked it from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepQuestionView {
    /// The round it belongs to.
    pub round: u32,
    /// The 1-based step it was raised on.
    pub step: u32,
    /// What the user wrote, as it was sent (already redacted, §7.10).
    pub text: String,
    /// When.
    pub at: Timestamp,
}

/// A note the user wrote on a step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepNoteView {
    /// The 1-based step.
    pub step: u32,
    /// What the user wrote.
    pub text: String,
    /// When, RFC 3339.
    pub at: String,
}

/// An agent's answer, on the step it referred to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepReplyView {
    /// The round it belongs to.
    pub round: u32,
    /// The 1-based step it answered.
    pub step: u32,
    /// What the agent wrote.
    pub text: String,
    /// When.
    pub at: Timestamp,
}

/// The interruption an agent has not answered yet (§8.4 "Question sent").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingView {
    /// `question` or `screenshot`.
    pub kind: &'static str,
    /// The 1-based step it was raised on.
    pub step: u32,
    /// What the user asked or wrote beside the picture, as it was sent (§7.6 shows the
    /// question, not only that there is one).
    pub text: Option<String>,
    /// What was sent, when a screenshot was (§7.6 "the question **or screenshot
    /// summary**"). The pixels are not in it and never were (LOG-03).
    pub screenshot: Option<PendingScreenshotView>,
}

/// The summary §7.6 shows in place of a screenshot the agent has not answered (PREV-04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingScreenshotView {
    /// `image` or `text`.
    pub mode: &'static str,
    /// The width of what was sent, when it is known.
    pub width: Option<u32>,
    /// The height of what was sent, when it is known.
    pub height: Option<u32>,
}

/// One closed round, collapsed (VER-09).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRoundView {
    /// 1-based round number.
    pub no: u32,
    /// Its steps' texts, in order.
    pub steps: Vec<String>,
    /// Which of them were confirmed.
    pub confirmed: Vec<u32>,
    /// Which of them were skipped.
    pub skipped: Vec<u32>,
    /// The notes written in it.
    pub notes: Vec<StepNoteView>,
    /// The questions asked in it.
    pub questions: Vec<StepQuestionView>,
    /// The replies given in it.
    pub replies: Vec<StepReplyView>,
    /// What the agent reported about it.
    pub verify: Option<VerifyResultView>,
    /// Whether this round was opened by a failed verification, which is what makes it a
    /// **correction** rather than the first pass (VER-09, VER-08).
    pub correction: bool,
    /// Whether the verification of this round came back negative — the marker §7.6 asks
    /// the History to carry.
    pub failed: bool,
}

/// A verification report, as the tab shows it (VER-05, §8.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResultView {
    /// `true`, `false`, or absent when the agent could not verify.
    pub ok: Option<bool>,
    /// What the agent found.
    pub detail: Option<String>,
    /// When it reported, RFC 3339.
    pub reported_at: String,
    /// Whether it arrived after the handoff had already been declared `not_verified`.
    pub late: bool,
}

impl From<&VerifyReport> for VerifyResultView {
    fn from(report: &VerifyReport) -> Self {
        Self {
            ok: report.ok,
            detail: report.detail.clone(),
            reported_at: report.reported_at.clone(),
            late: report.late,
        }
    }
}

/// The banner of §8.4: a catalogue key and, for the two rows that quote something, its
/// argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BannerView {
    /// The key in `src/locales/*.json`.
    pub key: &'static str,
    /// What `{text}` stands for, when the row quotes the spec or the agent.
    pub arg: Option<String>,
}

/// Which buttons the state of §8.4 offers.
///
/// The store refuses an action a state does not have (`Refusal::NotActive`) and §7.4 calls
/// that a defect of the view, so this block is the view's half of the same rule and the unit
/// tests below walk every state against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionsView {
    /// Step done, next step, or the end of the round on the last one.
    pub done: bool,
    /// Ask the agent (RESP-04).
    pub ask: bool,
    /// Annotate the step locally (RESP-02, RESP-03).
    pub note: bool,
    /// Skip the step (RESP-03).
    pub skip: bool,
    /// Defer (RESP-05, RESP-07).
    pub defer: bool,
    /// Abandon, always beside Defer (RESP-08).
    pub abandon: bool,
    /// Send what the user sees (CAP-01, PREV-01). Offered on the same states as Ask,
    /// because a screenshot interrupts a handoff being guided (§7.4) and nothing else.
    pub screenshot: bool,
    /// Pick a parked or deferred handoff up again (RESP-07, FM-31).
    pub resume: bool,
    /// Close a final outcome nobody collected (SRV-23).
    pub close_orphan: bool,
}

/// A runbook rewrite the user has not answered yet (§7.12 row 3, RUN-09).
///
/// The question is "update runbook `<name>` with the corrected sequence?", so what crosses
/// is what names the file: the id the answer refers to, the name the Runbooks page lists it
/// under, and its goal. The proposed document stays in the store — the webview never needs
/// it, and a document is not a question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookProposalView {
    /// The runbook the proposal would rewrite.
    pub runbook_id: String,
    /// Its file name (DD-17).
    pub file_name: String,
    /// Its goal, as it stands on disk.
    pub goal: String,
}

/// The request a handoff answers, when the link is not the id itself (OPEN-08, FM-20).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedRequestView {
    /// The request's id.
    pub id: String,
    /// The user's own words.
    pub text: Option<String>,
}

/// One handoff, whole, as the overlay draws it (§7.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffView {
    /// The tab this belongs to, so a window that has the view has the strip entry too.
    pub tab: TabView,
    /// Where it stands in §8.1, by its wire name.
    pub state: &'static str,
    /// The row of §8.4.
    pub ui_state: UiState,
    /// The banner that row shows, if any.
    pub banner: Option<BannerView>,
    /// What must be achieved.
    pub goal: Option<String>,
    /// Where to act.
    pub location: Option<String>,
    /// The spec's own starting point.
    pub url: Option<LinkView>,
    /// BCP-47 tag of the spec's texts (GUIDE-06).
    pub lang: Option<String>,
    /// The step the user is on; absent while there is no spec and once it is final.
    pub step: Option<StepView>,
    /// The `secrets` list (SEC-02).
    pub secrets: Vec<SecretEntryView>,
    /// Every note of the current round.
    pub notes: Vec<StepNoteView>,
    /// The interruption the agent owes an answer to.
    pub pending: Option<PendingView>,
    /// The previous rounds, collapsed (VER-09).
    pub history: Vec<HistoryRoundView>,
    /// What the agent will check (VER-04).
    pub verify: Option<String>,
    /// What it reported (VER-05); the label "declared by agent" belongs to it.
    pub verify_result: Option<VerifyResultView>,
    /// Why a `not_verified` handoff is not verified, when no report says it (VER-06).
    ///
    /// A catalogue key like every other text this module produces, and `None` whenever the
    /// tab has something better to show: any other state, and a `not_verified` the agent
    /// reported itself, where `verify_result` carries the agent's own detail.
    pub not_verified_reason: Option<&'static str>,
    /// Which buttons to draw.
    pub actions: ActionsView,
    /// The user's own words, when it grew from a request (OPEN-04).
    pub request_text: Option<String>,
    /// The request it answers, with the **Change** control of FM-20.
    pub linked_request: Option<LinkedRequestView>,
    /// The runbook rewrite waiting for an answer (RUN-09).
    pub runbook_proposal: Option<RunbookProposalView>,
    /// The opening session, when the current call comes from another one (TOOL-08).
    pub resumed_from: Option<ResumedFrom>,
    /// Whether a call is listening right now.
    pub call_attached: bool,
    /// How many outcomes wait for the next resume (DD-12).
    pub undelivered: usize,
    /// When it opened.
    pub created_at: Timestamp,
    /// When it closed.
    pub closed_at: Option<Timestamp>,
}

/// The tab strip, in the order the store lists the handoffs (oldest first).
///
/// `connected` answers "is the session that owns this handoff still there", which is the
/// registry's to know (SRV-21) and the reason it is a parameter: the store deliberately
/// keeps no notion of it.
#[must_use]
pub fn tabs(
    snapshots: &[HandoffSnapshot],
    connected: &dyn Fn(&str) -> bool,
    now: &Timestamp,
) -> Vec<TabView> {
    // `now` is the store's own instant for this read; the orphan flag is already decided
    // against it (`list_for_ui`), which is why nothing here compares timestamps.
    snapshots
        .iter()
        .map(|snapshot| tab_of(snapshot, ui_state_of(snapshot, connected), now))
        .collect()
}

/// One whole tab (§7.6).
#[must_use]
pub fn build(
    snapshot: &HandoffSnapshot,
    exchanges: &Exchanges,
    connected: &dyn Fn(&str) -> bool,
    now: &Timestamp,
) -> HandoffView {
    let ui_state = ui_state_of(snapshot, connected);
    let verify_result = snapshot.verify_report.as_ref().map(VerifyResultView::from);

    HandoffView {
        tab: tab_of(snapshot, ui_state, now),
        state: snapshot.state.as_str(),
        ui_state,
        banner: banner_of(snapshot, ui_state),
        goal: snapshot.goal.clone(),
        location: snapshot.location.clone(),
        url: snapshot.url.clone().map(LinkView::of),
        lang: snapshot.lang.clone(),
        step: step_of(snapshot, exchanges),
        secrets: secrets_of(snapshot),
        notes: snapshot.notes.iter().map(note_view).collect(),
        pending: pending_of(snapshot, exchanges),
        history: history_of(snapshot, exchanges),
        verify: snapshot.verify.clone(),
        verify_result,
        not_verified_reason: snapshot
            .not_verified_reason
            .map(crate::store::NotVerifiedReason::key),
        actions: actions_of(snapshot),
        request_text: snapshot.request_text.clone(),
        linked_request: snapshot
            .linked_request_id
            .clone()
            .map(|id| LinkedRequestView {
                id,
                text: snapshot.request_text.clone(),
            }),
        runbook_proposal: snapshot
            .runbook_proposal
            .as_ref()
            .map(|proposal| RunbookProposalView {
                runbook_id: proposal.runbook_id.clone(),
                file_name: proposal.file_name.clone(),
                goal: proposal.goal.clone(),
            }),
        resumed_from: snapshot.resumed_from.clone(),
        call_attached: snapshot.call_attached,
        undelivered: snapshot.undelivered,
        created_at: snapshot.created_at.clone(),
        closed_at: snapshot.closed_at.clone(),
    }
}

/// The row of §8.4 this handoff is on.
///
/// The order of the arms **is** the precedence, and the one place it had to be decided is
/// `Detached`: §8.4 gives it "any non-final, session disconnected", which overlaps every
/// other non-final row. It wins, because it is the only one that is still true — a tab that
/// says "waiting for the reply" when the agent's process is gone is telling the user to wait
/// for something that cannot arrive, while SRV-22's sentence stays correct in every case
/// (the outcome is delivered at the next resume, by whichever session does it).
///
/// With one exception, and it is the case the whole row exists for: **a handoff with a call
/// attached is not detached**, whatever its opener's session is doing. `session_ref` is the
/// session that *opened* the handoff, and TOOL-08 lets any session resume it — while §8.3
/// makes a server that comes back after a disconnection a **new** session with a new
/// `session_ref`, the old record kept as history. So after an app restart every restored
/// tab points at a session that can never reconnect under that name, and without this
/// clause SRV-22's banner would be permanent: the agent resumes, an agent is guiding the
/// user through the steps, and the tab still says the session is detached. A call cannot be
/// attached from a session that is gone — `Store::session_disconnected` detaches the calls
/// of a session as it goes — so "a call is attached" *is* "an agent is here right now".
fn ui_state_of(snapshot: &HandoffSnapshot, connected: &dyn Fn(&str) -> bool) -> UiState {
    if snapshot.state.is_final() {
        return UiState::Final;
    }
    if !snapshot.call_attached
        && snapshot
            .session_ref
            .as_deref()
            .is_some_and(|session_ref| !connected(session_ref))
    {
        return UiState::Detached;
    }
    match snapshot.state {
        HandoffState::AwaitingSpec => UiState::WaitingForSpec,
        HandoffState::Deferred => UiState::Deferred,
        HandoffState::Parked => UiState::Parked,
        HandoffState::AwaitingVerification => UiState::Verifying,
        HandoffState::Active if snapshot.pending_question.is_some() => UiState::QuestionSent,
        HandoffState::Active if snapshot.call_attached => UiState::Guiding,
        HandoffState::Active => UiState::AgentAway,
        // Unreachable: the five final states are answered above. Written as an arm rather
        // than as a panic because a `state_json` a future version wrote must not be able to
        // take the window down (FM-28).
        _ => UiState::Guiding,
    }
}

/// The banner of §8.4, as a key and its argument.
fn banner_of(snapshot: &HandoffSnapshot, ui_state: UiState) -> Option<BannerView> {
    let plain = |key| Some(BannerView { key, arg: None });
    match ui_state {
        UiState::WaitingForSpec => plain("banner.awaitingSpec"),
        UiState::Guiding => None,
        UiState::AgentAway => plain("banner.agentAway"),
        UiState::QuestionSent => plain("banner.questionSent"),
        UiState::Deferred => plain("banner.deferred"),
        UiState::Parked => plain("banner.parked"),
        UiState::Detached => plain("banner.detached"),
        UiState::Verifying => Some(BannerView {
            key: "banner.verifying",
            arg: snapshot.verify.clone(),
        }),
        // The final row is the state's own label, and the detail beside it — "declared by
        // agent", "orphan" — is drawn from `verifyResult` and `tab.orphan`.
        UiState::Final => Some(BannerView {
            key: state_key(snapshot.state),
            arg: None,
        }),
    }
}

/// The catalogue key of a state's label (§8.4).
///
/// `pub(super)` for the Log page, which labels a row of the record with the same table: two
/// mappings would be two names for one handoff, one in the strip and one in the log.
pub(super) fn state_key(state: HandoffState) -> &'static str {
    match state {
        HandoffState::AwaitingSpec => "state.awaitingSpec",
        HandoffState::Active => "state.active",
        HandoffState::Deferred => "state.deferred",
        HandoffState::Parked => "state.parked",
        HandoffState::AwaitingVerification => "state.awaitingVerification",
        HandoffState::Verified => "state.verified",
        HandoffState::Failed => "state.failed",
        HandoffState::NotVerified => "state.notVerified",
        HandoffState::ConfirmedByUser => "state.confirmedByUser",
        HandoffState::Abandoned => "state.abandoned",
    }
}

fn tab_of(snapshot: &HandoffSnapshot, ui_state: UiState, _now: &Timestamp) -> TabView {
    let label = snapshot
        .opener_label
        .as_ref()
        .map(|opener| format!("{}{LABEL_SEPARATOR}{}", opener.agent, opener.project))
        // A handoff with no opening session — a request the user typed before any agent
        // registered — still needs a tab, and its own id is the only thing it can be
        // called until a spec arrives.
        .unwrap_or_else(|| snapshot.id.clone());

    TabView {
        id: snapshot.id.clone(),
        label,
        agent: snapshot
            .opener_label
            .as_ref()
            .map(|opener| opener.agent.clone()),
        project: snapshot
            .opener_label
            .as_ref()
            .map(|opener| opener.project.clone()),
        state: snapshot.state.as_str(),
        ui_state,
        group: if snapshot.orphan || snapshot.state == HandoffState::Parked {
            TabGroup::Waiting
        } else {
            TabGroup::Open
        },
        goal: snapshot.goal.clone(),
        orphan: snapshot.orphan,
        actions: actions_of(snapshot),
        created_at: snapshot.created_at.clone(),
    }
}

/// The interruption the agent still owes an answer to (§8.4 "Question sent").
///
/// The store keeps that there **is** one and on which step (§7.4 has no field for the
/// words); the words are in the diary, and the one this names is the last question asked on
/// that step of the current round.
fn pending_of(snapshot: &HandoffSnapshot, exchanges: &Exchanges) -> Option<PendingView> {
    let pending = snapshot.pending_question.as_ref()?;
    // The last screenshot sent on this step of this round, which is the one being waited on.
    let shot = exchanges
        .screenshots
        .iter()
        .rev()
        .find(|shot| shot.round == snapshot.round && shot.step == pending.step);
    Some(PendingView {
        kind: match pending.kind {
            crate::store::PendingKind::Question => "question",
            crate::store::PendingKind::Screenshot => "screenshot",
        },
        step: pending.step,
        text: match pending.kind {
            crate::store::PendingKind::Question => exchanges
                .questions
                .iter()
                .rev()
                .find(|question| question.round == snapshot.round && question.step == pending.step)
                .map(|question| question.text.clone()),
            // The comment beside the picture, or the text that was sent in its place — in
            // both cases exactly what left (LOG-03), which is what the user is waiting on
            // an answer about.
            crate::store::PendingKind::Screenshot => shot.and_then(|shot| shot.text.clone()),
        },
        screenshot: match pending.kind {
            crate::store::PendingKind::Question => None,
            crate::store::PendingKind::Screenshot => Some(PendingScreenshotView {
                mode: shot.map_or("image", |shot| match shot.mode {
                    crate::format::outcome::ScreenshotMode::Image => "image",
                    crate::format::outcome::ScreenshotMode::Text => "text",
                }),
                width: shot.and_then(|shot| shot.width),
                height: shot.and_then(|shot| shot.height),
            }),
        },
    })
}

/// The step the cursor is on, with everything that hangs off it.
///
/// Absent when there is no spec yet and when the handoff is final: in both cases §8.4 gives
/// the tab a banner and no step to walk.
fn step_of(snapshot: &HandoffSnapshot, exchanges: &Exchanges) -> Option<StepView> {
    if snapshot.state == HandoffState::AwaitingSpec || snapshot.state.is_final() {
        return None;
    }
    let index = snapshot.step_index;
    let step = snapshot
        .steps
        .get(usize::try_from(index).ok()?.checked_sub(1)?)?;

    Some(StepView {
        counter: CounterView {
            key: if snapshot.round > 1 {
                "counter.correction"
            } else {
                "counter.step"
            },
            index,
            total: snapshot.step_total,
            round: snapshot.round,
        },
        text: step.text.clone(),
        warning: step.warning.clone(),
        // §7.6: the step's own starting point, and the spec's when the step has none.
        url: step
            .url
            .clone()
            .or_else(|| snapshot.url.clone())
            .map(LinkView::of),
        values: chips_of(snapshot, step.values.as_deref().unwrap_or_default()),
        confirmed: snapshot.confirmed.contains(&index),
        skipped: snapshot.skipped.contains(&index),
        notes: snapshot
            .notes
            .iter()
            .filter(|note| note.step == index)
            .map(note_view)
            .collect(),
        questions: exchanges
            .questions
            .iter()
            .filter(|question| question.round == snapshot.round && question.step == index)
            .map(question_view)
            .collect(),
        replies: exchanges
            .replies
            .iter()
            .filter(|reply| reply.round == snapshot.round && reply.step == index)
            .map(reply_view)
            .collect(),
        last: index >= snapshot.step_total,
    })
}

/// The chips of the values a step names, in the order it names them (GUIDE-02).
///
/// A key the spec does not declare cannot occur — S3 refuses it at ingress — and is dropped
/// here rather than drawn as an empty chip.
fn chips_of(snapshot: &HandoffSnapshot, names: &[String]) -> Vec<ValueChipView> {
    names
        .iter()
        .filter_map(|name| {
            let value = snapshot.values.get(name)?;
            let kind = secret_kind(snapshot, name);
            let items = match value {
                SpecValue::One(one) => vec![one.clone()],
                SpecValue::Many(many) => many.clone(),
            };
            let list = matches!(value, SpecValue::Many(_));
            Some(ValueChipView {
                name: name.clone(),
                masked: kind.is_some(),
                // The mask replaces the value here and not in the window: what crosses the
                // boundary for a secret-treated value is the mask, and the true one only
                // when the user presses **Show** or **Copy** (DET-04).
                items: if kind.is_some() {
                    items.iter().map(|_| MASK.to_owned()).collect()
                } else {
                    items
                },
                kind,
                list,
            })
        })
        .collect()
}

/// The family the certain detector reported for a top-level value, if it reported one.
///
/// The same rule as [`crate::store::Handoff::is_secret_value`], which cannot be reused here
/// because a snapshot is not a handoff: an array one of whose items matched is masked whole,
/// under the family of the first item reported.
fn secret_kind(snapshot: &HandoffSnapshot, name: &str) -> Option<String> {
    snapshot
        .secret_treated
        .iter()
        .find(|treated| treated.is_in_value(name))
        .map(|treated| treated.kind.clone())
}

fn secrets_of(snapshot: &HandoffSnapshot) -> Vec<SecretEntryView> {
    snapshot
        .secrets
        .as_ref()
        .map(|secrets| {
            secrets
                .iter()
                .map(|(name, file)| SecretEntryView {
                    name: name.clone(),
                    file: file.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn history_of(snapshot: &HandoffSnapshot, exchanges: &Exchanges) -> Vec<HistoryRoundView> {
    snapshot
        .history
        .iter()
        .map(|round| history_round(round, exchanges))
        .collect()
}

fn history_round(round: &RoundSummary, exchanges: &Exchanges) -> HistoryRoundView {
    HistoryRoundView {
        no: round.no,
        steps: round.steps.iter().map(|step| step.text.clone()).collect(),
        confirmed: round.confirmed.clone(),
        skipped: round.skipped.clone(),
        notes: round.notes.iter().map(note_view).collect(),
        questions: exchanges
            .questions
            .iter()
            .filter(|question| question.round == round.no)
            .map(question_view)
            .collect(),
        replies: exchanges
            .replies
            .iter()
            .filter(|reply| reply.round == round.no)
            .map(reply_view)
            .collect(),
        verify: round.verify.as_ref().map(VerifyResultView::from),
        // Every round after the first is a correction: §8.1 opens one only from a failed
        // verification, and the counter of the round itself already says so (GUIDE-01).
        correction: round.no > 1,
        failed: round
            .verify
            .as_ref()
            .is_some_and(|report| report.ok == Some(false)),
    }
}

fn note_view(note: &crate::format::outcome::OutcomeNote) -> StepNoteView {
    StepNoteView {
        step: note.step,
        text: note.text.clone(),
        at: note.at.clone(),
    }
}

fn question_view(question: &Question) -> StepQuestionView {
    StepQuestionView {
        round: question.round,
        step: question.step,
        text: question.text.clone(),
        at: question.at.clone(),
    }
}

fn reply_view(reply: &Reply) -> StepReplyView {
    StepReplyView {
        round: reply.round,
        step: reply.step,
        text: reply.text.clone(),
        at: reply.at.clone(),
    }
}

/// Which buttons the state offers, mirroring what the store accepts.
///
/// `close_orphan` is offered only on an orphan, although the store accepts it on any final
/// outcome nobody collected: SRV-23 gives the user the control on the orphan list, and a
/// narrower view than the store is safe in the direction that matters — it can never produce
/// a `Refusal::NotActive`.
fn actions_of(snapshot: &HandoffSnapshot) -> ActionsView {
    let active = snapshot.state == HandoffState::Active;
    let is_final = snapshot.state.is_final();
    ActionsView {
        done: active,
        ask: active,
        note: active,
        skip: active,
        defer: matches!(
            snapshot.state,
            HandoffState::Active | HandoffState::Deferred
        ),
        abandon: !is_final,
        // The button opens the two-choice popover of CAP-01 and ends in the preview, which
        // is where the two send buttons are. It is offered on the same states as Ask and
        // Note, because a screenshot is an interrupting action on a handoff being guided
        // (§7.4) and on nothing else.
        screenshot: active,
        resume: matches!(
            snapshot.state,
            HandoffState::Deferred | HandoffState::Parked
        ),
        close_orphan: snapshot.orphan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use indexmap::IndexMap;

    use crate::format::outcome::{OutcomeNote, SecretTreated};
    use crate::format::spec::HandoffStep;
    use crate::store::{PendingKind, PendingQuestion};

    const SESSION: &str = "ses_00000001";

    fn at(text: &str) -> Timestamp {
        Timestamp::parse(text).expect("rfc 3339")
    }

    fn now() -> Timestamp {
        at("2026-09-09T10:00:00Z")
    }

    fn step(text: &str) -> HandoffStep {
        HandoffStep {
            text: text.to_owned(),
            url: None,
            values: None,
            warning: None,
        }
    }

    /// A snapshot in the shape the store produces for an ordinary guided handoff.
    fn snapshot() -> HandoffSnapshot {
        let mut values = IndexMap::new();
        values.insert(
            "endpoint_url".to_owned(),
            SpecValue::One("https://api.example.test/hook".to_owned()),
        );
        values.insert(
            "events".to_owned(),
            SpecValue::Many(vec![
                "payment.succeeded".to_owned(),
                "payment.failed".to_owned(),
            ]),
        );
        values.insert(
            "api_key".to_owned(),
            SpecValue::One("sk_live_0123456789abcdef".to_owned()),
        );

        HandoffSnapshot {
            id: "hf_0000000001".to_owned(),
            state: HandoffState::Active,
            round: 1,
            step_index: 1,
            step_total: 2,
            steps: vec![
                HandoffStep {
                    text: "Open the dashboard and add the endpoint.".to_owned(),
                    url: Some("https://dashboard.example.test/webhooks".to_owned()),
                    values: Some(vec![
                        "endpoint_url".to_owned(),
                        "events".to_owned(),
                        "api_key".to_owned(),
                    ]),
                    warning: Some("This is the live account.".to_owned()),
                },
                step("Save and copy the signing secret."),
            ],
            confirmed: Vec::new(),
            skipped: Vec::new(),
            notes: Vec::new(),
            deferral_count: 0,
            pending_question: None,
            undelivered: 0,
            call_attached: true,
            session_ref: Some(SESSION.to_owned()),
            opener_label: Some(ResumedFrom {
                agent: "Claude Code".to_owned(),
                project: "baton".to_owned(),
            }),
            project_dir: Some("C:\\projects\\baton".to_owned()),
            goal: Some("Register the webhook".to_owned()),
            location: Some("Dashboard → Webhooks".to_owned()),
            url: Some("https://dashboard.example.test".to_owned()),
            lang: Some("en".to_owned()),
            values,
            secrets: Some(IndexMap::from([(
                "STRIPE_SIGNING_SECRET".to_owned(),
                ".env.local".to_owned(),
            )])),
            secret_treated: vec![SecretTreated {
                location: "values.api_key".to_owned(),
                kind: "api_key".to_owned(),
            }],
            history: Vec::new(),
            verify_report: None,
            verify: Some("the webhook fires".to_owned()),
            request_text: None,
            linked_request_id: None,
            resumed_from: None,
            final_outcome: None,
            orphan: false,
            runbook_proposal: None,
            not_verified_reason: None,
            created_at: at("2026-09-09T09:00:00Z"),
            closed_at: None,
        }
    }

    fn connected(_session_ref: &str) -> bool {
        true
    }

    fn gone(_session_ref: &str) -> bool {
        false
    }

    fn view(snapshot: &HandoffSnapshot) -> HandoffView {
        build(snapshot, &Exchanges::default(), &connected, &now())
    }

    /// The diary of a handoff nobody asked anything on.
    fn silent() -> Exchanges {
        Exchanges::default()
    }

    #[test]
    fn every_row_of_the_state_table_has_its_own_label_and_banner() {
        // §8.4, row by row. The banner keys are what the window looks up, so a row that
        // silently fell back to another row's text would show the wrong sentence with no
        // failure anywhere else.
        let mut it = snapshot();

        it.state = HandoffState::AwaitingSpec;
        assert_eq!(view(&it).ui_state, UiState::WaitingForSpec);
        assert_eq!(
            view(&it).banner.expect("a banner").key,
            "banner.awaitingSpec"
        );

        it = snapshot();
        assert_eq!(view(&it).ui_state, UiState::Guiding);
        assert!(view(&it).banner.is_none(), "the guided row has no banner");

        it.call_attached = false;
        assert_eq!(view(&it).ui_state, UiState::AgentAway);
        assert_eq!(view(&it).banner.expect("a banner").key, "banner.agentAway");

        it = snapshot();
        it.pending_question = Some(PendingQuestion {
            kind: PendingKind::Question,
            step: 1,
            at: at("2026-09-09T09:30:00Z"),
        });
        assert_eq!(view(&it).ui_state, UiState::QuestionSent);
        assert_eq!(
            view(&it).banner.expect("a banner").key,
            "banner.questionSent"
        );

        it = snapshot();
        it.state = HandoffState::Deferred;
        assert_eq!(view(&it).ui_state, UiState::Deferred);
        assert_eq!(view(&it).banner.expect("a banner").key, "banner.deferred");

        it.state = HandoffState::Parked;
        assert_eq!(view(&it).ui_state, UiState::Parked);
        assert_eq!(view(&it).banner.expect("a banner").key, "banner.parked");

        it = snapshot();
        it.state = HandoffState::AwaitingVerification;
        let banner = view(&it).banner.expect("a banner");
        assert_eq!(banner.key, "banner.verifying");
        assert_eq!(
            banner.arg.as_deref(),
            Some("the webhook fires"),
            "the verifying row quotes the spec"
        );

        it = snapshot();
        it.state = HandoffState::Verified;
        it.closed_at = Some(now());
        assert_eq!(view(&it).ui_state, UiState::Final);
        assert_eq!(view(&it).banner.expect("a banner").key, "state.verified");

        it = snapshot();
        it.call_attached = false;
        assert_eq!(
            build(&it, &silent(), &gone, &now()).ui_state,
            UiState::Detached,
            "a session that is gone outranks every other non-final row"
        );
        assert_eq!(
            build(&it, &silent(), &gone, &now())
                .banner
                .expect("a banner")
                .key,
            "banner.detached"
        );
    }

    #[test]
    fn an_attached_call_is_an_agent_that_is_here_now_whatever_the_opener_is_doing() {
        // The case the restart of §7.2 puts every restored tab in: a reconnection is a new
        // session (§8.3), so the opener's `session_ref` never comes back. Without this the
        // banner of SRV-22 would be permanent — an agent guiding the user through the steps,
        // and a tab saying the session is detached.
        let mut it = snapshot();
        it.state = HandoffState::Active;
        it.pending_question = None;

        it.call_attached = false;
        assert_eq!(
            build(&it, &silent(), &gone, &now()).ui_state,
            UiState::Detached
        );

        it.call_attached = true;
        assert_eq!(
            build(&it, &silent(), &gone, &now()).ui_state,
            UiState::Guiding,
            "a resumed handoff is guided, not detached"
        );
        assert_eq!(build(&it, &silent(), &gone, &now()).banner, None);
    }

    #[test]
    fn a_deferred_handoff_of_a_lost_session_still_says_it_is_detached() {
        // The clause above is confined to an attached call: everything the user is left
        // waiting for — deferred, parked, a question sent — still reports SRV-22's sentence,
        // which is the T-036 precedence and stays exactly as it was.
        let mut it = snapshot();
        it.call_attached = false;
        for state in [
            HandoffState::Deferred,
            HandoffState::Parked,
            HandoffState::AwaitingVerification,
            HandoffState::AwaitingSpec,
        ] {
            it.state = state;
            assert_eq!(
                build(&it, &silent(), &gone, &now()).ui_state,
                UiState::Detached,
                "{state:?} of a session that is gone"
            );
        }
    }

    #[test]
    fn each_of_the_ten_states_has_a_label_key_of_its_own() {
        let keys: Vec<&str> = [
            HandoffState::AwaitingSpec,
            HandoffState::Active,
            HandoffState::Deferred,
            HandoffState::Parked,
            HandoffState::AwaitingVerification,
            HandoffState::Verified,
            HandoffState::Failed,
            HandoffState::NotVerified,
            HandoffState::ConfirmedByUser,
            HandoffState::Abandoned,
        ]
        .into_iter()
        .map(state_key)
        .collect();

        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "two states share a label");
    }

    #[test]
    fn a_final_handoff_keeps_its_report_and_its_orphan_flag() {
        let mut it = snapshot();
        it.state = HandoffState::Failed;
        it.closed_at = Some(now());
        it.orphan = true;
        it.verify_report = Some(VerifyReport {
            ok: Some(false),
            detail: Some("the endpoint answered 404".to_owned()),
            reported_at: "2026-09-09T09:45:00.000Z".to_owned(),
            late: false,
        });

        let view = view(&it);
        assert_eq!(view.banner.expect("a banner").key, "state.failed");
        let report = view.verify_result.expect("a report");
        assert_eq!(report.ok, Some(false));
        assert_eq!(report.detail.as_deref(), Some("the endpoint answered 404"));
        assert!(view.tab.orphan);
        assert_eq!(view.tab.group, TabGroup::Waiting);
        assert!(view.step.is_none(), "a closed handoff has no step to walk");
        assert!(view.actions.close_orphan);
    }

    #[test]
    fn the_counter_says_step_in_the_first_round_and_correction_afterwards() {
        let mut it = snapshot();
        let counter = view(&it).step.expect("a step").counter;
        assert_eq!(counter.key, "counter.step");
        assert_eq!((counter.index, counter.total, counter.round), (1, 2, 1));

        it.round = 2;
        it.step_index = 1;
        it.step_total = 3;
        it.steps = vec![step("redo one"), step("redo two"), step("redo three")];
        let counter = view(&it).step.expect("a step").counter;
        assert_eq!(counter.key, "counter.correction");
        assert_eq!((counter.index, counter.total, counter.round), (1, 3, 2));
    }

    #[test]
    fn a_secret_treated_value_crosses_as_the_mask_and_never_as_itself() {
        let view = view(&snapshot());
        let chips = view.step.expect("a step").values;
        let key = chips
            .iter()
            .find(|chip| chip.name == "api_key")
            .expect("the api_key chip");

        assert!(key.masked);
        assert_eq!(key.kind.as_deref(), Some("api_key"));
        assert_eq!(key.items, vec![MASK.to_owned()]);

        let serialised = serde_json::to_string(&chips).expect("serialisable");
        assert!(
            !serialised.contains("sk_live_0123456789abcdef"),
            "the true value reached the webview"
        );
    }

    #[test]
    fn an_array_one_of_whose_items_is_a_secret_crosses_masked_whole() {
        // The server reports an array one item at a time (`values.events[1]`, §4.7.5).
        let mut it = snapshot();
        it.values.insert(
            "events".to_owned(),
            SpecValue::Many(vec![
                "payment.succeeded".to_owned(),
                "planted-secret-item".to_owned(),
            ]),
        );
        it.secret_treated.push(SecretTreated {
            location: "values.events[1]".to_owned(),
            kind: "webhook_secret".to_owned(),
        });
        let chips = view(&it).step.expect("a step").values;
        let events = chips
            .iter()
            .find(|chip| chip.name == "events")
            .expect("the events chip");

        assert!(events.masked);
        assert!(events.list);
        assert_eq!(events.kind.as_deref(), Some("webhook_secret"));
        assert_eq!(events.items, vec![MASK.to_owned(), MASK.to_owned()]);

        let serialised = serde_json::to_string(&chips).expect("serialisable");
        assert!(
            !serialised.contains("planted-secret-item")
                && !serialised.contains("payment.succeeded"),
            "an item of the secret-treated array reached the webview"
        );
    }

    #[test]
    fn an_ordinary_value_keeps_its_text_and_a_list_keeps_its_items() {
        let chips = view(&snapshot()).step.expect("a step").values;
        assert_eq!(
            chips
                .iter()
                .map(|chip| chip.name.as_str())
                .collect::<Vec<_>>(),
            vec!["endpoint_url", "events", "api_key"],
            "the chips follow the order the step names them in"
        );

        let url = &chips[0];
        assert!(!url.masked);
        assert!(!url.list);
        assert_eq!(url.items, vec!["https://api.example.test/hook".to_owned()]);

        let events = &chips[1];
        assert!(events.list, "an array is copyable as a whole and per item");
        assert_eq!(events.items.len(), 2);
    }

    #[test]
    fn a_step_with_no_url_falls_back_to_the_specs_and_only_allowed_schemes_open() {
        let mut it = snapshot();
        let step_url = view(&it).step.expect("a step").url.expect("a link");
        assert_eq!(step_url.href, "https://dashboard.example.test/webhooks");
        assert!(step_url.openable);

        it.step_index = 2;
        let fallback = view(&it).step.expect("a step").url.expect("a link");
        assert_eq!(
            fallback.href, "https://dashboard.example.test",
            "the second step has no url of its own"
        );

        it.url = Some("file:///etc/passwd".to_owned());
        let refused = view(&it).step.expect("a step").url.expect("a link");
        assert!(
            !refused.openable,
            "a scheme outside SPEC-07 is text, never a button"
        );
    }

    #[test]
    fn the_tab_is_labelled_the_way_a_session_names_itself() {
        let view = view(&snapshot());
        assert_eq!(view.tab.label, "Claude Code · baton");
        assert_eq!(view.tab.agent.as_deref(), Some("Claude Code"));
        assert_eq!(view.tab.project.as_deref(), Some("baton"));
        assert_eq!(view.tab.group, TabGroup::Open);
    }

    #[test]
    fn the_tab_a_user_request_opens_shows_their_words_and_offers_only_abandon() {
        // Exactly what `Store::open_request` produces (OPEN-04): no spec, no round, no step
        // to walk. What the user can do about it is give up on it, and copy the sentence
        // again — which is the view's own button and not an `act`.
        let mut it = snapshot();
        it.state = HandoffState::AwaitingSpec;
        it.step_total = 0;
        it.steps = Vec::new();
        it.call_attached = false;
        it.goal = None;
        it.location = None;
        it.values = IndexMap::new();
        it.secrets = None;
        it.request_text = Some("create the API key on Stripe".to_owned());

        let view = view(&it);
        assert_eq!(view.ui_state, UiState::WaitingForSpec);
        assert_eq!(
            view.request_text.as_deref(),
            Some("create the API key on Stripe")
        );
        assert!(view.step.is_none(), "there is no step until a spec arrives");
        assert!(view.goal.is_none());
        assert!(view.actions.abandon);
        assert!(!view.actions.done && !view.actions.ask && !view.actions.skip);
        assert!(!view.actions.defer && !view.actions.resume);
    }

    #[test]
    fn a_handoff_with_no_opening_session_is_still_a_tab() {
        let mut it = snapshot();
        it.opener_label = None;
        it.session_ref = None;
        it.state = HandoffState::AwaitingSpec;

        let view = view(&it);
        assert_eq!(view.tab.label, "hf_0000000001");
        assert_eq!(
            view.ui_state,
            UiState::WaitingForSpec,
            "with no session there is nothing that can be disconnected"
        );
    }

    #[test]
    fn the_buttons_follow_the_state_the_store_will_accept() {
        let mut it = snapshot();
        let actions = view(&it).actions;
        assert!(actions.done && actions.ask && actions.note && actions.skip);
        assert!(actions.defer && actions.abandon);
        assert!(
            actions.screenshot,
            "a screenshot interrupts a handoff being guided, like Ask"
        );
        assert!(!actions.resume && !actions.close_orphan);

        it.state = HandoffState::Deferred;
        let actions = view(&it).actions;
        assert!(!actions.done && !actions.ask && !actions.note && !actions.skip);
        assert!(
            !actions.screenshot,
            "there is nobody to send a screenshot to while it is deferred"
        );
        assert!(actions.defer, "a deferred handoff may be deferred again");
        assert!(actions.abandon && actions.resume);

        it.state = HandoffState::Parked;
        let actions = view(&it).actions;
        assert!(
            !actions.defer,
            "a parked handoff is the user's, not the agent's"
        );
        assert!(actions.resume && actions.abandon);

        it.state = HandoffState::Abandoned;
        it.closed_at = Some(now());
        let actions = view(&it).actions;
        assert!(!actions.abandon && !actions.defer && !actions.resume);
    }

    #[test]
    fn the_notes_and_the_reply_land_on_the_step_they_belong_to() {
        let mut it = snapshot();
        it.notes = vec![
            OutcomeNote {
                step: 1,
                text: "the button is called Add endpoint now".to_owned(),
                at: "2026-09-09T09:20:00.000Z".to_owned(),
            },
            OutcomeNote {
                step: 2,
                text: "not there yet".to_owned(),
                at: "2026-09-09T09:25:00.000Z".to_owned(),
            },
        ];
        let replies = vec![
            Reply {
                round: 1,
                step: 1,
                text: "use the Developers tab".to_owned(),
                at: at("2026-09-09T09:30:00Z"),
            },
            Reply {
                round: 1,
                step: 2,
                text: "not this step".to_owned(),
                at: at("2026-09-09T09:31:00Z"),
            },
        ];

        let view = build(
            &it,
            &Exchanges {
                questions: Vec::new(),
                replies,
                screenshots: Vec::new(),
            },
            &connected,
            &now(),
        );
        let step = view.step.expect("a step");
        assert_eq!(step.notes.len(), 1);
        assert_eq!(step.notes[0].step, 1);
        assert_eq!(step.replies.len(), 1);
        assert_eq!(step.replies[0].text, "use the Developers tab");
        assert_eq!(view.notes.len(), 2, "the tab keeps the round's notes whole");
    }

    #[test]
    fn a_correction_round_collapses_the_one_before_it() {
        let mut it = snapshot();
        it.round = 2;
        it.step_index = 1;
        it.step_total = 1;
        it.steps = vec![step("try again with the other endpoint")];
        it.history = vec![RoundSummary {
            no: 1,
            steps: vec![step("open the dashboard"), step("save")],
            confirmed: vec![1],
            skipped: vec![2],
            notes: vec![OutcomeNote {
                step: 1,
                text: "was already there".to_owned(),
                at: "2026-09-09T09:10:00.000Z".to_owned(),
            }],
            verify: Some(VerifyReport {
                ok: Some(false),
                detail: Some("404".to_owned()),
                reported_at: "2026-09-09T09:40:00.000Z".to_owned(),
                late: false,
            }),
        }];

        let replies = vec![Reply {
            round: 1,
            step: 2,
            text: "the endpoint was wrong".to_owned(),
            at: at("2026-09-09T09:41:00Z"),
        }];

        let view = build(
            &it,
            &Exchanges {
                questions: Vec::new(),
                replies,
                screenshots: Vec::new(),
            },
            &connected,
            &now(),
        );
        assert_eq!(view.history.len(), 1);
        let past = &view.history[0];
        assert_eq!(past.no, 1);
        assert_eq!(past.steps.len(), 2);
        assert_eq!(past.confirmed, vec![1]);
        assert_eq!(past.skipped, vec![2]);
        assert_eq!(past.notes.len(), 1);
        assert_eq!(past.replies.len(), 1);
        assert_eq!(past.verify.as_ref().expect("a report").ok, Some(false));
        assert_eq!(
            view.step.expect("a step").replies.len(),
            0,
            "the previous round's reply stays in the history"
        );
    }

    #[test]
    fn the_secrets_list_carries_names_and_files_and_no_value() {
        let view = view(&snapshot());
        assert_eq!(view.secrets.len(), 1);
        assert_eq!(view.secrets[0].name, "STRIPE_SIGNING_SECRET");
        assert_eq!(view.secrets[0].file, ".env.local");
    }

    #[test]
    fn the_strip_is_the_store_order_with_the_waiting_group_marked() {
        let mut parked = snapshot();
        parked.id = "hf_0000000002".to_owned();
        parked.state = HandoffState::Parked;
        parked.created_at = at("2026-09-09T09:30:00Z");

        let tabs = tabs(&[snapshot(), parked], &connected, &now());
        assert_eq!(
            tabs.iter().map(|tab| tab.id.as_str()).collect::<Vec<_>>(),
            vec!["hf_0000000001", "hf_0000000002"]
        );
        assert_eq!(tabs[0].group, TabGroup::Open);
        assert_eq!(tabs[1].group, TabGroup::Waiting);
        assert_eq!(tabs[1].ui_state, UiState::Parked);
    }

    #[test]
    fn a_tab_of_the_waiting_group_carries_the_buttons_that_group_offers() {
        // SRV-23 and RESP-07 put Resume and Close it in the list itself, so the entry has
        // to say which of them it has; the rule is the store's and not the strip's.
        let mut parked = snapshot();
        parked.state = HandoffState::Parked;
        let mut orphan = snapshot();
        orphan.id = "hf_0000000002".to_owned();
        orphan.state = HandoffState::NotVerified;
        orphan.orphan = true;

        let tabs = tabs(&[parked, orphan], &connected, &now());
        assert!(tabs[0].actions.resume, "a parked handoff is resumable");
        assert!(!tabs[0].actions.close_orphan);
        assert!(tabs[1].actions.close_orphan, "SRV-23: close it by hand");
        assert!(!tabs[1].actions.resume);
    }

    #[test]
    fn a_pending_question_carries_the_words_the_user_wrote() {
        // §7.6 shows the question, not only that there is one; the store keeps the step and
        // the diary keeps the words.
        let mut it = snapshot();
        it.pending_question = Some(PendingQuestion {
            kind: PendingKind::Question,
            step: 1,
            at: at("2026-09-09T09:20:00Z"),
        });
        let exchanges = Exchanges {
            questions: vec![
                Question {
                    round: 1,
                    step: 2,
                    text: "another step".to_owned(),
                    at: at("2026-09-09T09:15:00Z"),
                },
                Question {
                    round: 1,
                    step: 1,
                    text: "which button?".to_owned(),
                    at: at("2026-09-09T09:20:00Z"),
                },
            ],
            replies: Vec::new(),
            screenshots: Vec::new(),
        };

        let view = build(&it, &exchanges, &connected, &now());
        let pending = view.pending.expect("a pending question");
        assert_eq!(pending.kind, "question");
        assert_eq!(pending.step, 1);
        assert_eq!(pending.text.as_deref(), Some("which button?"));
        assert_eq!(
            view.step.expect("a step").questions.len(),
            1,
            "the step shows what was asked on it and nothing else"
        );
    }

    #[test]
    fn a_pending_screenshot_carries_the_summary_of_what_was_sent() {
        // §7.6 asks for "the question **or screenshot summary**". There are no pixels to
        // draw anywhere (LOG-03), so the summary is which button was pressed, how big what
        // left was, and the comment that went with it.
        let mut it = snapshot();
        it.pending_question = Some(PendingQuestion {
            kind: PendingKind::Screenshot,
            step: 1,
            at: at("2026-09-09T09:20:00Z"),
        });
        let exchanges = Exchanges {
            questions: Vec::new(),
            replies: Vec::new(),
            screenshots: vec![crate::store::Screenshot {
                round: 1,
                step: 1,
                mode: crate::format::outcome::ScreenshotMode::Image,
                text: Some("the button is not where the step says".to_owned()),
                width: Some(1200),
                height: Some(660),
                at: at("2026-09-09T09:20:00Z"),
            }],
        };

        let pending = build(&it, &exchanges, &connected, &now())
            .pending
            .expect("a pending screenshot");
        assert_eq!(pending.kind, "screenshot");
        assert_eq!(
            pending.text.as_deref(),
            Some("the button is not where the step says")
        );
        let shot = pending.screenshot.expect("a summary");
        assert_eq!(shot.mode, "image");
        assert_eq!((shot.width, shot.height), (Some(1200), Some(660)));
    }

    #[test]
    fn a_pending_screenshot_of_an_earlier_step_is_not_the_one_being_waited_on() {
        // The summary names the send this step is waiting for, not the last one of the run.
        let mut it = snapshot();
        it.pending_question = Some(PendingQuestion {
            kind: PendingKind::Screenshot,
            step: 2,
            at: at("2026-09-09T09:20:00Z"),
        });
        let exchanges = Exchanges {
            questions: Vec::new(),
            replies: Vec::new(),
            screenshots: vec![crate::store::Screenshot {
                round: 1,
                step: 1,
                mode: crate::format::outcome::ScreenshotMode::Text,
                text: Some("an older one".to_owned()),
                width: Some(800),
                height: Some(600),
                at: at("2026-09-09T09:10:00Z"),
            }],
        };

        let pending = build(&it, &exchanges, &connected, &now())
            .pending
            .expect("a pending screenshot");
        assert_eq!(pending.text, None);
        let shot = pending.screenshot.expect("a summary");
        assert_eq!((shot.width, shot.height), (None, None));
    }

    #[test]
    fn a_pending_question_carries_no_screenshot_summary() {
        let mut it = snapshot();
        it.pending_question = Some(PendingQuestion {
            kind: PendingKind::Question,
            step: 1,
            at: at("2026-09-09T09:20:00Z"),
        });
        assert!(view(&it)
            .pending
            .expect("a pending question")
            .screenshot
            .is_none());
    }

    #[test]
    fn a_history_round_says_whether_it_was_a_correction_and_whether_it_failed() {
        // VER-09 asks the History to carry the markers; §8.1 opens a second round only from
        // a failed verification, so the round number and the report say the same thing twice
        // and the window can draw either.
        let mut it = snapshot();
        it.round = 2;
        it.history = vec![
            RoundSummary {
                no: 1,
                steps: vec![step("open the dashboard")],
                confirmed: vec![1],
                skipped: Vec::new(),
                notes: Vec::new(),
                verify: Some(VerifyReport {
                    ok: Some(false),
                    detail: Some("404".to_owned()),
                    reported_at: "2026-09-09T09:40:00.000Z".to_owned(),
                    late: false,
                }),
            },
            RoundSummary {
                no: 2,
                steps: vec![step("try the other endpoint")],
                confirmed: vec![1],
                skipped: Vec::new(),
                notes: Vec::new(),
                verify: None,
            },
        ];

        let history = build(
            &it,
            &Exchanges {
                questions: vec![Question {
                    round: 1,
                    step: 1,
                    text: "is this the right page?".to_owned(),
                    at: at("2026-09-09T09:35:00Z"),
                }],
                replies: Vec::new(),
                screenshots: Vec::new(),
            },
            &connected,
            &now(),
        )
        .history;

        assert!(!history[0].correction, "the first round is the first pass");
        assert!(history[0].failed);
        assert_eq!(history[0].questions.len(), 1);
        assert!(history[1].correction);
        assert!(!history[1].failed, "a round with no report has not failed");
        assert!(history[1].questions.is_empty());
    }
}
