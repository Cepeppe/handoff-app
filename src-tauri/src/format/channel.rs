//! The internal channel protocol, version 1 (`protocol/channel/channel.v1.schema.json`
//! and its `README.md`, both vendored).
//!
//! One JSON-RPC 2.0 message per line, NDJSON, UTF-8. This module is the app's half of the
//! codec's vocabulary: the listener of T-031 reads and writes these types, and
//! `tests/contract/channel.rs` replays every golden of `fixtures/channel/*.jsonl` through
//! them, so the two implementations cannot drift.
//!
//! **The protocol is internal and may change in any release.** It lives in the open
//! repository because the server must be buildable from there alone, not because it is
//! published.
//!
//! # How a line is discriminated
//!
//! The schema's top-level `oneOf` separates requests, notifications, responses and error
//! responses; inside each group the `method` (or, for `hello`, the `role`) selects the
//! shape, and a response carries no method at all, so its `result` identifies it. The
//! enums below are `untagged`, and the required fields of their variants are what makes
//! exactly one of them match a line — a request has an id and a method, a notification only
//! a method, a response a result, an error response an error. That is the property
//! `tests/contract/channel.rs` checks against the goldens, message by message.
//!
//! Every payload object refuses a field it does not declare, as the schema does, with two
//! exceptions: [`Request`] and [`Notification`] carry their method and parameters through
//! `#[serde(flatten)]`, which serde does not allow to be combined with
//! `deny_unknown_fields`. That costs nothing in practice — the schema is what the listener
//! validates every incoming line against, and closure is its contract (see the vendored
//! `protocol/channel/README.md`) — but it is the reason a stray key at the envelope level
//! is caught by the validator rather than by the codec.
//!
//! # Two shapes the schema is deliberate about
//!
//! - `protocol_version` in a `hello` is any positive integer, not the constant 1: a peer
//!   speaking version 2 must parse well enough to be told to update, instead of being
//!   dropped as a framing error (§6.5).
//! - `image` is the one payload that is in no public schema. The published outcome is
//!   closed and carries no pixels, so a screenshot sent as an image travels beside its
//!   outcome, on the two messages that can carry one — `handoff.event` and the snapshot of
//!   `handoff.resume` — and nowhere else (§6.6, `DEVIATIONS.md`).

use serde::{Deserialize, Deserializer, Serialize};

use super::outcome::{Outcome, ResumedFrom, SecretTreated};
use super::spec::{HandoffSpec, HandoffStep};

/// Reads a field that is both optional and nullable, keeping the two apart.
///
/// Three of the protocol's fields are `anyOf [something, null]` **and** absent from their
/// object's `required` list, so `null` and "not there" are two different messages on the
/// wire and the goldens carry both. Plain `Option<Option<T>>` cannot tell them apart —
/// serde reads a JSON `null` into the outer `None`, which then serialises as an omission
/// and the message that comes back out is not the one that went in. With `default` for the
/// absent case and this for the present one, `None` means absent and `Some(None)` means
/// `null`.
fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// The `"2.0"` of every JSON-RPC message, as a type that can hold nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum JsonRpcVersion {
    /// The only accepted value.
    #[serde(rename = "2.0")]
    #[default]
    V2,
}

/// A JSON-RPC id. Both peers emit positive integers, increasing per connection; strings
/// are accepted for conformance with the specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    /// What both peers actually send.
    Number(u64),
    /// Accepted, never produced.
    Text(String),
}

/// One entry of the ancestor chain a peer walked (§5.8, DD-22).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AncestorProcess {
    /// The process id.
    pub pid: u32,
    /// Its executable name.
    pub name: String,
}

/// Who is connecting.
///
/// `project_dir` is required of a server and absent from a hook; the schema keeps it
/// optional here and requires it in `identity_server`, so one type covers both and the
/// schema is what enforces the difference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    /// The peer's own process id.
    pub pid: u32,
    /// Its parent's, `0` when it has none to report.
    pub ppid: u32,
    /// Best effort: empty on Windows, where the app completes the chain itself.
    pub ancestors: Vec<AncestorProcess>,
    /// The peer's working directory.
    pub cwd: String,
    /// The project folder, for a server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
}

/// `clientInfo` from the MCP initialize handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientInfo {
    /// The agent's own name, e.g. `claude-code`.
    pub name: String,
    /// Its version.
    pub version: String,
}

/// How well the system works with this agent (§5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    /// Everything the design promises.
    Full,
    /// Blocking calls and text; no hooks or no images.
    Base,
    /// The agent cannot carry the system.
    Unsupported,
}

/// How a user-opened request reaches the agent (§7.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRequestDelivery {
    /// The text is put on the clipboard and the terminal is focused.
    ClipboardFocus,
    /// The Stop hook blocks once and tells the agent.
    StopHook,
}

/// The capability row the server already resolved for this session (§5.6).
///
/// The app adapts its UI to it — hiding "Send image" when `images_in_results` is false —
/// and owns no agent facts of its own (ADPT-03). The five required fields are the ones the
/// design's example sends; the optional ones are the remaining columns of the table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRow {
    /// The capability-table key, e.g. `claude-code` or `unknown`.
    pub agent_id: String,
    /// How well the system works with it.
    pub support: SupportLevel,
    /// Whether an image block in a tool result reaches the model.
    pub images_in_results: bool,
    /// Whether the agent runs a Stop hook.
    pub stop_hook: bool,
    /// The resolved timeout for this session, `null` when nothing is known and the
    /// heartbeat falls back to 50 s (TOOL-06a).
    pub tool_timeout_ms: Option<u64>,
    /// The agent's name as a person reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Whether the agent runs a SubagentStop hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_stop_hook: Option<bool>,
    /// How a session identifies itself, when it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_identity: Option<String>,
    /// The ways a user-opened request can reach this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_request_delivery: Option<Vec<UserRequestDelivery>>,
    /// Whether a cancelled tool call is announced rather than abandoned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation_notifications: Option<bool>,
}

/// Which of the agent's two hook events fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookEventName {
    /// The agent is about to stop.
    Stop,
    /// A subagent is about to stop.
    SubagentStop,
}

/// The payload the agent wrote on the hook process's stdin (§5.11).
///
/// `agent_id` and `agent_type` are the agent's own `SubagentStop` fields and have nothing
/// to do with the capability table's `agent_id`. `transcript_path`, which the agent also
/// writes, is deliberately not forwarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookInput {
    /// The agent's session identifier.
    pub session_id: String,
    /// Which event fired.
    pub hook_event_name: HookEventName,
    /// The agent's own guard against a hook loop.
    pub stop_hook_active: bool,
    /// The subagent's id, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// The subagent's type, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

/// The `role` of a server's `hello`. One variant, because the schema writes it as a
/// constant and it is half of what discriminates the two shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ServerRole {
    /// A server, which registers a session and then carries handoff traffic.
    #[serde(rename = "server")]
    #[default]
    Server,
}

/// The `role` of a hook's `hello`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HookRole {
    /// A hook, which is answered once and closed.
    #[serde(rename = "hook")]
    #[default]
    Hook,
}

/// `hello` from a server (§6.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloServerParams {
    /// The version the peer claims to speak; any positive integer parses (§6.5).
    pub protocol_version: u32,
    /// The per-installation token, 64 lowercase hex characters.
    pub token: String,
    /// Always [`ServerRole::Server`].
    pub role: ServerRole,
    /// The server's own version.
    pub server_version: String,
    /// Who is connecting; a server always knows its project folder.
    pub identity: Identity,
    /// The capability-table key resolved for this session.
    pub agent_id: String,
    /// `clientInfo` from the MCP handshake.
    pub client: ClientInfo,
    /// The row the server resolved.
    pub capability_row: CapabilityRow,
}

/// `hello` from a hook (§6.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloHookParams {
    /// The version the peer claims to speak.
    pub protocol_version: u32,
    /// The per-installation token.
    pub token: String,
    /// Always [`HookRole::Hook`].
    pub role: HookRole,
    /// The server's version, when the hook knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    /// Who is connecting; a hook sends no project folder.
    pub identity: Identity,
    /// What the agent wrote on its stdin.
    pub hook: HookInput,
}

/// Either shape of a `hello`, discriminated by `role`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HelloParams {
    /// A server registering a session.
    Server(Box<HelloServerParams>),
    /// A hook asking one question.
    Hook(Box<HelloHookParams>),
}

/// The answer to a `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloResult {
    /// The app's version.
    pub app_version: String,
    /// The version this app speaks; always the current one.
    pub protocol_version: u32,
    /// Assigned when a server registers, `null` for a hook (§6.2).
    pub session_ref: Option<String>,
}

/// `handoff.open`: the agent opened a handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffOpenParams {
    /// The call that will wait on it.
    pub call_id: String,
    /// The spec, exactly as the agent sent it (DET-04).
    pub spec: HandoffSpec,
    /// What the certain detector found at ingress.
    pub secret_treated: Vec<SecretTreated>,
    /// The id of a user-opened request; the handoff then takes that id (DD-13).
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub request_id: Option<Option<String>>,
}

/// The answer to `handoff.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffOpenResult {
    /// The id the app assigned, or the request's id it took over.
    pub handoff_id: String,
    /// The opening session, when this call comes from another one.
    pub resumed_from: Option<ResumedFrom>,
}

/// `handoff.continue`: the agent answered a question (TOOL-04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffContinueParams {
    /// The call that waits on the handoff.
    pub call_id: String,
    /// Which handoff.
    pub handoff_id: String,
    /// What the agent replied.
    pub reply: String,
    /// New steps for the rest of the round, when the remaining ones must change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_steps: Option<Vec<HandoffStep>>,
}

/// The answer to `handoff.continue`: nothing but an acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffContinueResult {
    /// Always `true`.
    pub ok: bool,
}

/// `handoff.resume`: a call re-attaches to a handoff (TOOL-07).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffResumeParams {
    /// The call that attaches.
    pub call_id: String,
    /// Which handoff.
    pub handoff_id: String,
    /// Optional and redundant: the connection already identifies the session, so the app
    /// may ignore it (`DEVIATIONS.md`, T-007).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_ref: Option<String>,
}

/// The states of the app-side handoff lifecycle (§8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffState {
    /// A user-opened request with no spec yet.
    AwaitingSpec,
    /// Being worked on.
    Active,
    /// Deferred once.
    Deferred,
    /// Deferred twice; it waits in the overlay.
    Parked,
    /// Done, waiting for the agent's verification.
    AwaitingVerification,
    /// The agent reported ok.
    Verified,
    /// Done without a verification to make.
    ConfirmedByUser,
    /// The agent reported a failure.
    Failed,
    /// No verification arrived in time.
    NotVerified,
    /// The user abandoned it.
    Abandoned,
}

/// The snapshot of §5.7: a final outcome (returned with `already_delivered`), a queued
/// undelivered event, or nothing, in which case the call attaches and waits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffResumeResult {
    /// Where the handoff stands.
    pub state: HandoffState,
    /// The outcome to deliver, when there is one.
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub outcome: Option<Option<Outcome>>,
    /// The pixels of a screenshot queued while no call was attached (§6.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// The opening session, when this call comes from another one.
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub resumed_from: Option<Option<ResumedFrom>>,
}

/// What the agent reports about its own verification (§4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyInput {
    /// `null` means the agent could not verify.
    pub ok: Option<bool>,
    /// What it found.
    pub detail: Option<String>,
}

/// `handoff.verify`: the agent reports a verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffVerifyParams {
    /// Which handoff.
    pub handoff_id: String,
    /// The report.
    pub verify: VerifyInput,
}

/// The answer to `handoff.verify`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffVerifyResult {
    /// The outcome the agent gets back.
    pub outcome: Outcome,
}

/// Why a call stopped waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetachReason {
    /// It returned an `in_progress` outcome before the agent's timeout (TOOL-06).
    Heartbeat,
    /// The agent cancelled it.
    Cancelled,
}

/// `handoff.detach_call`: a call stopped waiting, so a tab can say so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffDetachCallParams {
    /// Which handoff.
    pub handoff_id: String,
    /// Which call.
    pub call_id: String,
    /// Why.
    pub reason: DetachReason,
}

/// `handoff.event`: the app delivers an outcome to the call that is waiting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffEventParams {
    /// The call to resolve.
    pub call_id: String,
    /// Which handoff.
    pub handoff_id: String,
    /// What happened.
    pub outcome: Outcome,
    /// The pixels, when the user sent an image (§6.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// `app.shutdown`: the app is going away.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppShutdownParams {
    /// Why, in a sentence the server can log.
    pub reason: String,
}

/// The answer to `hook.stop` (§7.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookStopResult {
    /// Whether the agent must be blocked once.
    pub block: bool,
    /// Present only when `block` is true: the text the hook prints for the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A method that takes no parameters. The schema still requires `params`, as `{}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

/// A result that carries nothing.
pub type EmptyResult = NoParams;

/// Every request of the protocol, with the method that names it.
///
/// Adjacently tagged: the discriminator is the sibling field `method` and the payload is
/// the sibling field `params`, which is exactly the JSON-RPC shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum RequestBody {
    /// The first message of every connection.
    #[serde(rename = "hello")]
    Hello(HelloParams),
    /// The agent opened a handoff.
    #[serde(rename = "handoff.open")]
    HandoffOpen(Box<HandoffOpenParams>),
    /// The agent answered a question.
    #[serde(rename = "handoff.continue")]
    HandoffContinue(Box<HandoffContinueParams>),
    /// A call re-attaches to a handoff.
    #[serde(rename = "handoff.resume")]
    HandoffResume(HandoffResumeParams),
    /// The agent reports a verification.
    #[serde(rename = "handoff.verify")]
    HandoffVerify(HandoffVerifyParams),
    /// The hook asks whether to block.
    #[serde(rename = "hook.stop")]
    HookStop(NoParams),
    /// Liveness, after 30 s of silence.
    #[serde(rename = "ping")]
    Ping(NoParams),
}

/// Every notification of the protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum NotificationBody {
    /// The server is leaving.
    #[serde(rename = "session.bye")]
    SessionBye(NoParams),
    /// A call stopped waiting.
    #[serde(rename = "handoff.detach_call")]
    HandoffDetachCall(HandoffDetachCallParams),
    /// An outcome for the call that waits.
    #[serde(rename = "handoff.event")]
    HandoffEvent(Box<HandoffEventParams>),
    /// The app is going away.
    #[serde(rename = "app.shutdown")]
    AppShutdown(AppShutdownParams),
}

/// Every result of the protocol.
///
/// A response carries no method, so the shape of its `result` identifies it. The seven
/// results are mutually exclusive by their required fields, and `{}` — the answer to a
/// `ping` — matches only [`ResultBody::Empty`], which is why that variant comes last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResultBody {
    /// The answer to `hello`.
    Hello(HelloResult),
    /// The answer to `handoff.open`.
    HandoffOpen(HandoffOpenResult),
    /// The answer to `handoff.continue`.
    HandoffContinue(HandoffContinueResult),
    /// The answer to `handoff.resume`.
    HandoffResume(Box<HandoffResumeResult>),
    /// The answer to `handoff.verify`.
    HandoffVerify(Box<HandoffVerifyResult>),
    /// The answer to `hook.stop`.
    HookStop(HookStopResult),
    /// The answer to `ping`, and to nothing else.
    Empty(EmptyResult),
}

/// The five application errors of §6.3 and the two the design fixes, with the wire codes
/// `protocol/channel/README.md` assigns them.
///
/// The name travels in `message` so the pair is checkable; a malformed or unknown message
/// is **not** answered with an error object, the connection closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelErrorCode {
    /// The token does not match.
    AuthFailed,
    /// The versions differ; `data.protocol_version` is the app's.
    ProtocolUnsupported,
    /// A continue names a value the spec does not declare.
    UnknownValueKey,
    /// A reply with no pending question and no `replacement_steps`.
    NotWaiting,
    /// `replacement_steps` on a handoff that is closed.
    Final,
    /// `handoff.verify` on a spec without `verify`.
    NoVerifyInSpec,
    /// Unknown `handoff_id`.
    NotFound,
}

impl ChannelErrorCode {
    /// The numeric code that travels on the wire.
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            Self::AuthFailed => -32001,
            Self::ProtocolUnsupported => -32002,
            Self::UnknownValueKey => -32010,
            Self::NotWaiting => -32011,
            Self::Final => -32012,
            Self::NoVerifyInSpec => -32013,
            Self::NotFound => -32014,
        }
    }

    /// The name that travels in `message`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::AuthFailed => "auth_failed",
            Self::ProtocolUnsupported => "protocol_unsupported",
            Self::UnknownValueKey => "unknown_value_key",
            Self::NotWaiting => "not_waiting",
            Self::Final => "final",
            Self::NoVerifyInSpec => "no_verify_in_spec",
            Self::NotFound => "not_found",
        }
    }

    /// Every code, in the order the README's table lists them.
    #[must_use]
    pub fn all() -> [Self; 7] {
        [
            Self::AuthFailed,
            Self::ProtocolUnsupported,
            Self::UnknownValueKey,
            Self::NotWaiting,
            Self::Final,
            Self::NoVerifyInSpec,
            Self::NotFound,
        ]
    }
}

/// The version the app speaks, sent with `protocol_unsupported` so the peer can log what
/// it should update to (§6.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolUnsupportedData {
    /// Always the current version.
    pub protocol_version: u32,
}

/// The value names a continue cited that the spec does not declare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnknownValueKeyData {
    /// At least one name.
    pub keys: Vec<String>,
}

/// The `error` object of a JSON-RPC error response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelError {
    /// The wire form of the name.
    pub code: i32,
    /// The error name of §6.3.
    pub message: String,
    /// The two codes that carry one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ChannelErrorData>,
}

/// The payload of the two errors that carry one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChannelErrorData {
    /// `protocol_unsupported`.
    ProtocolUnsupported(ProtocolUnsupportedData),
    /// `unknown_value_key`.
    UnknownValueKey(UnknownValueKeyData),
}

/// A request: `jsonrpc`, `id`, `method`, `params`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// Always `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// The id the answer will carry.
    pub id: RequestId,
    /// The method and its parameters.
    #[serde(flatten)]
    pub body: RequestBody,
}

/// A notification: a request without an id, and with no answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// Always `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// The method and its parameters.
    #[serde(flatten)]
    pub body: NotificationBody,
}

/// A successful answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    /// Always `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// The id of the request it answers.
    pub id: RequestId,
    /// The result, identified by its own shape.
    pub result: ResultBody,
}

/// A failed answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    /// Always `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// The id of the request it answers.
    pub id: RequestId,
    /// What went wrong.
    pub error: ChannelError,
}

/// One whole line of the channel.
///
/// The order of the variants is the order the schema's top-level `oneOf` lists them, and
/// it matters: a [`Request`] has an `id` and a `method`, a [`Notification`] only a
/// `method`, a [`Response`] a `result` and an [`ErrorResponse`] an `error`, so exactly one
/// variant can match a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChannelMessage {
    /// Something that expects an answer.
    Request(Box<Request>),
    /// Something that does not.
    Notification(Box<Notification>),
    /// An answer.
    Response(Box<Response>),
    /// A refusal.
    ErrorResponse(Box<ErrorResponse>),
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;

    fn open_with(request_id: Option<Value>) -> Value {
        let mut params = json!({
            "call_id": "call_2q7m8r1t",
            "spec": {
                "spec_version": 1, "goal": "g", "where": "w", "why_human": "y",
                "values": {}, "steps": [{ "text": "t" }]
            },
            "secret_treated": []
        });
        if let Some(value) = request_id {
            params["request_id"] = value;
        }
        json!({ "jsonrpc": "2.0", "id": 2, "method": "handoff.open", "params": params })
    }

    fn round_trip(line: &Value) -> Value {
        let parsed: ChannelMessage =
            serde_json::from_value(line.clone()).expect("the line deserialises");
        serde_json::to_value(&parsed).expect("it serialises")
    }

    #[test]
    fn a_null_and_an_absent_optional_field_are_two_different_messages() {
        // `handoff.open` sends `request_id: null` when the handoff is not a user request,
        // and omits it in the flows that predate one. Reading both into the same value
        // would send a different message back out than the one that came in — which is what
        // `fixtures/channel/f02-happy-path.jsonl` caught while this module was written.
        let absent = open_with(None);
        let null = open_with(Some(Value::Null));
        let named = open_with(Some(json!("hf_7k3m9p2q4r")));

        assert_eq!(round_trip(&absent), absent);
        assert_eq!(round_trip(&null), null);
        assert_eq!(round_trip(&named), named);
        assert_ne!(round_trip(&absent), null);
    }

    #[test]
    fn a_request_a_notification_a_response_and_an_error_are_told_apart() {
        let cases = [
            (
                json!({ "jsonrpc": "2.0", "id": 1, "method": "ping", "params": {} }),
                "request",
            ),
            (
                json!({ "jsonrpc": "2.0", "method": "session.bye", "params": {} }),
                "notification",
            ),
            (
                json!({ "jsonrpc": "2.0", "id": 1, "result": {} }),
                "response",
            ),
            (
                json!({ "jsonrpc": "2.0", "id": 1,
                        "error": { "code": -32001, "message": "auth_failed" } }),
                "error",
            ),
        ];
        for (line, expected) in cases {
            let parsed: ChannelMessage =
                serde_json::from_value(line.clone()).expect("the line deserialises");
            let got = match parsed {
                ChannelMessage::Request(_) => "request",
                ChannelMessage::Notification(_) => "notification",
                ChannelMessage::Response(_) => "response",
                ChannelMessage::ErrorResponse(_) => "error",
            };
            assert_eq!(got, expected, "{line} was read as a {got}");
            assert_eq!(round_trip(&line), line);
        }
    }

    #[test]
    fn an_empty_result_is_a_ping_and_nothing_else() {
        // Every other result has a required field, so `{}` can only be the answer to a
        // ping; the variant is last for that reason and this says so.
        let line = json!({ "jsonrpc": "2.0", "id": 7, "result": {} });
        let parsed: ChannelMessage = serde_json::from_value(line).expect("it deserialises");
        match parsed {
            ChannelMessage::Response(response) => {
                assert!(matches!(response.result, ResultBody::Empty(_)));
            }
            other => panic!("read as {other:?}"),
        }
    }

    #[test]
    fn a_version_other_than_two_is_not_a_channel_message() {
        let line = json!({ "jsonrpc": "1.0", "id": 1, "method": "ping", "params": {} });
        assert!(serde_json::from_value::<ChannelMessage>(line).is_err());
    }
}
