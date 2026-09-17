//! The bridge between the core and Tauri (§7.6, §7.16).
//!
//! The Tauri commands the frontend invokes, the events it listens to, and the *only*
//! implementations of the side-effect traits the core declares (clipboard, notification,
//! focus, opener, window). Everything that names `AppHandle`, `WebviewWindow` or a Tauri
//! plugin belongs here; nothing in this module holds state that the core does not own.
//!
//! What the skeleton puts here is the window and the tray:
//!
//! - the overlay is a narrow panel of **fixed width** whose **height follows the content**
//!   (WIN-02). Only the frontend knows how tall the content is, so it measures and calls
//!   [`resize_to_content`]; this side decides what the monitor can actually show.
//! - closing the window **hides it to the tray**, and quitting is only ever the tray's
//!   `Quit` (WIN-04). Both halves live here: the close handler and the menu.
//! - the frontend reports the language it resolved ([`set_ui_language`]) so the tray menu
//!   speaks the same language as the window (APP-02).
//!
//! T-036 added the rest: [`view`] is the projection the window draws, [`commands`] is
//! everything the window may ask of the core, and [`events::Notifier`] is how the core tells
//! the window that something changed without ever naming Tauri itself.
//!
//! T-037 added the last of it: [`window`] is where the panel is and how it behaves — the
//! per-monitor position of WIN-02, the focus change WIN-03 collapses on — and the tray
//! finally has the badge of WIN-05.
//!
//! T-038 added the two halves of a user-opened request that only Tauri can do: [`shortcut`]
//! is the combination of OPEN-03 that opens the sheet from anywhere, and [`requests`] is
//! what happens when something in the queue can be put in front of an agent — the clipboard,
//! the terminal's window, the notification (OPEN-05).
//!
//! T-040 added [`install`]: onboarding, the consent screen, the Agents settings page and the
//! two launch checks of §7.2 — the scan of INST-05 and the moved-bundle check of FM-23.
//!
//! T-041 added [`general`]: the language, the login entry of APP-01 and the `--hidden` launch
//! that goes with it, plus the wider layout the settings page opens the panel in (§7.6).
//!
//! T-045 added the last two settings pages the design names: [`log`] is the record of §7.11
//! with its deletion and its export (LOG-04), and [`runbooks`] is the folder of §7.12 with
//! the update proposals of RUN-09.
//!
//! T-046 added [`capture`]: the two-choice popover of CAP-01, the panel hiding itself for
//! the length of a shot (CAP-03) and the transparent selection overlays of DD-29 — one
//! window per monitor, alive only for the drag.
//!
//! T-049 added [`preview`], which is the other half of it and the only way a screenshot
//! leaves this machine: the OCR and the detectors over the capture, the boxes the user
//! edits, and the burn-in that produces what the agent is given (PREV-01..05, PRIN-09).
//!
//! T-051 added [`network`]: the Network page of NET-01, which lists what `net::egress`
//! recorded — nothing at all in this build, and the page says so (implementation decision 8).

pub mod capture;
pub mod commands;
pub mod crash;
pub mod events;
pub mod general;
pub mod install;
pub mod log;
pub mod network;
pub mod preview;
pub mod requests;
pub mod runbooks;
pub mod shortcut;
mod tray;
pub mod view;
pub mod window;

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter as _, LogicalSize, Manager as _, PhysicalPosition, Window, WindowEvent,
};

use crate::i18n::Language;
use crate::log::Db;

pub use commands::{Core, CoreState};
pub use events::{Notice, NoticeKind, Notifier};
pub use requests::RequestDelivery;

/// The label of the overlay window, as `tauri.conf.json` declares it.
pub const MAIN_WINDOW: &str = "main";

/// The arguments every webview of this application gives the WebView2 browser process
/// (§7.13, NET-02, PRIN-05).
///
/// wry passes `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection` when nobody
/// asks for anything and *replaces* it with whatever is asked, so the string starts with
/// that default. The rest switches off the runtime's own background traffic — configuration
/// and field-trial fetches, component updates, domain-reliability reports, hyperlink-auditing
/// pings — which Baton's code never asks for and `net::egress` cannot see, because the
/// process that sends it is `msedgewebview2.exe` and not this one. Measured on 2026-09-10:
/// without these switches the webview of an idle Baton connected to Microsoft addresses
/// within seconds of the launch (`docs/verify-trust.md` has what remains).
///
/// It is written twice, here and as the main window's `additionalBrowserArgs` in
/// `tauri.conf.json`, and the two must be identical: WebView2 runs one browser process per
/// profile and refuses a second webview in the same profile with different arguments, so a
/// selection overlay of [`capture`] would fail to open. `tests/egress_boundary.rs` holds the
/// two together.
pub const WEBVIEW2_BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
     --disable-background-networking --disable-component-update --disable-domain-reliability \
     --no-pings";

/// The event that asks the frontend to bring a view forward (§7.6).
///
/// The tray menu is outside the component tree, so `New request` and `Settings` cannot
/// switch the view by calling into it; they emit this instead. The same literal is in
/// `src/bridge.ts` and a test below reads that file to keep the two spellings together.
pub const EVENT_SHOW_VIEW: &str = "ui://show-view";

/// The request sheet, as `src/views.ts` names it (§7.7, OPEN-03).
///
/// Two things ask for it — the tray's `New request` and the global shortcut — so the name
/// is a constant rather than a literal in each of them; a test in [`shortcut`] reads
/// `src/views.ts` to prove the frontend still knows it.
pub const VIEW_REQUEST: &str = "request";

/// The settings page, as `src/views.ts` names it.
pub const VIEW_SETTINGS: &str = "settings";

/// The window gained or lost the focus. Payload: `true` when it has it (WIN-03).
///
/// The panel collapses to the one-line bar when the user clicks elsewhere and expands when
/// they come back, and *what* the bar shows is the frontend's — the current step, three
/// buttons — so what crosses here is the bare fact. It is emitted from
/// [`on_window_event`], the one place Tauri reports it.
pub const EVENT_WINDOW_FOCUS: &str = "ui://window-focus";

/// The fixed width of the panel (WIN-02, §7.6). `tauri.conf.json` declares the same value.
pub const WINDOW_WIDTH: f64 = 360.0;

/// The width the panel takes while the settings page is open (§7.6).
///
/// "Settings that need more room open the window in a wider layout temporarily": the
/// consent diffs, the configuration paths and the agent rows are lines the user has to
/// *read*, and at 360 px they wrap into columns of three words. The number is this design's
/// to choose — §7.6 fixes only the panel's own width — and it is the smallest one that
/// shows a hook command without wrapping on this machine. It is not remembered anywhere:
/// leaving the settings page puts the panel back at [`WINDOW_WIDTH`], which is the width
/// every other view is laid out for.
pub const SETTINGS_WINDOW_WIDTH: f64 = 520.0;

/// The width of the expanded view the user asks for with **Expand** (§7.6).
///
/// WIN-02 fixes the *panel's* width, and it is still fixed: the window is never resizable
/// and nothing here is remembered on disk. What this is is the second shape of §7.6 — the
/// handoff list beside the step instead of above it — which the user opens and closes with a
/// window control. 720 is the smallest width at which the 220-pixel list and a step column
/// wide enough for a value row both fit without either of them wrapping.
pub const EXPANDED_WINDOW_WIDTH: f64 = 720.0;

/// Which shape the window has, which is the only thing that decides its width (§7.6).
///
/// The frontend derives it in one place (`App.svelte`) and reports it here; this side never
/// guesses it from the view. The names are the ones `src/bridge.ts` spells, and a test below
/// reads that file to keep the two together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowLayout {
    /// The narrow panel of WIN-02, and the width the collapsed bar of WIN-03 keeps.
    #[default]
    Panel,
    /// The settings page, which needs more room to be read (§7.6).
    Settings,
    /// The expanded view: the handoff list on the left, one column of content.
    Expanded,
}

impl WindowLayout {
    /// The width this shape asks for.
    #[must_use]
    pub fn width(self) -> f64 {
        match self {
            Self::Panel => WINDOW_WIDTH,
            Self::Settings => SETTINGS_WINDOW_WIDTH,
            Self::Expanded => EXPANDED_WINDOW_WIDTH,
        }
    }

    /// The discriminant, for the atomic that holds the layout in force.
    fn code(self) -> u8 {
        match self {
            Self::Panel => 0,
            Self::Settings => 1,
            Self::Expanded => 2,
        }
    }

    /// The other direction. An unknown code is the panel: the width of WIN-02 is the safe one.
    fn of_code(code: u8) -> Self {
        match code {
            1 => Self::Settings,
            2 => Self::Expanded,
            _ => Self::Panel,
        }
    }
}

/// The tab moved on and the button the user pressed is no longer offered (§7.4).
pub const NOTICE_ACTION_REFUSED: &str = "notice.actionRefused";

/// The disk refused the transition; the state is unchanged and the action can be retried
/// (FM-28).
pub const NOTICE_ACTION_FAILED: &str = "notice.actionFailed";

/// The clipboard refused the value.
pub const NOTICE_COPY_FAILED: &str = "notice.copyFailed";

/// A link whose scheme SPEC-07 does not allow.
pub const NOTICE_URL_REFUSED: &str = "notice.urlRefused";

/// A URL or a file the operating system would not open.
pub const NOTICE_OPEN_FAILED: &str = "notice.openFailed";

/// Every notice this module can push, so one test can prove all of them are translated.
pub const NOTICE_KEYS: [&str; 5] = [
    NOTICE_ACTION_REFUSED,
    NOTICE_ACTION_FAILED,
    NOTICE_COPY_FAILED,
    NOTICE_URL_REFUSED,
    NOTICE_OPEN_FAILED,
];

/// The smallest window the content may ask for: the header alone is about this tall, and a
/// zero-height window would be a window the user cannot grab.
const MIN_WINDOW_HEIGHT: f64 = 64.0;

/// The tallest window to allow when no monitor answers. A panel is never full-screen, and a
/// bad measurement must not produce a window that covers the desktop.
const FALLBACK_MAX_WINDOW_HEIGHT: f64 = 1200.0;

/// What the Tauri side remembers about the interface.
///
/// It holds no handoff state and no product state: those live in the core, which knows
/// nothing about Tauri. What is here is what only Tauri can own — the language the menu
/// items are currently labelled in, and the handles needed to relabel them.
#[derive(Default)]
pub struct Ui {
    language: Mutex<Language>,
    tray: Mutex<Option<tray::Handles>>,
    notifier: Notifier,
    geometry: window::Geometry,
    shortcut: shortcut::State,
    db: Mutex<Option<Db>>,
    layout: AtomicU8,
    capture: capture::State,
    preview: preview::State,
}

impl Ui {
    /// The interface state, carrying the notifier the core was given before the window
    /// existed (`events::Notifier` says why it arrives in two halves).
    #[must_use]
    pub fn with_notifier(notifier: Notifier) -> Self {
        Self {
            notifier,
            ..Self::default()
        }
    }

    /// Gives the window its own connection to the log.
    ///
    /// A third connection beside the store's actor and the registry's, and it is the
    /// window's: it holds the panel's position per monitor (WIN-02), the settings §7.16
    /// gives the window, and the one `sessions` row the FM-22 picker binds when the user
    /// answers it. It is opened independently of the channel on purpose — a listener that
    /// failed to bind must not cost the user their window as well (`lib.rs`) — and
    /// `log::db` sets WAL and a busy timeout for exactly this traffic.
    #[must_use]
    pub fn with_settings(self, db: Db) -> Self {
        self.geometry.load(&db);
        *self.db.lock().expect("the settings mutex is poisoned") = Some(db);
        self
    }

    /// The language the interface is showing.
    pub fn language(&self) -> Language {
        *self
            .language
            .lock()
            .expect("the UI language mutex is poisoned")
    }

    /// Where the panel is, per monitor (WIN-02).
    pub fn geometry(&self) -> &window::Geometry {
        &self.geometry
    }

    /// The shape the window is in (§7.6). The panel of WIN-02 until the frontend says otherwise.
    pub fn layout(&self) -> WindowLayout {
        WindowLayout::of_code(self.layout.load(Ordering::Relaxed))
    }

    /// The width the window should have right now (§7.6).
    ///
    /// One of three values, never a remembered one: the fixed width of WIN-02, the wider
    /// settings page, or the expanded view the user asked for.
    pub fn window_width(&self) -> f64 {
        self.layout().width()
    }

    /// Records the shape the frontend derived. The next resize is what the user sees.
    pub fn set_layout(&self, layout: WindowLayout) {
        self.layout.store(layout.code(), Ordering::Relaxed);
    }

    /// The global shortcut in force, and whether the system accepted it (OPEN-03, FM-18).
    pub fn shortcut(&self) -> &shortcut::State {
        &self.shortcut
    }

    /// The capture waiting for the preview, and whether a selection is on screen (§7.8).
    pub fn capture(&self) -> &capture::State {
        &self.capture
    }

    /// Runs `read` against the window's connection, or answers `None` when there is none.
    pub fn with_db<T>(&self, read: impl FnOnce(&Db) -> T) -> Option<T> {
        let guard = self.db.lock().expect("the settings mutex is poisoned");
        guard.as_ref().map(read)
    }

    /// The same, for the one operation that needs the connection mutably.
    ///
    /// `log::maintenance::delete_all` opens a transaction of its own, which rusqlite will
    /// only hand out through `&mut Connection`. It is reachable from here at all because a
    /// listener that failed to bind leaves the app with no store to route the deletion
    /// through (`ui_bridge::log` says which path is taken when).
    pub fn with_db_mut<T>(&self, write: impl FnOnce(&mut Db) -> T) -> Option<T> {
        let mut guard = self.db.lock().expect("the settings mutex is poisoned");
        guard.as_mut().map(write)
    }

    /// Writes the panel's position, if it moved since the last write.
    pub fn flush_geometry(&self) {
        let guard = self.db.lock().expect("the settings mutex is poisoned");
        self.geometry.flush(guard.as_ref());
    }

    /// Whether the R-10 fallback collapse is switched on. Off unless the user said so.
    pub fn collapse_fallback(&self) -> bool {
        self.with_db(|db| {
            crate::log::settings::get::<bool>(db, window::COLLAPSE_FALLBACK_KEY)
                .ok()
                .flatten()
        })
        .flatten()
        .unwrap_or(false)
    }

    /// Switches the R-10 fallback collapse on or off (§7.16; the checkbox is T-041).
    ///
    /// # Errors
    ///
    /// The message of the write that failed, for the window to show.
    pub fn set_collapse_fallback(&self, enabled: bool) -> Result<(), String> {
        self.with_db(|db| crate::log::settings::set(db, window::COLLAPSE_FALLBACK_KEY, &enabled))
            .unwrap_or(Ok(()))
            .map_err(|error| error.to_string())
    }
}

/// The language the window reported, from anywhere that has the application.
fn language_of(app: &AppHandle) -> Language {
    app.state::<Ui>().language()
}

/// The notifier the core was built with, from anywhere that has the application.
fn notifier(app: &AppHandle) -> Notifier {
    app.state::<Ui>().notifier.clone()
}

/// Builds the tray and, in a development build, shows the window once.
///
/// Called from `setup()` once the application exists.
pub fn init(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let state = app.state::<Ui>();
    // From here on the core's observers have a window to emit through; everything they
    // fired while the channel was starting was dropped, and the view reads the core when it
    // mounts.
    state.notifier.attach(app.clone());
    let handles = tray::install(app, state.language())?;
    *state.tray.lock().expect("the tray mutex is poisoned") = Some(handles);

    // OPEN-03. After the tray, deliberately: when the combination is taken, `New request`
    // in the menu is the path the user is left with (FM-18), and it exists by now.
    shortcut::install(app);

    // APP-01: the login entry the user answered for in onboarding, put back in the state
    // they asked for. Nothing is written for a machine that has not been asked yet.
    general::sync_autostart(app);

    // §7.16 keeps the window hidden until there is something to show, and the tray is what
    // brings it back. The two things that open it by themselves are decided by the window
    // and not here: a first launch shows onboarding, and a moved bundle shows the repair
    // offer of FM-23 (`install::onboarding`, `install::scan_agents`). Both are sentences for
    // a person, and at this point there is no webview listening to be told anything.

    Ok(())
}

/// Everything Tauri reports about the one window (WIN-02, WIN-03, WIN-04).
///
/// Three events, three rules of §7.16:
///
/// - **Close** hides the panel to the tray; only the tray's `Quit` ends the process.
/// - **Focus** is what WIN-03 collapses on. The fact travels to the frontend, which owns
///   what the collapsed bar looks like; losing it is also when the remembered position is
///   written, because a drag has certainly finished by then.
/// - **Moved** is remembered in memory, per monitor, and written later (`window::Geometry`
///   says why not here).
///
/// It is registered for **every** window, and the three rules are the panel's alone: a
/// selection overlay of §7.8 must be closable, must not be hidden instead of destroyed, and
/// has no remembered position of its own. So the handler answers for the main window and
/// leaves anything else to Tauri's defaults.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW {
        return;
    }
    let app = window.app_handle();
    let ui = app.state::<Ui>();
    match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            hide_to_tray(window, &ui);
        }
        WindowEvent::Moved(_) => ui.geometry().remember(window),
        WindowEvent::Focused(focused) => {
            if !focused {
                ui.geometry().remember(window);
                ui.flush_geometry();
            }
            if let Err(error) = app.emit(EVENT_WINDOW_FOCUS, *focused) {
                tracing::warn!(error = %error, "the focus change reached no window");
            }
        }
        _ => {}
    }
}

/// Puts the window away into the notification area (WIN-04).
///
/// The one path both doors take: the window's own close button, which never ends the
/// process, and the **Minimize to tray** control of the title bar. The order is what makes
/// the position survive — remember where the window is, write it, then hide it — because a
/// hidden window has no position left to ask for.
fn hide_to_tray(window: &Window, ui: &Ui) {
    ui.geometry().remember(window);
    ui.flush_geometry();
    // A window that cannot hide is a window the user just failed to put away; there is
    // nothing to do about it here beyond leaving it open.
    if let Err(error) = window.hide() {
        tracing::warn!(error = %error, "the overlay window refused to hide");
    }
}

/// The title bar's **Minimize to tray** (WIN-04).
///
/// It cannot fail for the caller: a window the system refuses to hide is logged and stays
/// open, exactly as it does when the close button is pressed.
#[tauri::command]
pub fn hide_window(window: Window) {
    let app = window.app_handle();
    hide_to_tray(&window, &app.state::<Ui>());
}

/// Brings the overlay window to the front (the tray's `Show`, and the second launch of
/// APP-01's single instance).
///
/// The remembered position of WIN-02 is applied **before** the window is shown, so the
/// panel appears where the user left it rather than moving once it is already visible.
pub fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        tracing::warn!(window = MAIN_WINDOW, "no window to show");
        return;
    };
    app.state::<Ui>()
        .geometry()
        .restore(&window.as_ref().window());
    if let Err(error) = window.show() {
        tracing::warn!(error = %error, "the overlay window refused to show");
    }
    if let Err(error) = window.set_focus() {
        tracing::warn!(error = %error, "the overlay window refused the focus");
    }
}

/// The application is about to exit: write what only memory holds (WIN-02).
///
/// Called from the run-event callback rather than after it, so the window is still there to
/// be asked where it is.
pub fn on_exit_requested(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        app.state::<Ui>()
            .geometry()
            .remember(&window.as_ref().window());
    }
    app.state::<Ui>().flush_geometry();
}

/// Shows or hides the tray badge, from the count of handoffs that are not final (WIN-05).
pub fn set_tray_badge(app: &AppHandle, count: usize) {
    tray::set_badge(app, count);
}

/// Shows the window and asks the frontend for one of its views.
fn show_view(app: &AppHandle, view: &str) {
    show_main_window(app);
    if let Err(error) = app.emit(EVENT_SHOW_VIEW, view) {
        tracing::warn!(error = %error, "the view request reached no window");
    }
}

/// Gives the window the height the content measured, keeping the width in force (WIN-02).
#[tauri::command]
pub fn resize_to_content(window: Window, height: f64) -> Result<(), String> {
    let width = window.app_handle().state::<Ui>().window_width();
    let limit = monitor_height(&window);
    let clamped = clamp_height(height, limit);
    window
        .set_size(LogicalSize::new(width, clamped))
        .map_err(|error| error.to_string())
}

/// Gives the window the width the frontend's layout asks for (§7.6, WIN-02).
///
/// The settings page, the expanded view and the panel are one decision on the other side —
/// `App.svelte` derives it in one place — so what crosses here is the answer and never a
/// flag per view. Three things happen, in this order:
///
/// 1. the layout is recorded, so the next [`resize_to_content`] keeps the same width;
/// 2. the window is **moved** if the width changed, so that the edge the user aimed at stays
///    where it is and the window cannot grow off its screen ([`window::anchored_position`]);
/// 3. the width is applied, with the height untouched — the frontend remeasures its content
///    a moment later and [`resize_to_content`] applies the new one, so doing it here as well
///    would be one visible jump more than necessary.
///
/// # Errors
///
/// The window manager's message when it refuses the size.
#[tauri::command]
pub fn set_window_layout(window: Window, layout: WindowLayout) -> Result<(), String> {
    let app = window.app_handle();
    let ui = app.state::<Ui>();
    let before = ui.window_width();
    ui.set_layout(layout);
    let width = layout.width();
    // Only a real change moves the window: a repeated call for the layout already in force
    // would otherwise clamp a panel the user had deliberately dragged half off the screen.
    if (width - before).abs() > f64::EPSILON {
        anchor_to_width(&window, width);
    }
    window
        .set_size(LogicalSize::new(width, current_logical_height(&window)))
        .map_err(|error| error.to_string())
}

/// Moves the window so that a width change keeps the edge nearest the screen's edge (§7.6).
///
/// Everything here is the plumbing: reading the monitor, the position and the size, turning
/// the logical width into the physical pixels the position is in, and putting the window
/// where [`window::anchored_position`] says. The arithmetic itself is that function, which is
/// pure and unit-tested. A window with no monitor, no position or no size is left alone —
/// there is nothing sensible to compute against, and the size change below still happens.
fn anchor_to_width(window: &Window, next_logical_width: f64) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let (Ok(position), Ok(size)) = (window.outer_position(), window.outer_size()) else {
        return;
    };
    let scale = monitor.scale_factor();
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let physical = next_logical_width * scale;
    if !physical.is_finite() || physical < 1.0 {
        return;
    }
    let next = window::anchored_position(
        window::Rect {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        },
        window::work_area_of(&monitor),
        physical.round() as u32,
    );
    if let Err(error) = window.set_position(PhysicalPosition::new(next.x, next.y)) {
        tracing::warn!(error = %error, "the overlay window refused the anchored position");
    }
}

/// How tall the window is now, in the logical units [`LogicalSize`] takes.
fn current_logical_height(window: &Window) -> f64 {
    let scale = window.scale_factor().unwrap_or(1.0);
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    window
        .outer_size()
        .map_or(MIN_WINDOW_HEIGHT, |size| f64::from(size.height) / scale)
}

/// Records the language the frontend resolved and relabels the tray menu (APP-02, §7.16).
#[tauri::command]
pub fn set_ui_language(app: AppHandle, language: Language) -> Result<(), String> {
    let state = app.state::<Ui>();
    *state
        .language
        .lock()
        .expect("the UI language mutex is poisoned") = language;

    let tray = state.tray.lock().expect("the tray mutex is poisoned");
    match tray.as_ref() {
        Some(handles) => handles.relabel(language).map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

/// The height of the monitor the window is on, in the window's own logical units.
fn monitor_height(window: &Window) -> Option<f64> {
    let monitor = window.current_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    if scale <= 0.0 {
        return None;
    }
    Some(f64::from(monitor.size().height) / scale)
}

/// What the window may actually become: never smaller than the header, never taller than
/// the screen, and never anything at all for a measurement that is not a number.
fn clamp_height(requested: f64, monitor_height: Option<f64>) -> f64 {
    if !requested.is_finite() {
        return MIN_WINDOW_HEIGHT;
    }
    let max = monitor_height
        .filter(|value| value.is_finite())
        .unwrap_or(FALLBACK_MAX_WINDOW_HEIGHT)
        .max(MIN_WINDOW_HEIGHT);
    requested.clamp(MIN_WINDOW_HEIGHT, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The configuration the application actually ships with.
    const TAURI_CONF: &str = include_str!("../../tauri.conf.json");

    /// The frontend half of the bridge, read as text so the two sides can be compared.
    const BRIDGE_TS: &str = include_str!("../../../src/bridge.ts");

    /// The startup file, for the one list of commands the application registers.
    const LIB_RS: &str = include_str!("../lib.rs");

    fn main_window_config() -> serde_json::Value {
        let config: serde_json::Value =
            serde_json::from_str(TAURI_CONF).expect("tauri.conf.json is valid JSON");
        config["app"]["windows"]
            .as_array()
            .expect("the configuration declares windows")
            .iter()
            .find(|window| window["label"] == MAIN_WINDOW)
            .expect("the configuration declares the main window")
            .clone()
    }

    #[test]
    fn the_fixed_width_is_the_one_the_window_is_created_with() {
        // WIN-02 fixes the width, and it is written in two places: the window the
        // configuration creates and the size every resize keeps. They must agree, or the
        // panel would jump the first time the content is measured.
        assert_eq!(main_window_config()["width"].as_f64(), Some(WINDOW_WIDTH));
    }

    #[test]
    fn every_layout_has_its_own_width_and_the_panel_is_the_default() {
        // §7.6: neither the settings page nor the expanded view is remembered, so a fresh
        // `Ui` is at the fixed width of WIN-02 and coming back to it is what leaving either
        // of them does.
        let ui = Ui::default();
        assert_eq!(ui.layout(), WindowLayout::Panel);
        assert_eq!(ui.window_width(), WINDOW_WIDTH);
        for layout in [
            WindowLayout::Settings,
            WindowLayout::Expanded,
            WindowLayout::Panel,
        ] {
            ui.set_layout(layout);
            assert_eq!(ui.layout(), layout);
            assert_eq!(ui.window_width(), layout.width());
        }
        assert_eq!(ui.window_width(), WINDOW_WIDTH);
        const { assert!(SETTINGS_WINDOW_WIDTH > WINDOW_WIDTH) };
        const { assert!(EXPANDED_WINDOW_WIDTH > SETTINGS_WINDOW_WIDTH) };
    }

    #[test]
    fn the_three_layouts_are_spelled_the_way_the_frontend_spells_them() {
        // The word itself crosses the bridge, so a rename on one side alone would leave the
        // window asking for a layout the command cannot deserialise — and the panel would
        // never widen again, silently.
        for (layout, name) in [
            (WindowLayout::Panel, "panel"),
            (WindowLayout::Settings, "settings"),
            (WindowLayout::Expanded, "expanded"),
        ] {
            assert_eq!(
                serde_json::to_value(layout).expect("a layout serialises"),
                serde_json::Value::String(name.to_owned())
            );
            assert!(
                BRIDGE_TS.contains(&format!("'{name}'")),
                "src/bridge.ts does not mention the layout {name}"
            );
        }
    }

    #[test]
    fn the_window_the_tray_shows_is_the_one_the_configuration_declares() {
        assert_eq!(main_window_config()["label"].as_str(), Some(MAIN_WINDOW));
    }

    #[test]
    fn the_frontend_listens_to_the_events_this_module_emits() {
        // The two sides spell the event name independently; a rename on one side alone
        // would leave the tray's `New request` doing nothing at all, silently — and the
        // panel would never collapse again, just as quietly.
        for event in [EVENT_SHOW_VIEW, EVENT_WINDOW_FOCUS] {
            assert!(
                BRIDGE_TS.contains(event),
                "src/bridge.ts does not mention {event}"
            );
        }
    }

    #[test]
    fn every_command_the_window_invokes_is_one_the_application_registers() {
        // The gap nothing else can see. `invoke('delete_log_entrie')` compiles, type-checks,
        // passes every component test against a fake bridge, and fails only in a running
        // window with "command not found" — the frontend names a command by a string and
        // `generate_handler!` registers it by a path, and no compiler joins the two.
        //
        // Both sides are read as text here: the names inside `invoke(...)` in `bridge.ts`,
        // and the last segment of every path in the `invoke_handler` list of `lib.rs`.
        let invoked = regex::Regex::new(r"invoke(?:::<[^>]*>|<[^>]*>)?\(\s*'([a-z0-9_]+)'")
            .expect("a valid pattern");
        let called: Vec<&str> = invoked
            .captures_iter(BRIDGE_TS)
            .map(|found| found.get(1).expect("the name group").as_str())
            .collect();
        assert!(
            called.len() > 20,
            "the scan found almost nothing, so it is the scan that is broken: {called:?}"
        );

        let list = LIB_RS
            .split_once("generate_handler![")
            .expect("lib.rs registers commands")
            .1
            .split_once(']')
            .expect("the list is closed")
            .0;
        let registered: Vec<&str> = list
            .split(',')
            .filter_map(|entry| entry.trim().rsplit("::").next())
            .filter(|name| !name.is_empty())
            .collect();

        for name in called {
            assert!(
                registered.contains(&name),
                "src/bridge.ts invokes `{name}`, which lib.rs does not register"
            );
        }
    }

    #[test]
    fn a_measured_height_is_taken_as_it_is_when_the_screen_can_show_it() {
        assert_eq!(clamp_height(300.0, Some(1080.0)), 300.0);
        assert_eq!(clamp_height(64.0, Some(1080.0)), 64.0);
    }

    #[test]
    fn a_height_beyond_the_screen_is_cut_to_the_screen() {
        assert_eq!(clamp_height(4000.0, Some(1080.0)), 1080.0);
        assert_eq!(clamp_height(4000.0, None), FALLBACK_MAX_WINDOW_HEIGHT);
    }

    #[test]
    fn a_window_never_becomes_too_small_to_grab() {
        assert_eq!(clamp_height(0.0, Some(1080.0)), MIN_WINDOW_HEIGHT);
        assert_eq!(clamp_height(-10.0, Some(1080.0)), MIN_WINDOW_HEIGHT);
        // A monitor smaller than the minimum would otherwise make `clamp` panic.
        assert_eq!(clamp_height(10.0, Some(20.0)), MIN_WINDOW_HEIGHT);
    }

    #[test]
    fn a_measurement_that_is_not_a_number_leaves_the_window_usable() {
        assert_eq!(clamp_height(f64::NAN, Some(1080.0)), MIN_WINDOW_HEIGHT);
        assert_eq!(clamp_height(f64::INFINITY, Some(1080.0)), MIN_WINDOW_HEIGHT);
        assert_eq!(clamp_height(300.0, Some(f64::NAN)), 300.0);
    }
}
