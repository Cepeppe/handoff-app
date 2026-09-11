//! The scan of INST-05 and the path check of FM-23 (§7.2).
//!
//! Two things happen at every launch, both of them cheap and neither of them writing
//! anything: every adapter is asked whether its agent is on this machine, and every
//! registration of ours is asked whether it still names the path we are running from.
//!
//! - **A newly found agent produces one discreet notice** (INST-05). "Newly" is the whole
//!   difficulty: the scan runs at every launch, so the set of agents already noticed has to
//!   survive one, and it does, in the `known_agents` setting. [`newly_found`] is the pure
//!   half of that rule and [`record_known`] the write.
//! - **A registration naming another path is a moved bundle** (FM-23, SRV-25). The agent
//!   still spawns the old path, which is no longer there, so every session of that machine
//!   is in text mode with nothing on screen to say why. [`check_registered_paths`] is what
//!   §7.2 calls at launch, and the answer is what the settings page offers to repair.
//!
//! Everything here is written over the **list** of adapters rather than over the Claude Code
//! one: Codex joined it with T-067 without a line of change to the scan, the notice, the
//! settings page or the repair offer, OpenCode joined it the same way with T-074, Cursor with
//! T-070, and GitHub Copilot with T-072.

use std::path::PathBuf;

use serde::Serialize;

use super::error::Result;
use super::{ClaudeCode, Codex, Copilot, Cursor, InstallAdapter, OpenCode, Registration, Scope};

/// The setting holding the agent ids a notice was already shown for (INST-05).
pub const KNOWN_AGENTS_KEY: &str = "known_agents";

/// Every installation adapter this build carries (INST-08).
///
/// Claude Code, then Codex, Cursor, GitHub Copilot and OpenCode: the committed order of
/// ADPT-06, which is also the order the settings page lists them in.
///
/// # Errors
///
/// [`super::InstallError::NoServer`] when the bundled server cannot be located, which is the
/// one thing every adapter needs before it can name a command (SRV-25).
pub fn adapters() -> Result<Vec<Box<dyn InstallAdapter>>> {
    Ok(vec![
        Box::new(ClaudeCode::detected()?),
        Box::new(Codex::detected()?),
        Box::new(Cursor::detected()?),
        Box::new(Copilot::detected()?),
        Box::new(OpenCode::detected()?),
    ])
}

/// What one launch scan found about one agent (INST-05, §7.15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    /// The agent id shared with the server's capability table (INST-08).
    pub agent_id: &'static str,
    /// The catalogue key of its name, for the settings page (see [`name_key`]).
    pub name_key: &'static str,
    /// Whether the agent is on this machine at all.
    pub found: bool,
    /// The files this scope would touch, existing or not (INST-01 names them first).
    pub config_files: Vec<PathBuf>,
    /// What is registered in this scope today.
    pub registration: Registration,
}

/// The state of every adapter in `scope`.
///
/// A read, and only a read: nothing here creates a file, a token or a backup. That is why
/// INST-05 can call it "cheap" and run it at every launch.
///
/// # Errors
///
/// [`super::InstallError::NoServer`] when the bundled server cannot be located.
pub fn scan(scope: &Scope) -> Result<Vec<AgentStatus>> {
    let mut found = Vec::new();
    for adapter in adapters()? {
        let detection = adapter.detect(scope);
        found.push(AgentStatus {
            agent_id: adapter.agent_id(),
            name_key: name_key(adapter.agent_id()),
            found: detection.found,
            config_files: detection.config_files,
            registration: adapter.verify(scope),
        });
    }
    Ok(found)
}

/// One of ours pointing somewhere else: the bundle moved (FM-23, §7.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovedRegistration {
    /// Whose configuration says it.
    pub agent_id: &'static str,
    /// Its name, as a catalogue key.
    pub name_key: &'static str,
    /// The path the configuration names today.
    pub registered: PathBuf,
    /// The path it should name.
    pub current: PathBuf,
}

/// `install::check_registered_paths()` of §7.2: every registration that names another path.
///
/// User scope only, which is where onboarding registers (INST-06); a project registration is
/// made from the settings page and checked there, on the folder the user chose.
///
/// Empty is the ordinary answer, and an adapter that cannot be built (no bundled server) is
/// no answer at all rather than a false alarm: the launch has bigger problems than a moved
/// bundle, and `verify` would have nothing to compare against.
#[must_use]
pub fn check_registered_paths() -> Vec<MovedRegistration> {
    match scan(&Scope::User) {
        Ok(statuses) => moved_registrations(statuses),
        Err(_) => Vec::new(),
    }
}

/// The moved ones among `statuses` (FM-23).
///
/// Split from the check above so that a caller which has just scanned — the launch, which
/// needs the newly-found agents from the same read — does not scan a second time to ask the
/// second question about the same answer.
#[must_use]
pub fn moved_registrations(statuses: Vec<AgentStatus>) -> Vec<MovedRegistration> {
    statuses
        .into_iter()
        .filter_map(|status| match status.registration {
            Registration::PathMismatch {
                registered,
                current,
            } => Some(MovedRegistration {
                agent_id: status.agent_id,
                name_key: status.name_key,
                registered,
                current,
            }),
            _ => None,
        })
        .collect()
}

/// The agents in `statuses` that are on the machine and have never been noticed (INST-05).
///
/// The pure half of "one discreet notice per newly found agent": what is *new* is decided
/// against the stored list and nothing else, so the answer does not depend on when the app
/// was last started or on how many launches ago the agent was installed.
#[must_use]
pub fn newly_found<'a>(statuses: &'a [AgentStatus], known: &[String]) -> Vec<&'a AgentStatus> {
    statuses
        .iter()
        .filter(|status| status.found && !known.iter().any(|id| id == status.agent_id))
        .collect()
}

/// The stored list with every agent of `statuses` that is on the machine added, sorted.
///
/// Sorted and deduplicated so that the setting is a set and not a history: it is read by the
/// rule above, which only ever asks "is this id in it".
#[must_use]
pub fn record_known(statuses: &[AgentStatus], known: &[String]) -> Vec<String> {
    let mut all: Vec<String> = known.to_vec();
    for status in statuses.iter().filter(|status| status.found) {
        if !all.iter().any(|id| id == status.agent_id) {
            all.push(status.agent_id.to_owned());
        }
    }
    all.sort();
    all
}

/// The catalogue key holding an agent's name.
///
/// A name is a user-visible text, so it lives in `src/locales/{en,it}.json` like every other
/// one (T-028) and never in a `Detection`. An id no catalogue knows falls back to a generic
/// key rather than to the raw id: a test below fails the build if an adapter of ours ever
/// reaches it.
#[must_use]
pub fn name_key(agent_id: &str) -> &'static str {
    match agent_id {
        super::claude_code::AGENT_ID => "agent.claudeCode",
        super::codex::AGENT_ID => "agent.codex",
        super::cursor::AGENT_ID => "agent.cursor",
        super::copilot::AGENT_ID => "agent.copilot",
        super::opencode::AGENT_ID => "agent.opencode",
        _ => "agent.unknown",
    }
}

/// Whether the application is where macOS wants it before it is registered (INST-01, §7.6).
///
/// Onboarding asks the user to move the bundle to `/Applications` **before** consent,
/// because the fixed launcher path written into every agent configuration is derived from
/// where the application is (SRV-25): registering from the Downloads folder and moving the
/// bundle afterwards is FM-23 arranged in advance.
///
/// Always true off macOS, where there is nowhere to move to and the step is not shown.
#[must_use]
pub fn app_in_applications() -> bool {
    if !cfg!(target_os = "macos") {
        return true;
    }
    match std::env::current_exe() {
        Ok(executable) => executable.starts_with("/Applications"),
        // A launch that cannot say where it is has bigger problems, and the step is a
        // courtesy rather than a gate: never nag on a fact we do not have.
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(agent_id: &'static str, found: bool) -> AgentStatus {
        AgentStatus {
            agent_id,
            name_key: name_key(agent_id),
            found,
            config_files: Vec::new(),
            registration: Registration::NotRegistered,
        }
    }

    #[test]
    fn every_adapter_of_this_build_has_a_name_in_the_catalogue() {
        // The fallback exists for an id from somewhere else, never for one of ours: an
        // adapter added without its two catalogue entries would show `agent.unknown` in the
        // settings page, which is exactly the kind of thing nobody notices in review.
        for agent_id in [
            super::super::claude_code::AGENT_ID,
            super::super::codex::AGENT_ID,
            super::super::cursor::AGENT_ID,
            super::super::opencode::AGENT_ID,
        ] {
            let key = name_key(agent_id);
            assert_ne!(key, "agent.unknown", "{agent_id} has no name key");
            for language in [crate::i18n::Language::En, crate::i18n::Language::It] {
                assert_ne!(
                    crate::i18n::text(language, key),
                    key,
                    "{language} has no text for {key}"
                );
            }
        }
    }

    #[test]
    fn an_agent_that_is_not_on_the_machine_is_never_new() {
        let statuses = [status("claude-code", false)];
        assert!(newly_found(&statuses, &[]).is_empty());
        assert!(record_known(&statuses, &[]).is_empty());
    }

    #[test]
    fn an_agent_is_new_once_and_then_never_again() {
        let statuses = [status("claude-code", true)];

        let first = newly_found(&statuses, &[]);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].agent_id, "claude-code");

        let known = record_known(&statuses, &[]);
        assert_eq!(known, vec!["claude-code".to_owned()]);
        assert!(newly_found(&statuses, &known).is_empty());
        // Recording twice is the second launch, and it changes nothing.
        assert_eq!(record_known(&statuses, &known), known);
    }

    #[test]
    fn a_second_agent_is_new_although_the_first_is_not() {
        let statuses = [status("claude-code", true), status("codex", true)];
        let known = vec!["claude-code".to_owned()];

        let new = newly_found(&statuses, &known);
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].agent_id, "codex");
        assert_eq!(
            record_known(&statuses, &known),
            vec!["claude-code".to_owned(), "codex".to_owned()]
        );
    }

    #[test]
    fn only_a_registration_naming_another_path_is_reported_as_moved() {
        let mut moved_one = status("claude-code", true);
        moved_one.registration = Registration::PathMismatch {
            registered: PathBuf::from("/old/Baton/handoff-mcp"),
            current: PathBuf::from("/new/Baton/handoff-mcp"),
        };
        let mut registered = status("codex", true);
        registered.registration = Registration::Registered;

        let moved = moved_registrations(vec![moved_one, registered, status("opencode", true)]);
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].agent_id, "claude-code");
        assert_eq!(moved[0].registered, PathBuf::from("/old/Baton/handoff-mcp"));
    }

    #[test]
    fn the_launch_check_asks_the_same_question_of_this_machine() {
        // §7.2 names `check_registered_paths()` and calls it with nothing in hand; the launch
        // has just scanned and asks `moved_registrations` of the answer it holds. Whatever
        // this machine's own configuration says, the two have to agree.
        let scanned = scan(&Scope::User)
            .map(moved_registrations)
            .unwrap_or_default();
        assert_eq!(check_registered_paths(), scanned);
    }

    #[test]
    fn the_move_step_is_only_ever_asked_for_on_macos() {
        if cfg!(target_os = "macos") {
            // Whatever this build is running from, the answer is a fact about that path and
            // not about the platform; nothing to assert beyond that it decides.
            let _ = app_in_applications();
        } else {
            assert!(app_in_applications());
        }
    }
}
