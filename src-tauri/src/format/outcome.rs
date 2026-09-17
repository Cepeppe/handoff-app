//! The outcome, version 1 (§4.3, `schemas/handoff-outcome.v1.schema.json`).
//!
//! What `handoff_to_user` and `handoff_verify` return to the agent, what the log stores
//! (LOG-02) and what the runbook writer reads (TOOL-12). The app builds it (T-033) and
//! sends it across the channel; the server passes it through.
//!
//! **Every field is present**, with `null` or `[]` when it does not apply, so an agent
//! never branches on absence (TOOL-13, DD-08). That is why no field of [`Outcome`] carries
//! `skip_serializing_if`: an `Option` here is the schema's `null`, never an omission.
//!
//! The object carries no pixels. A screenshot the user sent as an image travels beside its
//! outcome in the channel's own `image` field ([`crate::format::channel`]), and
//! `screenshot.image_attached` is all the outcome says about it (§6.6).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::spec::{HandoffSpec, SpecValue};

/// One enumeration over the final states and the reasons a blocking call returns, so an
/// agent branches on one field (DD-15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    /// Heartbeat: the user is still working (TOOL-06).
    InProgress,
    /// The user pressed Ask (RESP-04).
    Question,
    /// The user sent a screenshot, as an image or as text.
    Screenshot,
    /// First deferral (RESP-05).
    Deferred,
    /// Second deferral: the handoff waits in the overlay (RESP-07).
    Parked,
    /// Done on the last step of a spec that carries a `verify` (RESP-09).
    AwaitingVerification,
    /// Done on the last step, no `verify`.
    ConfirmedByUser,
    /// `handoff_verify` reported `ok: true`.
    Verified,
    /// `handoff_verify` reported `ok: false` (VER-08).
    Failed,
    /// `ok: null`, timeout or disconnect (VER-06).
    NotVerified,
    /// The user pressed Abandon (RESP-08).
    Abandoned,
    /// Another session resumed the handoff (TOOL-08).
    TransferredToOtherSession,
    /// A new spec matched one or more runbooks (RUN-07).
    RunbookMatch,
    /// The app is unreachable and the handoff happens in the chat (SRV-14).
    TextMode,
}

/// How a screenshot was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotMode {
    /// The pixels travel beside the outcome and become an MCP image block.
    Image,
    /// Only the OCR text, as the user edited it, travels.
    Text,
}

/// The step the outcome is about, as the overlay's counter shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentStep {
    /// 1-based, the same number the overlay shows.
    pub index: u32,
    /// How many steps the round has.
    pub total: u32,
    /// The step's text.
    pub text: String,
}

/// What the user sent with a screenshot, and what the app did to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotInfo {
    /// Image or text.
    pub mode: ScreenshotMode,
    /// The OCR text as edited and sent; `null` when the user sent the image.
    pub text: Option<String>,
    /// Whether an MCP image block accompanies this outcome.
    pub image_attached: bool,
    /// Pixel width of what was captured.
    pub width: u32,
    /// Pixel height of what was captured.
    pub height: u32,
    /// How many regions were burned out before anything left the machine (PRIN-09).
    pub redactions: u32,
    /// Which OCR engine ran, `null` when none did.
    pub ocr_engine: Option<String>,
}

/// The step of `context`, which carries the two fields `current_step` does not (CTX-01).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextStep {
    /// 1-based.
    pub index: u32,
    /// How many steps the round has.
    pub total: u32,
    /// The step's text.
    pub text: String,
    /// The step's own url, or `null`.
    pub url: Option<String>,
    /// The step's warning, or `null`.
    pub warning: Option<String>,
}

/// Where the user is, sent with `question` and `screenshot` so the agent answers on the
/// right step (CTX-01).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeContext {
    /// The spec's goal.
    pub goal: String,
    /// The spec's `where`.
    #[serde(rename = "where")]
    pub r#where: String,
    /// The step the user is on.
    pub step: ContextStep,
    /// The step's value names mapped to their values, with `"[treated as secret]"` in
    /// place of the secret-treated ones.
    pub step_values: IndexMap<String, SpecValue>,
}

/// A note the user attached to a step (RESP-03).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeNote {
    /// 1-based step index.
    pub step: u32,
    /// What the user wrote.
    pub text: String,
    /// When, RFC 3339.
    pub at: String,
}

/// One certain secret found at ingress: where it is, and which family it belongs to.
///
/// The matched text is never here, and `kind` is the family — `api_key`, never
/// `stripe_secret_key` (§4.6, DET-04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretTreated {
    /// Display path of the string it was found in, e.g. `values.api_key`.
    pub location: String,
    /// The family of the pattern that matched.
    pub kind: String,
}

impl SecretTreated {
    /// Whether it was found inside the top-level value `name` (DET-04, §4.5.2).
    ///
    /// The server reports a single-valued entry as `values.<name>` and an array one item at a
    /// time, as `values.<name>[<index>]` (§4.7.5). §4.5.2 calls the **value** secret-treated
    /// however many of its items matched, so both shapes answer yes, and `name` has to be
    /// followed by the end of the location or by `[` so that `api` never matches `api_key`.
    #[must_use]
    pub fn is_in_value(&self, name: &str) -> bool {
        self.location
            .strip_prefix("values.")
            .and_then(|path| path.strip_prefix(name))
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('['))
    }
}

/// The verification report, once one exists (VER-05).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyReport {
    /// `null` means the agent could not verify.
    pub ok: Option<bool>,
    /// What the agent found.
    pub detail: Option<String>,
    /// When it reported, RFC 3339.
    pub reported_at: String,
    /// True when the report arrived after the handoff had become `not_verified` (DD-16).
    pub late: bool,
}

/// The opening session, when the current call comes from a different one (TOOL-08).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumedFrom {
    /// The agent that opened the handoff.
    pub agent: String,
    /// Its project folder.
    pub project: String,
}

/// What happened on a step of a runbook: a note, a question and its reply, a failed
/// verification, the correction that followed (§4.5.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Annotation {
    /// Which of the five kinds this is.
    pub kind: AnnotationKind,
    /// The text.
    pub text: String,
    /// The round it belongs to.
    pub round: u32,
}

/// The five annotation kinds of §4.5.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    /// Something the user wrote on the step.
    Note,
    /// Something the user asked.
    Question,
    /// What the agent answered.
    Reply,
    /// A failed verification's detail.
    Error,
    /// The first step of the round that followed a failure.
    Correction,
}

/// How much a runbook is trusted (RUN-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunbookTrust {
    /// An agent reported `ok: true`.
    Verified,
    /// The spec had no `verify` and the user said it was done.
    ConfirmedByUser,
}

/// One matching runbook with the draft spec derived from it (§4.5.4, RUN-07).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunbookMatch {
    /// The runbook's id.
    pub id: String,
    /// Absolute path of the file in `~/.handoff/runbooks/`.
    pub path: String,
    /// The runbook's `where`.
    #[serde(rename = "where")]
    pub r#where: String,
    /// The runbook's goal.
    pub goal: String,
    /// How much it is trusted.
    pub trust: RunbookTrust,
    /// When it was last verified, RFC 3339.
    pub last_verified_at: String,
    /// When a run from it last failed, or `null` (RUN-09).
    pub last_run_failed_at: Option<String>,
    /// How many verified or confirmed executions are folded into it.
    pub runs: u32,
    /// The shared goal tokens that produced the match, so the agent can see why.
    pub matched_words: Vec<String>,
    /// The runbook converted to a spec, intentionally invalid until the agent fills it.
    pub draft_spec: HandoffSpec,
    /// Value name to its description, or `null` when the value appeared in no step.
    pub values_to_fill: IndexMap<String, Option<String>>,
    /// The annotations of the runbook, returned beside the draft rather than inside it.
    pub annotations: Vec<Annotation>,
}

/// The outcome returned to the agent (§4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    /// `1` in this version.
    pub outcome_version: i64,
    /// `null` only for `runbook_match` and `text_mode`, where no handoff was opened.
    pub handoff_id: Option<String>,
    /// Why this outcome came back.
    pub status: OutcomeStatus,
    /// True for `verified`, `confirmed_by_user`, `failed`, `not_verified`, `abandoned`.
    ///
    /// `final` is a reserved word in Rust; the wire name is what the schema says.
    #[serde(rename = "final")]
    pub is_final: bool,
    /// What the agent should do next; the fallback that holds when hooks, raised timeouts
    /// or images do not (PRIN-10).
    pub instruction: String,
    /// The current round; a failed verification opens the next one (VER-10).
    pub round: u32,
    /// The step the user is on, or `null`.
    pub current_step: Option<CurrentStep>,
    /// The question, the comment attached to a screenshot, or the reason typed with Defer
    /// or Abandon.
    pub user_text: Option<String>,
    /// What the user sent, or `null`.
    pub screenshot: Option<ScreenshotInfo>,
    /// Where the user is, for `question` and `screenshot`.
    pub context: Option<OutcomeContext>,
    /// 1-based indices skipped in the current round (RESP-03).
    pub skipped_steps: Vec<u32>,
    /// The notes the user attached (RESP-03).
    pub notes: Vec<OutcomeNote>,
    /// Values and texts the certain detector matched at ingress (DET-04).
    pub secret_treated: Vec<SecretTreated>,
    /// The verification report, or `null` until one is made.
    pub verify: Option<VerifyReport>,
    /// 0, 1 or 2: a second deferral parks the handoff.
    pub deferral_count: u32,
    /// The opening session, when the current call comes from a different one.
    pub resumed_from: Option<ResumedFrom>,
    /// False only in `text_mode` (TOOL-13).
    pub app_reachable: bool,
    /// True when a resume returns a final outcome that was already delivered (TOOL-07).
    pub already_delivered: bool,
    /// Filled only for `runbook_match` and by `handoff_runbooks`.
    pub runbooks: Vec<RunbookMatch>,
    /// The rendered spec, only for `text_mode` (§5.9).
    pub spec_text: Option<String>,
}
