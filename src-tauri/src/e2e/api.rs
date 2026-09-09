//! What the automation channel offers, and how each method is served (DD-33, §11.5).
//!
//! Six methods, and one rule behind all of them: **the harness presses the buttons the user
//! presses, through the code the window calls.** Every method below is a thin adapter over
//! [`crate::ui_bridge::commands`] — the same functions `invoke_handler` registers, called
//! with the same managed state — so a scenario that passes here has exercised the path a
//! person exercises, and no second implementation of "confirm a step" can drift away from
//! the first one.
//!
//! | Method | Params | Answers |
//! |---|---|---|
//! | `auth` | `{ token }` | `{ app_version }`; every other method is refused before it |
//! | `state` | — | the store snapshot, the sessions and the request queue |
//! | `act` | `{ handoff_id, action, payload? }` | `{}` |
//! | `open_request` | `{ text, session_ref? }` | `{ id }` |
//! | `settings` | `{ op, key, value? }` | `{ value }` |
//! | `quit` | — | `{}`, then the app leaves through `RunEvent::ExitRequested` |
//!
//! Everything `state` answers with is spelled the way the **window** is given it —
//! `camelCase`, because that is what `ui_bridge::view` serialises and a payload that mixed
//! the two spellings would make a scenario guess which half it was reading.
//!
//! `state` answers with what the window is given, not with the store's own record: the
//! true `values` of a spec never cross this socket, exactly as they never cross into the
//! webview (DET-04, PRIN-03). A harness that needs a value knows it already — it wrote the
//! spec into the prompt — and the log-invariant check of §11.2 is worth nothing if the
//! channel it reads through hands the values out.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager as _};

use crate::log::Timestamp;
use crate::ui_bridge::commands::{self, Action, CoreState};
use crate::ui_bridge::view::HandoffView;
use crate::ui_bridge::Ui;

/// The settings key that shortens the verification window of VER-06 for one run (E2E-10).
///
/// It is not written to the settings table: thirty minutes is a product constant (§4.1) and
/// a persisted override would outlive the run that asked for it. The value goes straight to
/// the store actor, which re-reads its deadline after every command, so the injection takes
/// effect on the next transition and dies with the process.
pub const VERIFYING_TIMEOUT_KEY: &str = "e2e.verifying_timeout_ms";

/// The action name that stands in for the screenshot pipeline until it exists.
// TASK: T-049 — the preview and the redaction burn-in land there, and E2E-3 with them.
pub const SCREENSHOT_FIXTURE: &str = "screenshot_fixture";

/// JSON-RPC error codes. The three below `-32000` are ours; the rest are the standard ones.
pub mod codes {
    /// The line was not a JSON-RPC request this channel understands.
    pub const INVALID_REQUEST: i32 = -32600;
    /// No method of that name.
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// The params did not have the shape the method takes.
    pub const INVALID_PARAMS: i32 = -32602;
    /// A method was called before `auth`, or `auth` was given the wrong token.
    pub const UNAUTHENTICATED: i32 = -32000;
    /// The store refused the action (§7.4 `Refusal`).
    pub const REFUSED: i32 = -32001;
    /// The channel never came up, so there is no store to drive (`lib.rs`).
    pub const NO_CORE: i32 = -32002;
    /// The action exists in the vocabulary and its implementation is a later task.
    pub const NOT_YET_IMPLEMENTED: i32 = -32003;
}

/// One request from the harness.
#[derive(Debug, Deserialize)]
pub struct Request {
    /// The id to answer with. Absent is a notification, which this channel does not take.
    pub id: Option<Value>,
    /// The method name.
    pub method: String,
    /// Its params, always an object when there are any.
    #[serde(default)]
    pub params: Value,
}

/// What a method produced.
pub type MethodResult = Result<Value, Failure>;

/// A refusal, as the harness reads it.
#[derive(Debug)]
pub struct Failure {
    /// One of [`codes`].
    pub code: i32,
    /// One line, in English, saying what went wrong.
    pub message: String,
}

impl Failure {
    /// A failure with a code and a message.
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The answer to a method called before `auth` succeeded.
    #[must_use]
    pub fn unauthenticated() -> Self {
        Self::new(
            codes::UNAUTHENTICATED,
            "the automation channel needs auth with the token of <HANDOFF_HOME>/e2e.token",
        )
    }

    /// The JSON-RPC error object.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({ "code": self.code, "message": self.message })
    }
}

/// `act` params: which handoff, which button, and the text of a sheet if it has one.
#[derive(Debug, Deserialize)]
pub struct ActParams {
    /// The `hf_` id.
    pub handoff_id: String,
    /// The action name, in the vocabulary of [`Action`] plus [`SCREENSHOT_FIXTURE`].
    pub action: String,
    /// The note, question or reason, or the request id of a relink.
    #[serde(default)]
    pub payload: Option<String>,
}

/// `open_request` params: what the user typed, and which session they addressed it to.
#[derive(Debug, Deserialize)]
pub struct OpenRequestParams {
    /// The sentence the request sheet would have carried (OPEN-04).
    pub text: String,
    /// The session it is for, or nothing to queue it for the first one (OPEN-04a).
    #[serde(default)]
    pub session_ref: Option<String>,
}

/// `settings` params.
#[derive(Debug, Deserialize)]
pub struct SettingsParams {
    /// `get` or `set`.
    pub op: String,
    /// The settings key.
    pub key: String,
    /// The value, for a `set`.
    #[serde(default)]
    pub value: Value,
}

/// One session, as a scenario reads it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    /// `ses_` + 8 characters.
    pub session_ref: String,
    /// The capability-table key the server resolved (§5.6).
    pub agent_id: Option<String>,
    /// The MCP client's own name.
    pub client_name: Option<String>,
    /// Whether the connection is live (§8.3).
    pub connected: bool,
    /// The server's working directory.
    pub cwd: String,
    /// `CLAUDE_PROJECT_DIR`, when the agent set one.
    pub project_dir: Option<String>,
    /// The agent's own session id, once a hook proved this is the session (§7.5).
    pub claude_session_id: Option<String>,
    /// When it registered.
    pub first_seen: Timestamp,
    /// Its last sign of life.
    pub last_seen: Timestamp,
}

/// One entry of the request queue, as a scenario reads it (§7.7).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestState {
    /// The `hf_` id the handoff adopts (DD-13).
    pub id: String,
    /// What the user wrote.
    pub text: String,
    /// The session it is addressed to, or nothing while it waits for one.
    pub session_ref: Option<String>,
    /// How it reached the agent, once it did.
    pub delivered_via: Option<String>,
    /// The handoff that answered it (OPEN-08, FM-31).
    pub linked_handoff_id: Option<String>,
    /// The handoff a resume request is about, which is what tells the two kinds apart.
    pub about_handoff_id: Option<String>,
    /// When it was typed.
    pub created_at: Timestamp,
}

/// One handoff: the whole view the window is given, and the two counters it drops.
///
/// The view is what a scenario should assert on wherever it can — it is what a person would
/// be looking at — but §11.5 asks two questions the window has no reason to draw: E2E-4's
/// "deferral_count 1" and E2E-6's "two rounds". Both are on the store's snapshot, so they
/// travel beside the view rather than through a second method.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffState {
    /// The whole `HandoffView`, flattened so a scenario reads `handoff.state` and not
    /// `handoff.view.state`.
    #[serde(flatten)]
    pub view: HandoffView,
    /// The current round (VER-09).
    pub round: u32,
    /// 0, 1 or 2 (RESP-05, RESP-06).
    pub deferral_count: u32,
}

/// Everything `state` answers with.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    /// Every handoff the window would draw, in the store's order, with the whole view.
    pub handoffs: Vec<HandoffState>,
    /// The session registry (§7.5).
    pub sessions: Vec<SessionState>,
    /// The user-request queue (§7.7), open entries and closed ones alike.
    pub requests: Vec<RequestState>,
}

/// Serves one authenticated method.
///
/// `auth` is not here: it is the connection's own business and [`super::server`] answers it
/// before anything reaches this function.
pub async fn serve(app: &AppHandle, request: &Request) -> MethodResult {
    match request.method.as_str() {
        "state" => state(app).await,
        "act" => act(app, parse(&request.params)?).await,
        "open_request" => open_request(app, parse(&request.params)?).await,
        "settings" => settings(app, parse(&request.params)?).await,
        "quit" => {
            // The answer goes out before the exit: `AppHandle::exit` unwinds the event loop
            // and the process is gone a moment later, so a harness waiting for a reply that
            // was never written would read an EOF and call it a crash.
            Ok(json!({}))
        }
        "auth" => Err(Failure::new(
            codes::INVALID_REQUEST,
            "this connection is already authenticated",
        )),
        other => Err(Failure::new(
            codes::METHOD_NOT_FOUND,
            format!("no method named {other}"),
        )),
    }
}

/// Whether a method ends the process once its answer has been written.
#[must_use]
pub fn is_quit(method: &str) -> bool {
    method == "quit"
}

/// Params of the shape a method takes, or [`codes::INVALID_PARAMS`].
fn parse<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, Failure> {
    serde_json::from_value(params.clone())
        .map_err(|error| Failure::new(codes::INVALID_PARAMS, error.to_string()))
}

/// The core, or the failure that says the channel never came up.
fn core(app: &AppHandle) -> Result<commands::Core, Failure> {
    app.state::<CoreState>().inner().0.clone().ok_or_else(|| {
        Failure::new(
            codes::NO_CORE,
            "the channel could not start, so there is no store to drive",
        )
    })
}

/// `state`: the strip, every tab in full, the sessions and the queue.
async fn state(app: &AppHandle) -> MethodResult {
    let core = core(app)?;
    let tabs = commands::list_handoffs(app.clone(), app.state::<CoreState>())
        .await
        .map_err(|error| Failure::new(codes::REFUSED, error))?;

    let mut handoffs = Vec::with_capacity(tabs.len());
    for tab in &tabs {
        let view = commands::get_handoff_view(app.state::<CoreState>(), tab.id.clone())
            .await
            .map_err(|error| Failure::new(codes::REFUSED, error))?;
        let snapshot = core
            .store
            .snapshot(tab.id.clone(), crate::log::Timestamp::now())
            .await;
        // A tab that vanished between the reads is not an error: the strip is a snapshot and
        // the store moves on. Reporting the rest is the useful answer.
        if let (Some(view), Some(snapshot)) = (view, snapshot) {
            handoffs.push(HandoffState {
                view,
                round: snapshot.round,
                deferral_count: snapshot.deferral_count,
            });
        }
    }

    let sessions = {
        let registry = core
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        registry
            .sessions()
            .map(|session| SessionState {
                session_ref: session.session_ref.clone(),
                agent_id: session.agent_id.clone(),
                client_name: session.client.as_ref().map(|client| client.name.clone()),
                connected: session.connected,
                cwd: session.cwd.clone(),
                project_dir: session.project_dir.clone(),
                claude_session_id: session.claude_session_id.clone(),
                first_seen: session.first_seen.clone(),
                last_seen: session.last_seen.clone(),
            })
            .collect::<Vec<_>>()
    };

    let requests = app
        .state::<Ui>()
        .with_db(|db| crate::log::user_requests::list_open(db, None))
        .transpose()
        .map_err(|error| Failure::new(codes::REFUSED, error.to_string()))?
        .unwrap_or_default()
        .into_iter()
        .map(|row| RequestState {
            id: row.id,
            text: row.text,
            session_ref: row.session_ref,
            delivered_via: row.delivered_via.map(|via| via.as_str().to_owned()),
            linked_handoff_id: row.linked_handoff_id,
            about_handoff_id: row.about_handoff_id,
            created_at: row.created_at,
        })
        .collect();

    serde_json::to_value(AppState {
        handoffs,
        sessions,
        requests,
    })
    .map_err(|error| Failure::new(codes::REFUSED, error.to_string()))
}

/// `act`: one press of one button, through [`commands::act`].
async fn act(app: &AppHandle, params: ActParams) -> MethodResult {
    if params.action == SCREENSHOT_FIXTURE {
        // The capture backend, the OCR, the detector and the preview are T-046 to T-049;
        // the store already takes a `Screenshot` action and nothing here can build one
        // honestly. E2E-3 is added with the pipeline (T-043 Notes), and this answer is what
        // a scenario written early meets instead of a silent success.
        return Err(Failure::new(
            codes::NOT_YET_IMPLEMENTED,
            "screenshot_fixture arrives with the capture pipeline (T-049)",
        ));
    }

    let action: Action =
        serde_json::from_value(Value::String(params.action.clone())).map_err(|_| {
            Failure::new(
                codes::INVALID_PARAMS,
                format!("no action named {}", params.action),
            )
        })?;

    commands::act(
        app.clone(),
        app.state::<CoreState>(),
        params.handoff_id,
        action,
        params.payload,
    )
    .await
    .map(|()| json!({}))
    .map_err(|error| Failure::new(codes::REFUSED, error))
}

/// `open_request`: the request sheet of §7.7, without the sheet.
async fn open_request(app: &AppHandle, params: OpenRequestParams) -> MethodResult {
    commands::create_request(
        app.clone(),
        app.state::<CoreState>(),
        params.text,
        params.session_ref,
    )
    .await
    .map(|id| json!({ "id": id }))
    .map_err(|error| Failure::new(codes::REFUSED, error))
}

/// `settings`: the settings table, plus the one injected value of [`VERIFYING_TIMEOUT_KEY`].
async fn settings(app: &AppHandle, params: SettingsParams) -> MethodResult {
    if params.key == VERIFYING_TIMEOUT_KEY {
        return match params.op.as_str() {
            "set" => {
                let ms = params.value.as_i64().ok_or_else(|| {
                    Failure::new(
                        codes::INVALID_PARAMS,
                        "the verification window is a number of milliseconds",
                    )
                })?;
                core(app)?.store.set_verifying_timeout_ms(ms).await;
                Ok(json!({ "value": ms }))
            }
            "get" => Ok(json!({ "value": core(app)?.store.verifying_timeout_ms().await })),
            other => Err(Failure::new(
                codes::INVALID_PARAMS,
                format!("no settings op named {other}"),
            )),
        };
    }

    let ui = app.state::<Ui>();
    match params.op.as_str() {
        "get" => ui
            .with_db(|db| crate::log::settings::get::<Value>(db, &params.key))
            .transpose()
            .map_err(|error| Failure::new(codes::REFUSED, error.to_string()))?
            .flatten()
            .map_or_else(
                || Ok(json!({ "value": Value::Null })),
                |value| Ok(json!({ "value": value })),
            ),
        "set" => ui
            .with_db(|db| crate::log::settings::set(db, &params.key, &params.value))
            .transpose()
            .map_err(|error| Failure::new(codes::REFUSED, error.to_string()))
            .map(|_| json!({ "value": params.value })),
        other => Err(Failure::new(
            codes::INVALID_PARAMS,
            format!("no settings op named {other}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_action_names_are_the_windows_own() {
        // A rename in `commands::Action` must break a scenario here rather than be
        // discovered by a harness that presses a button nobody implements.
        for name in [
            "confirm",
            "note",
            "skip",
            "ask",
            "defer",
            "abandon",
            "done",
            "resume_from_overlay",
            "close_orphan",
            "relink",
        ] {
            let parsed: Result<Action, _> = serde_json::from_value(Value::String(name.to_owned()));
            assert!(parsed.is_ok(), "{name} is not an action of the window");
        }
    }

    #[test]
    fn the_screenshot_action_is_not_one_of_them_yet() {
        let parsed: Result<Action, _> =
            serde_json::from_value(Value::String(SCREENSHOT_FIXTURE.to_owned()));
        assert!(parsed.is_err());
    }

    #[test]
    fn a_request_without_params_parses() {
        let request: Request = serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"state"}"#)
            .expect("a request");
        assert_eq!(request.method, "state");
        assert!(request.params.is_null());
        assert!(is_quit("quit"));
        assert!(!is_quit("state"));
    }

    #[test]
    fn params_of_the_wrong_shape_are_refused_with_the_standard_code() {
        let failure = parse::<ActParams>(&json!({ "action": "confirm" })).expect_err("a failure");
        assert_eq!(failure.code, codes::INVALID_PARAMS);
    }
}
