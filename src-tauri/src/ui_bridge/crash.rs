//! The crash notice of the startup sequence (§7.2, §7.14, TEL-01, TEL-02).
//!
//! §7.14 gives the next launch after a panic one sentence — "the app crashed last time;
//! open the folder to send the report by hand" — and one action, opening the folder.
//! **Nothing is uploaded, ever**: the file stays on the machine and the user decides
//! whether it goes anywhere, which is the whole of TEL-01.
//!
//! # Why the window asks rather than `main()` telling it
//!
//! Like the agent scan and the FM-23 path check before it (T-040), this is a sentence for
//! a person, and at `setup()` no webview is listening: a notice pushed from there is
//! dropped by [`super::Notifier::emit`]. So `run()` does not announce anything; the window
//! asks [`crash_notice`] once when it mounts, and the answer is what it draws.
//!
//! # Said once, however many times it is asked
//!
//! The `known_agents` rule of INST-05, applied to crash files: [`crash_notice`] records the
//! report it reported, so a second call in the same run — a reload of the webview, a second
//! mount — says nothing, and the launch after this one says nothing either unless a new
//! crash happened in between. The marker is a name and never a path: the folder is the
//! user's and a settings row is not the place for it.

use serde::Serialize;
use tauri::{AppHandle, Manager as _};
use tauri_plugin_opener::OpenerExt as _;

use crate::crash;
use crate::log::settings;
use crate::paths;

/// The setting holding the name of the newest crash file the user has been told about.
pub const CRASH_SEEN_KEY: &str = "crash_seen";

/// What the window draws after a crash (§7.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashNotice {
    /// Whether the previous run of the application ended in a panic.
    pub crashed: bool,
}

impl CrashNotice {
    /// The ordinary launch: nothing happened and nothing is said.
    const QUIET: Self = Self { crashed: false };
}

/// Whether a crash file arrived since the last one the user was told about, and records it.
///
/// A launch with no database answers "no": the notice would then come back at every start,
/// which is the same trade `onboarded_from` makes for the same reason — an app that cannot
/// remember an answer must not keep asking the question.
#[tauri::command]
#[must_use]
pub fn crash_notice(app: AppHandle) -> CrashNotice {
    let folder = crash::crashes_dir(&paths::app_data_dir());
    app.state::<super::Ui>()
        .with_db(|db| {
            let seen = settings::get::<String>(db, CRASH_SEEN_KEY).unwrap_or_else(|error| {
                tracing::warn!(error = %error, "the last crash seen could not be read");
                None
            });
            let Some(unseen) = crash::unseen_report(&folder, seen.as_deref()) else {
                return CrashNotice::QUIET;
            };
            if let Err(error) = settings::set(db, CRASH_SEEN_KEY, &unseen) {
                // The notice is still worth showing: the cost of failing to record it is
                // that it is shown again, and the cost of withholding it is that a crash
                // nobody hears about is a crash nobody sends us.
                tracing::warn!(error = %error, "the crash notice could not be marked as seen");
            }
            tracing::info!("the previous run ended in a crash");
            CrashNotice { crashed: true }
        })
        .unwrap_or(CrashNotice::QUIET)
}

/// Opens the crash folder in the system's file manager (§7.14).
///
/// The folder rather than the file: what §7.14 offers is "open the folder to send the report
/// by hand", and a text file opened in whatever is registered for `.txt` is one step further
/// from attaching it to a mail than the folder it sits in.
///
/// # Errors
///
/// The platform's message, for the window to show. A folder that does not exist is one of
/// them: it is created by the panic hook, so the only way to be here without one is a
/// notice about a file somebody deleted in between.
#[tauri::command]
pub fn open_crashes_folder(app: AppHandle) -> Result<(), String> {
    let folder = crash::crashes_dir(&paths::app_data_dir());
    app.opener()
        .open_path(folder.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    use crate::log::Db;

    /// The body of [`crash_notice`] without a Tauri handle: the marker, the folder and the
    /// rule that binds them. What the command adds is `app.state()` and the log line.
    fn notice(db: &Db, folder: &std::path::Path) -> CrashNotice {
        let seen = settings::get::<String>(db, CRASH_SEEN_KEY).expect("a read");
        let Some(unseen) = crash::unseen_report(folder, seen.as_deref()) else {
            return CrashNotice::QUIET;
        };
        settings::set(db, CRASH_SEEN_KEY, &unseen).expect("a write");
        CrashNotice { crashed: true }
    }

    fn folder_with(names: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "baton-crash-notice-{}-{}",
            std::process::id(),
            crate::ids::new_session_ref()
        ));
        fs::create_dir_all(&dir).expect("a folder");
        for name in names {
            fs::write(dir.join(name), "report").expect("a report");
        }
        dir
    }

    #[test]
    fn a_launch_after_no_crash_says_nothing() {
        let db = Db::open_in_memory().expect("a database");
        let dir = folder_with(&[]);

        assert_eq!(notice(&db, &dir), CrashNotice::QUIET);
        assert_eq!(
            settings::get::<String>(&db, CRASH_SEEN_KEY).expect("a read"),
            None,
            "nothing to remember"
        );

        fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn the_launch_after_a_crash_says_so_once() {
        let db = Db::open_in_memory().expect("a database");
        let dir = folder_with(&["2026-09-08T14-40-00Z.txt"]);

        assert_eq!(notice(&db, &dir), CrashNotice { crashed: true });
        assert_eq!(
            settings::get::<String>(&db, CRASH_SEEN_KEY)
                .expect("a read")
                .as_deref(),
            Some("2026-09-08T14-40-00Z.txt")
        );

        // The INST-05 rule: a second mount, a reloaded webview, the launch after this one.
        assert_eq!(notice(&db, &dir), CrashNotice::QUIET);

        fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn a_crash_after_the_last_notice_is_said_again() {
        let db = Db::open_in_memory().expect("a database");
        let dir = folder_with(&["2026-09-08T14-40-00Z.txt"]);
        notice(&db, &dir);

        fs::write(dir.join("2026-09-09T07-15-00Z.txt"), "report").expect("a second report");

        assert_eq!(notice(&db, &dir), CrashNotice { crashed: true });
        assert_eq!(
            settings::get::<String>(&db, CRASH_SEEN_KEY)
                .expect("a read")
                .as_deref(),
            Some("2026-09-09T07-15-00Z.txt")
        );

        fs::remove_dir_all(&dir).expect("clean up");
    }
}
