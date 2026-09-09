//! The global shortcut that opens the request sheet (§7.16, OPEN-03, FM-18, A-19).
//!
//! One combination, registered at startup, that shows the window in request mode from
//! wherever the user is. `Ctrl`+`Alt`+`H` on Windows and `⌃⌥H` on macOS are the same two
//! modifiers and the same key, so one accelerator string covers both.
//!
//! # Never stealing one, and asking once (FM-18)
//!
//! The platform refuses a combination another application already holds — `RegisterHotKey`
//! fails on Windows, and the plugin reports it (A-19) — so "never steal it silently" is not
//! a rule this module has to enforce: it is what the failure *is*. What is left is what
//! OPEN-03 asks for on top: the app notices at startup and asks **once** for another
//! combination. Once is a setting ([`ASKED_KEY`]), because a dialog that comes back at every
//! launch is a dialog a user learns to dismiss without reading, and the tray's
//! `New request` is always there in the meantime.
//!
//! # What is stored
//!
//! Only a combination the user chose ([`ACCELERATOR_KEY`]). An empty setting means the
//! default, so a user who never had a problem has nothing written and inherits any later
//! change of the default. The accelerator is the plugin's own syntax
//! (`Control+Alt+H`, `CmdOrCtrl+Shift+K`), parsed by it and never by us.

use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use tauri::{AppHandle, Manager as _};
use tauri_plugin_global_shortcut::{GlobalShortcutExt as _, ShortcutState};

use crate::log::settings;

/// The combination of OPEN-03: `Ctrl`+`Alt`+`H` on Windows, `⌃⌥H` on macOS.
pub const DEFAULT_ACCELERATOR: &str = "Control+Alt+H";

/// The setting holding a combination the user chose instead of the default.
pub const ACCELERATOR_KEY: &str = "shortcut.accelerator";

/// The setting that remembers the "choose another combination" dialog was already offered.
pub const ASKED_KEY: &str = "shortcut.asked";

/// What the window is told about the shortcut (§7.16, FM-18).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// The combination in force, in the plugin's accelerator syntax.
    pub accelerator: String,
    /// Whether it is actually registered with the system.
    pub registered: bool,
    /// Whether the one-time dialog of FM-18 should be shown now.
    pub ask_for_another: bool,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            accelerator: DEFAULT_ACCELERATOR.to_owned(),
            registered: false,
            ask_for_another: false,
        }
    }
}

/// The shortcut as `Ui` holds it: one combination, and whether it took.
#[derive(Debug, Default)]
pub struct State {
    status: Mutex<Status>,
}

impl State {
    /// What the window should draw.
    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set(&self, status: Status) {
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = status;
    }
}

/// Registers the shortcut at startup, and decides whether to ask for another one (OPEN-03).
///
/// Called from `setup()`, once. A failure is not an error the user has to acknowledge here:
/// the tray's `New request` is the always-available path (FM-18) and the dialog is drawn by
/// the window when it mounts.
pub fn install(app: &AppHandle) {
    let ui = app.state::<super::Ui>();
    let chosen: Option<String> = ui
        .with_db(|db| settings::get::<String>(db, ACCELERATOR_KEY).ok().flatten())
        .flatten()
        .filter(|accelerator| !accelerator.trim().is_empty());
    let accelerator = chosen.unwrap_or_else(|| DEFAULT_ACCELERATOR.to_owned());

    let registered = match bind(app, &accelerator) {
        Ok(()) => {
            tracing::info!(accelerator, "the global shortcut is registered");
            true
        }
        Err(error) => {
            tracing::warn!(
                error,
                accelerator,
                "the global shortcut could not be registered"
            );
            false
        }
    };
    // FM-18: ask once, and only when there is something to ask about.
    let already_asked = ui
        .with_db(|db| settings::get::<bool>(db, ASKED_KEY).ok().flatten())
        .flatten()
        .unwrap_or(false);

    ui.shortcut().set(Status {
        accelerator,
        registered,
        ask_for_another: !registered && !already_asked,
    });
}

/// Puts `accelerator` in force, replacing whatever was registered (OPEN-03, FM-18).
///
/// The old combination is released first: leaving it registered would hold a shortcut the
/// user has just told us they want back for something else.
///
/// # Errors
///
/// The plugin's message, for the dialog to show: an accelerator it cannot parse, or one the
/// system refuses because another application holds it.
pub fn choose(app: &AppHandle, accelerator: &str) -> Result<(), String> {
    let accelerator = accelerator.trim();
    if accelerator.is_empty() {
        return Err("no combination was given".to_owned());
    }
    let ui = app.state::<super::Ui>();
    let previous = ui.shortcut().status();
    if previous.registered {
        if let Err(error) = app
            .global_shortcut()
            .unregister(previous.accelerator.as_str())
        {
            tracing::warn!(error = %error, "the previous shortcut could not be released");
        }
    }

    match bind(app, accelerator) {
        Ok(()) => {
            // Written before the state is updated: a setting that could not be stored is a
            // shortcut that works this run and comes back as the default at the next launch,
            // which is a smaller surprise than one the window says is in force and is not.
            let stored = ui.with_db(|db| settings::set(db, ACCELERATOR_KEY, &accelerator));
            if let Some(Err(error)) = stored {
                tracing::warn!(error = %error, "the chosen shortcut was not stored");
            }
            ui.shortcut().set(Status {
                accelerator: accelerator.to_owned(),
                registered: true,
                ask_for_another: false,
            });
            asked(app);
            tracing::info!(accelerator, "the global shortcut was changed by the user");
            Ok(())
        }
        Err(error) => {
            // The one the user asked for is not available and the previous one has been
            // released; put the previous one back so the app is not left with none.
            let restored = bind(app, &previous.accelerator).is_ok();
            ui.shortcut().set(Status {
                registered: restored,
                ..previous
            });
            Err(error)
        }
    }
}

/// The user closed the dialog without choosing: never ask again (FM-18).
pub fn asked(app: &AppHandle) {
    let ui = app.state::<super::Ui>();
    if let Some(Err(error)) = ui.with_db(|db| settings::set(db, ASKED_KEY, &true)) {
        tracing::warn!(error = %error, "the shortcut question was not marked as asked");
    }
    let mut status = ui.shortcut().status();
    status.ask_for_another = false;
    ui.shortcut().set(status);
}

/// Registers one accelerator with the handler of OPEN-03.
///
/// The handler fires on **press** only: a global hotkey reports both edges, and opening the
/// sheet on the release as well would show it and immediately show it again.
fn bind(app: &AppHandle, accelerator: &str) -> Result<(), String> {
    app.global_shortcut()
        .on_shortcut(accelerator, |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                super::show_view(app, super::VIEW_REQUEST);
            }
        })
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frontend's view vocabulary, read as text: the shortcut opens a view the frontend
    /// has to know by that name.
    const VIEWS_TS: &str = include_str!("../../../src/views.ts");

    #[test]
    fn the_shortcut_opens_a_view_the_frontend_knows() {
        assert!(
            VIEWS_TS.contains(&format!("'{}'", super::super::VIEW_REQUEST)),
            "src/views.ts does not know the view {}",
            super::super::VIEW_REQUEST
        );
    }

    #[test]
    fn the_default_is_the_combination_of_open_03() {
        // `Ctrl+Alt+H` and `⌃⌥H` are the same two modifiers and the same key; §7.16 prints
        // both spellings because the platforms name them differently.
        assert_eq!(DEFAULT_ACCELERATOR, "Control+Alt+H");
    }

    #[test]
    fn a_fresh_status_says_the_default_and_claims_nothing() {
        let status = Status::default();
        assert_eq!(status.accelerator, DEFAULT_ACCELERATOR);
        assert!(!status.registered);
        assert!(!status.ask_for_another);
    }

    #[test]
    fn the_state_is_what_was_last_written_into_it() {
        let state = State::default();
        assert_eq!(state.status(), Status::default());
        let taken = Status {
            accelerator: "Control+Alt+J".to_owned(),
            registered: true,
            ask_for_another: false,
        };
        state.set(taken.clone());
        assert_eq!(state.status(), taken);
    }
}
