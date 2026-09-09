//! What an installation adapter returns when it will not touch a file.
//!
//! Every variant here is a refusal, not a partial result: the adapters write a user's agent
//! configuration, and INST-04 promises that configuration is never replaced. So a file that
//! cannot be read, a file that is not JSON, and a file that changed under a plan the user
//! already consented to all stop the work rather than guess. The one thing this module must
//! never do is let a caller continue with half an installation and no error.
//!
//! Error texts here are English and name paths: they reach a log line and the settings
//! screen's failure state, never a chat. A path is not a spec value, so R-19 is not in
//! play; a token or a spec never reaches these strings because nothing here reads one.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// Why an adapter refused to detect, plan, apply or uninstall.
#[derive(Debug, Error)]
pub enum InstallError {
    /// The file is there and could not be read.
    #[error("{path} could not be read: {source}")]
    Unreadable {
        /// The file that could not be read.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },

    /// The file is there and is not a JSON object.
    ///
    /// Never repaired: a configuration we cannot read is a configuration we cannot promise
    /// to preserve, and overwriting it is the one failure INST-04 does not survive.
    #[error("{path} is not a JSON object and was left untouched: {detail}")]
    Malformed {
        /// The file that could not be parsed.
        path: PathBuf,
        /// The parser's complaint, or the shape that was found instead of an object.
        detail: String,
    },

    /// A file or a folder could not be written.
    #[error("{path} could not be written: {source}")]
    Unwritable {
        /// The file or folder that could not be written.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },

    /// The file changed between the plan the user saw and the moment it was applied.
    ///
    /// INST-01 shows the user each exact modification before it happens, so applying a plan
    /// against a file that no longer looks like the plan's `before` would write something
    /// nobody consented to. The caller re-plans and asks again.
    #[error("{path} changed since the plan was made ({location}); nothing was written")]
    Stale {
        /// The file that moved on.
        path: PathBuf,
        /// Where inside it, rendered as the consent screen renders it.
        location: String,
    },

    /// The file was written and does not hold what was written.
    ///
    /// §7.15 asks `apply` to re-read and verify. A failure here means another process is
    /// writing the same file — Claude Code rewrites `~/.claude.json` while it runs — and
    /// the user is told rather than left with a registration that does not exist.
    #[error("{path} does not hold the change after it was written ({location})")]
    NotVerified {
        /// The file that was written.
        path: PathBuf,
        /// Where inside it.
        location: String,
    },

    /// The bundled server executable could not be located (SRV-25, §3.5).
    ///
    /// There is nothing to register without it, and registering a path that does not exist
    /// would give every session text mode with no explanation (FM-23).
    #[error("the bundled server was not found next to the application: {detail}")]
    NoServer {
        /// What was tried.
        detail: String,
    },
}

impl InstallError {
    /// The file is there and could not be read.
    pub(super) fn unreadable(path: &Path, source: std::io::Error) -> Self {
        Self::Unreadable {
            path: path.to_path_buf(),
            source,
        }
    }

    /// The file is there and is not JSON.
    pub(super) fn malformed(path: &Path, source: serde_json::Error) -> Self {
        Self::Malformed {
            path: path.to_path_buf(),
            detail: source.to_string(),
        }
    }

    /// The file is JSON, and not an object.
    pub(super) fn not_an_object(path: &Path) -> Self {
        Self::Malformed {
            path: path.to_path_buf(),
            detail: "the top level is not an object".to_owned(),
        }
    }

    /// A file or folder could not be written.
    pub(super) fn unwritable(path: &Path, source: std::io::Error) -> Self {
        Self::Unwritable {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// The result of every fallible function of this module.
pub type Result<T> = std::result::Result<T, InstallError>;
