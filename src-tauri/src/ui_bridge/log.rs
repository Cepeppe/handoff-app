//! Settings → Log: the record of §7.11, as the window draws it (LOG-01..05).
//!
//! Three rules decide everything in this module, and the first is the one that made it a
//! module of its own rather than three commands in [`super::commands`]:
//!
//! - **It reads the database and never the store.** The store keeps the **true** spec in
//!   memory for the copy button of DET-04; the log keeps the masked one (LOG-02,
//!   T-030). A Log page fed from the store would put a secret-treated
//!   value on a settings page, which is the one thing §7.11 promises it does not hold. The
//!   window has a connection of its own ([`super::Ui::with_db`]) and that is what is used.
//! - **It lists closed handoffs.** `log::handoffs::list_final` is documented as "the Log
//!   page" since T-030, and it is the right set: what is not final is a tab in the overlay,
//!   where the user acts on it. Deleting a row of work in flight would leave a tab whose
//!   next transition writes it back.
//! - **Deleting goes through the store, reading does not.** The store owns the handoffs in
//!   memory (§7.4) and a row deleted behind its back would come back at the next write, so
//!   both deletions are its commands. Where there is no store at all — a listener that could
//!   not bind leaves the app running without one (`lib.rs`) — the window's own connection
//!   does it, because then there is nothing in memory to keep in step with.
//!
//! What the detail view shows is what the tables of §7.11 hold, and the deliverable's list
//! read column by column: the masked spec, the outcome the agent was given, the rounds with
//! their verification report **labelled by the window** as "declared by agent" (VER-05), and
//! the `sends` — the text exactly as it left, or a screenshot's hash, dimensions and boxes
//! (LOG-03: never pixels).
//!
//! There is no retention setting and no retention job (LOG-05); nothing here runs on a
//! timer, and the page says so by having no such control.

use serde::Serialize;
use tauri::{AppHandle, Manager as _, State};
use tauri_plugin_dialog::DialogExt as _;

use crate::format::outcome::{Outcome, OutcomeStatus};
use crate::format::spec::{HandoffSpec, SpecValue};
use crate::log::handoffs::HandoffRow;
use crate::log::rounds::RoundRow;
use crate::log::sends::SendRow;
use crate::log::{handoffs, maintenance, rounds, sends, Db, Timestamp};
use crate::sessions::registry::base_name;
use crate::store::Handoff;

use super::commands::CoreState;
use super::Ui;

/// The name the save dialog of LOG-04 opens with.
const EXPORT_FILE_NAME: &str = "baton-log.json";

/// What a command that **writes** answers when the window has no connection to the log.
///
/// It cannot happen in a launched application — `lib.rs` opens the window's own connection
/// whether or not the channel came up — so the one way to be here is a database that refused
/// to open at all. The reads answer an empty page in that case, which claims nothing; a
/// deletion or an export cannot do the same, because "done" would be a claim this side has
/// no way to make. The message reaches the user through the page's own failure line.
const NO_LOG: &str = "the log could not be opened";

/// One entry of the list (§7.11: date, agent · project, goal, final state, rounds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntryView {
    /// `hf_` + 10 characters.
    pub id: String,
    /// When it opened.
    pub created_at: Timestamp,
    /// When it closed. Always present on this page, which lists final handoffs only.
    pub closed_at: Option<Timestamp>,
    /// The agent, as the tab strip labels it.
    pub agent: Option<String>,
    /// The project folder's name.
    pub project: Option<String>,
    /// The spec's goal, or the user's own request when no spec ever arrived.
    pub goal: Option<String>,
    /// The catalogue key of the final state's label (§8.4).
    pub state_key: &'static str,
    /// How many rounds it took (VER-10).
    pub rounds: u32,
    /// Whether an agent ever collected the outcome (SRV-23).
    pub delivered: bool,
}

/// A spec as the log holds it: values already replaced by the mask of §5.9 (LOG-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSpecView {
    /// What had to be achieved.
    pub goal: String,
    /// Where.
    pub location: String,
    /// Why a person.
    pub why_human: String,
    /// The starting point, when the spec had one.
    pub url: Option<String>,
    /// Every declared value, in document order, as text.
    pub values: Vec<LogValueView>,
    /// Variable name to destination file (SEC-02). Names only, as everywhere.
    pub secrets: Vec<LogSecretView>,
    /// The steps as the spec declared them.
    pub steps: Vec<LogStepView>,
    /// What the agent was asked to check (VER-04).
    pub verify: Option<String>,
}

/// One declared value of the stored spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogValueView {
    /// The key of `values`.
    pub name: String,
    /// Its items: one for a single value, one per entry for a list.
    pub items: Vec<String>,
}

/// One entry of the spec's `secrets`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSecretView {
    /// The variable name.
    pub name: String,
    /// The destination file.
    pub file: String,
}

/// One step of the stored spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStepView {
    /// 1-based.
    pub index: u32,
    /// Its text.
    pub text: String,
    /// Its warning (GUIDE-04).
    pub warning: Option<String>,
}

/// One round, with what the agent declared about it (VER-05, VER-10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRoundView {
    /// 1-based.
    pub no: u32,
    /// When it opened.
    pub started_at: Timestamp,
    /// When it closed, if it did.
    pub ended_at: Option<Timestamp>,
    /// Its steps' texts, in order.
    pub steps: Vec<String>,
    /// `true`, `false`, or absent when the agent could not check.
    pub verify_ok: Option<bool>,
    /// What it found. Drawn under the "declared by agent" label (VER-05, PRIN-08).
    pub verify_detail: Option<String>,
    /// When it reported.
    pub verify_reported_at: Option<Timestamp>,
    /// Whether the report arrived after the handoff was already `not_verified` (DD-16).
    pub verify_late: bool,
}

/// One thing that left the machine (LOG-03).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSendView {
    /// When.
    pub at: Timestamp,
    /// `question`, `screenshot_text`, `screenshot_image`, `defer` or `abandon`.
    pub kind: &'static str,
    /// The text exactly as it left, after the redaction of §7.10.
    pub text: Option<String>,
    /// The image's hash. Never the pixels: the log stores none (LOG-03).
    pub image_sha256: Option<String>,
    /// Its width in pixels.
    pub image_w: Option<i64>,
    /// Its height in pixels.
    pub image_h: Option<i64>,
    /// How many redaction boxes were burnt into it.
    pub redaction_boxes: usize,
    /// Which engine read it (OCR-01).
    pub ocr_engine: Option<String>,
}

/// One whole entry (§7.11: the spec, the outcome, the rounds, what was sent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogDetailView {
    /// The list entry this expands.
    pub entry: LogEntryView,
    /// The user's own words, when it grew from a request (OPEN-04).
    pub request_text: Option<String>,
    /// BCP-47 tag of the spec's texts (GUIDE-06).
    pub lang: Option<String>,
    /// The spec, with the mask of §5.9 where the certain detector matched (LOG-02).
    pub spec: Option<LogSpecView>,
    /// The status of the outcome the agent was given (§4.3).
    pub outcome_status: Option<OutcomeStatus>,
    /// The sentence it carried, as the agent read it.
    pub outcome_instruction: Option<String>,
    /// What the user wrote with it, when they wrote something.
    pub outcome_user_text: Option<String>,
    /// The locations the certain detector masked, and the family of each (DET-04).
    pub secret_treated: Vec<LogSecretTreatedView>,
    /// The rounds, oldest first.
    pub rounds: Vec<LogRoundView>,
    /// What left the machine, oldest first.
    pub sends: Vec<LogSendView>,
}

/// One masked location the outcome reported (§4.7.5, DET-04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSecretTreatedView {
    /// The display path, `values.api_key` and its kind.
    pub location: String,
    /// The family: `api_key`, `token`, … never the pattern id.
    pub kind: String,
}

/// The list of the Log page, most recently closed first (LOG-04).
///
/// Empty when the window has no connection, which is the same answer every other command
/// gives in that case: an error the user cannot act on is worse than an empty page.
#[tauri::command]
pub fn log_entries(app: AppHandle) -> Vec<LogEntryView> {
    app.state::<Ui>()
        .with_db(entries_of)
        .unwrap_or_else(|| Ok(Vec::new()))
        .unwrap_or_else(|error| {
            tracing::warn!(error = %error, "the log could not be listed");
            Vec::new()
        })
}

/// One whole entry, or nothing when that id is not in the log any more.
#[tauri::command]
pub fn log_detail(app: AppHandle, id: String) -> Option<LogDetailView> {
    app.state::<Ui>()
        .with_db(|db| detail_of(db, &id))?
        .unwrap_or_else(|error| {
            tracing::warn!(error = %error, "the log entry could not be read");
            None
        })
}

/// Deletes one entry, with its rounds, events and sends (LOG-04, §7.11 cascade).
///
/// # Errors
///
/// The store's own message when the handoff is not final or the deletion fails.
#[tauri::command]
pub async fn delete_log_entry(
    app: AppHandle,
    core: State<'_, CoreState>,
    id: String,
) -> Result<(), String> {
    if let Some(core) = core.get().cloned() {
        return core
            .store
            .delete_log_entry(id)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string());
    }
    // No store: nothing in memory to keep in step with, so the window's own connection is
    // the whole of it.
    let Some(deleted) = app
        .state::<Ui>()
        .with_db(|db| handoffs::delete_handoff(db, &id))
    else {
        return Err(NO_LOG.to_owned());
    };
    deleted.map(|_| ()).map_err(|error| error.to_string())
}

/// Empties the log (LOG-04): every table but `settings`.
///
/// # Errors
///
/// The store's own message when the transaction fails.
#[tauri::command]
pub async fn delete_log(app: AppHandle, core: State<'_, CoreState>) -> Result<(), String> {
    if let Some(core) = core.get().cloned() {
        // Collected before the store is asked, and the guard is dropped here: no lock
        // crosses the actor's channel. `Store::delete_log` says what these are for.
        let live_sessions = core.registry().rows();
        return core
            .store
            .delete_log(live_sessions)
            .await
            .map_err(|error| error.to_string());
    }
    let Some(emptied) = app.state::<Ui>().with_db_mut(maintenance::delete_all) else {
        return Err(NO_LOG.to_owned());
    };
    emptied.map_err(|error| error.to_string())
}

/// Writes every table to a file the user chooses (LOG-04). `None` when they cancelled.
///
/// # Errors
///
/// The message of a write that failed, for the window to show.
#[tauri::command]
pub async fn export_log(app: AppHandle) -> Result<Option<String>, String> {
    let (answered, answer) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(EXPORT_FILE_NAME)
        .save_file(move |path| {
            // The receiver is gone only if the window closed while the dialog was open.
            let _ = answered.send(path);
        });
    let Some(path) = answer.await.ok().flatten() else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|error| error.to_string())?;
    let Some(written) = app
        .state::<Ui>()
        .with_db(|db| maintenance::export_json(db, &path))
    else {
        return Err(NO_LOG.to_owned());
    };
    written.map_err(|error| error.to_string())?;
    Ok(Some(path.display().to_string()))
}

/// The body of [`log_entries`], without a Tauri handle.
fn entries_of(db: &Db) -> crate::log::Result<Vec<LogEntryView>> {
    let counts = rounds::counts(db)?;
    Ok(handoffs::list_final(db)?
        .into_iter()
        .map(|row| entry_of(&row, counts.get(&row.id).copied().unwrap_or(0)))
        .collect())
}

/// The body of [`log_detail`], without a Tauri handle.
fn detail_of(db: &Db, id: &str) -> crate::log::Result<Option<LogDetailView>> {
    let Some(row) = handoffs::get(db, id)? else {
        return Ok(None);
    };
    let round_rows = rounds::list_for_handoff(db, id)?;
    let outcome = stored_outcome(&row);
    Ok(Some(LogDetailView {
        entry: entry_of(&row, u32::try_from(round_rows.len()).unwrap_or(u32::MAX)),
        request_text: row.request_text.clone(),
        lang: row.lang.clone(),
        spec: row.spec.as_ref().map(spec_view),
        outcome_status: outcome.as_ref().map(|outcome| outcome.status),
        outcome_instruction: outcome.as_ref().map(|outcome| outcome.instruction.clone()),
        outcome_user_text: outcome
            .as_ref()
            .and_then(|outcome| outcome.user_text.clone()),
        secret_treated: outcome
            .as_ref()
            .map(|outcome| {
                outcome
                    .secret_treated
                    .iter()
                    .map(|treated| LogSecretTreatedView {
                        location: treated.location.clone(),
                        kind: treated.kind.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        rounds: round_rows.iter().map(round_view).collect(),
        sends: sends::list_for_handoff(db, id)?
            .iter()
            .map(send_view)
            .collect(),
    }))
}

/// The outcome the agent was given, out of the state the row carries.
///
/// `state_json` is the store's own serialisation (DD-31), so it is read back through the
/// store's own type rather than by naming a key here: a field renamed there would then be a
/// compile error and not a Log page that quietly stopped showing outcomes. Nothing true
/// comes out of it — `Handoff::spec` is `serde(skip)`, and `log::handoffs::upsert` sweeps
/// the secret literals out of this very column (LOG-02, T-030).
fn stored_outcome(row: &HandoffRow) -> Option<Outcome> {
    serde_json::from_str::<Handoff>(&row.state_json)
        .inspect_err(
            |error| tracing::warn!(error = %error, handoff_id = %row.id, "unreadable handoff state"),
        )
        .ok()?
        .final_outcome
}

fn entry_of(row: &HandoffRow, rounds: u32) -> LogEntryView {
    LogEntryView {
        id: row.id.clone(),
        created_at: row.created_at.clone(),
        closed_at: row.closed_at.clone(),
        agent: row
            .client_name
            .clone()
            .or_else(|| row.agent_id.clone())
            .filter(|name| !name.is_empty()),
        project: row
            .project_dir
            .as_deref()
            .map(base_name)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned),
        goal: row
            .spec
            .as_ref()
            .map(|spec| spec.goal.clone())
            .or_else(|| row.request_text.clone()),
        state_key: super::view::state_key(row.final_state.unwrap_or(row.state)),
        rounds,
        delivered: row.delivered_at.is_some(),
    }
}

fn spec_view(spec: &HandoffSpec) -> LogSpecView {
    LogSpecView {
        goal: spec.goal.clone(),
        location: spec.r#where.clone(),
        why_human: spec.why_human.clone(),
        url: spec.url.clone(),
        values: spec
            .values
            .iter()
            .map(|(name, value)| LogValueView {
                name: name.clone(),
                items: match value {
                    SpecValue::One(one) => vec![one.clone()],
                    SpecValue::Many(many) => many.clone(),
                },
            })
            .collect(),
        secrets: spec
            .secrets
            .as_ref()
            .map(|secrets| {
                secrets
                    .iter()
                    .map(|(name, file)| LogSecretView {
                        name: name.clone(),
                        file: file.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        steps: spec
            .steps
            .iter()
            .enumerate()
            .map(|(index, step)| LogStepView {
                index: u32::try_from(index + 1).unwrap_or(u32::MAX),
                text: step.text.clone(),
                warning: step.warning.clone(),
            })
            .collect(),
        verify: spec.verify.clone(),
    }
}

fn round_view(row: &RoundRow) -> LogRoundView {
    LogRoundView {
        no: u32::try_from(row.no).unwrap_or(u32::MAX),
        started_at: row.started_at.clone(),
        ended_at: row.ended_at.clone(),
        steps: serde_json::from_str::<Vec<crate::format::spec::HandoffStep>>(&row.steps_json)
            .map(|steps| steps.into_iter().map(|step| step.text).collect())
            .unwrap_or_default(),
        verify_ok: row.verify_ok,
        verify_detail: row.verify_detail.clone(),
        verify_reported_at: row.verify_reported_at.clone(),
        verify_late: row.verify_late,
    }
}

fn send_view(row: &SendRow) -> LogSendView {
    LogSendView {
        at: row.at.clone(),
        kind: row.kind.as_str(),
        text: row.text_as_sent.clone(),
        image_sha256: row.image_sha256.clone(),
        image_w: row.image_w,
        image_h: row.image_h,
        redaction_boxes: row
            .redaction_boxes_json
            .as_deref()
            .and_then(|boxes| serde_json::from_str::<Vec<serde_json::Value>>(boxes).ok())
            .map_or(0, |boxes| boxes.len()),
        ocr_engine: row.ocr_engine.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::log::events::EventKind;
    use crate::log::handoffs::HandoffState;
    use crate::log::sends::SendKind;
    use crate::log::testing::{
        at, event, handoff, populated, round, send, session, spec_with_secret, STRIPE_KEY,
    };
    use crate::log::{events, sessions};

    /// A database with one closed handoff whose spec carried a certain secret.
    fn closed() -> Db {
        let db = Db::open_in_memory().expect("a database");
        sessions::register(&db, &session("ses_00000001")).expect("a session");
        let mut row = handoff("hf_0000000001");
        row.session_ref = Some("ses_00000001".to_owned());
        row.spec = Some(spec_with_secret());
        row.state = HandoffState::Verified;
        row.final_state = Some(HandoffState::Verified);
        row.closed_at = Some(at("2026-09-08T12:00:00Z"));
        handoffs::upsert(&db, &row).expect("a handoff");
        let mut first = round("hf_0000000001", 1);
        first.verify_ok = Some(false);
        first.verify_detail = Some("the signature did not validate".to_owned());
        first.verify_reported_at = Some(at("2026-09-08T11:40:00Z"));
        rounds::upsert(&db, &first).expect("a round");
        let mut second = round("hf_0000000001", 2);
        second.verify_ok = Some(true);
        second.verify_late = true;
        rounds::upsert(&db, &second).expect("a second round");
        sends::append(&db, &send("hf_0000000001", SendKind::Question)).expect("a send");
        events::append(&db, &event("hf_0000000001", EventKind::Confirm)).expect("an event");
        db
    }

    #[test]
    fn the_list_carries_what_the_page_draws() {
        let entries = entries_of(&closed()).expect("a list");
        let [entry] = entries.as_slice() else {
            panic!("one closed handoff, {entries:?}");
        };
        assert_eq!(entry.id, "hf_0000000001");
        assert_eq!(entry.agent.as_deref(), Some("claude-code"));
        assert_eq!(entry.project.as_deref(), Some("baton"));
        assert_eq!(entry.state_key, "state.verified");
        assert_eq!(entry.rounds, 2);
        assert!(!entry.delivered);
    }

    #[test]
    fn a_handoff_that_is_still_open_is_not_on_this_page() {
        // §7.11's Log page is the record; what is not final is a tab in the overlay.
        let db = populated();
        assert!(entries_of(&db).expect("a list").is_empty());
    }

    #[test]
    fn the_detail_shows_the_masked_spec_and_never_the_value() {
        let detail = detail_of(&closed(), "hf_0000000001")
            .expect("a read")
            .expect("an entry");
        let spec = detail.spec.expect("a spec");
        let rendered = serde_json::to_string(&spec).expect("serialisable");
        assert!(
            !rendered.contains(STRIPE_KEY),
            "the Log page drew a certain secret: {rendered}"
        );
        assert!(
            rendered.contains("[treated as secret: api_key]"),
            "the mask itself is what a reader must see: {rendered}"
        );
        assert_eq!(spec.values.len(), 1);
        assert_eq!(spec.steps.len(), 1);
        assert_eq!(spec.steps[0].index, 1);
    }

    #[test]
    fn the_detail_carries_every_round_with_what_the_agent_declared() {
        let detail = detail_of(&closed(), "hf_0000000001")
            .expect("a read")
            .expect("an entry");
        assert_eq!(detail.rounds.len(), 2);
        assert_eq!(detail.rounds[0].verify_ok, Some(false));
        assert_eq!(
            detail.rounds[0].verify_detail.as_deref(),
            Some("the signature did not validate")
        );
        assert!(!detail.rounds[0].verify_late);
        assert!(detail.rounds[1].verify_late);
        assert_eq!(
            detail.rounds[0].steps,
            vec!["open the dashboard".to_owned()]
        );
    }

    #[test]
    fn the_detail_carries_what_left_the_machine_and_no_pixels() {
        let detail = detail_of(&closed(), "hf_0000000001")
            .expect("a read")
            .expect("an entry");
        let [send] = detail.sends.as_slice() else {
            panic!("one send, {:?}", detail.sends);
        };
        assert_eq!(send.kind, "question");
        assert_eq!(send.text.as_deref(), Some("the dashboard shows the key"));
        assert_eq!(send.image_sha256.as_deref(), Some("0".repeat(64).as_str()));
        assert_eq!(send.image_w, Some(1600));
        assert_eq!(send.redaction_boxes, 0);
    }

    #[test]
    fn an_id_the_log_does_not_hold_is_nothing_rather_than_an_error() {
        assert!(detail_of(&closed(), "hf_9999999999")
            .expect("a read")
            .is_none());
    }

    #[test]
    fn the_outcome_the_agent_was_given_is_read_back_from_the_state() {
        // The one field this module takes out of `state_json`, through the store's own type.
        let db = closed();
        let row = handoffs::get(&db, "hf_0000000001")
            .expect("a read")
            .expect("a row");
        assert!(stored_outcome(&row).is_none(), "the fixture stores none");

        let mut with_state = row;
        with_state.state_json = serde_json::to_string(&serde_json::json!({
            "id": "hf_0000000001",
            "created_at": "2026-09-08T11:00:00.000Z",
            "closed_at": null,
            "session_ref": null,
            "agent_id": null,
            "client_name": null,
            "project_dir": null,
            "opener_label": null,
            "request_text": null,
            "linked_request_id": null,
            "lang": null,
            "secret_treated": [],
            "rounds": [],
            "cursor": {"round": 1, "step_index": 1},
            "deferral_count": 0,
            "pending_question": null,
            "undelivered": [],
            "state": "verified",
            "final_outcome": {
                "outcome_version": 1,
                "handoff_id": "hf_0000000001",
                "status": "verified",
                "final": true,
                "instruction": "Recorded as verified; a runbook was saved.",
                "round": 1,
                "current_step": null,
                "user_text": null,
                "screenshot": null,
                "context": null,
                "skipped_steps": [],
                "notes": [],
                "secret_treated": [],
                "verify": null,
                "deferral_count": 0,
                "resumed_from": null,
                "app_reachable": true,
                "already_delivered": false,
                "runbooks": [],
                "spec_text": null
            },
            "delivered_at": null,
            "verifying_since": null,
            "resumed_from": null
        }))
        .expect("serialisable");
        handoffs::upsert(&db, &with_state).expect("a handoff");

        let detail = detail_of(&db, "hf_0000000001")
            .expect("a read")
            .expect("an entry");
        assert_eq!(detail.outcome_status, Some(OutcomeStatus::Verified));
        assert_eq!(
            detail.outcome_instruction.as_deref(),
            Some("Recorded as verified; a runbook was saved.")
        );
    }
}
