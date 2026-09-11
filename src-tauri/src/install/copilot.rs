//! The GitHub Copilot installation adapter (§7.15, INST-01..08, ADPT-04, ADPT-06 item 3).
//!
//! Two modifications, one per surface: GitHub Copilot's two surfaces read two files of two
//! different shapes (T-071, T-072), and each gets the entry it was measured with.
//!
//! | File | Location | What |
//! |---|---|---|
//! | `~/.copilot/mcp-config.json` (`.github/mcp.json` in a project) | `mcpServers.handoff` | the Copilot CLI's `local` entry: the fixed launcher path, `HANDOFF_AGENT` and `HANDOFF_TOOL_TIMEOUT_MS`, every tool, and the per-server `timeout` |
//! | VS Code's user `mcp.json` (`.vscode/mcp.json` in a project) | `servers.handoff` | VS Code's `stdio` entry: the fixed launcher path and `HANDOFF_AGENT` |
//!
//! What the measurements of T-072 decide (`docs/agent-facts.md` of `handoff-mcp`):
//!
//! - **The CLI's entry raises the timeout; VS Code's has none to raise.** The CLI honours the
//!   entry's `timeout`, in milliseconds (20 000 cut a call at 20 010 ms, with an MCP
//!   cancellation), so its entry carries the same 30 minutes as Claude Code's, mirrored in
//!   `HANDOFF_TOOL_TIMEOUT_MS`. VS Code's `mcp.json` has no such field, and the limit VS Code's
//!   chat puts on a tool call is not measured, so that entry carries neither: the server's
//!   50-second heartbeat keeps a long handoff alive there, which is the safe direction.
//! - **There is no hook to register.** Both surfaces run hooks, and neither answers ours in a
//!   way the agent acts on across both: VS Code reads a block from a field of its own, and the
//!   capability row, which serves both surfaces, says `stop_hook: false`.
//! - **No permission is written.** VS Code asks before a chat runs a tool of ours (T-071), and
//!   the CLI's print mode takes its allowance on its own command line (`--allow-tool=handoff`).
//! - **The entries are the shapes the surfaces read**: the CLI's is what `copilot mcp add`
//!   writes (`type: "local"`, `tools: ["*"]`, `timeout`), VS Code's is its own `mcp.json`'s
//!   (`type: "stdio"`).
//!
//! A comment in either file makes it one this adapter refuses rather than rewrites
//! ([`InstallError::Malformed`]), the rule every adapter keeps for a file it cannot read back
//! whole.
//!
//! **Project scope** writes `<project>/.github/mcp.json` for the CLI — not `.mcp.json`, which is
//! Claude Code's project file and holds that adapter's own `handoff` entry — and
//! `<project>/.vscode/mcp.json` for VS Code. The CLI reads a project's files only in a folder it
//! trusts, and VS Code asks before it starts a server of a workspace file for the first time;
//! this adapter writes neither trust.
//!
//! # Recognising ours
//!
//! By the fixed path in `command` (§7.15), in each file on its own, tested with
//! [`fixed_path::is_ours`] as for the other adapters, wherever the bundle has been moved since
//! (FM-23).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::channel::token;
use crate::paths;

use super::error::{InstallError, Result};
use super::fixed_path;
use super::json::Document;
use super::{
    Description, Detection, InstallAdapter, Modification, Registration, Scope, ENV_HANDOFF_AGENT,
    ENV_HANDOFF_TOOL_TIMEOUT_MS, RAISED_TOOL_TIMEOUT_MS,
};

/// The agent id, shared with the server's capability table (INST-08, §5.6).
pub const AGENT_ID: &str = "copilot";

/// The key our entry has in both files (§7.15).
pub const SERVER_KEY: &str = "handoff";

/// Where the CLI's entry is, from the root of its file.
pub const CLI_ENTRY_PATH: [&str; 2] = ["mcpServers", SERVER_KEY];

/// Where VS Code's entry is, from the root of its file.
pub const VSCODE_ENTRY_PATH: [&str; 2] = ["servers", SERVER_KEY];

/// The catalogue key of the CLI's line of the consent screen.
pub const KEY_CLI_ENTRY: &str = "install.copilot.cliEntry";

/// The catalogue key of VS Code's line of the consent screen.
pub const KEY_VSCODE_ENTRY: &str = "install.copilot.vscodeEntry";

/// The variable that moves the CLI's folder, as the CLI documents it (T-071).
pub const ENV_COPILOT_HOME: &str = "COPILOT_HOME";

/// The CLI's folder in the home folder.
const COPILOT_FOLDER: &str = ".copilot";

/// The file the CLI reads its user-level servers from.
const CLI_CONFIG_FILE: &str = "mcp-config.json";

/// The folder of a project the CLI reads `mcp.json` from, beside the `.mcp.json` it also reads.
const CLI_PROJECT_FOLDER: &str = ".github";

/// The folder of a workspace VS Code reads `mcp.json` from.
const VSCODE_PROJECT_FOLDER: &str = ".vscode";

/// The name of VS Code's file, in its user folder and in a workspace alike, and of the CLI's
/// project file.
const MCP_JSON: &str = "mcp.json";

/// The GitHub Copilot adapter.
///
/// Its paths are fields for the reason the other adapters' are: a golden file points the whole
/// adapter at a temporary folder, and what it writes there is what it writes on a real machine.
#[derive(Debug, Clone)]
pub struct Copilot {
    /// The CLI's folder: `$COPILOT_HOME`, else `~/.copilot`.
    copilot_dir: PathBuf,
    /// VS Code's user folder, where its user-level `mcp.json` is.
    vscode_user_dir: PathBuf,
    /// The fixed launcher path written into both files (SRV-25).
    server: PathBuf,
    /// `~/.handoff/channel.token`, created by the first `apply` (INST-07, SRV-07).
    token_path: PathBuf,
}

impl Copilot {
    /// The adapter for this machine and this build.
    ///
    /// # Errors
    ///
    /// [`InstallError::NoServer`] when the bundled server cannot be located.
    pub fn detected() -> Result<Self> {
        let home = paths::home_dir();
        Ok(Self {
            copilot_dir: copilot_dir_from(std::env::var_os(ENV_COPILOT_HOME), &home),
            vscode_user_dir: vscode_user_dir_from(
                std::env::var_os("APPDATA"),
                std::env::var_os("XDG_CONFIG_HOME"),
                &home,
            ),
            server: fixed_path::server_path()?,
            token_path: paths::token_path(),
        })
    }

    /// An adapter over explicit paths, for the golden files.
    #[must_use]
    pub fn with(
        copilot_dir: impl Into<PathBuf>,
        vscode_user_dir: impl Into<PathBuf>,
        server: impl Into<PathBuf>,
        token: impl Into<PathBuf>,
    ) -> Self {
        Self {
            copilot_dir: copilot_dir.into(),
            vscode_user_dir: vscode_user_dir.into(),
            server: server.into(),
            token_path: token.into(),
        }
    }

    /// The path this adapter registers.
    #[must_use]
    pub fn server(&self) -> &Path {
        &self.server
    }

    /// The CLI's file: `mcp-config.json` in its folder, or `.github/mcp.json` in the project.
    #[must_use]
    pub fn cli_config_file(&self, scope: &Scope) -> PathBuf {
        match scope {
            Scope::User => self.copilot_dir.join(CLI_CONFIG_FILE),
            Scope::Project { path } => path.join(CLI_PROJECT_FOLDER).join(MCP_JSON),
        }
    }

    /// VS Code's file: `mcp.json` in its user folder, or `.vscode/mcp.json` in the project.
    #[must_use]
    pub fn vscode_config_file(&self, scope: &Scope) -> PathBuf {
        match scope {
            Scope::User => self.vscode_user_dir.join(MCP_JSON),
            Scope::Project { path } => path.join(VSCODE_PROJECT_FOLDER).join(MCP_JSON),
        }
    }

    /// The CLI's entry, in the order a person reads it: what kind of server, what runs, with
    /// what, which of its tools, for how long.
    ///
    /// The command is the fixed path alone, with no arguments, as for the other adapters: it
    /// is the whole command (SRV-19).
    #[must_use]
    pub fn cli_entry(&self) -> Value {
        json!({
            "type": "local",
            "command": self.server.display().to_string(),
            "args": [],
            "env": {
                ENV_HANDOFF_AGENT: AGENT_ID,
                ENV_HANDOFF_TOOL_TIMEOUT_MS: RAISED_TOOL_TIMEOUT_MS.to_string(),
            },
            "tools": ["*"],
            "timeout": RAISED_TOOL_TIMEOUT_MS,
        })
    }

    /// VS Code's entry: what kind of server, what runs, with what. Nothing about a timeout, for
    /// the reason the module gives.
    #[must_use]
    pub fn vscode_entry(&self) -> Value {
        json!({
            "type": "stdio",
            "command": self.server.display().to_string(),
            "args": [],
            "env": {
                ENV_HANDOFF_AGENT: AGENT_ID,
            },
        })
    }

    /// Each file of the scope with the place our entry has in it.
    fn files(&self, scope: &Scope) -> [(PathBuf, &'static [&'static str]); 2] {
        [
            (self.cli_config_file(scope), &CLI_ENTRY_PATH),
            (self.vscode_config_file(scope), &VSCODE_ENTRY_PATH),
        ]
    }

    /// The server paths our entries name today, where the entry under our key is ours (FM-23).
    fn registered_paths(&self, scope: &Scope) -> Vec<PathBuf> {
        self.files(scope)
            .iter()
            .filter_map(|(file, entry)| {
                let document = Document::read(file).ok()?;
                command_of(&document, entry)
                    .map(Path::new)
                    .filter(|path| fixed_path::is_ours(path))
                    .map(Path::to_path_buf)
            })
            .collect()
    }
}

/// `$COPILOT_HOME`, else `~/.copilot`: where the CLI keeps its configuration (T-071).
///
/// Blank counts as unset, the rule [`crate::paths`] applies to our own variables. A value that
/// is not Unicode counts as unset too: it is not a path a consent screen could name.
#[must_use]
pub fn copilot_dir_from(variable: Option<OsString>, home: &Path) -> PathBuf {
    set_path(variable).unwrap_or_else(|| home.join(COPILOT_FOLDER))
}

/// VS Code's user folder: `%APPDATA%\Code\User` on Windows,
/// `~/Library/Application Support/Code/User` on macOS, and `$XDG_CONFIG_HOME/Code/User` (else
/// `~/.config/Code/User`) elsewhere.
///
/// VS Code Insiders and VSCodium keep folders of their own, and are not VS Code to this
/// adapter. Both variables are read on every platform and used on one, so that no branch of
/// this function is code only one CI leg compiles.
#[must_use]
pub fn vscode_user_dir_from(
    appdata: Option<OsString>,
    xdg_config_home: Option<OsString>,
    home: &Path,
) -> PathBuf {
    let root = if cfg!(target_os = "windows") {
        set_path(appdata).unwrap_or_else(|| home.join("AppData").join("Roaming"))
    } else if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else {
        set_path(xdg_config_home).unwrap_or_else(|| home.join(".config"))
    };
    root.join("Code").join("User")
}

/// A variable's value as a path, when it is set to something: trimmed, and blank as unset.
fn set_path(variable: Option<OsString>) -> Option<PathBuf> {
    variable
        .and_then(|value| value.into_string().ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The program of the entry at `entry`, whoever wrote it.
fn command_of<'a>(document: &'a Document, entry: &[&str]) -> Option<&'a str> {
    document
        .get_at(entry)
        .and_then(|value| value.get("command"))
        .and_then(Value::as_str)
}

/// The name of a file, as the consent screen names it.
fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?")
        .to_owned()
}

/// Whether any of `names` is a file in a folder of `PATH`.
///
/// No process is spawned, for the reason the other adapters give: INST-05 runs the scan at
/// every launch.
fn on_path(names: &[&str]) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|folder| names.iter().any(|name| folder.join(name).is_file()))
}

/// Whether the Copilot CLI or VS Code is on `PATH`: an npm install of the CLI leaves the `.cmd`
/// shim and an extensionless script, and VS Code puts `code` there.
fn a_surface_on_path() -> bool {
    if cfg!(windows) {
        on_path(&["copilot.cmd", "copilot.exe", "copilot", "code.cmd"])
    } else {
        on_path(&["copilot", "code"])
    }
}

/// Refuses a file whose parent of our entry is there and is not an object.
fn refuse_a_foreign_shape(document: &Document, file: &Path, entry: &[&str]) -> Result<()> {
    if document
        .get_at(&entry[..1])
        .is_some_and(|servers| !servers.is_object())
    {
        // Writing our entry would mean replacing the value the user has there.
        return Err(InstallError::not_editable(
            file,
            "the servers of this file are not an object, the one shape this adapter adds to",
        ));
    }
    Ok(())
}

impl InstallAdapter for Copilot {
    fn agent_id(&self) -> &'static str {
        AGENT_ID
    }

    fn detect(&self, scope: &Scope) -> Detection {
        let [(cli_file, _), (vscode_file, _)] = self.files(scope);
        // Either surface's folder counts as well as its file: the CLI and VS Code both create
        // theirs the first time they run, before anybody writes an MCP server into it, and VS
        // Code carries Copilot's chat built in.
        let found = a_surface_on_path()
            || self.copilot_dir.is_dir()
            || self.vscode_user_dir.is_dir()
            || cli_file.exists()
            || vscode_file.exists();
        Detection {
            agent_id: AGENT_ID,
            found,
            config_files: vec![cli_file, vscode_file],
            // Not read, for the reason `on_path` gives.
            version: None,
        }
    }

    fn plan(&self, scope: &Scope) -> Result<Vec<Modification>> {
        let [(cli_file, _), (vscode_file, _)] = self.files(scope);
        let cli = Document::read(&cli_file)?;
        refuse_a_foreign_shape(&cli, &cli_file, &CLI_ENTRY_PATH)?;
        let vscode = Document::read(&vscode_file)?;
        refuse_a_foreign_shape(&vscode, &vscode_file, &VSCODE_ENTRY_PATH)?;

        let server = self.server.display().to_string();
        let minutes = (RAISED_TOOL_TIMEOUT_MS / 60_000).to_string();
        Ok(vec![
            Modification::new(
                &cli_file,
                CLI_ENTRY_PATH.to_vec(),
                Description::new(KEY_CLI_ENTRY)
                    .with("file", file_name(&cli_file))
                    .with("server", server.clone())
                    .with("minutes", minutes),
                &cli,
                &self.cli_entry(),
            ),
            Modification::new(
                &vscode_file,
                VSCODE_ENTRY_PATH.to_vec(),
                Description::new(KEY_VSCODE_ENTRY)
                    .with("file", file_name(&vscode_file))
                    .with("server", server),
                &vscode,
                &self.vscode_entry(),
            ),
        ])
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
        let registered = self.registered_paths(scope);
        if let Some(other) = registered
            .iter()
            .find(|path| !fixed_path::is_current(path, &self.server))
        {
            // FM-23: ours, and not where we are. The repair offer rewrites both entries.
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

        if missing.len() == plan.len() && registered.is_empty() {
            Registration::NotRegistered
        } else if missing.is_empty() {
            Registration::Registered
        } else {
            Registration::Partial { missing }
        }
    }

    fn uninstall(&self, scope: &Scope) -> Result<()> {
        for (file, entry) in self.files(scope) {
            let mut document = Document::read(&file)?;
            let ours = command_of(&document, entry)
                .is_some_and(|command| fixed_path::is_ours(Path::new(command)));
            if ours {
                super::back_up(&file)?;
                document.remove_at(entry);
                document.write(&file)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> Copilot {
        Copilot::with(
            "/home/someone/.copilot",
            "/home/someone/.config/Code/User",
            "/apps/Baton/handoff-mcp",
            "/home/someone/.handoff/channel.token",
        )
    }

    #[test]
    fn the_cli_entry_is_the_shape_copilot_mcp_add_writes() {
        // T-072: `copilot mcp add --timeout 1800000 --env …` writes a local entry with every
        // tool, and the CLI cut a call at the entry's timeout, in milliseconds.
        assert_eq!(
            super::super::json::render_with_indent(&adapter().cli_entry(), "  "),
            concat!(
                "{\n",
                "  \"type\": \"local\",\n",
                "  \"command\": \"/apps/Baton/handoff-mcp\",\n",
                "  \"args\": [],\n",
                "  \"env\": {\n",
                "    \"HANDOFF_AGENT\": \"copilot\",\n",
                "    \"HANDOFF_TOOL_TIMEOUT_MS\": \"1800000\"\n",
                "  },\n",
                "  \"tools\": [\n",
                "    \"*\"\n",
                "  ],\n",
                "  \"timeout\": 1800000\n",
                "}",
            )
        );
    }

    #[test]
    fn the_vscode_entry_is_a_stdio_server_with_no_timeout_of_any_kind() {
        // VS Code's `mcp.json` has no timeout field, and its chat's own limit is not measured:
        // a `HANDOFF_TOOL_TIMEOUT_MS` would move the heartbeat past a cut nobody knows.
        assert_eq!(
            super::super::json::render_with_indent(&adapter().vscode_entry(), "  "),
            concat!(
                "{\n",
                "  \"type\": \"stdio\",\n",
                "  \"command\": \"/apps/Baton/handoff-mcp\",\n",
                "  \"args\": [],\n",
                "  \"env\": {\n",
                "    \"HANDOFF_AGENT\": \"copilot\"\n",
                "  }\n",
                "}",
            )
        );
        let entry = adapter().vscode_entry();
        assert!(entry.get("timeout").is_none());
        let env = entry["env"].as_object().expect("an env object");
        assert!(!env.contains_key(ENV_HANDOFF_TOOL_TIMEOUT_MS));
    }

    #[test]
    fn a_windows_path_survives_the_json_it_is_written_into() {
        let adapter = Copilot::with(
            "C:\\Users\\someone\\.copilot",
            "C:\\Users\\someone\\AppData\\Roaming\\Code\\User",
            "C:\\Users\\someone\\AppData\\Local\\Baton\\handoff-mcp.exe",
            "C:\\Users\\someone\\.handoff\\channel.token",
        );
        for entry in [adapter.cli_entry(), adapter.vscode_entry()] {
            let rendered = super::super::json::render_with_indent(&entry, "  ");
            let parsed: Value = serde_json::from_str(&rendered).expect("valid JSON");
            assert_eq!(
                parsed["command"],
                json!("C:\\Users\\someone\\AppData\\Local\\Baton\\handoff-mcp.exe")
            );
        }
    }

    #[test]
    fn the_files_of_each_scope() {
        let adapter = adapter();
        assert_eq!(
            adapter.cli_config_file(&Scope::User),
            PathBuf::from("/home/someone/.copilot/mcp-config.json")
        );
        assert_eq!(
            adapter.vscode_config_file(&Scope::User),
            PathBuf::from("/home/someone/.config/Code/User/mcp.json")
        );
        // Not `.mcp.json`: that is Claude Code's project file, with its own `handoff` entry.
        assert_eq!(
            adapter.cli_config_file(&Scope::project("/work/project")),
            PathBuf::from("/work/project/.github/mcp.json")
        );
        assert_eq!(
            adapter.vscode_config_file(&Scope::project("/work/project")),
            PathBuf::from("/work/project/.vscode/mcp.json")
        );
    }

    #[test]
    fn copilot_home_moves_the_clis_folder_and_blank_is_unset() {
        let home = Path::new("/home/someone");
        assert_eq!(
            copilot_dir_from(Some(OsString::from("/elsewhere/copilot")), home),
            PathBuf::from("/elsewhere/copilot")
        );
        assert_eq!(
            copilot_dir_from(Some(OsString::from("  ")), home),
            PathBuf::from("/home/someone/.copilot")
        );
        assert_eq!(
            copilot_dir_from(None, home),
            PathBuf::from("/home/someone/.copilot")
        );
    }

    #[test]
    fn vscodes_user_folder_is_where_this_platform_keeps_it() {
        let home = Path::new("/home/someone");
        let found = vscode_user_dir_from(
            Some(OsString::from("/roaming")),
            Some(OsString::from("/xdg")),
            home,
        );
        let expected = if cfg!(target_os = "windows") {
            PathBuf::from("/roaming").join("Code").join("User")
        } else if cfg!(target_os = "macos") {
            home.join("Library")
                .join("Application Support")
                .join("Code")
                .join("User")
        } else {
            PathBuf::from("/xdg").join("Code").join("User")
        };
        assert_eq!(found, expected);
        // With nothing set, the folder each platform documents under the home folder.
        let unset = vscode_user_dir_from(None, None, home);
        assert!(unset.starts_with(home), "{}", unset.display());
        assert!(unset.ends_with(Path::new("Code").join("User")));
    }

    #[test]
    fn the_names_it_writes_survive_project_scope() {
        for entry in [adapter().cli_entry(), adapter().vscode_entry()] {
            let env = entry["env"].as_object().expect("an env object");
            for name in env.keys() {
                assert!(super::super::survives_project_scope(name), "{name}");
            }
        }
    }
}
