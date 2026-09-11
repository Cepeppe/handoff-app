//! Installation adapters (§7.15, INST-01..08, SRV-07, SRV-25).
//!
//! One [`InstallAdapter`] per agent — detect, plan, apply, verify, uninstall — over that
//! agent's own configuration files. The plan is what the consent screen renders, so it has
//! to name the exact file and the exact change; [`apply`] writes a backup, edits the file
//! keeping everything unrelated, and re-reads to verify.
//!
//! Claude Code writes three modifications (T-026, Option B): the MCP entry with the fixed
//! launcher path and the per-server `timeout`, and the Stop and SubagentStop hooks.
//! `env.MCP_TOOL_TIMEOUT` is never written and never restored. Codex writes one (T-067): its
//! `[mcp_servers.handoff]` section in `config.toml`, with the approval mode and the timeout in
//! seconds, and no hook, because `codex exec` runs none. OpenCode writes one too (T-074): its
//! `mcp.handoff` entry in `opencode.json`, with the timeout in milliseconds, and no hook,
//! because OpenCode has none to register.
//!
//! # Three rules that shape everything below
//!
//! - **Nothing is written that the user has not seen** (INST-01). [`plan`] computes the
//!   value each location will hold and the diff that shows it; [`apply`] writes exactly
//!   that and refuses a file that moved on in between ([`InstallError::Stale`]). There is
//!   no path from a decision to a file that does not go through a [`Modification`].
//! - **Existing configuration is never replaced** (INST-04). The document helpers of
//!   [`json`] preserve key order, unrelated keys and the file's own indentation, and those of
//!   [`toml`] every comment and spelling besides; the hook arrays keep every entry that is not
//!   ours, in place; `uninstall` removes our entries and the parents it emptied, and nothing
//!   else.
//! - **The plan always lists every modification of the agent**, even where a location is
//!   already correct — the three of INST-02 for Claude Code. A modification whose `before`
//!   equals its `after` is a no-op ([`Modification::is_noop`]): the consent screen still shows
//!   every line, `apply` touches only the files that really change, and a second `apply`
//!   therefore writes nothing at all.
//!
//! # Where the texts are
//!
//! A [`Modification`] carries a catalogue key and its arguments, never a sentence:
//! `src/locales/{en,it}.json` is the single place a user-visible text is written on both
//! sides (T-028), and the consent screen of T-040 renders the key in the user's language.

pub mod claude_code;
pub mod cleanup;
pub mod codex;
pub mod diff;
pub mod error;
pub mod fixed_path;
pub mod json;
pub mod opencode;
pub mod scan;
pub mod toml;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

pub use claude_code::ClaudeCode;
pub use codex::Codex;
pub use error::{InstallError, Result};
pub use opencode::OpenCode;
pub use scan::{scan, AgentStatus, MovedRegistration};

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
///
/// Deserialized as well as serialized because the settings page sends it back: the scope
/// selector of T-040 is `{ "kind": "user" }` or `{ "kind": "project", "path": … }`, and a
/// second type mirroring this one on the way in would be a second place for the two spellings
/// to drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// The syntax of the file a modification is written into.
///
/// Claude Code's configuration is JSON, Codex's TOML and OpenCode's JSON again (§7.15). A plan
/// never mixes the two
/// inside one file, and [`apply`] reads, edits and verifies each file in its own syntax, so the
/// rules around the edit — the stale check, the backup, the re-read — stay one piece of code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// `serde_json` with `preserve_order`, through [`json::Document`].
    Json,
    /// `toml_edit`, through [`toml::Document`].
    Toml,
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
    /// The syntax `file` is written in, which is how [`apply`] reads it back.
    pub format: Format,
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
    /// Builds the modification that puts the JSON value `after` at `path` in `file`.
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
            format: Format::Json,
            path,
            description,
            before,
            after,
            diff,
        }
    }

    /// Builds the modification that puts the TOML section `after` at `path` in `file`.
    ///
    /// `before` and `after` are both in the canonical rendering of [`toml::render_at`], so a
    /// section that already holds our values is a no-op however the user spelled it, and the
    /// diff behind **Show** is about values rather than about quotes and comments.
    fn toml(
        file: &Path,
        path: Vec<&'static str>,
        description: Description,
        document: &toml::Document,
        after: &toml_edit::Table,
    ) -> Self {
        let before = document.rendered_at(&path);
        let after = toml::render_at(&path, &toml_edit::Item::Table(after.clone()));
        let location = render_location(file, &path);
        let diff = diff::render(&location, before.as_deref(), &after);
        Self {
            file: file.to_path_buf(),
            format: Format::Toml,
            path,
            description,
            before,
            after,
            diff,
        }
    }

    /// Whether the value is already what it should be.
    ///
    /// The plan lists every modification of the agent whatever the machine looks like, so
    /// this is what tells `apply` there is nothing to write and the repair screen which
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

    /// The JSON value this modification writes, parsed back.
    fn value(&self) -> Value {
        serde_json::from_str(&self.after).expect("the plan rendered this value itself")
    }
}

/// One line of the consent screen (INST-01, INST-02).
///
/// A line and a [`Modification`] are **not** the same thing, and INST-02 is where the two
/// part company: Claude Code makes three modifications and the screen lists the two hooks on
/// one line, "two hooks (Stop and SubagentStop), same command". So the count the user is told
/// is the number of modifications and the number of rows they read is the number of lines,
/// and both are right.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentLine {
    /// What the line says, as a catalogue key (see [`Description`]).
    pub description: Description,
    /// The places it covers, as [`Modification::location`] renders them.
    pub locations: Vec<String>,
    /// Everything the **Show** control reveals: the diffs of those places, in order.
    pub diff: String,
    /// Whether every place it covers is already what it should be.
    pub is_noop: bool,
}

impl ConsentLine {
    /// The line of one modification.
    #[must_use]
    pub fn of(modification: &Modification) -> Self {
        Self {
            description: modification.description.clone(),
            locations: vec![modification.location()],
            diff: modification.diff.clone(),
            is_noop: modification.is_noop(),
        }
    }

    /// One line covering several modifications, under a description of its own.
    ///
    /// The diffs are joined with a blank line between them: **Show** reveals the whole of
    /// what the line stands for, and the user reading it has to be able to tell the two
    /// hooks apart. It is a no-op only when every place it covers is already in order — a
    /// line that says "already in order" while half of it is missing would be a lie the
    /// repair flow of the settings page reads.
    #[must_use]
    pub fn of_many(description: Description, modifications: &[&Modification]) -> Self {
        Self {
            description,
            locations: modifications
                .iter()
                .map(|modification| modification.location())
                .collect(),
            diff: modifications
                .iter()
                .map(|modification| modification.diff.as_str())
                .collect::<Vec<&str>>()
                .join("\n"),
            is_noop: modifications
                .iter()
                .all(|modification| modification.is_noop()),
        }
    }
}

/// The fingerprint of a plan (INST-01).
///
/// The consent screen shows a plan and the user accepts *that* plan; by the time they press
/// the button the file may have moved on, and [`apply`] refuses it then
/// ([`InstallError::Stale`]). But a screen that re-plans before applying — which it must,
/// because a `Modification` cannot be trusted to come back from a webview unchanged — would
/// always apply a plan nobody had seen. So the screen carries this digest of what it showed
/// and hands it back: a re-plan that hashes differently is the same refusal, arrived at from
/// the other side.
///
/// Over the location, the `before` and the `after` of every modification, in the plan's own
/// order, with a length prefix on each part so that no two plans can be spelled into one.
#[must_use]
pub fn digest(plan: &[Modification]) -> String {
    let mut hasher = Sha256::new();
    for modification in plan {
        for part in [
            modification.location(),
            modification.before.clone().unwrap_or_default(),
            modification.after.clone(),
        ] {
            hasher.update(part.len().to_le_bytes());
            hasher.update(part.as_bytes());
        }
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
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

    /// The exact changes this scope needs, all of them, no-ops included (INST-01).
    ///
    /// # Errors
    ///
    /// When a configuration file exists and cannot be read, or is not a configuration the
    /// adapter can edit.
    fn plan(&self, scope: &Scope) -> Result<Vec<Modification>>;

    /// The plan as the consent screen lists it (INST-01, INST-02).
    ///
    /// One line per modification, which is the honest default for an adapter that has
    /// nothing to group. Claude Code overrides it, because INST-02 asks for its two hooks on
    /// one line; the count of *modifications* is unchanged by any of this.
    fn consent_lines(&self, plan: &[Modification]) -> Vec<ConsentLine> {
        plan.iter().map(ConsentLine::of).collect()
    }

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
/// file that failed and never half-applied inside one. Each file is read and written in the
/// syntax its modifications name ([`Format`]).
///
/// # Errors
///
/// [`InstallError::Stale`] when a location no longer holds the plan's `before`,
/// [`InstallError::NotVerified`] when the re-read does not find what was written, and the
/// io and parse failures of [`json::Document`] and [`toml::Document`].
pub fn apply(plan: &[Modification]) -> Result<()> {
    for file in files_of(plan) {
        let changes: Vec<&Modification> = plan
            .iter()
            .filter(|modification| modification.file == file && !modification.is_noop())
            .collect();
        let Some(first) = changes.first() else {
            continue;
        };

        // One file is one syntax: the plan that made its first change made them all.
        let format = first.format;
        let mut document = Config::read(format, &file)?;
        for change in &changes {
            if document.rendered_at(&change.path) != change.before {
                return Err(InstallError::Stale {
                    path: file.clone(),
                    location: change.location(),
                });
            }
        }

        back_up(&file)?;
        for change in &changes {
            document.put(change);
        }
        document.write(&file)?;

        // §7.15: re-read to verify. The file is the agent's own and the agent may be running.
        let written = Config::read(format, &file)?;
        for change in &changes {
            if !written.holds(change) {
                return Err(InstallError::NotVerified {
                    path: file.clone(),
                    location: change.location(),
                });
            }
        }
    }
    Ok(())
}

/// One configuration file as [`apply`] edits it, in the syntax its agent reads.
enum Config {
    /// Claude Code's `~/.claude.json`, `settings.json`, a project's `.mcp.json`; OpenCode's
    /// `opencode.json`.
    Json(Document),
    /// Codex's `config.toml`.
    Toml(toml::Document),
}

impl Config {
    /// The file at `path`, read in `format`.
    fn read(format: Format, path: &Path) -> Result<Self> {
        Ok(match format {
            Format::Json => Self::Json(Document::read(path)?),
            Format::Toml => Self::Toml(toml::Document::read(path)?),
        })
    }

    /// The value at `path`, rendered the way a plan renders its `before`.
    fn rendered_at(&self, path: &[&str]) -> Option<String> {
        match self {
            Self::Json(document) => document
                .get_at(path)
                .map(|value| json::render_with_indent(value, "  ")),
            Self::Toml(document) => document.rendered_at(path),
        }
    }

    /// Puts a modification's `after` where its `before` was.
    fn put(&mut self, change: &Modification) {
        match self {
            Self::Json(document) => document.set_at(&change.path, change.value()),
            Self::Toml(document) => {
                document.set_at(&change.path, toml::table_of(&change.after, &change.path));
            }
        }
    }

    /// Whether the place a modification changes now holds what it wrote.
    fn holds(&self, change: &Modification) -> bool {
        match self {
            Self::Json(document) => document.get_at(&change.path) == Some(&change.value()),
            Self::Toml(document) => {
                document.rendered_at(&change.path).as_deref() == Some(change.after.as_str())
            }
        }
    }

    /// Writes the file back.
    fn write(&self, path: &Path) -> Result<()> {
        match self {
            Self::Json(document) => document.write(path),
            Self::Toml(document) => document.write(path),
        }
    }
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

    /// One modification, spelled out, for the digest cases below.
    fn modification(before: Option<&str>, after: &str) -> Modification {
        Modification {
            file: PathBuf::from("/home/x/.claude.json"),
            format: Format::Json,
            path: vec!["mcpServers", "handoff"],
            description: Description::new("install.claudeCode.mcpEntry"),
            before: before.map(str::to_owned),
            after: after.to_owned(),
            diff: String::new(),
        }
    }

    #[test]
    fn a_plan_hashes_to_itself_and_to_nothing_else() {
        let plan = vec![modification(None, "{}")];
        assert_eq!(digest(&plan), digest(&plan.clone()));
        // The value that will be written is what the user consented to.
        assert_ne!(digest(&plan), digest(&[modification(None, "{ }")]));
        // And so is the value that is there now: the same target reached from a different
        // starting point is a different diff on the screen.
        assert_ne!(digest(&plan), digest(&[modification(Some("null"), "{}")]));
    }

    #[test]
    fn no_two_plans_can_be_spelled_into_one_digest() {
        // Without the length prefix, ("ab", "c") and ("a", "bc") would hash alike, and a
        // file could move a character across a boundary without changing the fingerprint.
        assert_ne!(
            digest(&[modification(Some("ab"), "c")]),
            digest(&[modification(Some("a"), "bc")])
        );
    }

    #[test]
    fn an_empty_plan_still_has_a_digest() {
        // A machine already in order plans three no-ops, never nothing; but the function is
        // total, and a caller comparing digests must not have to special-case a length.
        assert_eq!(digest(&[]).len(), 64);
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
