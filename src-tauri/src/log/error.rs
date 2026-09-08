//! What the log returns when it cannot do what it was asked (FM-28).
//!
//! One variant, three causes. FM-28 asks for a single behaviour from every caller — the
//! transition is refused with a visible error, the in-memory state is kept and the action
//! is retried — so the store (T-033) should not have to decide which of a disk-full, a
//! locked database and an unserialisable state deserves which treatment. They are all
//! [`StoreError::Persistence`], and the cause is there for the crash file and the log line.
//!
//! `operation` is a short static phrase, never a value: an error message reaches the UI and
//! a crash file, and R-19 forbids either from carrying a spec value.

use thiserror::Error;

/// The log could not complete what it was asked to do.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The database refused a read or a write, or the row could not be encoded.
    ///
    /// FM-28: the caller keeps its in-memory state, shows the failure and retries; nothing
    /// is lost silently.
    #[error("the local database could not complete {operation}: {source}")]
    Persistence {
        /// What was being done, as a phrase that fits after "could not complete".
        operation: &'static str,
        /// Why it failed.
        #[source]
        source: PersistenceCause,
    },
}

/// Why a [`StoreError::Persistence`] happened.
#[derive(Debug, Error)]
pub enum PersistenceCause {
    /// SQLite refused the statement: a disk full, a locked database, a constraint.
    #[error("{0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A JSON column could not be written or read back.
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    /// The export file could not be written.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// An instant was not RFC 3339. On the way in this is a caller mistake; on the way out
    /// it means the database was edited by hand.
    #[error("{0} is not an RFC 3339 instant")]
    Timestamp(String),
    /// The file on disk is not a schema this build can work with.
    #[error("{0}")]
    Schema(String),
}

impl StoreError {
    /// The failure of `operation`, whatever refused it.
    pub(crate) fn of(operation: &'static str, source: impl Into<PersistenceCause>) -> Self {
        Self::Persistence {
            operation,
            source: source.into(),
        }
    }
}

/// The result of every function of this module.
pub type Result<T> = std::result::Result<T, StoreError>;
