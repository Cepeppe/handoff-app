//! Everything the window may ask of the core (§7.6).
//!
//! One rule runs through all of it: **the window is given what it draws and nothing more.**
//! A secret-treated value crosses only when the user presses **Show** or **Copy**
//! ([`reveal_value`], [`copy_value`]), a URL is opened by its scheme and never by its
//! spelling ([`open_url`]), a `secrets` destination is resolved here against the session's
//! project folder rather than trusted as a path from the webview ([`open_secret_file`]), and
//! typed text is scanned before it can be sent ([`scan_typed_text`], and again inside
//! [`act`]).
//!
//! The core is optional: a listener that could not bind leaves the app running with no store
//! (`lib.rs` says why), and every command here answers the empty answer rather than an error
//! the user cannot act on.

use std::sync::{Arc, Mutex, MutexGuard};

use serde::Deserialize;
use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt as _;
use tauri_plugin_opener::OpenerExt as _;

use crate::log::Timestamp;
use crate::redaction::typed::{redact, Redacted};
use crate::sessions::Registry;
use crate::store::{Refusal, StoreHandle, UserAction};

use super::events::NoticeKind;
use super::view::{self, HandoffView, TabView};

/// The core, as the window reaches it.
///
/// The store is the single owner of handoff state and the registry the single owner of who
/// is connected (SRV-21); both are held by the channel task as well, which is why the
/// registry is behind a mutex and the store behind its own command channel.
#[derive(Clone)]
pub struct Core {
    /// The handoff store (§7.4).
    pub store: StoreHandle,
    /// The session registry (§7.5), shared with the channel dispatch.
    pub registry: Arc<Mutex<Registry>>,
}

impl Core {
    /// The registry, for one synchronous read. Never held across an `await`.
    fn registry(&self) -> MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The `session_ref`s that are connected right now.
    ///
    /// Collected into an owned set rather than answered from the guard, so that no lock
    /// crosses an `await` and the view can be built afterwards.
    fn connected(&self) -> std::collections::HashSet<String> {
        self.registry()
            .connected()
            .map(|session| session.session_ref.clone())
            .collect()
    }
}

/// The core, or nothing when the channel could not start.
pub struct CoreState(pub Option<Core>);

impl CoreState {
    fn get(&self) -> Option<&Core> {
        self.0.as_ref()
    }
}

/// The tab strip (§7.6, MULTI-01).
#[tauri::command]
pub async fn list_handoffs(core: State<'_, CoreState>) -> Result<Vec<TabView>, String> {
    let Some(core) = core.get().cloned() else {
        return Ok(Vec::new());
    };
    let now = Timestamp::now();
    let snapshots = core.store.list_for_ui(now.clone()).await;
    let connected = core.connected();
    Ok(view::tabs(
        &snapshots,
        &|session_ref| connected.contains(session_ref),
        &now,
    ))
}

/// One whole tab (§7.6).
#[tauri::command]
pub async fn get_handoff_view(
    core: State<'_, CoreState>,
    id: String,
) -> Result<Option<HandoffView>, String> {
    let Some(core) = core.get().cloned() else {
        return Ok(None);
    };
    let now = Timestamp::now();
    let Some(snapshot) = core.store.snapshot(id.clone(), now.clone()).await else {
        return Ok(None);
    };
    let replies = core.store.replies(id).await;
    let connected = core.connected();
    Ok(Some(view::build(
        &snapshot,
        &replies,
        &|session_ref| connected.contains(session_ref),
        &now,
    )))
}

/// What the user pressed (RESP-01..09, SRV-23, FM-20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// This step is done, go to the next one (RESP-01, RESP-03).
    Confirm,
    /// Annotate the step locally (RESP-02).
    Note,
    /// Skip the step (RESP-03).
    Skip,
    /// Ask the agent (RESP-04).
    Ask,
    /// Defer (RESP-05, RESP-07).
    Defer,
    /// Abandon (RESP-08).
    Abandon,
    /// The **last** step is done, so the round is (RESP-09): the tab moves to verifying, or
    /// to `confirmed_by_user` when the spec asked for no verification. One button in the
    /// window sends this or `confirm` depending on where the cursor stands; they are two
    /// different transitions and not two names for one.
    Done,
    /// Pick a parked or deferred handoff up again (RESP-07, FM-31).
    ResumeFromOverlay,
    /// Close a final outcome nobody collected (SRV-23).
    CloseOrphan,
    /// Correct which request this handoff answers (FM-20, §12.4).
    Relink,
}

/// Applies one user action (§7.4).
///
/// `payload` is the text of a sheet, or the request id of a relink; the actions that take
/// none ignore it. The three that **send** what the user typed — Ask, Defer, Abandon — are
/// redacted here as well as in the sheet: the sheet shows the user what will be sent
/// (§7.10), this makes sure that is what is sent whatever called the command.
#[tauri::command]
pub async fn act(
    app: AppHandle,
    core: State<'_, CoreState>,
    id: String,
    action: Action,
    payload: Option<String>,
) -> Result<(), String> {
    let Some(core) = core.get().cloned() else {
        return Err("the store is not running".to_owned());
    };

    let sent = |what: Option<String>| what.map(|text| redact(&text).text);
    let user_action = match action {
        Action::Confirm => UserAction::Confirm,
        Action::Done => UserAction::Done,
        Action::Skip => UserAction::Skip,
        Action::Note => UserAction::Note(payload.unwrap_or_default()),
        Action::Ask => UserAction::Ask(sent(payload).unwrap_or_default()),
        Action::Defer => UserAction::Defer(sent(payload)),
        Action::Abandon => UserAction::Abandon(sent(payload)),
        Action::ResumeFromOverlay => UserAction::ResumeFromOverlay,
        Action::CloseOrphan => UserAction::CloseOrphan,
        Action::Relink => match payload {
            Some(request_id) => UserAction::Relink(request_id),
            None => return Err("relink needs the id of the request".to_owned()),
        },
    };

    match core.store.user(id, user_action, Timestamp::now()).await {
        Ok(()) => Ok(()),
        Err(refusal) => {
            let key = match refusal {
                // §7.4 calls this a defect of the view: the button should not have been
                // there. The user is told the tab moved on rather than nothing at all.
                Refusal::NotActive { .. } | Refusal::NotFound => super::NOTICE_ACTION_REFUSED,
                _ => super::NOTICE_ACTION_FAILED,
            };
            super::notifier(&app).notice(NoticeKind::Warning, key);
            Err(refusal.to_string())
        }
    }
}

/// Puts the **true** value on the clipboard (GUIDE-02, DET-04).
///
/// With an index it copies that item of a list, without one the whole value — a list joined
/// by newlines, which is what "copyable as a whole" means for something the user pastes into
/// a form. A secret-treated value is copied like any other: the mask exists so that it is
/// not *shown*, and the whole point of DET-04 is that the user can still paste it.
#[tauri::command]
pub async fn copy_value(
    app: AppHandle,
    core: State<'_, CoreState>,
    id: String,
    key: String,
    index: Option<usize>,
) -> Result<(), String> {
    let Some(core) = core.get().cloned() else {
        return Err("the store is not running".to_owned());
    };
    let items = core.store.value(id, key).await;
    let text = match index {
        Some(index) => items.get(index).cloned(),
        None => (!items.is_empty()).then(|| items.join("\n")),
    };
    let Some(text) = text else {
        return Err("no such value".to_owned());
    };

    app.clipboard().write_text(text).map_err(|error| {
        super::notifier(&app).notice(NoticeKind::Error, super::NOTICE_COPY_FAILED);
        error.to_string()
    })
}

/// The **true** value, for the ten-second local reveal of DET-04.
///
/// One entry per item, so a list is revealed item by item exactly as it is drawn. The window
/// hides it again on a timer; nothing here remembers that it was ever asked for, and the
/// value is never part of a repaint.
#[tauri::command]
pub async fn reveal_value(
    core: State<'_, CoreState>,
    id: String,
    key: String,
) -> Result<Vec<String>, String> {
    let Some(core) = core.get().cloned() else {
        return Ok(Vec::new());
    };
    Ok(core.store.value(id, key).await)
}

/// Opens a URL with the operating system, if its scheme is one of the four (SPEC-07).
///
/// The window already refuses to draw a button for anything else, so reaching this with a
/// refused scheme means the check on that side was bypassed; it is repeated here because
/// this is the side that can actually launch something.
#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    if !crate::format::spec::url_allowed(&url) {
        tracing::warn!("a url with a scheme outside SPEC-07 was refused");
        super::notifier(&app).notice(NoticeKind::Warning, super::NOTICE_URL_REFUSED);
        return Err("the scheme of that link is not allowed".to_owned());
    }
    app.opener().open_url(url, None::<&str>).map_err(|error| {
        super::notifier(&app).notice(NoticeKind::Error, super::NOTICE_OPEN_FAILED);
        error.to_string()
    })
}

/// Opens the file a `secrets` entry names, with its default application (SEC-02).
///
/// The window sends the **name** of the entry and never a path: the destination is read here
/// from the spec the store holds and resolved against the session's project folder, so a
/// webview cannot ask the app to open an arbitrary file. A relative destination with no
/// project folder to resolve it against is refused rather than guessed.
#[tauri::command]
pub async fn open_secret_file(
    app: AppHandle,
    core: State<'_, CoreState>,
    id: String,
    name: String,
) -> Result<(), String> {
    let Some(core) = core.get().cloned() else {
        return Err("the store is not running".to_owned());
    };
    let Some(snapshot) = core.store.snapshot(id, Timestamp::now()).await else {
        return Err("no such handoff".to_owned());
    };
    let Some(file) = snapshot
        .secrets
        .as_ref()
        .and_then(|secrets| secrets.get(&name))
    else {
        return Err("the spec declares no such secret".to_owned());
    };

    let path = std::path::Path::new(file);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        match snapshot.project_dir.as_deref() {
            Some(project_dir) => std::path::Path::new(project_dir).join(path),
            None => {
                super::notifier(&app).notice(NoticeKind::Warning, super::NOTICE_OPEN_FAILED);
                return Err("the session has no project folder to resolve the file in".to_owned());
            }
        }
    };

    app.opener()
        .open_path(resolved.to_string_lossy(), None::<&str>)
        .map_err(|error| {
            // The ordinary case is a file the user has not created yet, which SEC-02's own
            // flow expects: the spec says where to paste the secret, and it may not be there.
            super::notifier(&app).notice(NoticeKind::Warning, super::NOTICE_OPEN_FAILED);
            error.to_string()
        })
}

/// Brings the overlay to the front (MULTI-03).
///
/// The window decides *when*, because the rule is about what the user is looking at: the
/// first handoff opened while none is active is worth interrupting them for, a second one
/// arriving beside it is not — it gets a badge and waits. The core cannot tell the two apart
/// (it has no notion of which tab is on screen), so it reports the change and the window
/// asks for the front when the rule says so.
#[tauri::command]
pub fn show_window(app: AppHandle) {
    super::show_main_window(&app);
}

/// Runs the certain detector over what the user typed (§7.10, DET-01).
///
/// The sheets for Ask, Defer and Abandon call it as the text changes and show what will be
/// sent, so the redaction is something the user sees before pressing the button rather than
/// something that happened to their words afterwards.
#[tauri::command]
pub fn scan_typed_text(text: String) -> Redacted {
    redact(&text)
}
