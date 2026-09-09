//! Runbooks (§4.5, §7.12, RUN-01..10, DD-18).
//!
//! The app writes `~/.handoff/runbooks/*.json`; the server only reads them (§3.4). What is
//! written is the sequence actually executed, with the values replaced by placeholders — a
//! value never travels into a runbook file, which is the failure the writer exists to
//! prevent.
//!
//! The three steps of §7.12, in the order the writer performs them:
//!
//! 1. [`sequence`] builds the sequence actually executed from the handoff's rounds and the
//!    diary the log kept of them (§4.5.1).
//! 2. [`placeholders`] replaces every value by `{{name}}`, describes each name by the step
//!    it appeared in, and runs the certain detector once more over the finished text
//!    (§4.5.2).
//! 3. [`writer`] infers the origin with the matching rule (DD-18), decides which row of the
//!    §7.12 table this run is, and writes the file atomically.
//!
//! [`matching`] is here because the rule of §4.5.3 is shared with the server and is checked
//! against the same fixtures.

use serde::{Deserialize, Serialize};

use crate::format::runbook::Runbook;

pub mod matching;
pub mod placeholders;
pub mod sequence;
pub mod writer;

pub use writer::RunbookWriter;

/// A rewrite of an existing runbook that the user has not decided on yet (§7.12, RUN-09).
///
/// The third row of the §7.12 table: a handoff that came from a runbook, was corrected, and
/// ended well produces a sequence that differs from the one on disk. The file is **not**
/// touched — the user is asked, one click, and until they answer the new sequence lives in
/// the handoff's `state_json`, which is why this is serialisable and why it holds the whole
/// proposed document rather than a diff.
///
/// It carries no value: [`Runbook`] holds names and placeholders only, which is the same
/// reason a runbook may be written at all (RUN-04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunbookProposal {
    /// The id of the runbook on disk, which the proposed document keeps.
    pub runbook_id: String,
    /// Where that file is.
    ///
    /// A string and not a `PathBuf`: this travels through `state_json`, and a path that
    /// failed to serialise would make the row itself unwritable long after the proposal was
    /// made. Both supported platforms give UTF-8 paths (REQUIREMENTS §1.5).
    pub path: String,
    /// The goal of the runbook as it stands on disk, for the sentence that asks the user.
    pub goal: String,
    /// What the file would become.
    pub runbook: Runbook,
}

impl RunbookProposal {
    /// The file's own name, which is what the Runbooks page lists a runbook by (DD-17).
    #[must_use]
    pub fn file_name(&self) -> String {
        std::path::Path::new(&self.path).file_name().map_or_else(
            || self.path.clone(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    /// What a window needs to draw the question; the document itself stays in the store.
    #[must_use]
    pub fn summary(&self) -> RunbookProposalSummary {
        RunbookProposalSummary {
            runbook_id: self.runbook_id.clone(),
            file_name: self.file_name(),
            goal: self.goal.clone(),
        }
    }
}

/// The proposal as a projection carries it (§7.6).
///
/// A snapshot is cloned on every repaint and on every Stop hook, so what crosses is the
/// three strings the notice needs and never the proposed document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunbookProposalSummary {
    /// The runbook the proposal would rewrite.
    pub runbook_id: String,
    /// Its file name.
    pub file_name: String,
    /// Its goal, as it stands on disk.
    pub goal: String,
}
