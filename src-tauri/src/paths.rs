//! Where the app keeps its files and where it meets the server (§4.1, §7.2, DD-28).
//!
//! Two roots, and the difference between them is the whole point (§7.2):
//!
//! - `~/.handoff/` is the **contract** folder. The server reads the token from it, the
//!   agent may browse the runbooks in it, and it survives an uninstall (RUN-03). Every
//!   name under it is fixed by the design because a second implementation — the server —
//!   computes the same names independently.
//! - the **app data** directory (`%APPDATA%\Baton\`, `~/Library/Application Support/Baton/`)
//!   is private: the database, the crash files, the update cache. The server never reads
//!   it and nothing outside this application writes it.
//!
//! Both roots can be moved by an environment variable, and only for tests: `HANDOFF_HOME`
//! (§5.12, and the e2e isolation of `TASKS.md` §0.4 item 4) and `HANDOFF_APP_DATA_DIR`
//! (§0.4 item 4). Nothing in production sets either one. The rule for reading them is the
//! server's rule, copied deliberately: the value is trimmed and a blank value counts as
//! unset, so the two peers can never disagree about which instance is being addressed.
//!
//! Every value is computed on each call rather than remembered, so a folder created after
//! the app started is picked up without a restart.

use std::env;
use std::path::{Path, PathBuf};

/// Overrides `~/.handoff` (tests and e2e isolation only).
pub const ENV_HANDOFF_HOME: &str = "HANDOFF_HOME";

/// Overrides the app data directory (tests and e2e isolation only).
pub const ENV_HANDOFF_APP_DATA_DIR: &str = "HANDOFF_APP_DATA_DIR";

/// The product name, from the D2 decision of T-001. It names the app data directory, the
/// bundle and the window; `handoff` stays the concept-bound name of everything shared with
/// the server.
pub const PRODUCT_NAME: &str = "Baton";

/// The folder under the user's home when `HANDOFF_HOME` is not set (§4.1).
pub const HOME_FOLDER_NAME: &str = ".handoff";

/// The folder holding the runbook files, the single root of §12.3 (RUN-03a).
pub const RUNBOOKS_FOLDER_NAME: &str = "runbooks";

/// The per-installation channel token, written by the installer (SRV-07, INST-07).
pub const TOKEN_FILE_NAME: &str = "channel.token";

/// The Unix socket the app listens on (§4.1).
pub const SOCKET_FILE_NAME: &str = "app.sock";

/// Where the app writes the real socket path when the default one does not fit (FM-12).
pub const SOCKET_POINTER_FILE_NAME: &str = "app.sock.path";

/// `HANDOFF_HOME` when it is set, else `~/.handoff` (§4.1, §5.12).
pub fn handoff_home() -> PathBuf {
    resolve_handoff_home(env_value(ENV_HANDOFF_HOME), &home_dir())
}

/// The value of `HANDOFF_HOME`, trimmed, or nothing when it is unset or blank.
///
/// The Windows pipe name mixes it in when it is set, and the server derives the same name
/// from the same raw value (§0.4 item 4): what the digest eats is this string, not the
/// resolved folder, so the two peers cannot disagree about which instance is addressed.
#[must_use]
pub fn handoff_home_override() -> Option<String> {
    env_value(ENV_HANDOFF_HOME)
}

/// `~/.handoff/runbooks/`: the one root the reader is configured with (§12.3).
pub fn runbooks_dir() -> PathBuf {
    handoff_home().join(RUNBOOKS_FOLDER_NAME)
}

/// `~/.handoff/channel.token`: created at first install, or at startup if missing (INST-07).
pub fn token_path() -> PathBuf {
    handoff_home().join(TOKEN_FILE_NAME)
}

/// The app's private directory: `%APPDATA%\Baton\` on Windows,
/// `~/Library/Application Support/Baton/` on macOS (§7.2).
pub fn app_data_dir() -> PathBuf {
    resolve_app_data_dir(env_value(ENV_HANDOFF_APP_DATA_DIR), &platform_data_root())
}

/// `~/.handoff/app.sock`: where the app listens on POSIX while the path fits in `sun_path`.
///
/// It is a name of the contract folder and nothing more. Which endpoint the app actually
/// binds — this path, the shorter one the pointer file of FM-12 points at, or the named
/// pipe of Windows — is [`crate::channel::endpoint`], because that resolution belongs with
/// the listener that owns the pipe DACL and writes the pointer file.
pub fn socket_path() -> PathBuf {
    handoff_home().join(SOCKET_FILE_NAME)
}

/// The pointer file of FM-12: the app writes the real socket path here when the default
/// one does not fit in `sun_path`, and the server reads it.
pub fn socket_pointer_path() -> PathBuf {
    handoff_home().join(SOCKET_POINTER_FILE_NAME)
}

/// The user's home directory. It falls back to the working directory, which is wrong but
/// visible: on both supported platforms the value is always available, and a caller that
/// cannot find `~/.handoff/` reports it as a missing token or a missing socket rather than
/// panicking during startup.
fn home_dir() -> PathBuf {
    #[allow(deprecated)]
    env::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// The platform's per-user application data root, before the product name is appended.
fn platform_data_root() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        // `%APPDATA%` is the roaming folder; the fallback is the same path spelled out, for
        // a session where the variable was stripped.
        if let Some(appdata) = env_value("APPDATA") {
            return PathBuf::from(appdata);
        }
        home_dir().join("AppData").join("Roaming")
    }

    #[cfg(target_os = "macos")]
    {
        home_dir().join("Library").join("Application Support")
    }

    // Linux is not a supported platform (REQUIREMENTS §1.5). It gets the XDG shape because
    // the code is written once and a developer running the unit tests there should not
    // meet a missing arm.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        match env_value("XDG_DATA_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => home_dir().join(".local").join("share"),
        }
    }
}

fn resolve_handoff_home(override_home: Option<String>, home: &Path) -> PathBuf {
    match override_home {
        Some(dir) => PathBuf::from(dir),
        None => home.join(HOME_FOLDER_NAME),
    }
}

fn resolve_app_data_dir(override_dir: Option<String>, platform_root: &Path) -> PathBuf {
    match override_dir {
        Some(dir) => PathBuf::from(dir),
        None => platform_root.join(PRODUCT_NAME),
    }
}

/// An environment variable as configuration: trimmed, and blank counts as unset.
///
/// `pub(crate)` for [`crate::channel::endpoint`], which reads `USERDOMAIN` and `USERNAME`
/// under the same rule: an unset or blank variable contributes an empty string to the pipe
/// digest, never a substitute from the OS user database, because the server derives the
/// same name from the same two variables (§5.8).
pub(crate) fn env_value(name: &str) -> Option<String> {
    read_env(env::var(name).ok())
}

/// The pure half of [`env_value`], so the rule can be tested without touching the process
/// environment (which every other test in the binary shares).
fn read_env(raw: Option<String>) -> Option<String> {
    let value = raw?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_or_whitespace_variable_counts_as_unset() {
        assert_eq!(read_env(None), None);
        assert_eq!(read_env(Some(String::new())), None);
        assert_eq!(read_env(Some("   ".to_string())), None);
        assert_eq!(read_env(Some("\t\n".to_string())), None);
    }

    #[test]
    fn a_set_variable_is_trimmed_exactly_as_the_server_trims_it() {
        assert_eq!(
            read_env(Some("  C:\\tmp\\home  ".to_string())).as_deref(),
            Some("C:\\tmp\\home")
        );
        assert_eq!(
            read_env(Some("/tmp/home".to_string())).as_deref(),
            Some("/tmp/home")
        );
    }

    #[test]
    fn handoff_home_is_the_home_folder_unless_the_variable_names_another() {
        let home = Path::new("/users/someone");
        assert_eq!(
            resolve_handoff_home(None, home),
            home.join(HOME_FOLDER_NAME)
        );
        assert_eq!(
            resolve_handoff_home(Some("/tmp/isolated".to_string()), home),
            PathBuf::from("/tmp/isolated")
        );
    }

    #[test]
    fn the_app_data_dir_carries_the_product_name_unless_it_is_overridden() {
        let root = Path::new("/users/someone/AppData/Roaming");
        assert_eq!(resolve_app_data_dir(None, root), root.join("Baton"));
        assert_eq!(
            resolve_app_data_dir(Some("/tmp/app-data".to_string()), root),
            PathBuf::from("/tmp/app-data")
        );
    }

    #[test]
    fn the_contract_folder_holds_the_token_the_runbooks_and_the_socket() {
        let home = handoff_home();
        assert_eq!(runbooks_dir(), home.join(RUNBOOKS_FOLDER_NAME));
        assert_eq!(token_path(), home.join(TOKEN_FILE_NAME));
        assert_eq!(socket_path(), home.join(SOCKET_FILE_NAME));
        assert_eq!(socket_pointer_path(), home.join(SOCKET_POINTER_FILE_NAME));
    }

    #[test]
    fn the_two_roots_are_different_folders() {
        // §7.2 keeps the database out of the folder agents may browse. A refactor that
        // collapsed the two would be invisible until an agent read the log by hand.
        assert_ne!(handoff_home(), app_data_dir());
    }
}
