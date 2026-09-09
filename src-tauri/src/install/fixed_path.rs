//! The fixed launcher path: what the agent is told to run (SRV-25, SRV-19, §3.5, FM-23).
//!
//! The path registered in every agent configuration is the bundled server binary itself,
//! next to the application executable:
//!
//! | Platform | Path |
//! |---|---|
//! | Windows | `%LOCALAPPDATA%\Baton\handoff-mcp.exe` |
//! | macOS | `/Applications/Baton.app/Contents/MacOS/handoff-mcp` |
//!
//! Both are "the folder [`std::env::current_exe`] is in", which is why this module is four
//! functions rather than a platform table: Tauri places an external binary beside the
//! executable on both platforms, updates replace the file in place, and the path in the
//! agent configuration therefore never changes (SRV-25, UPD-02, FM-24).
//!
//! SRV-19 is the reason it is the binary and not a script: on Windows a `.cmd` launcher
//! would make `cmd.exe` the server's parent, and the ancestor chain the hook is bound
//! through (§7.5) would point at a shell instead of at the agent.
//!
//! # Recognising our own entries
//!
//! §7.15 says our entries are recognisable by the fixed path in `command`, and `uninstall`
//! removes exactly those. "Ours" is deliberately weaker than "the current path": after the
//! user moves the bundle (FM-23) the registered path is the *old* one, and an uninstall or
//! a repair that did not recognise it would leave a dead entry behind for ever. So
//! [`is_ours`] asks only whether the command names a file called `handoff-mcp`, and
//! [`is_current`] is the stricter question the repair offer asks.

use std::env;
use std::path::{Path, PathBuf};

use super::error::{InstallError, Result};

/// The file name of the bundled server on this platform.
pub const SERVER_FILE_NAME: &str = if cfg!(windows) {
    "handoff-mcp.exe"
} else {
    "handoff-mcp"
};

/// The file stem every spelling of the server shares, dev layout included.
const SERVER_STEM: &str = "handoff-mcp";

/// The arguments the Stop and SubagentStop hooks run the server with (§5.11).
pub const HOOK_ARGUMENTS: &str = "hook stop";

/// The path registered in agent configurations, from this process's own location.
///
/// # Errors
///
/// [`InstallError::NoServer`] when the executable's own path is unavailable or has no
/// parent folder, which on both supported platforms means the process was started in a way
/// nothing here can reason about.
pub fn server_path() -> Result<PathBuf> {
    let executable = env::current_exe().map_err(|error| InstallError::NoServer {
        detail: format!("the application's own path is unknown: {error}"),
    })?;
    let folder = executable
        .parent()
        .ok_or_else(|| InstallError::NoServer {
            detail: format!("{} has no parent folder", executable.display()),
        })?
        .to_path_buf();
    Ok(server_in(&folder))
}

/// The server as it is named inside `folder`.
///
/// A packaged application has exactly one spelling, [`SERVER_FILE_NAME`], because that is
/// what Tauri writes into the bundle. A development tree has the sidecar under its
/// target-triple name (`handoff-mcp-x86_64-pc-windows-msvc.exe`), which is what
/// `scripts/fetch-server.mjs` fills `src-tauri/binaries/` with, so `cargo tauri dev`
/// registers a path that exists instead of one that will exist after packaging.
#[must_use]
pub fn server_in(folder: &Path) -> PathBuf {
    let bundled = folder.join(SERVER_FILE_NAME);
    if bundled.exists() {
        return bundled;
    }
    let sidecar = folder.join(sidecar_file_name());
    if sidecar.exists() {
        return sidecar;
    }
    // Neither is there: name the packaged spelling. Registering the path the next build
    // will produce is more useful than failing, and `verify` reports the mismatch.
    bundled
}

/// `handoff-mcp-<target triple>[.exe]`, the name Tauri gives an external binary before it
/// is bundled.
#[must_use]
fn sidecar_file_name() -> String {
    format!(
        "{SERVER_STEM}-{}{}",
        TARGET_TRIPLE,
        if cfg!(windows) { ".exe" } else { "" }
    )
}

/// The Rust target triple of this build, as Tauri spells it in a sidecar name.
///
/// Written out rather than read from a build script because the four values below are the
/// whole of REQUIREMENTS §1.5 and a fifth would need a platform this product does not ship
/// on.
const TARGET_TRIPLE: &str = if cfg!(all(windows, target_arch = "x86_64")) {
    "x86_64-pc-windows-msvc"
} else if cfg!(all(windows, target_arch = "aarch64")) {
    "aarch64-pc-windows-msvc"
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    "aarch64-apple-darwin"
} else {
    "x86_64-apple-darwin"
};

/// The `command` string of a Stop or SubagentStop hook entry (§7.15, SRV-11).
///
/// **The path is always quoted**, where §7.15 prints it bare. Measured against Claude Code
/// 2.1.266 on Windows: a hook command is run through `bash`, which reads every backslash of
/// an unquoted `C:\Users\…` as an escape, so what it tried to start was
/// `C:UsersgiuseGiuseppe…exe` and the hook died with exit 127, "command not found". Inside
/// double quotes bash keeps a backslash that is not followed by `$`, a backtick, a quote or
/// another backslash, so the quoted form is the one that survives — and it is equally
/// correct for `cmd.exe` and for a POSIX shell, and it covers a path holding a space, which
/// the bare form never did.
///
/// [`server_in_hook_command`] still reads the bare spelling back, because a machine
/// installed before this fix carries it and `uninstall` has to recognise it.
#[must_use]
pub fn hook_command(server: &Path) -> String {
    format!("\"{}\" {HOOK_ARGUMENTS}", server.display())
}

/// The server path a hook `command` names, or nothing when it is not one of ours.
///
/// The command has to end with `hook stop` and start with a path whose file name is the
/// server's: anything else belongs to the user and INST-04 keeps it.
#[must_use]
pub fn server_in_hook_command(command: &str) -> Option<PathBuf> {
    let head = command.trim().strip_suffix(HOOK_ARGUMENTS)?;
    let head = head.trim_end();
    // Collapse the two spellings `hook_command` can produce.
    let path = match head.strip_prefix('"') {
        Some(rest) => rest.strip_suffix('"')?,
        None => head,
    };
    let path = Path::new(path);
    is_ours(path).then(|| path.to_path_buf())
}

/// Whether `command` names our server, wherever it currently lives (FM-23).
#[must_use]
pub fn is_ours(command: &Path) -> bool {
    command
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem == SERVER_STEM || stem.starts_with(&format!("{SERVER_STEM}-")))
}

/// Whether `registered` is the path this build would register today.
///
/// Windows path comparison is case-insensitive and treats the two separators alike, which
/// is what the file system does; macOS is compared as written, because a bundle path is
/// produced by the installer and never typed.
#[must_use]
pub fn is_current(registered: &Path, current: &Path) -> bool {
    normalise(registered) == normalise(current)
}

/// The comparable form of a path.
fn normalise(path: &Path) -> String {
    let rendered = path.display().to_string();
    if cfg!(windows) {
        rendered.replace('/', "\\").to_lowercase()
    } else {
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registered_path_is_the_server_beside_the_application() {
        let folder = if cfg!(windows) {
            PathBuf::from("C:\\Users\\someone\\AppData\\Local\\Baton")
        } else {
            PathBuf::from("/Applications/Baton.app/Contents/MacOS")
        };
        // Neither spelling exists in this folder, so the packaged one is named.
        assert_eq!(server_in(&folder), folder.join(SERVER_FILE_NAME));
    }

    #[test]
    fn the_development_sidecar_is_taken_when_it_is_the_one_that_exists() {
        let dir = std::env::temp_dir().join(format!(
            "handoff-fixed-{}-{}",
            std::process::id(),
            crate::ids::new_session_ref()
        ));
        std::fs::create_dir_all(&dir).expect("the folder is created");
        let sidecar = dir.join(sidecar_file_name());
        std::fs::write(&sidecar, b"not a real binary").expect("the sidecar is written");

        assert_eq!(server_in(&dir), sidecar);

        // Once the packaged spelling is there it wins: that is what a bundle carries.
        let bundled = dir.join(SERVER_FILE_NAME);
        std::fs::write(&bundled, b"not a real binary either").expect("the binary is written");
        assert_eq!(server_in(&dir), bundled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hook_command_is_the_quoted_path_followed_by_the_subcommand() {
        let server = Path::new("/Applications/Baton.app/Contents/MacOS/handoff-mcp");
        assert_eq!(
            hook_command(server),
            "\"/Applications/Baton.app/Contents/MacOS/handoff-mcp\" hook stop"
        );
    }

    #[test]
    fn a_windows_path_keeps_its_backslashes_through_the_shell_that_runs_it() {
        // The measured failure this quoting exists for: unquoted, `bash` on Windows eats
        // every backslash and the hook exits 127 with "command not found".
        let server = Path::new("C:\\Users\\someone\\AppData\\Local\\Baton\\handoff-mcp.exe");
        let command = hook_command(server);
        assert_eq!(
            command,
            "\"C:\\Users\\someone\\AppData\\Local\\Baton\\handoff-mcp.exe\" hook stop"
        );

        // Reading it back is a Windows question, and only on Windows does it have this
        // answer: `Path` parses with the host's separators, so on a Unix host the whole
        // `C:\…\handoff-mcp.exe` is one component and `file_stem` is that whole string.
        // Nothing in production meets the case — a Windows path only ever appears in the
        // configuration of a Windows installation, which is read by a Windows build — and
        // the reverse direction needs no gate: `Path` on Windows accepts `/` as a
        // separator, which is why the POSIX-looking golden fixtures are read correctly on
        // both platforms.
        #[cfg(windows)]
        assert_eq!(server_in_hook_command(&command).as_deref(), Some(server));
    }

    #[test]
    fn a_path_with_a_space_is_quoted_and_read_back() {
        let server = Path::new("/Users/some one/Baton.app/Contents/MacOS/handoff-mcp");
        let command = hook_command(server);
        assert_eq!(
            command,
            "\"/Users/some one/Baton.app/Contents/MacOS/handoff-mcp\" hook stop"
        );
        assert_eq!(server_in_hook_command(&command).as_deref(), Some(server));
    }

    #[test]
    fn our_hook_is_recognised_wherever_the_bundle_has_been_moved() {
        // FM-23: the registered path is the old one and must still be recognised, or an
        // uninstall would leave it behind for ever.
        let old = Path::new("/Users/someone/Desktop/Baton.app/Contents/MacOS/handoff-mcp");
        assert_eq!(
            server_in_hook_command(&hook_command(old)).as_deref(),
            Some(old)
        );
        // The bare spelling every machine installed before the quoting fix carries; a
        // Windows one, so it only parses on Windows (see the test above).
        #[cfg(windows)]
        assert_eq!(
            server_in_hook_command("C:\\Baton\\handoff-mcp.exe hook stop"),
            Some(PathBuf::from("C:\\Baton\\handoff-mcp.exe"))
        );
        assert_eq!(
            server_in_hook_command("/old/Baton/handoff-mcp hook stop"),
            Some(PathBuf::from("/old/Baton/handoff-mcp"))
        );
        // The development sidecar spelling is ours too.
        assert!(server_in_hook_command(
            "/w/target/debug/handoff-mcp-x86_64-apple-darwin hook stop"
        )
        .is_some());
    }

    #[test]
    fn a_hook_that_is_not_ours_is_left_alone() {
        for command in [
            "npm test",
            "/usr/local/bin/other-tool hook stop",
            "/Applications/Baton.app/Contents/MacOS/handoff-mcp serve",
            "handoff-mcp",
            "",
        ] {
            assert!(
                server_in_hook_command(command).is_none(),
                "{command} was claimed as ours"
            );
        }
    }

    #[test]
    fn the_repair_offer_compares_the_registered_path_with_the_current_one() {
        let current = Path::new("/Applications/Baton.app/Contents/MacOS/handoff-mcp");
        assert!(is_current(current, current));
        assert!(!is_current(
            Path::new("/Users/someone/Desktop/Baton.app/Contents/MacOS/handoff-mcp"),
            current
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_compares_paths_as_the_file_system_does() {
        assert!(is_current(
            Path::new("c:/users/someone/appdata/local/baton/handoff-mcp.exe"),
            Path::new("C:\\Users\\someone\\AppData\\Local\\Baton\\handoff-mcp.exe")
        ));
    }
}
