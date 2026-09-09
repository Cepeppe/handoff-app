//! The Screenshot button, from the press to the preview (§7.8, CAP-01..04, DD-29).
//!
//! `crate::capture` produces pixels and knows nothing about Tauri; this is everything
//! around it that only Tauri can do:
//!
//! - **the choice.** CAP-01 is emphatic that the button never starts a capture by itself:
//!   two options appear, the last one is highlighted, and the user picks every time. The
//!   highlight is the only thing remembered, in the settings table, so it survives a
//!   restart the way every other preference does.
//! - **hiding the panel** (CAP-03). The overlay is always on top, so a capture taken with
//!   it on screen would have it in the middle of the shot. It is hidden before, put back
//!   after, and put back on every failure path as well — a panel that vanished because a
//!   capture went wrong is a bug the user cannot get out of.
//! - **the region overlays** (DD-29). One transparent, non-click-through window per
//!   monitor, created for the selection and destroyed the moment it ends. They are capture
//!   *tools*, not handoff windows, which is what makes them compatible with MULTI-04's
//!   "never two windows": nothing about a handoff is ever drawn in one, and none of them
//!   outlives the drag.
//! - **the pixels reaching the webview.** A capture is a few megabytes of PNG, so it
//!   crosses as a raw IPC response — bytes, not a base64 string in a JSON document — and
//!   the preview turns it into a blob URL. That is the reason `img-src` in the CSP names
//!   `blob:` and nothing else.
//!
//! # Why the outcome travels as an event rather than as the command's answer
//!
//! A region selection is started by one window and finished by another: the drag ends in a
//! selection overlay, which is destroyed as part of handling it, so its `invoke` promise
//! never settles. Reporting through [`EVENT_CAPTURE_READY`] instead gives both flows one
//! shape and the frontend one listener.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter as _, Manager as _, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindowBuilder,
};

use crate::capture::permission::{self, Permission};
use crate::capture::{self, Backend, Capture, CaptureError, MonitorId, Point, Selection};
use crate::log::settings;

use super::{Ui, MAIN_WINDOW};

/// A capture is ready to be previewed, or could not be taken (§7.8, FM-17).
pub const EVENT_CAPTURE_READY: &str = "ui://capture-ready";

/// The settings key holding the choice to highlight next time (CAP-01).
pub const LAST_CHOICE_KEY: &str = "capture.last_choice";

/// The label every selection overlay's own is prefixed with, and the monitor it covers.
const SELECTION_PREFIX: &str = "capture-selection-";

/// How long the compositor is given to actually take the overlay off the screen.
///
/// `hide()` returns as soon as the request is posted; the window is gone one frame later,
/// and a capture taken before that frame has the panel in it. The value is a compromise
/// nobody can compute — long enough for a 60 Hz compositor with a frame to spare, short
/// enough that the user does not notice a pause between the click and the preview — and it
/// is the only wait in the whole flow, which is why the commands here are `async`: the
/// event loop that has to process the hide is the main thread, so this must not run on it.
const HIDE_SETTLE: Duration = Duration::from_millis(160);

/// Which of the two the user picked (CAP-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Choice {
    /// The monitor the cursor is on (CAP-02).
    FullScreen,
    /// A rectangle dragged over the transparent overlays (CAP-02, DD-29).
    Region,
}

/// What the popover needs to draw itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSettings {
    /// The choice to highlight, or `None` on a machine that has never taken one.
    pub last_choice: Option<Choice>,
}

/// What one selection overlay has to know about itself.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionSetup {
    /// The monitor this window covers, which travels back with the drag.
    pub monitor: MonitorId,
    /// Its scale factor, for the "1280 × 720" label: the numbers a user reads are the
    /// pixels the image will have, not the CSS pixels the drag was measured in.
    pub scale_factor: f64,
}

/// How a press of the Screenshot button ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum Outcome {
    /// The pixels are held and the preview may fetch them.
    Ready {
        width: u32,
        height: u32,
        monitor: MonitorId,
    },
    /// macOS has not granted the screen-recording permission (FM-17, CAP-04).
    Denied,
    /// Anything else. The message is the platform's, shown as it is.
    Failed { message: String },
}

/// What this module remembers between a capture and the preview that draws it.
///
/// Three facts, all of them about the *flow* and none about the screen: the pixels waiting
/// to be shown, whether the panel has to be put back, and whether a selection is on screen.
/// `crate::capture` itself holds nothing at all (PRIN-04, NFR-03).
#[derive(Default)]
pub struct State {
    ready: Mutex<Option<Capture>>,
    restore_overlay: AtomicBool,
    selecting: AtomicBool,
}

impl State {
    /// Keeps a capture until the preview asks for it.
    fn hold(&self, capture: Capture) {
        *self.ready.lock().expect("the capture mutex is poisoned") = Some(capture);
    }

    /// The capture waiting to be drawn, encoded as a PNG.
    fn png(&self) -> Option<Result<Vec<u8>, CaptureError>> {
        self.ready
            .lock()
            .expect("the capture mutex is poisoned")
            .as_ref()
            .map(Capture::to_png)
    }

    /// Drops the pixels. Called when the preview is left, so a screenshot the user decided
    /// against does not sit in memory until the next one replaces it.
    fn forget(&self) {
        *self.ready.lock().expect("the capture mutex is poisoned") = None;
    }

    /// Whether a region selection is on screen (the e2e channel and the tests ask).
    #[must_use]
    pub fn is_selecting(&self) -> bool {
        self.selecting.load(Ordering::Relaxed)
    }
}

/// The backend a capture runs on.
///
/// The fixture backend replaces the screen entirely when the e2e channel has named a
/// fixture (`--features fake-capture`, which `--features e2e` turns on). In every other
/// build, and in an `e2e` build where no scenario asked for one, it is the real screen.
fn backend(app: &AppHandle) -> Box<dyn Backend> {
    #[cfg(feature = "fake-capture")]
    {
        use std::path::PathBuf;

        use crate::capture::fake::{FakeCapture, FIXTURE_KEY, FIXTURE_SCALE_KEY};
        let ui = app.state::<Ui>();
        let fixture: Option<PathBuf> = ui
            .with_db(|db| settings::get::<PathBuf>(db, FIXTURE_KEY).ok().flatten())
            .flatten();
        if let Some(fixture) = fixture {
            let scale = ui
                .with_db(|db| settings::get::<f64>(db, FIXTURE_SCALE_KEY).ok().flatten())
                .flatten()
                .unwrap_or(1.0);
            return Box::new(FakeCapture::new(fixture, scale));
        }
    }
    let _ = app;
    Box::new(capture::XcapBackend::new())
}

/// The choice to highlight next time (CAP-01).
#[tauri::command]
pub fn capture_settings(app: AppHandle) -> CaptureSettings {
    let last_choice = app
        .state::<Ui>()
        .with_db(|db| settings::get::<Choice>(db, LAST_CHOICE_KEY).ok().flatten())
        .flatten();
    CaptureSettings { last_choice }
}

/// Records which of the two the user just picked. A write that fails costs a highlight.
fn remember(app: &AppHandle, choice: Choice) {
    let written = app
        .state::<Ui>()
        .with_db(|db| settings::set(db, LAST_CHOICE_KEY, &choice));
    if let Some(Err(error)) = written {
        tracing::warn!(error = %error, "the last capture choice was not remembered");
    }
}

/// Hides the overlay for the length of a capture (CAP-03), remembering whether to put it
/// back. Called once per capture, including at the start of a region selection.
fn hide_overlay(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    app.state::<Ui>()
        .capture
        .restore_overlay
        .store(visible, Ordering::Relaxed);
    if visible {
        if let Err(error) = window.hide() {
            tracing::warn!(error = %error, "the overlay would not hide for a capture");
        }
    }
}

/// Puts the panel back if it was on screen when the capture started.
fn restore_overlay(app: &AppHandle) {
    if !app
        .state::<Ui>()
        .capture
        .restore_overlay
        .swap(false, Ordering::Relaxed)
    {
        return;
    }
    super::show_main_window(app);
}

/// Where the cursor is, in the coordinates the monitors are in (CAP-02).
///
/// A platform that will not say falls back to the origin, which [`capture::monitor_at`]
/// resolves to the primary monitor: a full-screen capture of the main screen is the
/// degraded answer, and no capture at all is not (PRIN-10).
fn cursor(app: &AppHandle) -> Point {
    app.cursor_position().map_or(Point::new(0, 0), |at| {
        Point::new(at.x.round() as i32, at.y.round() as i32)
    })
}

/// Holds the capture and tells the window about it, whichever way it went.
fn report(app: &AppHandle, taken: Result<Capture, CaptureError>) {
    let outcome = match taken {
        Ok(capture) => {
            let (width, height) = capture.image.dimensions();
            let monitor = capture.origin_monitor;
            app.state::<Ui>().capture.hold(capture);
            Outcome::Ready {
                width,
                height,
                monitor,
            }
        }
        Err(CaptureError::EmptyRegion) => {
            // A click with no drag, or a rectangle that landed on no screen. There is
            // nothing to show and nothing went wrong: the panel is already back.
            return;
        }
        Err(CaptureError::PermissionDenied) => Outcome::Denied,
        Err(error) => {
            tracing::warn!(error = %error, "a capture failed");
            Outcome::Failed {
                message: error.to_string(),
            }
        }
    };
    if let Err(error) = app.emit_to(MAIN_WINDOW, EVENT_CAPTURE_READY, outcome) {
        tracing::warn!(error = %error, "the capture reached no window");
    }
}

/// The whole screen the cursor is on (CAP-01 **Full screen**, CAP-02).
///
/// # Errors
///
/// Never: what went wrong travels on [`EVENT_CAPTURE_READY`], so both flows report the same
/// way. The signature keeps the shape every other command has.
#[tauri::command]
pub async fn capture_full_screen(app: AppHandle) -> Result<(), String> {
    remember(&app, Choice::FullScreen);
    if permission::screen_recording() != Permission::Granted {
        report(&app, Err(CaptureError::PermissionDenied));
        return Ok(());
    }
    hide_overlay(&app);
    tokio::time::sleep(HIDE_SETTLE).await;
    let taken = capture::full_screen(backend(&app).as_ref(), cursor(&app));
    restore_overlay(&app);
    report(&app, taken);
    Ok(())
}

/// Opens one transparent overlay per monitor and waits for the drag (CAP-01 **Select
/// region**, CAP-02, DD-29).
///
/// # Errors
///
/// The message of a window that could not be created. The overlays already created are
/// destroyed and the panel comes back, so a partial selection screen is never left behind.
#[tauri::command]
pub async fn start_region_capture(app: AppHandle) -> Result<(), String> {
    remember(&app, Choice::Region);
    if permission::screen_recording() != Permission::Granted {
        report(&app, Err(CaptureError::PermissionDenied));
        return Ok(());
    }
    let monitors = match backend(&app).list_monitors() {
        Ok(monitors) => monitors,
        Err(error) => {
            report(&app, Err(error));
            return Ok(());
        }
    };
    hide_overlay(&app);
    app.state::<Ui>()
        .capture
        .selecting
        .store(true, Ordering::Relaxed);

    for monitor in &monitors {
        let label = format!("{SELECTION_PREFIX}{}", monitor.id);
        // The query is what tells the frontend to mount the selection overlay rather than
        // the application; everything else about the monitor comes back from
        // [`selection_setup`], which reads the label, so there is one source for it.
        let url = format!("index.html?selection={}", monitor.id);
        let built = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
            .title("")
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .visible(false)
            .build();
        let window = match built {
            Ok(window) => window,
            Err(error) => {
                close_selection(&app);
                restore_overlay(&app);
                return Err(error.to_string());
            }
        };
        // The position and the size are applied afterwards, in physical pixels: the
        // builder takes logical ones, and "logical" is meaningless before the window is on
        // a monitor whose scale factor decides what a logical pixel is.
        let placed = window
            .set_position(PhysicalPosition::new(monitor.bounds.x, monitor.bounds.y))
            .and_then(|()| {
                window.set_size(PhysicalSize::new(
                    monitor.bounds.width,
                    monitor.bounds.height,
                ))
            })
            .and_then(|()| window.show());
        if let Err(error) = placed {
            close_selection(&app);
            restore_overlay(&app);
            return Err(error.to_string());
        }
    }

    // The overlay of the monitor the cursor is on takes the focus, so Esc reaches a window
    // without the user having to click first.
    let under_cursor = capture::monitor_at(&monitors, cursor(&app)).map(|monitor| monitor.id);
    if let Some(window) =
        under_cursor.and_then(|id| app.get_webview_window(&format!("{SELECTION_PREFIX}{id}")))
    {
        if let Err(error) = window.set_focus() {
            tracing::warn!(error = %error, "a selection overlay refused the focus");
        }
    }
    Ok(())
}

/// What the overlay this window is needs to know about itself.
///
/// # Errors
///
/// When the window's label is not one of ours, which cannot happen from a selection
/// overlay and is the honest answer to anything else asking.
#[tauri::command]
pub fn selection_setup(window: tauri::Window) -> Result<SelectionSetup, String> {
    let monitor: MonitorId = window
        .label()
        .strip_prefix(SELECTION_PREFIX)
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| format!("{} is not a selection overlay", window.label()))?;
    let scale_factor = backend(window.app_handle())
        .list_monitors()
        .ok()
        .and_then(|monitors| {
            capture::monitor_with(&monitors, monitor).map(crate::capture::Monitor::usable_scale)
        })
        .unwrap_or(1.0);
    Ok(SelectionSetup {
        monitor,
        scale_factor,
    })
}

/// The user let go of the mouse: close the overlays and crop what they drew (CAP-02).
///
/// # Errors
///
/// Never; the outcome travels on [`EVENT_CAPTURE_READY`].
#[tauri::command]
pub async fn region_captured(app: AppHandle, selection: Selection) -> Result<(), String> {
    close_selection(&app);
    tokio::time::sleep(HIDE_SETTLE).await;
    let taken = capture::region(backend(&app).as_ref(), selection);
    restore_overlay(&app);
    report(&app, taken);
    Ok(())
}

/// Esc, or a drag that selected nothing: no capture, and the panel comes back (CAP-01).
///
/// # Errors
///
/// Never.
#[tauri::command]
pub fn cancel_region_capture(app: AppHandle) -> Result<(), String> {
    close_selection(&app);
    restore_overlay(&app);
    Ok(())
}

/// The capture the preview draws, as PNG bytes.
///
/// # Errors
///
/// When there is nothing to draw, or when the encoder refused. Both are sentences the
/// preview shows in place of the image.
#[tauri::command]
pub fn capture_preview(app: AppHandle) -> Result<tauri::ipc::Response, String> {
    app.state::<Ui>()
        .capture
        .png()
        .ok_or_else(|| "there is no capture to show".to_owned())?
        .map(tauri::ipc::Response::new)
        .map_err(|error| error.to_string())
}

/// Drops the pixels the preview was showing (PRIN-04: nothing is kept between captures).
///
/// # Errors
///
/// Never.
#[tauri::command]
pub fn discard_capture(app: AppHandle) -> Result<(), String> {
    app.state::<Ui>().capture.forget();
    Ok(())
}

/// Destroys every selection overlay, whatever state the selection was in.
///
/// `destroy` rather than `close`: a close request goes through the window event handler,
/// which is written for the one window the user can close, and would ask this one to hide
/// itself instead of going away.
fn close_selection(app: &AppHandle) {
    app.state::<Ui>()
        .capture
        .selecting
        .store(false, Ordering::Relaxed);
    for (label, window) in app.webview_windows() {
        if !label.starts_with(SELECTION_PREFIX) {
            continue;
        }
        if let Err(error) = window.destroy() {
            tracing::warn!(error = %error, label = %label, "a selection overlay would not close");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frontend half of the bridge, so the two spellings of the event and of the
    /// commands stay together.
    const BRIDGE_TS: &str = include_str!("../../../src/bridge.ts");

    #[test]
    fn the_two_choices_round_trip_through_the_settings_table() {
        // They are stored as JSON, so the spelling is part of the file format: a rename
        // would silently lose every user's highlight.
        assert_eq!(
            serde_json::to_string(&Choice::FullScreen).expect("a choice serialises"),
            "\"fullScreen\""
        );
        assert_eq!(
            serde_json::to_string(&Choice::Region).expect("a choice serialises"),
            "\"region\""
        );
        assert_eq!(
            serde_json::from_str::<Choice>("\"region\"").expect("a choice reads back"),
            Choice::Region
        );
    }

    #[test]
    fn the_outcome_says_which_of_the_three_it_is() {
        let ready = serde_json::to_value(Outcome::Ready {
            width: 800,
            height: 600,
            monitor: 3,
        })
        .expect("an outcome serialises");
        assert_eq!(ready["status"], "ready");
        assert_eq!(ready["width"], 800);
        assert_eq!(ready["monitor"], 3);

        assert_eq!(
            serde_json::to_value(Outcome::Denied).expect("an outcome serialises")["status"],
            "denied"
        );
        let failed = serde_json::to_value(Outcome::Failed {
            message: "no monitor answered".to_owned(),
        })
        .expect("an outcome serialises");
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["message"], "no monitor answered");
    }

    #[test]
    fn the_frontend_listens_to_the_event_this_module_emits() {
        assert!(
            BRIDGE_TS.contains(EVENT_CAPTURE_READY),
            "src/bridge.ts does not mention {EVENT_CAPTURE_READY}"
        );
    }

    #[test]
    fn a_selection_overlays_label_names_its_monitor() {
        // The label is the only channel between the window and the drag it reports, so the
        // two halves of the format are pinned together here.
        let label = format!("{SELECTION_PREFIX}{}", 7_u32);
        assert_eq!(label, "capture-selection-7");
        assert_eq!(
            label
                .strip_prefix(SELECTION_PREFIX)
                .and_then(|id| id.parse::<MonitorId>().ok()),
            Some(7)
        );
        assert_eq!(MAIN_WINDOW.strip_prefix(SELECTION_PREFIX), None);
    }

    #[test]
    fn nothing_is_held_before_a_capture_and_the_panel_is_not_owed_a_restore() {
        let state = State::default();
        assert!(state.png().is_none());
        assert!(!state.is_selecting());
        assert!(!state.restore_overlay.load(Ordering::Relaxed));
    }

    #[test]
    fn the_held_capture_is_dropped_when_the_preview_is_left() {
        use crate::log::time::Timestamp;
        let state = State::default();
        state.hold(Capture {
            image: image::RgbaImage::new(2, 2),
            origin_monitor: 1,
            taken_at: Timestamp::now(),
        });
        assert!(state.png().is_some());
        state.forget();
        assert!(
            state.png().is_none(),
            "a screenshot the user decided against must not outlive the preview"
        );
    }
}
