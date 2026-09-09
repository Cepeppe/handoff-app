//! The Claude Code installation adapter (§7.15, INST-01..08, ADPT-08, A-21, A-23).
//!
//! Three modifications, two files, and one rule underneath all of them: whatever is already
//! in those files is still there afterwards, in the same order, spelled the same way
//! (INST-04).
//!
//! | # | File | Location | What |
//! |---|---|---|---|
//! | 1 | `~/.claude.json` (`.mcp.json` in a project) | `mcpServers.handoff` | the stdio entry with the fixed launcher path, our two environment variables and the per-server `timeout` |
//! | 2 | `~/.claude/settings.json` | `hooks.Stop` | our entry appended |
//! | 3 | `~/.claude/settings.json` | `hooks.SubagentStop` | the same command (ADPT-08) |
//!
//! **`env.MCP_TOOL_TIMEOUT` is not one of them** (T-026, Option B, OI-02 closed on
//! 2026-09-08). The per-server `timeout` field bounds our server alone; the global variable
//! bounds every MCP server the user has, and the measured default is already longer than
//! thirty minutes (A-03, A-04). Nothing here writes it, restores it or removes it, and a
//! value the user set by hand is left byte-identical — which the golden files pin, because
//! "we never touch it" is only a promise until a test fails when it is broken.
//!
//! # Why the hooks are edited rather than replaced
//!
//! A-21: user and project hooks are merged by Claude Code, so adding ours does not replace
//! the user's — but only if we add rather than assign. [`hook_entries`] keeps every group
//! that holds no hook of ours exactly as it found it, keeps the foreign half of a group
//! that holds both, and puts our own group in the place ours already had. The result is
//! idempotent: planning twice over the same file produces the same array, so the second
//! `apply` writes nothing.
//!
//! # Recognising ours
//!
//! By the fixed path in `command` (§7.15) — `mcpServers.handoff` by its key as well, the
//! hooks by their command alone, since a hook entry has no name. The test is
//! [`super::fixed_path::is_ours`], which asks whether the command names a file called
//! `handoff-mcp` and not whether it names *today's* path: after the bundle moves (FM-23)
//! the registered path is the old one, and an uninstall that did not recognise it would
//! leave a dead hook behind for ever.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::channel::token;
use crate::paths;

use super::error::{InstallError, Result};
use super::fixed_path;
use super::json::Document;
use super::{
    Description, Detection, InstallAdapter, Modification, Registration, Scope, ENV_HANDOFF_AGENT,
    ENV_HANDOFF_TOOL_TIMEOUT_MS, HOOK_TIMEOUT_SECONDS, RAISED_TOOL_TIMEOUT_MS,
};

/// The agent id, shared with the server's capability table (INST-08, §5.6).
pub const AGENT_ID: &str = "claude-code";

/// The key our MCP entry has in `mcpServers` (§7.15).
pub const SERVER_KEY: &str = "handoff";

/// The two hook events we register, with the same command (ADPT-08).
pub const HOOK_EVENTS: [&str; 2] = ["Stop", "SubagentStop"];

/// The catalogue key of the MCP-entry line of the consent screen.
pub const KEY_MCP_ENTRY: &str = "install.claudeCode.mcpEntry";

/// The catalogue key of the Stop-hook line.
pub const KEY_STOP_HOOK: &str = "install.claudeCode.stopHook";

/// The catalogue key of the SubagentStop-hook line.
pub const KEY_SUBAGENT_STOP_HOOK: &str = "install.claudeCode.subagentStopHook";

/// The Claude Code adapter.
///
/// Its three paths are fields rather than calls so that a test can point the whole adapter
/// at a temporary home: what it writes is then exactly what it writes on a real machine,
/// which is the only way a golden file proves anything.
#[derive(Debug, Clone)]
pub struct ClaudeCode {
    /// The user's home folder: `~/.claude.json` and `~/.claude/settings.json` hang off it.
    home: PathBuf,
    /// The fixed launcher path written into the configuration (SRV-25).
    server: PathBuf,
    /// `~/.handoff/channel.token`, created by the first `apply` (INST-07, SRV-07).
    token_path: PathBuf,
}

impl ClaudeCode {
    /// The adapter for this machine and this build.
    ///
    /// # Errors
    ///
    /// [`InstallError::NoServer`] when the bundled server cannot be located.
    pub fn detected() -> Result<Self> {
        Ok(Self {
            home: paths::home_dir(),
            server: fixed_path::server_path()?,
            token_path: paths::token_path(),
        })
    }

    /// An adapter over explicit paths, for the golden files.
    #[must_use]
    pub fn with(
        home: impl Into<PathBuf>,
        server: impl Into<PathBuf>,
        token: impl Into<PathBuf>,
    ) -> Self {
        Self {
            home: home.into(),
            server: server.into(),
            token_path: token.into(),
        }
    }

    /// The path this adapter registers.
    #[must_use]
    pub fn server(&self) -> &Path {
        &self.server
    }

    /// The file holding `mcpServers`: `~/.claude.json`, or the project's `.mcp.json`.
    #[must_use]
    pub fn mcp_file(&self, scope: &Scope) -> PathBuf {
        match scope {
            Scope::User => self.home.join(".claude.json"),
            Scope::Project { path } => path.join(".mcp.json"),
        }
    }

    /// The file holding `hooks`: `~/.claude/settings.json`, or the project's.
    #[must_use]
    pub fn settings_file(&self, scope: &Scope) -> PathBuf {
        let root = match scope {
            Scope::User => self.home.clone(),
            Scope::Project { path } => path.clone(),
        };
        root.join(".claude").join("settings.json")
    }

    /// The MCP entry of §7.15, exactly.
    ///
    /// The key order is the design's, and it is preserved on the way to disk
    /// (`serde_json` with `preserve_order`), so the file reads the way the consent screen
    /// showed it.
    #[must_use]
    pub fn mcp_entry(&self) -> Value {
        json!({
            "type": "stdio",
            "command": self.server.display().to_string(),
            "args": [],
            "env": {
                ENV_HANDOFF_AGENT: AGENT_ID,
                ENV_HANDOFF_TOOL_TIMEOUT_MS: RAISED_TOOL_TIMEOUT_MS.to_string(),
            },
            "timeout": RAISED_TOOL_TIMEOUT_MS,
        })
    }

    /// Our hook group: one matcher matching everything, one command hook (§7.15, SRV-11).
    #[must_use]
    pub fn hook_group(&self) -> Value {
        json!({
            "matcher": "",
            "hooks": [ {
                "type": "command",
                "command": fixed_path::hook_command(&self.server),
                "timeout": HOOK_TIMEOUT_SECONDS,
            } ],
        })
    }

    /// The `hooks.<event>` array after our entry is put in it, keeping the user's.
    #[must_use]
    fn hook_entries(&self, current: Option<&Value>) -> Value {
        let existing = current
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out: Vec<Value> = Vec::with_capacity(existing.len() + 1);
        let mut placed = false;

        for group in existing {
            match split_group(&group) {
                // Nothing of ours: kept exactly as it was found (INST-04, A-21).
                GroupHalves::TheirsOnly => out.push(group),
                // Entirely ours: our canonical group takes its place, so a moved bundle is
                // repaired where it already was rather than moved to the end. A second copy
                // of ours — which only a hand edit can produce — is dropped.
                GroupHalves::OursOnly => {
                    if !placed {
                        out.push(self.hook_group());
                        placed = true;
                    }
                }
                // A group somebody merged by hand: their hooks stay in it, ours leaves and
                // comes back as our own group below.
                GroupHalves::Both { theirs } => out.push(with_hooks(&group, theirs)),
            }
        }
        if !placed {
            out.push(self.hook_group());
        }
        Value::Array(out)
    }

    /// The `hooks.<event>` array with every hook of ours taken out (INST-04).
    #[must_use]
    fn hook_entries_without_ours(current: Option<&Value>) -> Vec<Value> {
        let existing = current
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        existing
            .into_iter()
            .filter_map(|group| match split_group(&group) {
                GroupHalves::TheirsOnly => Some(group),
                GroupHalves::OursOnly => None,
                GroupHalves::Both { theirs } => Some(with_hooks(&group, theirs)),
            })
            .collect()
    }

    /// The server path each of our three locations names today, for the FM-23 check.
    fn registered_paths(&self, scope: &Scope) -> Vec<PathBuf> {
        let mut found = Vec::new();

        if let Ok(document) = Document::read(&self.mcp_file(scope)) {
            if let Some(command) = document
                .get_at(&["mcpServers", SERVER_KEY])
                .and_then(|entry| entry.get("command"))
                .and_then(Value::as_str)
            {
                let path = Path::new(command);
                if fixed_path::is_ours(path) {
                    found.push(path.to_path_buf());
                }
            }
        }

        if let Ok(document) = Document::read(&self.settings_file(scope)) {
            for event in HOOK_EVENTS {
                let Some(groups) = document.get_at(&["hooks", event]).and_then(Value::as_array)
                else {
                    continue;
                };
                for group in groups {
                    found.extend(ours_in_group(group));
                }
            }
        }
        found
    }
}

/// What a hook group holds, from our point of view.
enum GroupHalves {
    /// Nothing of ours.
    TheirsOnly,
    /// Only ours.
    OursOnly,
    /// Both, with the user's half kept.
    Both { theirs: Vec<Value> },
}

/// Splits a hook group into the user's hooks and ours.
fn split_group(group: &Value) -> GroupHalves {
    let Some(hooks) = group.get("hooks").and_then(Value::as_array) else {
        // No `hooks` array at all: not a shape we produce, so it is the user's.
        return GroupHalves::TheirsOnly;
    };
    let (ours, theirs): (Vec<&Value>, Vec<&Value>) = hooks.iter().partition(|hook| is_ours(hook));
    if ours.is_empty() {
        GroupHalves::TheirsOnly
    } else if theirs.is_empty() {
        GroupHalves::OursOnly
    } else {
        GroupHalves::Both {
            theirs: theirs.into_iter().cloned().collect(),
        }
    }
}

/// `group` with its `hooks` replaced, every other key kept where it was.
fn with_hooks(group: &Value, hooks: Vec<Value>) -> Value {
    let mut object = group.as_object().cloned().unwrap_or_else(Map::new);
    object.insert("hooks".to_owned(), Value::Array(hooks));
    Value::Object(object)
}

/// Whether one hook object is ours.
fn is_ours(hook: &Value) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .and_then(fixed_path::server_in_hook_command)
        .is_some()
}

/// The server paths our hooks inside `group` name.
fn ours_in_group(group: &Value) -> Vec<PathBuf> {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .map(|hooks| {
            hooks
                .iter()
                .filter_map(|hook| hook.get("command").and_then(Value::as_str))
                .filter_map(fixed_path::server_in_hook_command)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `claude` is on `PATH`.
///
/// No process is spawned: INST-05 runs the scan at every launch, and a launch that starts a
/// subprocess per adapter is not the "cheap" the requirement promises.
fn claude_on_path() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let names: &[&str] = if cfg!(windows) {
        &["claude.exe", "claude.cmd", "claude.bat", "claude"]
    } else {
        &["claude"]
    };
    std::env::split_paths(&path).any(|folder| names.iter().any(|name| folder.join(name).is_file()))
}

impl InstallAdapter for ClaudeCode {
    fn agent_id(&self) -> &'static str {
        AGENT_ID
    }

    fn detect(&self, scope: &Scope) -> Detection {
        let files = vec![self.mcp_file(scope), self.settings_file(scope)];
        let found = claude_on_path() || files.iter().any(|file| file.exists());
        Detection {
            agent_id: AGENT_ID,
            found,
            config_files: files,
            // Not read: see `claude_on_path`. `detect` stays free of subprocesses.
            version: None,
        }
    }

    fn plan(&self, scope: &Scope) -> Result<Vec<Modification>> {
        let mcp_file = self.mcp_file(scope);
        let mcp = Document::read(&mcp_file)?;
        let settings_file = self.settings_file(scope);
        let settings = Document::read(&settings_file)?;

        let file_name = |path: &Path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("?")
                .to_owned()
        };
        let server = self.server.display().to_string();
        let minutes = (RAISED_TOOL_TIMEOUT_MS / 60_000).to_string();

        let mut plan = vec![Modification::new(
            &mcp_file,
            vec!["mcpServers", SERVER_KEY],
            Description::new(KEY_MCP_ENTRY)
                .with("file", file_name(&mcp_file))
                .with("server", server.clone())
                .with("minutes", minutes),
            &mcp,
            &self.mcp_entry(),
        )];

        for (event, key) in HOOK_EVENTS
            .iter()
            .zip([KEY_STOP_HOOK, KEY_SUBAGENT_STOP_HOOK])
        {
            let path = vec!["hooks", *event];
            let after = self.hook_entries(settings.get_at(&path));
            plan.push(Modification::new(
                &settings_file,
                path,
                Description::new(key)
                    .with("file", file_name(&settings_file))
                    .with("server", server.clone()),
                &settings,
                &after,
            ));
        }
        Ok(plan)
    }

    fn apply(&self, plan: &[Modification]) -> Result<()> {
        // The token before the configuration: a registered agent that cannot authenticate
        // gets text mode with no explanation (FM-10), while a token nobody is registered
        // for costs nothing (INST-07).
        if let Some(parent) = self.token_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| InstallError::unwritable(parent, error))?;
        }
        token::ensure(&self.token_path)
            .map_err(|error| InstallError::unwritable(&self.token_path, error))?;

        super::apply(plan)
    }

    fn verify(&self, scope: &Scope) -> Registration {
        let registered = self.registered_paths(scope);
        if let Some(other) = registered
            .iter()
            .find(|path| !fixed_path::is_current(path, &self.server))
        {
            // FM-23: ours, and not where we are. The repair offer rewrites all three.
            return Registration::PathMismatch {
                registered: other.clone(),
                current: self.server.clone(),
            };
        }

        let Ok(plan) = self.plan(scope) else {
            // A configuration file that cannot be read or parsed. Nothing of ours can be
            // said to be registered in it, and `apply` will refuse it for the same reason.
            return Registration::NotRegistered;
        };
        let missing: Vec<String> = plan
            .iter()
            .filter(|modification| !modification.is_noop())
            .map(Modification::location)
            .collect();

        if missing.len() == plan.len() && registered.is_empty() {
            Registration::NotRegistered
        } else if missing.is_empty() {
            Registration::Registered
        } else {
            Registration::Partial { missing }
        }
    }

    fn uninstall(&self, scope: &Scope) -> Result<()> {
        let mcp_file = self.mcp_file(scope);
        let mut mcp = Document::read(&mcp_file)?;
        if mcp.existed() {
            let entry_path = ["mcpServers", SERVER_KEY];
            let ours = mcp
                .get_at(&entry_path)
                .and_then(|entry| entry.get("command"))
                .and_then(Value::as_str)
                .is_some_and(|command| fixed_path::is_ours(Path::new(command)));
            if ours {
                super::back_up(&mcp_file)?;
                mcp.remove_at(&entry_path);
                mcp.write(&mcp_file)?;
            }
        }

        let settings_file = self.settings_file(scope);
        let mut settings = Document::read(&settings_file)?;
        if settings.existed() {
            let mut changed = false;
            let mut edits: Vec<(Vec<&'static str>, Vec<Value>)> = Vec::new();
            for event in HOOK_EVENTS {
                let path = vec!["hooks", event];
                let Some(current) = settings.get_at(&path) else {
                    continue;
                };
                let kept = Self::hook_entries_without_ours(Some(current));
                if current.as_array() != Some(&kept) {
                    changed = true;
                }
                edits.push((path, kept));
            }
            if changed {
                super::back_up(&settings_file)?;
                for (path, kept) in edits {
                    // An event we emptied loses its key, and `hooks` with it: the file goes
                    // back to the shape it had before the install, not to a shape with our
                    // leftovers in it.
                    if kept.is_empty() {
                        settings.remove_at(&path);
                    } else {
                        settings.set_at(&path, Value::Array(kept));
                    }
                }
                settings.write(&settings_file)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> ClaudeCode {
        ClaudeCode::with(
            "/home/someone",
            "/apps/Baton/handoff-mcp",
            "/home/someone/.handoff/channel.token",
        )
    }

    #[test]
    fn the_mcp_entry_is_the_object_of_the_design() {
        assert_eq!(
            adapter().mcp_entry(),
            json!({
                "type": "stdio",
                "command": "/apps/Baton/handoff-mcp",
                "args": [],
                "env": {
                    "HANDOFF_AGENT": "claude-code",
                    "HANDOFF_TOOL_TIMEOUT_MS": "1800000"
                },
                "timeout": 1_800_000
            })
        );
    }

    #[test]
    fn the_hook_group_is_the_object_of_the_design() {
        assert_eq!(
            adapter().hook_group(),
            json!({
                "matcher": "",
                "hooks": [ {
                    "type": "command",
                    "command": "\"/apps/Baton/handoff-mcp\" hook stop",
                    "timeout": 5
                } ]
            })
        );
    }

    #[test]
    fn the_two_files_of_each_scope() {
        let adapter = adapter();
        assert_eq!(
            adapter.mcp_file(&Scope::User),
            PathBuf::from("/home/someone/.claude.json")
        );
        assert_eq!(
            adapter.settings_file(&Scope::User),
            PathBuf::from("/home/someone/.claude/settings.json")
        );
        assert_eq!(
            adapter.mcp_file(&Scope::project("/work/project")),
            PathBuf::from("/work/project/.mcp.json")
        );
        assert_eq!(
            adapter.settings_file(&Scope::project("/work/project")),
            PathBuf::from("/work/project/.claude/settings.json")
        );
    }

    #[test]
    fn an_empty_hook_list_gets_ours_alone() {
        let adapter = adapter();
        assert_eq!(adapter.hook_entries(None), json!([adapter.hook_group()]));
    }

    #[test]
    fn an_existing_hook_is_kept_and_ours_is_appended() {
        let adapter = adapter();
        let theirs = json!({
            "matcher": "Bash",
            "hooks": [ { "type": "command", "command": "./scripts/lint.sh" } ]
        });
        assert_eq!(
            adapter.hook_entries(Some(&json!([theirs.clone()]))),
            json!([theirs, adapter.hook_group()])
        );
    }

    #[test]
    fn planning_twice_produces_the_same_array() {
        let adapter = adapter();
        let once = adapter.hook_entries(None);
        assert_eq!(adapter.hook_entries(Some(&once)), once);
    }

    #[test]
    fn our_group_is_repaired_where_it_already_sits() {
        // The bundle moved (FM-23): the old command is ours, so the group is replaced in
        // place rather than dropped and re-appended after the user's.
        let adapter = adapter();
        let old = json!({
            "matcher": "",
            "hooks": [ {
                "type": "command",
                "command": "/old/Baton/handoff-mcp hook stop",
                "timeout": 5
            } ]
        });
        let theirs =
            json!({ "matcher": "", "hooks": [ { "type": "command", "command": "echo hi" } ] });
        assert_eq!(
            adapter.hook_entries(Some(&json!([old, theirs.clone()]))),
            json!([adapter.hook_group(), theirs])
        );
    }

    #[test]
    fn a_group_holding_both_keeps_the_users_half() {
        let adapter = adapter();
        let merged = json!({
            "matcher": "",
            "hooks": [
                { "type": "command", "command": "/old/Baton/handoff-mcp hook stop" },
                { "type": "command", "command": "make check" }
            ]
        });
        assert_eq!(
            adapter.hook_entries(Some(&json!([merged]))),
            json!([
                { "matcher": "", "hooks": [ { "type": "command", "command": "make check" } ] },
                adapter.hook_group()
            ])
        );
    }

    #[test]
    fn a_second_copy_of_ours_is_collapsed_into_one() {
        let adapter = adapter();
        let duplicated = json!([adapter.hook_group(), adapter.hook_group()]);
        assert_eq!(
            adapter.hook_entries(Some(&duplicated)),
            json!([adapter.hook_group()])
        );
    }

    #[test]
    fn uninstall_takes_out_ours_and_nothing_else() {
        let adapter = adapter();
        let theirs = json!({ "matcher": "Bash", "hooks": [ { "type": "command", "command": "./lint.sh" } ] });
        let with_ours = json!([theirs.clone(), adapter.hook_group()]);
        assert_eq!(
            ClaudeCode::hook_entries_without_ours(Some(&with_ours)),
            vec![theirs]
        );
        assert!(
            ClaudeCode::hook_entries_without_ours(Some(&json!([adapter.hook_group()]))).is_empty()
        );
    }

    #[test]
    fn a_hook_shaped_like_nothing_we_write_is_left_alone() {
        // A group with no `hooks` array, and one whose hooks are not objects: both are the
        // user's, and INST-04 keeps them.
        let odd = json!([{ "matcher": "x" }, { "matcher": "y", "hooks": ["not an object"] }]);
        assert_eq!(
            ClaudeCode::hook_entries_without_ours(Some(&odd)),
            odd.as_array().cloned().expect("an array")
        );
    }
}
