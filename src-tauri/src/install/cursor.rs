//! The Cursor installation adapter (§7.15, INST-01..08, ADPT-04, ADPT-06 item 2).
//!
//! One modification, one file, and the Claude Code adapter's own shape: Cursor keeps its MCP
//! servers in `mcp.json` under `mcpServers`, as Claude Code keeps them in `.claude.json`, and
//! the file is edited through [`super::json`] with its rule that whatever is already in it is
//! still there afterwards, in the same order (INST-04).
//!
//! | File | Location | What |
//! |---|---|---|
//! | `~/.cursor/mcp.json` (`.cursor/mcp.json` in a project) | `mcpServers.handoff` | the stdio entry with the fixed launcher path and `HANDOFF_AGENT` |
//!
//! Both of Cursor's surfaces read those two files — the editor, which starts the servers of the
//! user file as a window opens, and the Agent CLI — so one registration covers the two (T-068).
//! Four facts measured against Cursor 3.20.10 and its CLI 2026.09.10 decide the rest (T-069,
//! and `docs/agent-facts.md` of `handoff-mcp`):
//!
//! - **There is no hook to register.** Cursor has hooks of its own and runs Claude Code's as
//!   well, but a Cursor `stop` payload carries no `stop_hook_active`, so `handoff-mcp hook stop`
//!   answers it silently before it reads the token, and under `agent -p` no hook ran at all.
//!   The capability row says `stop_hook: false`, and the consent screen has one line.
//! - **There is no timeout to write.** Neither surface reads a timeout from an MCP entry. The
//!   entry therefore carries no `timeout`, and no `HANDOFF_TOOL_TIMEOUT_MS` either: the server
//!   reads that variable as the agent's limit, so a larger value would move the heartbeat past
//!   the sixty seconds after which the CLI cuts a call. The row's own 60 000 ms is what puts the
//!   heartbeat at the 50 s floor.
//! - **No permission is written.** The editor asks before it runs a tool of ours (T-068). The
//!   CLI's print mode, `agent -p`, refuses a tool that is not annotated read-only unless a
//!   permission rule allows it (`Mcp(handoff:*)` in a `cli.json`); that rule is a standing grant
//!   for a mode nobody uses to talk to a person, so it is left to the user
//!   (`docs/agents/cursor.md` says where it goes) and the consent line grants nothing beyond the
//!   entry.
//! - **The entry is `command`, `args` and `env`**, the shape both surfaces were measured with;
//!   no `type`.
//!
//! A comment in `mcp.json` makes the file one this adapter refuses rather than rewrites
//! ([`InstallError::Malformed`]), the rule every adapter keeps for a file it cannot read back
//! whole.
//!
//! **Project scope** writes `<project>/.cursor/mcp.json`. Cursor loads a server of a project
//! file only once it has been approved — the editor asks, the CLI refuses until
//! `agent mcp enable handoff` — and this adapter never writes that approval, which Cursor keeps
//! in its own state under `~/.cursor/projects/`.
//!
//! # Recognising ours
//!
//! By the fixed path in `command` (§7.15), tested with [`fixed_path::is_ours`] as for the other
//! adapters, wherever the bundle has been moved since (FM-23).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::channel::token;
use crate::paths;

use super::error::{InstallError, Result};
use super::fixed_path;
use super::json::Document;
use super::{
    Description, Detection, InstallAdapter, Modification, Registration, Scope, ENV_HANDOFF_AGENT,
};

/// The agent id, shared with the server's capability table (INST-08, §5.6).
pub const AGENT_ID: &str = "cursor";

/// The key our entry has under `mcpServers` (§7.15).
pub const SERVER_KEY: &str = "handoff";

/// Where our entry is, from the root of the file.
pub const ENTRY_PATH: [&str; 2] = ["mcpServers", SERVER_KEY];

/// The catalogue key of the one line of the consent screen.
pub const KEY_MCP_ENTRY: &str = "install.cursor.mcpEntry";

/// Cursor's own folder, in the home folder and in a project alike.
const CURSOR_FOLDER: &str = ".cursor";

/// The file both of Cursor's surfaces read their MCP servers from.
const CONFIG_FILE: &str = "mcp.json";

/// The Cursor adapter.
///
/// Its paths are fields for the reason the other adapters' are: a golden file points the whole
/// adapter at a temporary folder, and what it writes there is what it writes on a real machine.
#[derive(Debug, Clone)]
pub struct Cursor {
    /// Cursor's folder in the home folder, `~/.cursor`.
    config_dir: PathBuf,
    /// The fixed launcher path written into the configuration (SRV-25).
    server: PathBuf,
    /// `~/.handoff/channel.token`, created by the first `apply` (INST-07, SRV-07).
    token_path: PathBuf,
}

impl Cursor {
    /// The adapter for this machine and this build.
    ///
    /// # Errors
    ///
    /// [`InstallError::NoServer`] when the bundled server cannot be located.
    pub fn detected() -> Result<Self> {
        Ok(Self {
            config_dir: paths::home_dir().join(CURSOR_FOLDER),
            server: fixed_path::server_path()?,
            token_path: paths::token_path(),
        })
    }

    /// An adapter over explicit paths, for the golden files.
    #[must_use]
    pub fn with(
        config_dir: impl Into<PathBuf>,
        server: impl Into<PathBuf>,
        token: impl Into<PathBuf>,
    ) -> Self {
        Self {
            config_dir: config_dir.into(),
            server: server.into(),
            token_path: token.into(),
        }
    }

    /// The path this adapter registers.
    #[must_use]
    pub fn server(&self) -> &Path {
        &self.server
    }

    /// The file holding `mcpServers`: `mcp.json` in Cursor's folder, or in the project's.
    #[must_use]
    pub fn config_file(&self, scope: &Scope) -> PathBuf {
        match scope {
            Scope::User => self.config_dir.join(CONFIG_FILE),
            Scope::Project { path } => path.join(CURSOR_FOLDER).join(CONFIG_FILE),
        }
    }

    /// Our entry, in the order a person reads it: what runs, with what, for which agent.
    ///
    /// The command is the fixed path alone, with no arguments, as for Claude Code: it is the
    /// whole command (SRV-19). Nothing about a timeout, for the reason the module gives.
    #[must_use]
    pub fn mcp_entry(&self) -> Value {
        json!({
            "command": self.server.display().to_string(),
            "args": [],
            "env": {
                ENV_HANDOFF_AGENT: AGENT_ID,
            },
        })
    }

    /// The server path our entry names today, when the entry under our key is ours (FM-23).
    fn registered_path(&self, scope: &Scope) -> Option<PathBuf> {
        let document = Document::read(&self.config_file(scope)).ok()?;
        command_of(&document)
            .map(Path::new)
            .filter(|path| fixed_path::is_ours(path))
            .map(Path::to_path_buf)
    }
}

/// The program of the entry under our key, whoever wrote it.
fn command_of(document: &Document) -> Option<&str> {
    document
        .get_at(&ENTRY_PATH)
        .and_then(|entry| entry.get("command"))
        .and_then(Value::as_str)
}

/// The name of a file, as the consent screen names it.
fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?")
        .to_owned()
}

/// Whether the editor's `cursor` launcher or the Agent CLI's `cursor-agent` is on `PATH`.
///
/// No process is spawned, for the reason the other adapters give: INST-05 runs the scan at
/// every launch. The CLI's other name, `agent`, is not looked for: it says nothing about which
/// program it is.
fn cursor_on_path() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let names: &[&str] = if cfg!(windows) {
        &[
            "cursor.cmd",
            "cursor-agent.cmd",
            "cursor.exe",
            "cursor",
            "cursor-agent",
        ]
    } else {
        &["cursor", "cursor-agent"]
    };
    std::env::split_paths(&path).any(|folder| names.iter().any(|name| folder.join(name).is_file()))
}

impl InstallAdapter for Cursor {
    fn agent_id(&self) -> &'static str {
        AGENT_ID
    }

    fn detect(&self, scope: &Scope) -> Detection {
        let file = self.config_file(scope);
        // Cursor's folder counts as well as the file: the editor and the CLI both create it the
        // first time they run, long before anybody writes an `mcp.json` (T-068 found it with no
        // `mcp.json` in it).
        let found = cursor_on_path() || self.config_dir.is_dir() || file.exists();
        Detection {
            agent_id: AGENT_ID,
            found,
            config_files: vec![file],
            // Not read, for the reason `cursor_on_path` gives.
            version: None,
        }
    }

    fn plan(&self, scope: &Scope) -> Result<Vec<Modification>> {
        let file = self.config_file(scope);
        let document = Document::read(&file)?;
        if document
            .get_at(&ENTRY_PATH[..1])
            .is_some_and(|servers| !servers.is_object())
        {
            // Writing our entry would mean replacing the value the user has there.
            return Err(InstallError::not_editable(
                &file,
                "`mcpServers` is not an object of servers, the one shape this adapter adds to",
            ));
        }

        let server = self.server.display().to_string();
        Ok(vec![Modification::new(
            &file,
            ENTRY_PATH.to_vec(),
            Description::new(KEY_MCP_ENTRY)
                .with("file", file_name(&file))
                .with("server", server),
            &document,
            &self.mcp_entry(),
        )])
    }

    fn apply(&self, plan: &[Modification]) -> Result<()> {
        // The token before the configuration, for the reason the Claude Code adapter gives: a
        // registered agent that cannot authenticate gets text mode with no explanation (FM-10).
        if let Some(parent) = self.token_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| InstallError::unwritable(parent, error))?;
        }
        token::ensure(&self.token_path)
            .map_err(|error| InstallError::unwritable(&self.token_path, error))?;

        super::apply(plan)
    }

    fn verify(&self, scope: &Scope) -> Registration {
        let registered = self.registered_path(scope);
        if let Some(other) = registered
            .as_ref()
            .filter(|path| !fixed_path::is_current(path, &self.server))
        {
            // FM-23: ours, and not where we are. The repair offer rewrites the entry.
            return Registration::PathMismatch {
                registered: other.clone(),
                current: self.server.clone(),
            };
        }

        let Ok(plan) = self.plan(scope) else {
            // A configuration file that cannot be read or parsed. Nothing of ours can be said
            // to be registered in it, and `apply` will refuse it for the same reason.
            return Registration::NotRegistered;
        };
        let missing: Vec<String> = plan
            .iter()
            .filter(|modification| !modification.is_noop())
            .map(Modification::location)
            .collect();

        if missing.len() == plan.len() && registered.is_none() {
            Registration::NotRegistered
        } else if missing.is_empty() {
            Registration::Registered
        } else {
            Registration::Partial { missing }
        }
    }

    fn uninstall(&self, scope: &Scope) -> Result<()> {
        let file = self.config_file(scope);
        let mut document = Document::read(&file)?;
        let ours =
            command_of(&document).is_some_and(|command| fixed_path::is_ours(Path::new(command)));
        if ours {
            super::back_up(&file)?;
            document.remove_at(&ENTRY_PATH);
            document.write(&file)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> Cursor {
        Cursor::with(
            "/home/someone/.cursor",
            "/apps/Baton/handoff-mcp",
            "/home/someone/.handoff/channel.token",
        )
    }

    #[test]
    fn the_entry_is_the_one_cursor_was_measured_with() {
        // T-069: the fixed path as the whole command, no arguments and the agent's id, the
        // shape from which Cursor's editor and its CLI both started a server.
        assert_eq!(
            super::super::json::render_with_indent(&adapter().mcp_entry(), "  "),
            concat!(
                "{\n",
                "  \"command\": \"/apps/Baton/handoff-mcp\",\n",
                "  \"args\": [],\n",
                "  \"env\": {\n",
                "    \"HANDOFF_AGENT\": \"cursor\"\n",
                "  }\n",
                "}",
            )
        );
    }

    #[test]
    fn the_entry_carries_no_timeout_of_any_kind() {
        // Cursor reads no timeout from an entry (T-069), and a `HANDOFF_TOOL_TIMEOUT_MS` above
        // the CLI's sixty seconds would put the heartbeat after the cut.
        let entry = adapter().mcp_entry();
        assert!(entry.get("timeout").is_none());
        let env = entry["env"].as_object().expect("an env object");
        assert!(!env.contains_key(super::super::ENV_HANDOFF_TOOL_TIMEOUT_MS));
        assert_eq!(env.len(), 1, "HANDOFF_AGENT and nothing else");
    }

    #[test]
    fn a_windows_path_survives_the_json_it_is_written_into() {
        // JSON escapes every backslash, and Cursor reads the string back as the path.
        let adapter = Cursor::with(
            "C:\\Users\\someone\\.cursor",
            "C:\\Users\\someone\\AppData\\Local\\Baton\\handoff-mcp.exe",
            "C:\\Users\\someone\\.handoff\\channel.token",
        );
        let rendered = super::super::json::render_with_indent(&adapter.mcp_entry(), "  ");
        assert!(
            rendered.contains(
                "\"C:\\\\Users\\\\someone\\\\AppData\\\\Local\\\\Baton\\\\handoff-mcp.exe\""
            ),
            "{rendered}"
        );
        let parsed: Value = serde_json::from_str(&rendered).expect("valid JSON");
        assert_eq!(
            parsed["command"],
            json!("C:\\Users\\someone\\AppData\\Local\\Baton\\handoff-mcp.exe")
        );
    }

    #[test]
    fn the_file_of_each_scope() {
        let adapter = adapter();
        assert_eq!(
            adapter.config_file(&Scope::User),
            PathBuf::from("/home/someone/.cursor/mcp.json")
        );
        assert_eq!(
            adapter.config_file(&Scope::project("/work/project")),
            PathBuf::from("/work/project/.cursor/mcp.json")
        );
    }

    #[test]
    fn the_names_it_writes_survive_project_scope() {
        // Cursor strips nothing that was measured (T-069: a TOKEN-named variable arrived), but
        // the rule of every adapter costs nothing and the failure it prevents is invisible.
        let entry = adapter().mcp_entry();
        let env = entry["env"].as_object().expect("an env object");
        for name in env.keys() {
            assert!(super::super::survives_project_scope(name), "{name}");
        }
    }
}
