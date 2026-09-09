//! Onboarding, consent, and the Agents settings page (§7.6, §7.15, F-13, INST-01..07).
//!
//! The adapters of [`crate::install`] know how to read and write an agent's configuration;
//! this module is everything between them and the person who has to say yes. Three rules
//! shape it, and all three come straight from INST-01:
//!
//! - **Nothing is written that the user has not seen.** [`consent_plan`] returns the lines
//!   the screen draws *and* a [`crate::install::digest`] of the plan behind them;
//!   [`install_agent`] re-plans, re-digests, and refuses when the two differ. A plan cannot
//!   travel to a webview and come back trusted, and one that is re-planned silently would be
//!   a plan nobody consented to.
//! - **The count is the count of modifications, and the rows are the rows.** INST-02 gives
//!   Claude Code three modifications on two lines; [`ConsentView`] carries both numbers so
//!   the screen never has to derive one from the other.
//! - **Texts are catalogue keys.** An agent's name, a line's sentence, a status: all of them
//!   are keys the window renders in the user's language (T-028). The only strings that cross
//!   are paths and diffs, which are the user's own files.
//!
//! # What is not here
//!
//! The macOS screen-recording *permission call* (T-059): onboarding explains it and opens
//! the settings pane, which is what CAP-04 asks for — the prompt itself needs a restart and
//! must never appear mid-handoff (FM-17).

use serde::Serialize;
use tauri::{AppHandle, Manager as _};
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_opener::OpenerExt as _;

use crate::channel::token;
use crate::install::scan::{self, KNOWN_AGENTS_KEY};
use crate::install::{self, AgentStatus, ConsentLine, InstallAdapter, MovedRegistration, Scope};
use crate::log::settings;
use crate::paths;

/// The setting that says onboarding has been through once (§7.6).
pub const ONBOARDED_KEY: &str = "onboarded";

/// The setting holding the answer to the autostart checkbox of APP-01.
///
/// Written by onboarding and by the General settings page, read at every launch by
/// [`super::general::sync_autostart`]. On by default, which is what the pre-checked box
/// means; until the box is answered there is no row and nothing is written to the user's
/// login items.
pub const AUTOSTART_KEY: &str = "autostart";

/// Welcome (§7.6).
pub const STEP_WELCOME: &str = "welcome";

/// "Move me to /Applications first" — macOS, and only when the bundle is elsewhere.
pub const STEP_MOVE: &str = "move";

/// Agents and consent (INST-01, INST-02).
pub const STEP_AGENTS: &str = "agents";

/// The autostart checkbox of APP-01.
pub const STEP_AUTOSTART: &str = "autostart";

/// The macOS screen-recording explanation (CAP-04).
pub const STEP_SCREEN_RECORDING: &str = "screenRecording";

/// The shortcut check (OPEN-03).
pub const STEP_SHORTCUT: &str = "shortcut";

/// Done.
pub const STEP_DONE: &str = "done";

/// The macOS pane CAP-04 sends the user to.
///
/// It has a command of its own rather than going through [`super::commands::open_url`] —
/// which would accept it, since `x-apple.systempreferences:` is one of the four schemes of
/// SPEC-07 — because this URL is *ours* and not an agent's: the platform guard below belongs
/// with it, and a product constant does not belong in a webview.
const SCREEN_RECORDING_PANE: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// Whether onboarding runs, and which steps it has (§7.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingView {
    /// Whether this is a first launch.
    pub needed: bool,
    /// The steps, in order. Two of them exist on macOS alone.
    pub steps: Vec<&'static str>,
}

/// An agent named for a sentence: its id and the key its name is under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrief {
    /// The agent id (INST-08).
    pub agent_id: &'static str,
    /// The catalogue key of its name.
    pub name_key: &'static str,
}

/// What the launch scan found worth saying something about (INST-05, FM-23).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    /// The agents seen for the first time. Each is announced once, ever.
    pub new_agents: Vec<AgentBrief>,
    /// The registrations naming a path that is no longer ours (FM-23).
    pub moved: Vec<MovedRegistration>,
}

/// A plan as the consent screen shows it (INST-01, INST-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentView {
    /// Whose configuration changes.
    pub agent_id: String,
    /// Its name, as a catalogue key.
    pub name_key: &'static str,
    /// How many modifications: the number INST-02 fixes at three for Claude Code.
    pub modification_count: usize,
    /// The rows the user reads. Fewer than the modifications when an adapter groups some.
    pub lines: Vec<ConsentLine>,
    /// The fingerprint [`install_agent`] checks the plan against.
    pub digest: String,
    /// Whether every modification is already in place: a repair with nothing to repair.
    pub already_in_order: bool,
}

/// The steps of onboarding on this platform, given where the application is.
///
/// Written as a pure function of the two facts it depends on so that both platforms' answers
/// are testable from either one: on the launch platform only one of these branches ever runs,
/// and a step list that can only be read on a Mac is a step list nobody checks.
fn steps_for(macos: bool, in_applications: bool) -> Vec<&'static str> {
    let mut steps = vec![STEP_WELCOME];
    // Before consent, deliberately: the fixed launcher path written into every agent
    // configuration is derived from where the application is (SRV-25), so moving the bundle
    // after registering is FM-23 arranged in advance.
    if macos && !in_applications {
        steps.push(STEP_MOVE);
    }
    steps.push(STEP_AGENTS);
    steps.push(STEP_AUTOSTART);
    if macos {
        steps.push(STEP_SCREEN_RECORDING);
    }
    steps.push(STEP_SHORTCUT);
    steps.push(STEP_DONE);
    steps
}

/// Whether onboarding has already run.
fn onboarded(app: &AppHandle) -> bool {
    onboarded_from(
        app.state::<super::Ui>()
            .with_db(|db| settings::get::<bool>(db, ONBOARDED_KEY).ok().flatten()),
    )
}

/// The same question over what the settings table answered.
///
/// Three answers, and the two that look alike are the ones that matter:
///
/// - **A database with no `onboarded` key** (`Some(None)`) is a first launch. It is what
///   every fresh installation looks like — the table exists from the first migration and the
///   key is written only when onboarding finishes — so reading it as "already done" would
///   mean nobody is ever asked for consent.
/// - **No database at all** (`None`) reports "already done". A launch that cannot record the
///   answer would show onboarding at every start, which is worse than not showing it: the app
///   still works and the settings page still registers.
fn onboarded_from(read: Option<Option<bool>>) -> bool {
    match read {
        Some(stored) => stored.unwrap_or(false),
        None => {
            tracing::warn!("no database to read the onboarding setting from");
            true
        }
    }
}

/// Whether this launch shows onboarding, and what it consists of (§7.6, F-13).
#[tauri::command]
#[must_use]
pub fn onboarding(app: AppHandle) -> OnboardingView {
    OnboardingView {
        needed: !onboarded(&app),
        steps: steps_for(cfg!(target_os = "macos"), scan::app_in_applications()),
    }
}

/// Onboarding is over: remember it, and act on the autostart answer (APP-01).
///
/// The agents found now are recorded as known, so the agent the user has just been shown does
/// not come back as a discovery at the next launch (INST-05).
///
/// The login entry is written here and not at the next launch, because the checkbox is the
/// moment the user answered: a machine that is never restarted would otherwise never get the
/// entry it was promised. A platform that refuses to write it is a warning and not a refusal
/// of the whole step — the answer is stored either way, the General settings page shows what
/// is actually in place, and onboarding is not the screen on which to argue about it.
///
/// # Errors
///
/// The message of the write that failed, for the window to show.
#[tauri::command]
pub fn finish_onboarding(app: AppHandle, autostart: bool) -> Result<(), String> {
    let known = install::scan(&Scope::User)
        .map(|statuses| scan::record_known(&statuses, &[]))
        .unwrap_or_default();

    if let Err(error) = super::general::apply_autostart(&app, autostart) {
        tracing::warn!(error, autostart, "the login entry could not be written");
    }

    let written = app.state::<super::Ui>().with_db(|db| {
        settings::set(db, AUTOSTART_KEY, &autostart)?;
        settings::set(db, KNOWN_AGENTS_KEY, &known)?;
        settings::set(db, ONBOARDED_KEY, &true)
    });
    match written {
        Some(Err(error)) => Err(error.to_string()),
        Some(Ok(())) | None => Ok(()),
    }
}

/// The state of every adapter in `scope` (INST-05, §7.15).
///
/// The Agents settings page and its "Find agents" action, which are the same read: INST-05
/// makes the scan cheap enough to run at every launch, so a button that re-runs it needs no
/// machinery of its own.
///
/// # Errors
///
/// When the bundled server cannot be located, which is the one thing every adapter needs
/// before it can say what it would register (SRV-25).
#[tauri::command]
pub fn agents(scope: Scope) -> Result<Vec<AgentStatus>, String> {
    install::scan(&scope).map_err(|error| error.to_string())
}

/// The launch scan: what is new, and what has moved (INST-05, FM-23, §7.2).
///
/// It is called by the window rather than from `setup()`, and that is not an accident: both
/// of its answers are sentences for a person, and at `setup()` there is no webview listening
/// yet. What it writes is the `known_agents` setting, so an agent is announced once however
/// many times this is called.
#[tauri::command]
#[must_use]
pub fn scan_agents(app: AppHandle) -> ScanReport {
    let Ok(statuses) = install::scan(&Scope::User) else {
        return ScanReport {
            new_agents: Vec::new(),
            moved: Vec::new(),
        };
    };

    let state = app.state::<super::Ui>();
    let known: Vec<String> = state
        .with_db(|db| {
            settings::get::<Vec<String>>(db, KNOWN_AGENTS_KEY)
                .ok()
                .flatten()
        })
        .flatten()
        .unwrap_or_default();

    let new_agents: Vec<AgentBrief> = scan::newly_found(&statuses, &known)
        .into_iter()
        .map(|status| AgentBrief {
            agent_id: status.agent_id,
            name_key: status.name_key,
        })
        .collect();

    if !new_agents.is_empty() {
        let recorded = scan::record_known(&statuses, &known);
        if let Some(Err(error)) = state.with_db(|db| settings::set(db, KNOWN_AGENTS_KEY, &recorded))
        {
            // The cost of not recording it is one repeated notice, so this is a warning and
            // not a failure the user has to answer.
            tracing::warn!(error = %error, "the agents found were not recorded as known");
        }
    }

    ScanReport {
        new_agents,
        // The other half of `install::check_registered_paths()` (§7.2, FM-23), over the scan
        // this call already has rather than over a second one.
        moved: scan::moved_registrations(statuses),
    }
}

/// The plan for `agent_id` in `scope`, as the consent screen shows it (INST-01, INST-02).
///
/// # Errors
///
/// When the agent is not one of ours, or a configuration file cannot be read or is not JSON.
#[tauri::command]
pub fn consent_plan(agent_id: String, scope: Scope) -> Result<ConsentView, String> {
    let adapter = adapter_for(&agent_id)?;
    let plan = adapter.plan(&scope).map_err(|error| error.to_string())?;
    Ok(ConsentView {
        agent_id,
        name_key: scan::name_key(adapter.agent_id()),
        modification_count: plan.len(),
        lines: adapter.consent_lines(&plan),
        digest: install::digest(&plan),
        already_in_order: plan.iter().all(install::Modification::is_noop),
    })
}

/// Writes the plan the user accepted (INST-01, INST-07).
///
/// `digest` is the fingerprint [`consent_plan`] handed the screen. The plan is made again
/// here — the one that crossed to the webview is a rendering, not a promise — and a
/// fingerprint that no longer matches means the file moved on between the screen and the
/// button: nothing is written, and the caller shows the new plan instead.
///
/// # Errors
///
/// The refusal of the adapter, rendered: an unknown agent, a stale plan, a file that cannot
/// be read, written or verified.
#[tauri::command]
pub fn install_agent(agent_id: String, scope: Scope, digest: String) -> Result<(), String> {
    let adapter = adapter_for(&agent_id)?;
    let plan = adapter.plan(&scope).map_err(|error| error.to_string())?;
    if install::digest(&plan) != digest {
        tracing::info!(
            agent_id,
            "the configuration changed since the plan was shown"
        );
        return Err("the configuration changed since it was shown; look at it again".to_owned());
    }
    adapter.apply(&plan).map_err(|error| error.to_string())
}

/// Removes exactly our entries from `scope` (INST-04).
///
/// # Errors
///
/// The refusal of the adapter, rendered.
#[tauri::command]
pub fn uninstall_agent(agent_id: String, scope: Scope) -> Result<(), String> {
    let adapter = adapter_for(&agent_id)?;
    adapter.uninstall(&scope).map_err(|error| error.to_string())
}

/// Regenerates `~/.handoff/channel.token` (FM-10, SRV-07).
///
/// The one repair for "token missing or mismatch": the app owns the file, the server re-reads
/// it at every connection attempt, so nothing has to be restarted — the next call of a session
/// that was in text mode authenticates. A session already connected keeps its connection; it
/// authenticated against the token that was there when it arrived.
///
/// # Errors
///
/// The io failure, rendered: the folder or the file could not be written.
#[tauri::command]
pub fn repair_token() -> Result<(), String> {
    let path = paths::token_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    token::regenerate(&path)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The folder picker of the project scope selector (INST-06).
///
/// `None` when the user cancelled, which is not a failure and says nothing to anybody.
#[tauri::command]
pub async fn pick_project_folder(app: AppHandle) -> Option<String> {
    let (answered, answer) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |folder| {
        // The receiver is gone only if the window closed while the dialog was open.
        let _ = answered.send(folder);
    });
    answer
        .await
        .ok()
        .flatten()
        .and_then(|folder| folder.into_path().ok())
        .map(|folder| folder.display().to_string())
}

/// Opens the macOS screen-recording pane (CAP-04).
///
/// # Errors
///
/// On any other platform, where there is no such pane and the button is not drawn, and when
/// the system refuses to open it.
#[tauri::command]
pub fn open_screen_recording_settings(app: AppHandle) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("there is no screen-recording pane on this platform".to_owned());
    }
    app.opener()
        .open_url(SCREEN_RECORDING_PANE, None::<&str>)
        .map_err(|error| error.to_string())
}

/// The adapter of `agent_id`, or a message naming what went wrong.
fn adapter_for(agent_id: &str) -> Result<Box<dyn InstallAdapter>, String> {
    scan::adapters()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|adapter| adapter.agent_id() == agent_id)
        .ok_or_else(|| format!("{agent_id} is not an agent this build knows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_and_linux_have_five_steps_and_neither_macos_one() {
        let steps = steps_for(false, true);
        assert_eq!(
            steps,
            vec![
                STEP_WELCOME,
                STEP_AGENTS,
                STEP_AUTOSTART,
                STEP_SHORTCUT,
                STEP_DONE
            ]
        );
        // The bundle's location is a macOS question; off macOS it changes nothing.
        assert_eq!(steps_for(false, false), steps);
    }

    #[test]
    fn macos_explains_the_screen_recording_permission() {
        assert!(steps_for(true, true).contains(&STEP_SCREEN_RECORDING));
        assert!(!steps_for(false, true).contains(&STEP_SCREEN_RECORDING));
    }

    #[test]
    fn macos_asks_for_the_move_before_consent_and_only_when_it_is_needed() {
        let outside = steps_for(true, false);
        let inside = steps_for(true, true);

        assert!(!inside.contains(&STEP_MOVE));
        let move_at = outside
            .iter()
            .position(|step| *step == STEP_MOVE)
            .expect("the move step");
        let agents_at = outside
            .iter()
            .position(|step| *step == STEP_AGENTS)
            .expect("the agents step");
        assert!(
            move_at < agents_at,
            "the move is asked for after consent: {outside:?}"
        );
    }

    #[test]
    fn every_step_name_is_a_catalogue_key_in_both_languages() {
        // The window titles each step from `onboarding.<step>Title`; a step added without
        // its two texts would show the key.
        for step in steps_for(true, false) {
            for language in [crate::i18n::Language::En, crate::i18n::Language::It] {
                let key = format!("onboarding.{step}Title");
                assert_ne!(
                    crate::i18n::text(language, &key),
                    key,
                    "{language} has no text for {key}"
                );
            }
        }
    }

    #[test]
    fn the_screen_recording_pane_is_the_one_cap_04_names() {
        // The pane the user has to reach is the screen-recording one and not the Privacy
        // root: CAP-04 asks for "a deep link to the settings pane", and a link that lands one
        // level up is a link the user has to search from.
        assert!(SCREEN_RECORDING_PANE.contains("Privacy_ScreenCapture"));
        // And it is a URL the opener will take, which is the same list SPEC-07 fixes.
        assert!(crate::format::spec::url_allowed(SCREEN_RECORDING_PANE));
    }

    #[test]
    fn a_fresh_installation_has_a_database_and_no_answer_in_it() {
        // The case a first launch actually presents, and the one that is easy to read as its
        // opposite: the `settings` table is there from the first migration, and the key is
        // written only when onboarding finishes. Measured against the running app before this
        // was pinned — it showed no onboarding at all on a brand-new data directory.
        assert!(!onboarded_from(Some(None)));
        assert!(!onboarded_from(Some(Some(false))));
        assert!(onboarded_from(Some(Some(true))));
        // No database: nothing could record an answer, so nothing is asked.
        assert!(onboarded_from(None));
    }

    #[test]
    fn an_unknown_agent_is_refused_by_name() {
        let Err(refusal) = adapter_for("not-an-agent") else {
            panic!("`not-an-agent` was answered with an adapter");
        };
        assert!(refusal.contains("not-an-agent"), "{refusal}");
    }
}
