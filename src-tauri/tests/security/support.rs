//! What several checks of this suite need: a private folder and a private endpoint.

use std::path::{Path, PathBuf};

use handoff_app_lib::channel::Endpoint;
use handoff_app_lib::ids;

/// A temporary folder of this test's own, removed when the guard is dropped.
///
/// The name is short on purpose: a Unix socket goes inside it, `sun_path` is 104 bytes, and
/// a macOS temporary directory already spends about fifty of them (§5.8, FM-12).
pub struct TempDir(PathBuf);

impl TempDir {
    /// A new, empty folder.
    ///
    /// # Panics
    ///
    /// When it cannot be created.
    #[must_use]
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("bsec-{name}-{}", short_id()));
        std::fs::create_dir_all(&dir).expect("the temporary directory is created");
        Self(dir)
    }

    /// Where it is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Best effort: a file still held open on Windows is not worth failing a green test.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An endpoint nobody else uses — a pipe name of this process's own, or a socket inside
/// `dir` — so that the checks run in parallel and never touch the real one.
#[must_use]
pub fn private_endpoint(dir: &Path) -> Endpoint {
    let unique = short_id();
    if cfg!(windows) {
        Endpoint::Pipe {
            name: format!(r"\\.\pipe\handoff-sec-{}-{unique}", std::process::id()),
        }
    } else {
        Endpoint::Unix {
            path: dir.join(format!("{unique}.sock")),
            pointer: None,
        }
    }
}

/// Eight characters, unique enough for one test's folder or endpoint.
fn short_id() -> String {
    ids::new_session_ref()
        .strip_prefix("ses_")
        .expect("a session ref is prefixed")
        .to_owned()
}
