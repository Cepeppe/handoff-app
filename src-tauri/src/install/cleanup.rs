//! The last step of the startup sequence: the binaries a Windows update left behind
//! (§7.2, §3.5, FM-24, SRV-25).
//!
//! An executable that is in use cannot be overwritten on Windows, but it can be renamed.
//! The installer's hook therefore renames `handoff-mcp.exe` to `handoff-mcp.<oldver>.old.exe`
//! before copying the new file: the sessions running at that moment keep the binary they
//! opened, the agent configurations keep pointing at the same fixed path (SRV-25, UPD-02),
//! and the old file is left for somebody to delete once nobody holds it. That somebody is
//! this module, at the next launch.
//!
//! # "No longer locked" is asked by trying
//!
//! There is no portable way to ask Windows whether a file is open, and every way of asking
//! is a race — the answer can be stale before the delete is issued. So the delete *is* the
//! question: a file a session still has open refuses to go with a sharing violation, and it
//! is left exactly where it was for the launch after this one. Nothing is retried and
//! nothing is reported to the user: an old binary sitting beside the current one costs a
//! few megabytes and breaks nothing, which is why FM-24 makes this a cleanup and not a
//! failure mode.
//!
//! # Only ours, and only the renamed ones
//!
//! §3.5 says "vendor `*.old.exe`", and that is narrower than every `*.old.exe`: the
//! bundle's folder is `%LOCALAPPDATA%\Baton\` on Windows, but a development tree is
//! `src-tauri/target/debug/`, where anything at all can be lying about. The name has to
//! start with the server's stem *and* end with the suffix the installer writes, so the only
//! files that can be removed are the ones an update of ours created.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use super::fixed_path::SERVER_STEM;

/// The suffix the installer's rename hook gives the outgoing server (§3.5).
pub const OLD_BINARY_SUFFIX: &str = ".old.exe";

/// What one pass of the cleanup did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cleaned {
    /// Files deleted in this pass.
    pub removed: usize,
    /// Files a running session still holds, left for the next launch.
    pub kept: usize,
}

impl Cleaned {
    /// Whether the pass found anything at all, which is what decides if it is worth a log
    /// line: on every ordinary launch there is nothing there.
    #[must_use]
    pub fn found_something(self) -> bool {
        self.removed > 0 || self.kept > 0
    }
}

/// Whether `name` is a server binary an update renamed out of the way.
///
/// Both halves are compared case-insensitively: this runs on Windows, where the installer's
/// spelling and the file system's are not required to agree.
#[must_use]
pub fn is_superseded_binary(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    lowered.starts_with(SERVER_STEM) && lowered.ends_with(OLD_BINARY_SUFFIX)
}

/// The superseded binaries sitting in `folder`, sorted by name so a caller's output is
/// stable.
#[must_use]
pub fn superseded_binaries(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else {
        // A folder that cannot be read is a folder with nothing to clean up as far as this
        // step is concerned. The application does not exist to tidy it.
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(is_superseded_binary)
        })
        .collect();
    found.sort();
    found
}

/// Deletes every superseded binary of `folder` that nothing holds open (FM-24).
pub fn remove_superseded_binaries(folder: &Path) -> Cleaned {
    let mut cleaned = Cleaned::default();
    for path in superseded_binaries(folder) {
        match fs::remove_file(&path) {
            Ok(()) => cleaned.removed += 1,
            Err(error) => {
                // The ordinary case is a session that still has it open; anything else is
                // as harmless and is treated the same way. No file name in the record:
                // `log::` and the crash file take identifiers only (R-19), and the folder
                // is the user's.
                tracing::debug!(error = %error, "a superseded server binary is still in use");
                cleaned.kept += 1;
            }
        }
    }
    cleaned
}

/// The folder the bundled server lives in: the one holding this executable (§3.5).
///
/// # Errors
///
/// Nothing to clean when the process's own path is unavailable, which on both supported
/// platforms means it was started in a way nothing here can reason about.
#[must_use]
pub fn bundle_folder() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(Path::to_path_buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs::File;

    /// A temporary directory of this test binary's own, removed when the guard is dropped.
    /// The name carries the process id and a fresh identifier, so two runs of this suite
    /// never share a folder and one cannot delete the other's files.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "handoff-cleanup-{name}-{}-{}",
                std::process::id(),
                crate::ids::new_session_ref()
            ));
            fs::create_dir_all(&dir).expect("the temporary directory is created");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A folder holding what an update leaves behind, and three files it must not touch.
    fn folder_with_an_update_in_it(name: &str) -> TempDir {
        let dir = TempDir::new(name);
        for file in [
            "handoff-mcp.exe",
            "handoff-mcp.0.1.0.old.exe",
            "handoff-mcp.0.2.0.old.exe",
            "handoff-mcp-x86_64-pc-windows-msvc.old.exe",
            "handoff-app.exe",
            "notes.old.exe",
        ] {
            File::create(dir.path().join(file)).expect("a file");
        }
        dir
    }

    #[test]
    fn only_a_renamed_server_binary_is_superseded() {
        assert!(is_superseded_binary("handoff-mcp.0.1.0.old.exe"));
        assert!(is_superseded_binary(
            "handoff-mcp-x86_64-pc-windows-msvc.old.exe"
        ));
        // Windows spells file names how it likes and the installer is not this code.
        assert!(is_superseded_binary("HANDOFF-MCP.0.1.0.OLD.EXE"));

        // The current server, the application itself and somebody else's file: §3.5 renames
        // the vendored server and nothing else, so nothing else may be deleted.
        assert!(!is_superseded_binary("handoff-mcp.exe"));
        assert!(!is_superseded_binary("handoff-app.exe"));
        assert!(!is_superseded_binary("notes.old.exe"));
        assert!(!is_superseded_binary("handoff-mcp.old.exe.bak"));
    }

    #[test]
    fn the_cleanup_removes_the_superseded_binaries_and_leaves_everything_else() {
        let dir = folder_with_an_update_in_it("removes");

        let cleaned = remove_superseded_binaries(dir.path());
        assert_eq!(
            cleaned,
            Cleaned {
                removed: 3,
                kept: 0
            }
        );

        assert!(dir.path().join("handoff-mcp.exe").exists());
        assert!(dir.path().join("handoff-app.exe").exists());
        assert!(dir.path().join("notes.old.exe").exists());
        assert!(!dir.path().join("handoff-mcp.0.1.0.old.exe").exists());
        assert!(!dir.path().join("handoff-mcp.0.2.0.old.exe").exists());
    }

    #[test]
    fn a_second_launch_finds_nothing_left_to_do() {
        let dir = folder_with_an_update_in_it("idempotent");
        remove_superseded_binaries(dir.path());

        let again = remove_superseded_binaries(dir.path());
        assert_eq!(again, Cleaned::default());
        assert!(!again.found_something());
    }

    #[test]
    fn a_folder_that_is_not_there_is_not_an_error() {
        // A development tree has no bundle folder until something has been built into it,
        // and a launch is not the moment to complain about that.
        let absent = std::env::temp_dir().join("handoff-cleanup-no-such-folder-8f21");
        assert_eq!(remove_superseded_binaries(&absent), Cleaned::default());
        assert!(superseded_binaries(&absent).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn a_binary_a_session_still_holds_is_kept_for_the_next_launch() {
        // The whole of "when no longer locked": Windows refuses to delete a file that is
        // open, and FM-24 wants that file left alone rather than reported. Proved against a
        // real handle, because the behaviour under test is the operating system's.
        use std::os::windows::fs::OpenOptionsExt as _;

        let dir = TempDir::new("locked");
        let held = dir.path().join("handoff-mcp.0.1.0.old.exe");
        File::create(&held).expect("a file");
        File::create(dir.path().join("handoff-mcp.0.2.0.old.exe")).expect("a file");

        // No sharing flags: what a loaded executable image looks like to a deleter.
        let open = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&held)
            .expect("the file opens");

        let cleaned = remove_superseded_binaries(dir.path());
        assert_eq!(
            cleaned,
            Cleaned {
                removed: 1,
                kept: 1
            }
        );
        assert!(held.exists());

        // Closed before the guard removes the folder, so the temporary directory goes.
        drop(open);
    }
}
