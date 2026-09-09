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

pub mod commands;
pub mod events;
pub mod general;
pub mod install;
pub mod requests;
pub mod shortcut;
mod tray;
pub mod view;
pub mod window;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter as _, LogicalSize, Manager as _, Window, WindowEvent};

use crate::i18n::Language;
use crate::log::Db;

pub use commands::{Core, CoreState};
pub use events::{Notice, NoticeKind, Notifier};
pub use requests::RequestDelivery;

/// The label of the overlay window, as `tauri.conf.json` declares it.
pub const MAIN_WINDOW: &str = "main";

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
    wide: AtomicBool,
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

    /// The width the panel should have right now (§7.6).
    ///
    /// One of two values, never a remembered one: the fixed width of WIN-02, or the wider
    /// layout while the settings page is open.
    pub fn window_width(&self) -> f64 {
        if self.wide.load(Ordering::Relaxed) {
            SETTINGS_WINDOW_WIDTH
        } else {
            WINDOW_WIDTH
        }
    }

    /// Opens or closes the wider layout. The next resize is what the user sees.
    pub fn set_wide(&self, wide: bool) {
        self.wide.store(wide, Ordering::Relaxed);
    }

    /// The global shortcut in force, and whether the system accepted it (OPEN-03, FM-18).
    pub fn shortcut(&self) -> &shortcut::State {
        &self.shortcut
    }

    /// Runs `read` against the window's connection, or answers `None` when there is none.
    pub fn with_db<T>(&self, read: impl FnOnce(&Db) -> T) -> Option<T> {
        let guard = self.db.lock().expect("the settings mutex is poisoned");
        guard.as_ref().map(read)
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
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let app = window.app_handle();
    let ui = app.state::<Ui>();
    match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            ui.geometry().remember(window);
            ui.flush_geometry();
            // A window that cannot hide is a window the user just failed to close; there is
            // nothing to do about it here beyond leaving it open.
            if let Err(error) = window.hide() {
                tracing::warn!(error = %error, "the overlay window refused to hide");
            }
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

/// Widens the panel for the settings page, and narrows it back when the page closes (§7.6).
///
/// The height is kept as it is: the frontend remeasures its content a moment later and
/// [`resize_to_content`] applies the new one, so doing it here as well would be one visible
/// jump more than necessary.
///
/// # Errors
///
/// The window manager's message when it refuses the size.
#[tauri::command]
pub fn set_wide_layout(window: Window, wide: bool) -> Result<(), String> {
    let ui = window.app_handle().state::<Ui>();
    ui.set_wide(wide);
    let width = ui.window_width();
    window
        .set_size(LogicalSize::new(width, current_logical_height(&window)))
        .map_err(|error| error.to_string())
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
    fn the_settings_layout_is_wider_than_the_panel_and_the_panel_is_the_default() {
        // §7.6: the wider layout is temporary, so a fresh `Ui` is at the fixed width of
        // WIN-02 and going back to it is what closing the settings page does.
        let ui = Ui::default();
        assert_eq!(ui.window_width(), WINDOW_WIDTH);
        ui.set_wide(true);
        assert_eq!(ui.window_width(), SETTINGS_WINDOW_WIDTH);
        ui.set_wide(false);
        assert_eq!(ui.window_width(), WINDOW_WIDTH);
        const { assert!(SETTINGS_WINDOW_WIDTH > WINDOW_WIDTH) };
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
