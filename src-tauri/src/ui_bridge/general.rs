//! Settings → General: language, autostart and the login launch (§7.16, APP-01, APP-02).
//!
//! Three preferences that have nothing in common except where the user finds them, and one
//! rule that binds them: what the window shows is what is actually in force, never what was
//! once asked for.
//!
//! - **The language** is a `settings` row, and its absence is a value: "System" is *no*
//!   setting, so a machine that changes its system language follows it afterwards (§7.16).
//!   The row is only ever read to seed the frontend; the language in force is what the
//!   frontend resolved and reported through [`super::set_ui_language`], because it is the
//!   side that can see the system's preference list.
//! - **Autostart** is a `settings` row *and* an entry in the operating system's login
//!   items, which the user can also remove from the system's own screens. The toggle
//!   therefore reads the entry (`is_enabled`) and falls back to the row only when the
//!   platform will not answer.
//! - **`--hidden`** is the argument the login entry carries. A launch at login must leave
//!   the panel where APP-01 promises it will be: in the tray, "doing nothing until an agent
//!   asks for a handoff".
//!
//! # The setting that has never been answered
//!
//! [`crate::log::settings::get`] behind [`super::Ui::with_db`] answers `Option<Option<T>>`,
//! and the two `None`s are different facts: no database, and no value written yet. Flattening
//! them together is what cost onboarding its first launch in T-040. Here the distinction
//! decides whether we touch the user's login items at all: until onboarding has stored an
//! answer, nothing has been consented to and nothing is written (INST-01, APP-01).

use serde::Serialize;
use tauri::{AppHandle, Manager as _};
use tauri_plugin_autostart::ManagerExt as _;

use crate::i18n::Language;
use crate::log::settings;

use super::install::AUTOSTART_KEY;

/// The setting holding the language the user chose (APP-02).
///
/// Absent means "follow the system", which is what the **System** choice writes.
pub const LANGUAGE_KEY: &str = "language";

/// The argument the login entry passes, so a launch at login stays in the tray (APP-01).
pub const HIDDEN_ARG: &str = "--hidden";

/// The General settings page, and what `main.ts` needs before the first paint (§7.16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralSettings {
    /// The language the user chose, or `None` for "follow the system" (APP-02).
    pub language: Option<Language>,
    /// Whether Baton is in the operating system's login items (APP-01).
    pub autostart: bool,
    /// Whether this launch came from those login items, and must not open the panel.
    pub started_hidden: bool,
}

/// Whether `args` (a whole command line, program name included) carries [`HIDDEN_ARG`].
#[must_use]
pub fn hidden_in<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .skip(1)
        .any(|arg| arg.as_ref() == HIDDEN_ARG)
}

/// Whether this process was launched by the login entry (APP-01).
#[must_use]
pub fn started_hidden() -> bool {
    hidden_in(std::env::args())
}

/// What the toggle shows: the login entry itself, and the stored answer only if it cannot
/// be read.
///
/// The entry is the truth — the user can delete it from the system's own login-items screen,
/// and a toggle that then still said "on" would be lying about the one thing it is for. The
/// stored answer is the fallback for a platform that refuses to answer, where showing the
/// last thing the user asked for beats showing `false`.
#[must_use]
fn autostart_shown(entry: Option<bool>, stored: Option<Option<bool>>) -> bool {
    entry.unwrap_or_else(|| stored.flatten().unwrap_or(false))
}

/// What to do with the login entry at startup, or `None` to leave it alone.
///
/// The only case that writes is a stored answer: onboarding asked the question of APP-01 and
/// the user answered it. A database with no answer in it is a machine where onboarding has
/// not finished, and writing "on by default" there would put Baton in the user's login items
/// before they were ever asked (INST-01). No database at all is the same, with less to go on.
#[must_use]
fn autostart_to_apply(stored: Option<Option<bool>>) -> Option<bool> {
    match stored {
        Some(Some(wanted)) => Some(wanted),
        Some(None) | None => None,
    }
}

/// The language the user chose, as the settings table holds it.
fn stored_language(app: &AppHandle) -> Option<Language> {
    app.state::<super::Ui>()
        .with_db(|db| {
            settings::get::<Language>(db, LANGUAGE_KEY).unwrap_or_else(|error| {
                tracing::warn!(error = %error, "the chosen language could not be read");
                None
            })
        })
        .flatten()
}

/// The autostart answer the settings table holds, keeping "no database" apart from "no answer".
fn stored_autostart(app: &AppHandle) -> Option<Option<bool>> {
    app.state::<super::Ui>().with_db(|db| {
        settings::get::<bool>(db, AUTOSTART_KEY).unwrap_or_else(|error| {
            tracing::warn!(error = %error, "the autostart answer could not be read");
            None
        })
    })
}

/// Whether the login entry is in place, or `None` when the platform would not say.
fn autostart_entry(app: &AppHandle) -> Option<bool> {
    match app.autolaunch().is_enabled() {
        Ok(enabled) => Some(enabled),
        Err(error) => {
            tracing::warn!(error = %error, "the login items could not be read");
            None
        }
    }
}

/// Puts the login entry in the state `enabled` asks for (APP-01).
///
/// Idempotent by inspection rather than by the plugin: `disable()` deletes a registry value
/// on Windows and fails when there is none, so switching off something that is already off
/// would report an error the user has no way to act on.
///
/// # Errors
///
/// The platform's message, for the settings page to show.
pub fn apply_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    if manager.is_enabled().map_err(|error| error.to_string())? == enabled {
        return Ok(());
    }
    if enabled {
        manager.enable()
    } else {
        manager.disable()
    }
    .map_err(|error| error.to_string())?;
    tracing::info!(enabled, "the login entry was rewritten");
    Ok(())
}

/// Brings the login entry into line with the stored answer, once, at startup.
///
/// The two can differ without anyone doing anything wrong: the application was reinstalled
/// somewhere else (the entry names a path), or the user removed it from the system's own
/// login items. Nothing is written for a machine that has never answered the question.
pub fn sync_autostart(app: &AppHandle) {
    let Some(wanted) = autostart_to_apply(stored_autostart(app)) else {
        return;
    };
    if let Err(error) = apply_autostart(app, wanted) {
        tracing::warn!(
            error,
            wanted,
            "the login entry could not be brought into line"
        );
    }
}

/// The General settings, and the two facts `main.ts` needs before the first paint (§7.16).
#[tauri::command]
#[must_use]
pub fn general_settings(app: AppHandle) -> GeneralSettings {
    GeneralSettings {
        language: stored_language(&app),
        autostart: autostart_shown(autostart_entry(&app), stored_autostart(&app)),
        started_hidden: started_hidden(),
    }
}

/// Stores the language the user chose, or forgets it for **System** (APP-02).
///
/// It does not change what the window shows: the frontend resolves the language it will run
/// in — this setting, else the system's preference list — and reports it through
/// [`super::set_ui_language`], which is also what relabels the tray. One decision, one place.
///
/// # Errors
///
/// The message of the write that failed, for the settings page to show.
#[tauri::command]
pub fn set_language(app: AppHandle, language: Option<Language>) -> Result<(), String> {
    let written = app.state::<super::Ui>().with_db(|db| match language {
        Some(chosen) => settings::set(db, LANGUAGE_KEY, &chosen),
        None => settings::remove(db, LANGUAGE_KEY).map(|_| ()),
    });
    match written {
        Some(Err(error)) => Err(error.to_string()),
        Some(Ok(())) | None => Ok(()),
    }
}

/// Switches the login entry on or off, and remembers the answer (APP-01).
///
/// The entry is written first: a setting stored for an entry that could not be created would
/// make the toggle say "on" at the next launch about a login item that is not there, and
/// [`sync_autostart`] would then try to create it again at every start.
///
/// # Errors
///
/// The platform's message, or the message of the write that failed.
#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    apply_autostart(&app, enabled)?;
    let written = app
        .state::<super::Ui>()
        .with_db(|db| settings::set(db, AUTOSTART_KEY, &enabled));
    match written {
        Some(Err(error)) => Err(error.to_string()),
        Some(Ok(())) | None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::log::Db;

    #[test]
    fn the_login_argument_is_recognised_after_the_program_name() {
        // The first element of a command line is the program, and a bundle installed in a
        // folder called `--hidden` is not a launch at login.
        assert!(hidden_in(["handoff-app.exe", HIDDEN_ARG]));
        assert!(hidden_in(["handoff-app.exe", "--other", HIDDEN_ARG]));
        assert!(!hidden_in(["handoff-app.exe"]));
        assert!(!hidden_in([HIDDEN_ARG]));
        assert!(!hidden_in(["handoff-app.exe", "--hidden=1"]));
        assert!(!hidden_in(Vec::<String>::new()));
    }

    #[test]
    fn a_machine_that_has_not_answered_keeps_its_login_items() {
        // The T-040 trap, in the place where it would cost the most: `Some(None)` is a
        // database with no answer in it, which is every machine whose onboarding has not
        // finished. Reading it as "on by default" would put Baton in the user's login items
        // before the checkbox of APP-01 was ever shown.
        assert_eq!(autostart_to_apply(Some(None)), None);
        assert_eq!(autostart_to_apply(None), None);
        assert_eq!(autostart_to_apply(Some(Some(true))), Some(true));
        assert_eq!(autostart_to_apply(Some(Some(false))), Some(false));
    }

    #[test]
    fn the_toggle_shows_the_login_entry_and_not_the_stored_answer() {
        // The user can remove the entry from the system's own login-items screen; the
        // setting then says one thing and the machine another, and the machine is right.
        assert!(!autostart_shown(Some(false), Some(Some(true))));
        assert!(autostart_shown(Some(true), Some(Some(false))));
    }

    #[test]
    fn a_platform_that_will_not_answer_leaves_the_stored_answer_showing() {
        assert!(autostart_shown(None, Some(Some(true))));
        assert!(!autostart_shown(None, Some(Some(false))));
        assert!(!autostart_shown(None, Some(None)));
        assert!(!autostart_shown(None, None));
    }

    #[test]
    fn the_chosen_language_round_trips_and_system_is_its_absence() {
        // "System" is not a third value: it is no row at all, so a machine that changes its
        // system language follows it (§7.16, and `resolveLanguage` on the other side).
        let db = Db::open_in_memory().expect("a database");
        assert_eq!(
            settings::get::<Language>(&db, LANGUAGE_KEY).expect("a read"),
            None
        );

        settings::set(&db, LANGUAGE_KEY, &Language::It).expect("a write");
        assert_eq!(
            settings::get::<Language>(&db, LANGUAGE_KEY).expect("a read"),
            Some(Language::It)
        );

        assert!(settings::remove(&db, LANGUAGE_KEY).expect("a removal"));
        assert_eq!(
            settings::get::<Language>(&db, LANGUAGE_KEY).expect("a read"),
            None
        );
    }

    #[test]
    fn the_stored_language_is_the_tag_the_frontend_writes() {
        // The frontend stores `"en"` / `"it"` and `i18n::resolve` reads exactly those, so
        // the row has to be that string and not a Rust variant name.
        let db = Db::open_in_memory().expect("a database");
        settings::set(&db, LANGUAGE_KEY, &Language::En).expect("a write");
        assert_eq!(
            settings::get::<String>(&db, LANGUAGE_KEY).expect("a read"),
            Some(Language::En.tag().to_owned())
        );
    }
}
