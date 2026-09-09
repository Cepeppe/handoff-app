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

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager as _, State};
use tauri_plugin_clipboard_manager::ClipboardExt as _;
use tauri_plugin_opener::OpenerExt as _;

use crate::log::{HandoffState, Timestamp};
use crate::redaction::typed::{redact, Redacted};
use crate::requests::queue::Queue;
use crate::requests::text::render_request_text;
use crate::sessions::Registry;
use crate::store::{Opener, Refusal, StoreHandle, UserAction};

use super::events::NoticeKind;
use super::requests::RequestDelivery;
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
    /// The user-request queue (§7.7), shared with the store and the channel dispatch.
    pub queue: Arc<Queue>,
    /// The clipboard, focus and notification path of OPEN-05.
    pub delivery: RequestDelivery,
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

    /// The sessions the request sheet may address, oldest registration first (OPEN-04).
    ///
    /// Owned, for the same reason: the guard is dropped before anything else happens.
    fn choices(&self) -> Vec<SessionChoice> {
        let registry = self.registry();
        let mut sessions: Vec<&crate::sessions::Session> = registry.connected().collect();
        sessions.sort_by(|left, right| {
            left.first_seen
                .cmp(&right.first_seen)
                .then_with(|| left.session_ref.cmp(&right.session_ref))
        });
        sessions
            .into_iter()
            .map(|session| SessionChoice {
                session_ref: session.session_ref.clone(),
                label: session.display_name(),
            })
            .collect()
    }

    /// The opener of a handoff a request is addressed to, when a session was selected.
    fn opener(&self, session_ref: Option<&str>) -> Option<Opener> {
        let session_ref = session_ref?;
        Some(Opener::from(self.registry().get(session_ref)?))
    }
}

/// The core, or nothing when the channel could not start.
pub struct CoreState(pub Option<Core>);

impl CoreState {
    fn get(&self) -> Option<&Core> {
        self.0.as_ref()
    }
}

/// The tab strip (§7.6, MULTI-01), and the tray badge that counts the same handoffs
/// (WIN-05).
///
/// The badge is set here rather than from a command of its own, and that is deliberate: the
/// strip and the badge are the same fact seen twice, the window re-reads the strip on every
/// change already, and a second path would be a second answer to "how many are active" for
/// the two to disagree about. What counts as active is decided on this side, next to the
/// states: anything not final, which is everything the user still has something to do about.
#[tauri::command]
pub async fn list_handoffs(
    app: AppHandle,
    core: State<'_, CoreState>,
) -> Result<Vec<TabView>, String> {
    let Some(core) = core.get().cloned() else {
        super::set_tray_badge(&app, 0);
        return Ok(Vec::new());
    };
    let now = Timestamp::now();
    let snapshots = core.store.list_for_ui(now.clone()).await;
    let connected = core.connected();
    super::set_tray_badge(
        &app,
        snapshots
            .iter()
            .filter(|snapshot| !snapshot.state.is_final())
            .count(),
    );
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
    let exchanges = core.store.exchanges(id).await;
    let connected = core.connected();
    Ok(Some(view::build(
        &snapshot,
        &exchanges,
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

/// Puts the OPEN-05 sentence for a handoff waiting for its spec back on the clipboard.
///
/// The "Copy request again" of the Waiting-for-spec view (§7.6): the first copy happened
/// when the request was opened, and by the time the user comes back to the tab their
/// clipboard has moved on. The sentence is rendered here, in the language the window is
/// showing, from the same `requests::text` the queue and the Stop hook use — the id and the
/// tool name are invariant across both languages, which is what the agent acts on.
///
/// Refused for a handoff that is not waiting for a spec: there is no request to re-send.
#[tauri::command]
pub async fn copy_request_text(
    app: AppHandle,
    core: State<'_, CoreState>,
    id: String,
) -> Result<(), String> {
    let Some(core) = core.get().cloned() else {
        return Err("the store is not running".to_owned());
    };
    let Some(snapshot) = core.store.snapshot(id, Timestamp::now()).await else {
        return Err("no such handoff".to_owned());
    };
    if snapshot.state != HandoffState::AwaitingSpec {
        return Err("that handoff is not waiting for a spec".to_owned());
    }
    // The request the user typed, and the id the agent has to quote: for a handoff the user
    // opened they are the same entry (§7.7 mints one id for both), and `linked_request_id`
    // names it when a spec was linked to a request of another id (OPEN-08).
    let request_id = snapshot
        .linked_request_id
        .as_deref()
        .unwrap_or(&snapshot.id);
    let text = render_request_text(
        super::language_of(&app),
        request_id,
        snapshot.request_text.as_deref().unwrap_or_default(),
    );

    app.clipboard().write_text(text).map_err(|error| {
        super::notifier(&app).notice(NoticeKind::Error, super::NOTICE_COPY_FAILED);
        error.to_string()
    })
}

/// Puts a handoff's own id on the clipboard (SRV-23).
///
/// The third thing the orphan list of SRV-23 offers, beside viewing it and closing it by
/// hand: an id the user pastes into a new session to resume the work. It goes through the
/// core like every other copy, so the window names a handoff and never a string to copy.
#[tauri::command]
pub async fn copy_handoff_id(
    app: AppHandle,
    core: State<'_, CoreState>,
    id: String,
) -> Result<(), String> {
    let Some(core) = core.get().cloned() else {
        return Err("the store is not running".to_owned());
    };
    let Some(snapshot) = core.store.snapshot(id, Timestamp::now()).await else {
        return Err("no such handoff".to_owned());
    };

    app.clipboard().write_text(snapshot.id).map_err(|error| {
        super::notifier(&app).notice(NoticeKind::Error, super::NOTICE_COPY_FAILED);
        error.to_string()
    })
}

/// The sessions the request sheet may address (OPEN-04, OPEN-04a).
///
/// Only the connected ones: a request addressed to a session whose server has gone would be
/// re-queued the moment it was written (FM-34), so offering it would be offering a choice
/// that undoes itself. An empty answer is the "no active session" notice of OPEN-04a, and
/// the sheet still opens — the request is queued for the first session that registers.
#[tauri::command]
pub fn sessions(core: State<'_, CoreState>) -> Vec<SessionChoice> {
    core.get().map(Core::choices).unwrap_or_default()
}

/// What the user typed in the request sheet (§7.7, OPEN-04, OPEN-05).
///
/// Three things happen and the order is the pseudocode of §7.7: the entry is queued, the tab
/// appears in "waiting for spec", and the sentence goes on the clipboard while the session's
/// terminal is brought forward. The id is the same for all three — the tab *is* the request
/// (DD-13) — and it is what the agent quotes back as `request_id`.
///
/// Returns the id, so the window can select the tab it has just created.
#[tauri::command]
pub async fn create_request(
    app: AppHandle,
    core: State<'_, CoreState>,
    text: String,
    session_ref: Option<String>,
) -> Result<String, String> {
    let Some(core) = core.get().cloned() else {
        return Err("the store is not running".to_owned());
    };
    // OPEN-04 gives the sheet one field and Enter sends it; a blank one is not a request,
    // and the sheet refuses it too. Both, because this is the side that can be called by
    // anything.
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err("a request needs a sentence".to_owned());
    }
    // A session that has gone between the sheet opening and Enter is treated as no session
    // at all: OPEN-04a queues it for the first one that registers, which is better than
    // addressing it to a connection nobody is on.
    let opener = core.opener(session_ref.as_deref());
    let session_ref = opener.as_ref().map(|opener| opener.session_ref.clone());
    let now = Timestamp::now();

    let ui = app.state::<super::Ui>();
    let queued = ui.with_db(|db| core.queue.create(db, &text, session_ref.as_deref(), &now));
    let id = match queued {
        Some(Ok(id)) => id,
        Some(Err(error)) => return Err(error.to_string()),
        None => return Err("the log is not available".to_owned()),
    };

    // The tab, before the clipboard: OPEN-04 says it appears immediately, and a user who
    // pastes at once should find the handoff already there.
    if let Err(error) = core.store.open_request(id.clone(), text, opener, now).await {
        return Err(error.to_string());
    }

    // The fast path of OPEN-05. It is deliberately outside the two writes above: a clipboard
    // that refuses, or a terminal that cannot be found, leaves a queued request the Stop hook
    // delivers at the end of the turn (OPEN-06, FM-21).
    let entry = ui
        .with_db(|db| core.queue.get(db, &id))
        .and_then(|read| match read {
            Ok(entry) => entry,
            Err(error) => {
                tracing::warn!(error = %error, "the queued request could not be read back");
                None
            }
        });
    if let Some(entry) = entry {
        ui.with_db(|db| {
            core.delivery.deliver(db, &entry, session_ref.as_deref());
        });
    }
    Ok(id)
}

/// One entry of the queue, as the **Change** control lists it (FM-20, §12.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestChoice {
    /// The `hf_` id of the queue entry.
    pub id: String,
    /// The user's own words.
    pub text: String,
    /// When they typed it, so the list can say which is the older one.
    pub created_at: String,
}

/// The requests a handoff could be answering instead of the one it is (FM-20).
///
/// §7.7 links a spec that quoted no `request_id` to the **oldest** open request of its
/// session, and §12.4 accepts that two open requests in one session may be matched the wrong
/// way round. This is what makes the correction one click: the still-open requests of the
/// same session, plus the one this handoff currently answers, so the user can also put it
/// back. Resume entries are left out — they ask an agent to come back to a handoff and are
/// not something another handoff can answer (FM-31).
#[tauri::command]
pub async fn open_requests(
    app: AppHandle,
    core: State<'_, CoreState>,
    id: String,
) -> Result<Vec<RequestChoice>, String> {
    let Some(core) = core.get().cloned() else {
        return Ok(Vec::new());
    };
    let Some(snapshot) = core.store.snapshot(id, Timestamp::now()).await else {
        return Ok(Vec::new());
    };
    let Some(session_ref) = snapshot.session_ref.clone() else {
        return Ok(Vec::new());
    };

    let ui = app.state::<super::Ui>();
    let read = ui.with_db(|db| {
        let mut entries = core.queue.open_for_session(db, &session_ref)?;
        // The one it answers today is closed, so it is not in the list above; it belongs
        // there, because "put it back" is as much a correction as "move it".
        if let Some(current) = snapshot.linked_request_id.as_deref() {
            if let Some(entry) = core.queue.get(db, current)? {
                entries.push(entry);
            }
        }
        crate::log::Result::Ok(entries)
    });
    let entries = match read {
        Some(Ok(entries)) => entries,
        Some(Err(error)) => return Err(error.to_string()),
        None => return Ok(Vec::new()),
    };

    Ok(entries
        .into_iter()
        .filter(|entry| entry.about_handoff_id.is_none())
        .map(|entry| RequestChoice {
            id: entry.id,
            text: entry.text,
            created_at: entry.created_at.to_string(),
        })
        .collect())
}

/// The global shortcut in force, and whether the user should be asked for another (OPEN-03,
/// FM-18).
#[tauri::command]
pub fn shortcut_status(app: AppHandle) -> super::shortcut::Status {
    app.state::<super::Ui>().shortcut().status()
}

/// The user chose another combination in the FM-18 dialog.
///
/// # Errors
///
/// The plugin's own message, which the dialog shows: a combination it cannot parse, or one
/// another application already holds — the app never takes one that is taken (OPEN-03).
#[tauri::command]
pub fn set_shortcut(app: AppHandle, accelerator: String) -> Result<(), String> {
    super::shortcut::choose(&app, &accelerator)
}

/// The user closed the FM-18 dialog without choosing: never ask again.
#[tauri::command]
pub fn dismiss_shortcut_question(app: AppHandle) {
    super::shortcut::asked(&app);
}

/// One of the sessions the FM-22 picker asks the user to choose between.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionChoice {
    /// The `ses_` reference, which is what the answer names.
    pub session_ref: String,
    /// Agent and project folder in one line, as the tab strip labels it (OPEN-02).
    pub label: String,
}

/// The "which session is this?" question, when a hook left one (FM-22, SRV-18).
///
/// Empty most of the time. The registry raises it when a hook's ancestor chain matched
/// several sessions and the working directory did not separate them; the window draws it the
/// next time the user is looking, which is what §7.5 asks for.
#[tauri::command]
pub fn session_picker(core: State<'_, CoreState>) -> Vec<SessionChoice> {
    let Some(core) = core.get() else {
        return Vec::new();
    };
    let registry = core.registry();
    registry
        .session_picker()
        .iter()
        .filter_map(|session_ref| {
            let session = registry.get(session_ref)?;
            Some(SessionChoice {
                session_ref: session_ref.clone(),
                label: session.display_name(),
            })
        })
        .collect()
}

/// The user answered the picker: this session is the one that ran the hook (FM-22).
///
/// The answer binds the agent session id the hook carried, so the safety net of SRV-12 stops
/// being delayed for that session. `dismiss` is the other answer a person may give — "I do
/// not know" — which drops the question without binding anything.
#[tauri::command]
pub fn answer_session_picker(
    app: AppHandle,
    core: State<'_, CoreState>,
    session_ref: Option<String>,
) -> Result<(), String> {
    let Some(core) = core.get() else {
        return Ok(());
    };
    let mut registry = core.registry();
    let Some(session_ref) = session_ref else {
        registry.session_picker_answered();
        return Ok(());
    };
    // The binding goes through the window's own connection: the registry's belongs to the
    // channel task, and this write comes from the user rather than from a peer.
    let written = app
        .state::<super::Ui>()
        .with_db(|db| registry.answer_session_picker(db, &session_ref));
    match written {
        Some(Err(error)) => {
            super::notifier(&app).notice(NoticeKind::Warning, super::NOTICE_ACTION_FAILED);
            Err(error.to_string())
        }
        // No log to write to: the answer is taken in memory for the life of the process,
        // which is exactly as long as the question could have been asked.
        Some(Ok(_)) | None => Ok(()),
    }
}

/// What the window needs to know about its own behaviour (§7.16, R-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSettings {
    /// Whether the fallback collapse of R-10 is switched on.
    pub collapse_fallback: bool,
    /// How long after the last interaction it fires, in milliseconds.
    pub collapse_fallback_ms: u32,
}

/// The window settings of §7.16, read at mount.
#[tauri::command]
pub fn window_settings(app: AppHandle) -> WindowSettings {
    WindowSettings {
        collapse_fallback: app.state::<super::Ui>().collapse_fallback(),
        collapse_fallback_ms: super::window::COLLAPSE_FALLBACK_MS,
    }
}

/// Switches the R-10 fallback collapse on or off.
///
/// It exists here because the fallback is useless without a way to turn it on and the
/// General settings page is T-041; that page is where the checkbox belongs and this is what
/// it will call.
#[tauri::command]
pub fn set_collapse_fallback(app: AppHandle, enabled: bool) -> Result<(), String> {
    app.state::<super::Ui>().set_collapse_fallback(enabled)
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
