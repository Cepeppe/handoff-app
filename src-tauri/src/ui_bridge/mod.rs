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

pub mod commands;
pub mod events;
mod tray;
pub mod view;

use std::sync::Mutex;

use tauri::{AppHandle, Emitter as _, LogicalSize, Manager as _, Window, WindowEvent};

use crate::i18n::Language;

pub use commands::{Core, CoreState};
pub use events::{Notice, NoticeKind, Notifier};

/// The label of the overlay window, as `tauri.conf.json` declares it.
pub const MAIN_WINDOW: &str = "main";

/// The event that asks the frontend to bring a view forward (§7.6).
///
/// The tray menu is outside the component tree, so `New request` and `Settings` cannot
/// switch the view by calling into it; they emit this instead. The same literal is in
/// `src/bridge.ts` and a test below reads that file to keep the two spellings together.
pub const EVENT_SHOW_VIEW: &str = "ui://show-view";

/// The fixed width of the panel (WIN-02, §7.6). `tauri.conf.json` declares the same value.
pub const WINDOW_WIDTH: f64 = 360.0;

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

    /// The language the interface is showing.
    pub fn language(&self) -> Language {
        *self
            .language
            .lock()
            .expect("the UI language mutex is poisoned")
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

    // §7.16 keeps the window hidden until there is something to show, and from here on the
    // tray is what brings it back. A development build still shows it once: onboarding is
    // what will open the window on a first launch, and until that exists every run of
    // `cargo tauri dev` would otherwise show nothing but a tray icon.
    // TASK: T-040 — delete this when onboarding decides the first-launch window.
    #[cfg(debug_assertions)]
    show_main_window(app);

    Ok(())
}

/// Closing the window hides it to the tray; only the tray's `Quit` ends the process (WIN-04).
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        // A window that cannot hide is a window the user just failed to close; there is
        // nothing to do about it here beyond leaving it open.
        if let Err(error) = window.hide() {
            tracing::warn!(error = %error, "the overlay window refused to hide");
        }
    }
}

/// Brings the overlay window to the front (the tray's `Show`, and the second launch of
/// APP-01's single instance).
pub fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        tracing::warn!(window = MAIN_WINDOW, "no window to show");
        return;
    };
    if let Err(error) = window.show() {
        tracing::warn!(error = %error, "the overlay window refused to show");
    }
    if let Err(error) = window.set_focus() {
        tracing::warn!(error = %error, "the overlay window refused the focus");
    }
}

/// Shows the window and asks the frontend for one of its views.
fn show_view(app: &AppHandle, view: &str) {
    show_main_window(app);
    if let Err(error) = app.emit(EVENT_SHOW_VIEW, view) {
        tracing::warn!(error = %error, "the view request reached no window");
    }
}

/// Gives the window the height the content measured, keeping the fixed width (WIN-02).
#[tauri::command]
pub fn resize_to_content(window: Window, height: f64) -> Result<(), String> {
    let limit = monitor_height(&window);
    let clamped = clamp_height(height, limit);
    window
        .set_size(LogicalSize::new(WINDOW_WIDTH, clamped))
        .map_err(|error| error.to_string())
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
    fn the_window_the_tray_shows_is_the_one_the_configuration_declares() {
        assert_eq!(main_window_config()["label"].as_str(), Some(MAIN_WINDOW));
    }

    #[test]
    fn the_frontend_listens_to_the_event_this_module_emits() {
        // The two sides spell the event name independently; a rename on one side alone
        // would leave the tray's `New request` doing nothing at all, silently.
        assert!(
            BRIDGE_TS.contains(EVENT_SHOW_VIEW),
            "src/bridge.ts does not mention {EVENT_SHOW_VIEW}"
        );
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
