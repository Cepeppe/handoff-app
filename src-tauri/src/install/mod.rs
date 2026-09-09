//! Installation adapters (§7.15, INST-01..08, SRV-07, SRV-25).
//!
//! One [`InstallAdapter`] per agent — detect, plan, apply, verify, uninstall — over that
//! agent's own configuration files. The plan is what the consent screen renders, so it has
//! to name the exact file and the exact change; [`apply`] writes a backup, edits the JSON
//! keeping unrelated keys, and re-reads to verify.
//!
//! Claude Code writes three modifications (T-026, Option B): the MCP entry with the fixed
//! launcher path and the per-server `timeout`, and the Stop and SubagentStop hooks.
//! `env.MCP_TOOL_TIMEOUT` is never written and never restored.
//!
//! # Three rules that shape everything below
//!
//! - **Nothing is written that the user has not seen** (INST-01). [`plan`] computes the
//!   value each location will hold and the diff that shows it; [`apply`] writes exactly
//!   that and refuses a file that moved on in between ([`InstallError::Stale`]). There is
//!   no path from a decision to a file that does not go through a [`Modification`].
//! - **Existing configuration is never replaced** (INST-04). The document helpers of
//!   [`json`] preserve key order, unrelated keys and the file's own indentation; the hook
//!   arrays keep every entry that is not ours, in place; `uninstall` removes our entries
//!   and the objects it emptied, and nothing else.
//! - **The plan always lists the three modifications of INST-02**, even where a location is
//!   already correct. A modification whose `before` equals its `after` is a no-op
//!   ([`Modification::is_noop`]): the consent screen still shows three lines, `apply`
//!   touches only the files that really change, and a second `apply` therefore writes
//!   nothing at all.
//!
//! # Where the texts are
//!
//! A [`Modification`] carries a catalogue key and its arguments, never a sentence:
//! `src/locales/{en,it}.json` is the single place a user-visible text is written on both
//! sides (T-028), and the consent screen of T-040 renders the key in the user's language.

pub mod claude_code;
pub mod diff;
pub mod error;
pub mod fixed_path;
pub mod json;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

pub use claude_code::ClaudeCode;
pub use error::{InstallError, Result};

use json::Document;

/// The tool timeout the installer writes, `RAISED_TOOL_TIMEOUT_MS` of §4.1 (INST-03).
///
/// Thirty minutes, as the per-server `timeout` field of the MCP entry and nowhere else
/// (T-026, Option B): the global `MCP_TOOL_TIMEOUT` applies to every MCP server of the
/// agent and is never written, never restored and never removed.
pub const RAISED_TOOL_TIMEOUT_MS: u64 = 1_800_000;

/// The `timeout` written in a hook entry, in seconds (§4.1, SRV-11).
///
/// The hook's own budget is 1 800 ms (`HOOK_TOTAL_BUDGET_MS`); five seconds is the agent's
/// backstop, so that a hook wedged by something outside its own control cannot hold a turn.
pub const HOOK_TIMEOUT_SECONDS: u64 = 5;

/// The environment variable naming the agent to the server (§5.12, DD-09).
pub const ENV_HANDOFF_AGENT: &str = "HANDOFF_AGENT";

/// The environment variable mirroring the per-server timeout (§5.12, §5.6).
pub const ENV_HANDOFF_TOOL_TIMEOUT_MS: &str = "HANDOFF_TOOL_TIMEOUT_MS";

/// The substrings Claude Code strips from the environment of a project-scope server (A-23).
///
/// Any variable an adapter writes is checked against them, because a stripped variable is
/// invisible: the server would fall back to `clientInfo` for its identity and to the table
/// for its timeout, and nothing would say why.
pub const STRIPPED_ENV_SUBSTRINGS: [&str; 5] = ["TOKEN", "SECRET", "PASSWORD", "KEY", "AUTH"];

/// Where a registration is written (INST-06).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Scope {
    /// The user's own configuration, the default of INST-06 and the only scope onboarding
    /// offers.
    User,
    /// One project folder. Offered in settings, and the scope A-23 is about: Claude Code
    /// strips some environment variable names from servers declared here.
    Project {
        /// The project folder holding `.mcp.json` and `.claude/`.
        path: PathBuf,
    },
}

impl Scope {
    /// A project scope over `path`.
    #[must_use]
    pub fn project(path: impl Into<PathBuf>) -> Self {
        Self::Project { path: path.into() }
    }
}

/// What a scan found (INST-05).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Detection {
    /// The agent id shared with the server's capability table (INST-08), e.g. `claude-code`.
    pub agent_id: &'static str,
    /// Whether the agent is on this machine: its executable is on `PATH`, or it has left a
    /// configuration file behind.
    pub found: bool,
    /// The configuration files of the scope that were looked at, existing or not — the
    /// consent screen names them before anything is written.
    pub config_files: Vec<PathBuf>,
    /// The agent version, when it can be had without spawning a process.
    ///
    /// Always `None` for Claude Code: the scan runs at every launch (INST-05) and running
    /// `claude --version` there would spend a process start on a fact nothing needs.
    pub version: Option<String>,
}

/// What [`InstallAdapter::verify`] found (§7.15, FM-23).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Registration {
    /// Everything of ours is there and names the current path.
    Registered,
    /// Some of ours is there and some is not: an interrupted install, or a settings file
    /// the user rewrote. The repair of T-040 applies the missing modifications.
    Partial {
        /// The locations that are missing or wrong, rendered as the plan renders them.
        missing: Vec<String>,
    },
    /// Ours is there and names another path: the bundle was moved (FM-23). The repair offer
    /// rewrites the MCP entry and the hook commands.
    PathMismatch {
        /// The path the configuration names today.
        registered: PathBuf,
        /// The path it should name.
        current: PathBuf,
    },
    /// Nothing of ours is in this scope.
    NotRegistered,
}

/// The sentence the consent screen shows for one modification, as a catalogue key.
///
/// Never a rendered sentence: `src/locales/{en,it}.json` is the single catalogue of the
/// product (T-028) and the consent screen renders the key in the user's language. The
/// arguments are values the catalogue substitutes, and every one of them is a file name, a
/// path or a number — never a spec value, which R-19 keeps out of anything that is stored
/// or displayed outside the overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Description {
    /// The catalogue key, e.g. `install.claudeCode.mcpEntry`.
    pub key: &'static str,
    /// Its `{name}` substitutions.
    pub args: BTreeMap<&'static str, String>,
}

impl Description {
    /// A description with no arguments.
    #[must_use]
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            args: BTreeMap::new(),
        }
    }

    /// The same description with one more argument.
    #[must_use]
    pub fn with(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.args.insert(name, value.into());
        self
    }
}

/// One exact change to one exact place of one file (§7.15, INST-01).
///
/// `before` and `after` are the value *at that place*, rendered as it appears in the file,
/// not the whole document: three modifications over two files is what INST-02 counts, and a
/// user reading a diff of `~/.claude.json` wants our five lines and not their project list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Modification {
    /// The file that changes.
    pub file: PathBuf,
    /// Where inside the document, from the root: `["mcpServers", "handoff"]`.
    ///
    /// Not in the task's field list and needed by two callers: the consent screen says
    /// which key of the file changes, and [`apply`] has to put `after` back exactly where
    /// `before` came from.
    pub path: Vec<&'static str>,
    /// What the change is, as a catalogue key (see [`Description`]).
    pub description: Description,
    /// The value at `path` today, rendered; `None` when it is not there at all.
    pub before: Option<String>,
    /// The value it will hold, rendered.
    pub after: String,
    /// The two of them as a diff, for the **Show** control (INST-01).
    pub diff: String,
}

impl Modification {
    /// Builds the modification that puts `after` at `path` in `file`.
    fn new(
        file: &Path,
        path: Vec<&'static str>,
        description: Description,
        document: &Document,
        after: &Value,
    ) -> Self {
        let indent = "  ";
        let before = document
            .get_at(&path)
            .map(|value| json::render_with_indent(value, indent));
        let after = json::render_with_indent(after, indent);
        let location = render_location(file, &path);
        let diff = diff::render(&location, before.as_deref(), &after);
        Self {
            file: file.to_path_buf(),
            path,
            description,
            before,
            after,
            diff,
        }
    }

    /// Whether the value is already what it should be.
    ///
    /// The plan lists all three modifications of INST-02 whatever the machine looks like,
    /// so this is what tells `apply` there is nothing to write and the repair screen which
    /// lines are already in order.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.before.as_ref() == Some(&self.after)
    }

    /// The place this modification changes, as the consent screen prints it.
    #[must_use]
    pub fn location(&self) -> String {
        render_location(&self.file, &self.path)
    }

    /// The value this modification writes, parsed back.
    fn value(&self) -> Value {
        serde_json::from_str(&self.after).expect("the plan rendered this value itself")
    }
}

/// `<file name> · <a.b.c>`: short enough for a line of the consent screen, exact enough to
/// find by hand.
fn render_location(file: &Path, path: &[&str]) -> String {
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?");
    format!("{name} · {}", path.join("."))
}

/// The per-agent half of installation (§7.15, INST-08).
///
/// Every adapter implements the same six operations over its own agent's files, keyed by
/// the agent id the server's capability table uses. Three of them return a [`Result`] where
/// §7.15 prints a bare value: reading somebody else's configuration can fail, and INST-04
/// makes "carry on anyway" the one answer that is not allowed. `verify` and `uninstall`
/// take a [`Scope`] for the same reason `plan` does — INST-06 makes project scope a real
/// choice, and a `verify` that could only speak about the user scope would report a project
/// installation as absent.
pub trait InstallAdapter {
    /// The agent id shared with the server's capability table (INST-08).
    fn agent_id(&self) -> &'static str;

    /// Whether the agent is on this machine, and which files this scope would touch
    /// (INST-05).
    fn detect(&self, scope: &Scope) -> Detection;

    /// The exact changes this scope needs, all three of them, no-ops included (INST-01).
    ///
    /// # Errors
    ///
    /// When a configuration file exists and cannot be read or is not a JSON object.
    fn plan(&self, scope: &Scope) -> Result<Vec<Modification>>;

    /// Writes a plan: backup, edit, re-read, verify (§7.15). Also creates the channel token
    /// on the first run (INST-07).
    ///
    /// # Errors
    ///
    /// When a file changed since the plan was made, or cannot be read, written or verified.
    fn apply(&self, plan: &[Modification]) -> Result<()>;

    /// What this scope holds today (§7.15, FM-23).
    fn verify(&self, scope: &Scope) -> Registration;

    /// Removes exactly our entries from this scope (INST-04).
    ///
    /// # Errors
    ///
    /// When a configuration file cannot be read, written or verified.
    fn uninstall(&self, scope: &Scope) -> Result<()>;
}

/// Writes a plan, file by file (§7.15).
///
/// Shared by every adapter because none of it is agent-specific: group the modifications by
/// file in the order the plan gives them, skip the files nothing changes, check that each
/// location still holds what the user was shown, back the file up, write, re-read and
/// verify. A failure anywhere stops before the next file, so a plan is applied up to the
/// file that failed and never half-applied inside one.
///
/// # Errors
///
/// [`InstallError::Stale`] when a location no longer holds the plan's `before`,
/// [`InstallError::NotVerified`] when the re-read does not find what was written, and the
/// io and parse failures of [`json::Document`].
pub fn apply(plan: &[Modification]) -> Result<()> {
    for file in files_of(plan) {
        let changes: Vec<&Modification> = plan
            .iter()
            .filter(|modification| modification.file == file && !modification.is_noop())
            .collect();
        if changes.is_empty() {
            continue;
        }

        let mut document = Document::read(&file)?;
        for change in &changes {
            let current = document
                .get_at(&change.path)
                .map(|value| json::render_with_indent(value, "  "));
            if current != change.before {
                return Err(InstallError::Stale {
                    path: file.clone(),
                    location: change.location(),
                });
            }
        }

        back_up(&file)?;
        for change in &changes {
            document.set_at(&change.path, change.value());
        }
        document.write(&file)?;

        // §7.15: re-read to verify. The file is Claude Code's own and it may be running.
        let written = Document::read(&file)?;
        for change in &changes {
            if written.get_at(&change.path) != Some(&change.value()) {
                return Err(InstallError::NotVerified {
                    path: file.clone(),
                    location: change.location(),
                });
            }
        }
    }
    Ok(())
}

/// The files a plan touches, in the order they first appear in it.
fn files_of(plan: &[Modification]) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    for modification in plan {
        if !files.contains(&modification.file) {
            files.push(modification.file.clone());
        }
    }
    files
}

/// Copies `path` to `<path>.handoff-backup-<timestamp>` before it is edited (§7.15).
///
/// A file that is not there yet has nothing to back up: the first install of a machine
/// creates `settings.json`, and an empty backup would only be noise in the user's folder.
///
/// The timestamp is the canonical instant of the log with its separators removed
/// (`20260909T051500789Z`): sortable, unambiguous, and a legal file name on Windows, where
/// `:` is not.
fn back_up(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let backup = path.with_file_name(format!("{name}.handoff-backup-{}", backup_stamp()));
    std::fs::copy(path, &backup).map_err(|error| InstallError::unwritable(&backup, error))?;
    Ok(())
}

/// The instant a backup name carries.
fn backup_stamp() -> String {
    crate::log::time::Timestamp::now()
        .as_str()
        .replace(['-', ':', '.'], "")
}

/// Whether an environment variable name survives project scope (A-23).
///
/// A name carrying one of [`STRIPPED_ENV_SUBSTRINGS`] is removed by Claude Code from the
/// environment of a server declared in a project file, silently. Every adapter checks the
/// names it writes; the check costs nothing and the failure it prevents is invisible.
#[must_use]
pub fn survives_project_scope(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    !STRIPPED_ENV_SUBSTRINGS
        .iter()
        .any(|stripped| upper.contains(stripped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_we_write_survive_project_scope() {
        // A-23, and the reason the two variables are called what they are called.
        assert!(survives_project_scope(ENV_HANDOFF_AGENT));
        assert!(survives_project_scope(ENV_HANDOFF_TOOL_TIMEOUT_MS));
    }

    #[test]
    fn the_stripped_substrings_are_recognised_wherever_they_appear() {
        for name in [
            "HANDOFF_TOKEN",
            "MY_SECRET_THING",
            "PASSWORD",
            "API_KEY",
            "AUTH_HEADER",
            "handoff_token",
        ] {
            assert!(!survives_project_scope(name), "{name} was allowed");
        }
    }

    #[test]
    fn a_backup_name_is_a_legal_windows_file_name() {
        let stamp = backup_stamp();
        assert!(
            !stamp.contains([':', '-', '.', '/', '\\']),
            "the stamp {stamp} cannot be a file name on Windows"
        );
        assert!(stamp.ends_with('Z'), "the stamp {stamp} is not UTC");
    }

    #[test]
    fn a_location_names_the_file_and_the_key() {
        assert_eq!(
            render_location(
                Path::new("/home/x/.claude.json"),
                &["mcpServers", "handoff"]
            ),
            ".claude.json · mcpServers.handoff"
        );
    }
}
