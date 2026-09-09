//! Settings → Runbooks: the folder of §7.12, as the window draws it (RUN-01, RUN-09).
//!
//! The list is `~/.handoff/runbooks/` read from disk, not from the log: a runbook is a file
//! (RUN-03a, §3.4), the server reads the same folder, and a page that listed a table would
//! be listing our record of the folder rather than the folder. `RunbookWriter::stored`
//! already skips a file it cannot parse with a warning, which is the app's half of FM-19 —
//! one broken file must not empty the page.
//!
//! Three controls, and one rule each:
//!
//! - **Open folder** hands the path to the operating system. It is the whole of RUN-03's
//!   "the user owns them": there is no editor here and there never will be.
//! - **Delete** moves the file to the OS trash (§7.12, RUN-01 "the user can delete any
//!   runbook"). The trash and not `remove_file`, because a recipe the user spent a handoff
//!   producing is not a temporary file. **The webview names the file and never the path**:
//!   the name is resolved against the writer's own folder here and refused if it is not a
//!   plain `*.json` name, the same rule [`super::commands::open_secret_file`] applies to a
//!   `secrets` destination.
//! - **Accept / Decline** answers the update proposal of RUN-09. The store owns it — the
//!   document lives in `state_json` until it is decided (§7.12 row 3) — so this is one
//!   command on the store and no document crosses the boundary in either direction. §7.6
//!   asks for the proposals **here**, so [`runbook_proposals`] lists every handoff that has
//!   one; the same question is also drawn on the tab that produced it, where the user can see
//!   the correction it came from, and both call the same command.
//!
//! A runbook may show two things that look like defects and are not, both recorded in
//! `DEVIATIONS.md` under T-044: a step text carrying `[treated as secret: <kind>]`, where
//! the ingress detector matched outside a declared value, and a `last_run_failed_at` newer
//! or older than `last_verified_at` — a run that failed and was corrected keeps both, and
//! RUN-09 never unmarks the first.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt as _;

use crate::format::outcome::RunbookTrust;
use crate::log::Timestamp;
use crate::runbooks::matching::StoredRunbook;
use crate::runbooks::RunbookWriter;

use super::commands::CoreState;

/// One row of the page (§7.12: name, where, goal, trust, last verified, last run failed,
/// runs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookEntryView {
    /// `rb_` plus ten characters.
    pub id: String,
    /// The file's own name (DD-17), which is also what **Delete** names.
    pub file_name: String,
    /// Where the work happens.
    pub location: String,
    /// What it achieves.
    pub goal: String,
    /// `verified` or `confirmed_by_user` (RUN-01).
    pub trust: RunbookTrust,
    /// When it was last verified, RFC 3339 (RUN-08).
    pub last_verified_at: String,
    /// When a run from it last failed, or nothing (RUN-09).
    pub last_run_failed_at: Option<String>,
    /// How many verified or confirmed executions are folded into it.
    pub runs: u32,
    /// How many steps its sequence has.
    pub steps: usize,
}

/// One rewrite waiting for an answer, as the Runbooks page lists it (§7.12 row 3, RUN-09).
///
/// The handoff that produced it is what the answer names, so it travels beside the runbook:
/// the page asks the question and the store, which owns the document, performs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookProposalRow {
    /// The handoff carrying the proposal; `resolve_runbook_proposal` takes this id.
    pub handoff_id: String,
    /// The runbook the rewrite would replace.
    pub runbook_id: String,
    /// Its file name (DD-17), which is what the question names.
    pub file_name: String,
    /// Its goal, as it stands on disk.
    pub goal: String,
}

/// Every runbook rewrite waiting for an answer (§7.6's Runbooks row, RUN-09).
///
/// Read from the store rather than from disk: a proposal is a fact about a handoff that
/// finished, kept in its `state_json` until it is decided, and the file it names is
/// deliberately untouched until then.
///
/// # Errors
///
/// None it can produce: the `Result` is what Tauri asks of an asynchronous command, and a
/// store that is not there answers with an empty list like every other command here.
#[tauri::command]
pub async fn runbook_proposals(
    core: State<'_, CoreState>,
) -> Result<Vec<RunbookProposalRow>, String> {
    let Some(core) = core.get().cloned() else {
        return Ok(Vec::new());
    };
    Ok(core
        .store
        .list_for_ui(Timestamp::now())
        .await
        .into_iter()
        .filter_map(|snapshot| {
            let proposal = snapshot.runbook_proposal?;
            Some(RunbookProposalRow {
                handoff_id: snapshot.id,
                runbook_id: proposal.runbook_id,
                file_name: proposal.file_name,
                goal: proposal.goal,
            })
        })
        .collect())
}

/// The Runbooks page (§7.12). Empty when the folder does not exist yet.
#[tauri::command]
#[must_use]
pub fn runbooks() -> Vec<RunbookEntryView> {
    RunbookWriter::in_handoff_home()
        .stored()
        .iter()
        .map(entry_of)
        .collect()
}

/// Opens `~/.handoff/runbooks/` with the operating system's file manager (RUN-03).
///
/// # Errors
///
/// The platform's message, for the window to show. A folder that does not exist yet is one
/// of them: it is created when the channel starts, so the only way to be here without one is
/// a folder somebody removed in between.
#[tauri::command]
pub fn open_runbooks_folder(app: AppHandle) -> Result<(), String> {
    let folder = RunbookWriter::in_handoff_home().dir().to_owned();
    app.opener()
        .open_path(folder.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|error| error.to_string())
}

/// Moves one runbook to the operating system's trash (§7.12, RUN-01).
///
/// # Errors
///
/// A name that is not a plain `*.json` file name of the runbook folder, and the platform's
/// message when the file cannot be moved.
#[tauri::command]
pub fn delete_runbook(file_name: String) -> Result<(), String> {
    let writer = RunbookWriter::in_handoff_home();
    let path = resolve(writer.dir(), &file_name)?;
    trash::delete(&path).map_err(|error| error.to_string())
}

/// The user answered the rewrite of RUN-09: `accept` rewrites the file, otherwise both are
/// kept (§7.12 row 3).
///
/// # Errors
///
/// The store's own message: an unknown handoff, a document that no longer validates, or a
/// write the disk refused. A proposal that was already answered is not an error.
#[tauri::command]
pub async fn resolve_runbook_proposal(
    core: State<'_, CoreState>,
    id: String,
    accept: bool,
) -> Result<(), String> {
    let Some(core) = core.get().cloned() else {
        return Ok(());
    };
    core.store
        .resolve_runbook_proposal(id, accept)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The file `file_name` names inside `dir`, or the reason it is not one.
///
/// The webview is the one part of this application that must never be believed about a path
/// (§7.6), so what crosses is a name and what is checked here is that it is one: no
/// separator, no `..`, no drive letter, and the `.json` extension every runbook has. The
/// answer is then `dir` joined with it and nothing else, which cannot leave the folder.
fn resolve(dir: &Path, file_name: &str) -> Result<PathBuf, String> {
    let plain = Path::new(file_name)
        .file_name()
        .is_some_and(|name| name == file_name);
    if !plain || !file_name.ends_with(".json") {
        return Err(format!("{file_name} is not a runbook of this folder"));
    }
    let path = dir.join(file_name);
    if !path.is_file() {
        return Err(format!("{file_name} is not in the runbook folder"));
    }
    Ok(path)
}

fn entry_of(stored: &StoredRunbook) -> RunbookEntryView {
    let runbook = &stored.runbook;
    RunbookEntryView {
        id: runbook.id.clone(),
        file_name: stored
            .path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
        location: runbook.r#where.clone(),
        goal: runbook.goal.clone(),
        trust: runbook.trust,
        last_verified_at: runbook.last_verified_at.clone(),
        last_run_failed_at: runbook.last_run_failed_at.clone(),
        runs: runbook.runs,
        steps: runbook.steps.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    use crate::log::testing::{clean, tempdir};

    #[test]
    fn a_name_with_a_separator_in_it_never_becomes_a_path() {
        let dir = tempdir();
        for name in [
            "..\\other.json",
            "../other.json",
            "sub/one.json",
            "C:\\keys\\id_rsa.json",
        ] {
            assert!(resolve(&dir, name).is_err(), "{name}");
        }
        clean(&dir);
    }

    #[test]
    fn a_name_that_is_not_a_runbook_is_refused_before_the_disk_is_touched() {
        let dir = tempdir();
        fs::write(dir.join("notes.txt"), "not a runbook").expect("a file");
        assert!(resolve(&dir, "notes.txt").is_err());
        assert!(resolve(&dir, "missing.json").is_err());
        clean(&dir);
    }

    #[test]
    fn a_runbook_of_the_folder_resolves_to_its_own_path() {
        let dir = tempdir();
        let name = "stripe__webhook__rb_0123456789.json";
        fs::write(dir.join(name), "{}").expect("a file");
        assert_eq!(resolve(&dir, name).expect("a path"), dir.join(name));
        clean(&dir);
    }

    /// One runbook file, written as the document the writer produces (§4.5).
    fn write_runbook(dir: &Path, name: &str, id: &str) {
        let document = serde_json::json!({
            "runbook_version": 1,
            "id": id,
            "where": "Stripe Dashboard",
            "goal": "Register the webhook",
            "why_human": "only a person can log in",
            "url": null,
            "lang": "en",
            "values": {"endpoint_url": {"description": "paste {{endpoint_url}}"}},
            "secrets": {},
            "steps": [{
                "text": "paste {{endpoint_url}}",
                "url": null,
                "values": ["endpoint_url"],
                "warning": null,
                "annotations": []
            }],
            "verify": null,
            "trust": "verified",
            "last_verified_at": "2026-09-09T10:00:00.000Z",
            "last_run_failed_at": null,
            "runs": 1,
            "created_at": "2026-09-09T10:00:00.000Z",
            "updated_at": "2026-09-09T10:00:00.000Z",
            "origin": {"app": "handoff-app", "app_version": "0.1.0"}
        });
        fs::write(
            dir.join(name),
            serde_json::to_string_pretty(&document).expect("serialisable"),
        )
        .expect("a runbook file");
    }

    #[test]
    fn the_page_lists_what_the_folder_holds() {
        let dir = tempdir();
        write_runbook(&dir, "stripe__webhook__rb_0123456789.json", "rb_0123456789");
        let entries: Vec<RunbookEntryView> = RunbookWriter::at(&dir)
            .stored()
            .iter()
            .map(entry_of)
            .collect();
        let [entry] = entries.as_slice() else {
            panic!("one runbook, {entries:?}");
        };
        assert_eq!(entry.id, "rb_0123456789");
        assert_eq!(entry.file_name, "stripe__webhook__rb_0123456789.json");
        assert_eq!(entry.location, "Stripe Dashboard");
        assert_eq!(entry.trust, RunbookTrust::Verified);
        assert_eq!(entry.runs, 1);
        assert_eq!(entry.steps, 1);
        assert_eq!(entry.last_run_failed_at, None);
        clean(&dir);
    }

    #[test]
    fn a_file_the_folder_cannot_parse_costs_that_row_and_no_other() {
        // FM-19, the app's half: one broken file must not empty the page.
        let dir = tempdir();
        write_runbook(&dir, "good__one__rb_0123456789.json", "rb_0123456789");
        fs::write(dir.join("broken.json"), "{ not json").expect("a file");
        let entries: Vec<RunbookEntryView> = RunbookWriter::at(&dir)
            .stored()
            .iter()
            .map(entry_of)
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "rb_0123456789");
        clean(&dir);
    }
}
