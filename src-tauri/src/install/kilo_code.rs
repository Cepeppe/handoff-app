//! The Kilo Code installation adapter (§7.15, INST-01..08, ADPT-04, and implementation
//! decision 13, which makes Kilo Code the fifth agent of ADPT-06).
//!
//! One modification, one file, OpenCode's shape: Kilo's CLI is a fork of OpenCode, and its VS
//! Code extension runs the same program as `kilo serve`, so both surfaces read the same
//! configuration and one entry registers Baton for the two. The file is JSON, edited through
//! [`super::json`] with its rule that whatever is already in the file is still there
//! afterwards, in the same order (INST-04).
//!
//! | File | Location | What |
//! |---|---|---|
//! | `~/.config/kilo/kilo.json` (under `$XDG_CONFIG_HOME` when it is set; `kilo.json` in a project) | `mcp.handoff` | the local entry with the fixed launcher path, our two environment variables and the per-server `timeout` |
//!
//! Five facts measured against Kilo 7.6.2 decide that shape (T-080, T-081, and
//! `docs/agent-facts.md` of `handoff-mcp`):
//!
//! - **There is no hook to register.** Kilo, like OpenCode, has plugins — code inside its own
//!   process — and no command it runs at the end of a turn, so the capability row says
//!   `stop_hook: false` and this adapter makes one modification. The consent screen has one
//!   line.
//! - **No permission is needed.** Neither `kilo run` nor the VS Code extension asked before
//!   calling an MCP tool, so the entry carries no `permission` key and the consent line grants
//!   nothing beyond the entry itself.
//! - **The timeout is in milliseconds.** `"timeout": 1800000` is the thirty minutes of
//!   INST-03, and it matters here as it does for OpenCode: with nothing configured Kilo cuts a
//!   call after sixty seconds. `HANDOFF_TOOL_TIMEOUT_MS` mirrors it so the server heartbeats a
//!   minute before it (§5.6).
//! - **The command is one array and the variables are `environment`**, as for OpenCode:
//!   `{ "type": "local", "command": [<fixed path>], … }`. `kilo debug config` printed back a
//!   decoy of exactly that shape.
//! - **Kilo rewrites the files it reads.** On its first load of a `kilo.json` it adds a
//!   `"$schema"` line at the top and re-indents with two spaces. That touches nothing this
//!   adapter writes: the stale check of `apply` ([`InstallError::Stale`]) and the consent
//!   screen's digest are both about the location `mcp.handoff`, so a file Kilo rewrote between
//!   the screen and `apply` is still written, with Kilo's line kept and its indentation used.
//!   What cannot hold once a real Kilo has loaded the file is byte identity with what this
//!   adapter wrote, which is why the golden files are files no Kilo loaded.
//!
//! # Which file
//!
//! Kilo reads `config.json`, `kilo.json` and `kilo.jsonc` of its configuration folder and
//! merges them, in that order (T-080 read the list in its own log). This adapter writes
//! **`kilo.json`**, and never the `.jsonc`, which is where the extension keeps its own
//! settings and where comments are allowed: a JSON parse would lose them, which is the one
//! thing INST-04 forbids. A comment in `kilo.json` itself makes the file one this adapter
//! refuses rather than rewrites ([`InstallError::Malformed`]), the rule every adapter keeps for
//! a file it cannot read. The folder is `$XDG_CONFIG_HOME/kilo`, else `~/.config/kilo` — on
//! Windows too, which is where `kilo debug paths` put it.
//!
//! **Project scope** writes `<project>/kilo.json`, which Kilo reads and merges over the global
//! configuration with no trust step (measured with `kilo mcp list` from an untrusted folder).
//!
//! # Finding Kilo Code
//!
//! Either surface alone is Kilo Code on this machine: the CLI on `PATH`, its configuration
//! folder, or the VS Code extension's folder under `~/.vscode/extensions` — the extension
//! carries the whole CLI in its `bin` folder, so a machine with the extension and no CLI still
//! runs everything this entry declares.
//!
//! # Recognising ours
//!
//! By the fixed path (§7.15): the first element of `command` names a file called
//! `handoff-mcp`, tested with [`fixed_path::is_ours`] as for the other adapters, wherever the
//! bundle has been moved since (FM-23).

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
pub const AGENT_ID: &str = "kilo-code";

/// The key our entry has under `mcp` (§7.15).
pub const SERVER_KEY: &str = "handoff";

/// Where our entry is, from the root of the file.
pub const ENTRY_PATH: [&str; 2] = ["mcp", SERVER_KEY];

/// The variable Kilo resolves its configuration folder from, as XDG prescribes.
pub const ENV_XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";

/// The catalogue key of the one line of the consent screen.
pub const KEY_MCP_ENTRY: &str = "install.kiloCode.mcpEntry";

/// The folder Kilo keeps its configuration in, under the XDG configuration folder.
const KILO_FOLDER: &str = "kilo";

/// The configuration file this adapter writes, in that folder or in a project.
const CONFIG_FILE: &str = "kilo.json";

/// The start of the folder name VS Code gives Kilo Code's extension, followed by its version
/// and platform (`kilocode.kilo-code-7.6.2-win32-x64`).
const EXTENSION_FOLDER_PREFIX: &str = "kilocode.kilo-code-";

/// The Kilo Code adapter.
///
/// Its paths are fields for the reason the other adapters' are: a golden file points the whole
/// adapter at a temporary folder, and what it writes there is what it writes on a real machine.
#[derive(Debug, Clone)]
pub struct KiloCode {
    /// Kilo's configuration folder: `$XDG_CONFIG_HOME/kilo`, else `~/.config/kilo`.
    config_dir: PathBuf,
    /// VS Code's extensions folder, `~/.vscode/extensions`, where the extension surface shows.
    extensions_dir: PathBuf,
    /// The fixed launcher path written into the configuration (SRV-25).
    server: PathBuf,
    /// `~/.handoff/channel.token`, created by the first `apply` (INST-07, SRV-07).
    token_path: PathBuf,
}

impl KiloCode {
    /// The adapter for this machine and this build.
    ///
    /// # Errors
    ///
    /// [`InstallError::NoServer`] when the bundled server cannot be located.
    pub fn detected() -> Result<Self> {
        let home = paths::home_dir();
        Ok(Self {
            config_dir: config_dir_from(std::env::var_os(ENV_XDG_CONFIG_HOME), &home),
            extensions_dir: home.join(".vscode").join("extensions"),
            server: fixed_path::server_path()?,
            token_path: paths::token_path(),
        })
    }

    /// An adapter over explicit paths, for the golden files.
    #[must_use]
    pub fn with(
        config_dir: impl Into<PathBuf>,
        extensions_dir: impl Into<PathBuf>,
        server: impl Into<PathBuf>,
        token: impl Into<PathBuf>,
    ) -> Self {
        Self {
            config_dir: config_dir.into(),
            extensions_dir: extensions_dir.into(),
            server: server.into(),
            token_path: token.into(),
        }
    }

    /// The path this adapter registers.
    #[must_use]
    pub fn server(&self) -> &Path {
        &self.server
    }

    /// The file holding `mcp`: `kilo.json` in Kilo's configuration folder, or in the project.
    #[must_use]
    pub fn config_file(&self, scope: &Scope) -> PathBuf {
        match scope {
            Scope::User => self.config_dir.join(CONFIG_FILE),
            Scope::Project { path } => path.join(CONFIG_FILE),
        }
    }

    /// Our entry, in the order a person reads it: what kind of server, what runs, with what,
    /// for how long.
    ///
    /// The command is the fixed path alone, as for the other adapters: it is the whole command
    /// (SRV-19), and Kilo takes a command and its arguments as one array.
    #[must_use]
    pub fn mcp_entry(&self) -> Value {
        json!({
            "type": "local",
            "command": [self.server.display().to_string()],
            "environment": {
                ENV_HANDOFF_AGENT: AGENT_ID,
                ENV_HANDOFF_TOOL_TIMEOUT_MS: RAISED_TOOL_TIMEOUT_MS.to_string(),
            },
            "timeout": RAISED_TOOL_TIMEOUT_MS,
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

/// `$XDG_CONFIG_HOME/kilo`, else `~/.config/kilo`: where Kilo looks for its own configuration,
/// on every platform.
///
/// Blank counts as unset, the rule [`crate::paths`] applies to our own variables. A value that
/// is not Unicode counts as unset too: it is not a path a consent screen could name.
#[must_use]
pub fn config_dir_from(variable: Option<OsString>, home: &Path) -> PathBuf {
    variable
        .and_then(|value| value.into_string().ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map_or_else(|| home.join(".config"), PathBuf::from)
        .join(KILO_FOLDER)
}

/// Whether `extensions_dir` holds a folder of Kilo Code's VS Code extension, whatever its
/// version: the extension surface, found without spawning VS Code.
#[must_use]
pub fn extension_installed(extensions_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(extensions_dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.path().is_dir()
            && entry.file_name().to_str().is_some_and(|name| {
                name.to_ascii_lowercase()
                    .starts_with(EXTENSION_FOLDER_PREFIX)
            })
    })
}

/// The program of the entry under our key, whoever wrote it: the first element of `command`.
fn command_of(document: &Document) -> Option<&str> {
    document
        .get_at(&ENTRY_PATH)
        .and_then(|entry| entry.get("command"))
        .and_then(Value::as_array)
        .and_then(|command| command.first())
        .and_then(Value::as_str)
}

/// The name of a file, as the consent screen names it.
fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?")
        .to_owned()
}

/// Whether `kilo` is on `PATH`.
///
/// No process is spawned, for the reason the other adapters give: INST-05 runs the scan at
/// every launch. An npm install leaves the `.cmd` shim and an extensionless script beside it; a
/// native install puts `kilo.exe` there.
fn kilo_on_path() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let names: &[&str] = if cfg!(windows) {
        &["kilo.exe", "kilo.cmd", "kilo.bat", "kilo"]
    } else {
        &["kilo"]
    };
    std::env::split_paths(&path).any(|folder| names.iter().any(|name| folder.join(name).is_file()))
}

impl InstallAdapter for KiloCode {
    fn agent_id(&self) -> &'static str {
        AGENT_ID
    }

    fn detect(&self, scope: &Scope) -> Detection {
        let file = self.config_file(scope);
        // Kilo's configuration folder counts as well as the file: Kilo creates the folder the
        // first time it runs, before anybody has written a `kilo.json`. And the extension alone
        // is Kilo Code on this machine: it carries the whole CLI.
        let found = kilo_on_path()
            || self.config_dir.is_dir()
            || file.exists()
            || extension_installed(&self.extensions_dir);
        Detection {
            agent_id: AGENT_ID,
            found,
            config_files: vec![file],
            // Not read, for the reason `kilo_on_path` gives.
            version: None,
        }
    }

    fn plan(&self, scope: &Scope) -> Result<Vec<Modification>> {
        let file = self.config_file(scope);
        let document = Document::read(&file)?;
        if document
            .get_at(&ENTRY_PATH[..1])
            .is_some_and(|mcp| !mcp.is_object())
        {
            // Writing our entry would mean replacing the value the user has there.
            return Err(InstallError::not_editable(
                &file,
                "`mcp` is not an object of servers, the one shape this adapter adds to",
            ));
        }

        let server = self.server.display().to_string();
        let minutes = (RAISED_TOOL_TIMEOUT_MS / 60_000).to_string();
        Ok(vec![Modification::new(
            &file,
            ENTRY_PATH.to_vec(),
            Description::new(KEY_MCP_ENTRY)
                .with("file", file_name(&file))
                .with("server", server)
                .with("minutes", minutes),
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

    fn adapter() -> KiloCode {
        KiloCode::with(
            "/home/someone/.config/kilo",
            "/home/someone/.vscode/extensions",
            "/apps/Baton/handoff-mcp",
            "/home/someone/.handoff/channel.token",
        )
    }

    #[test]
    fn the_entry_is_the_one_kilo_read_back() {
        // T-080: `kilo debug config` printed a decoy of this shape back as written, and T-081's
        // preflight asks it about the golden file: a local server, the fixed path as the whole
        // command, our two variables under `environment`, and thirty minutes in milliseconds.
        assert_eq!(
            super::super::json::render_with_indent(&adapter().mcp_entry(), "  "),
            concat!(
                "{\n",
                "  \"type\": \"local\",\n",
                "  \"command\": [\n",
                "    \"/apps/Baton/handoff-mcp\"\n",
                "  ],\n",
                "  \"environment\": {\n",
                "    \"HANDOFF_AGENT\": \"kilo-code\",\n",
                "    \"HANDOFF_TOOL_TIMEOUT_MS\": \"1800000\"\n",
                "  },\n",
                "  \"timeout\": 1800000\n",
                "}",
            )
        );
    }

    #[test]
    fn the_entry_names_no_provider_no_model_and_no_permission() {
        // The owner's requirement (T-080): Baton must work whatever provider and model a user
        // runs Kilo with, so the entry says nothing about either; and no approval is asked
        // before a call, so it carries no `permission` key.
        let entry = adapter().mcp_entry();
        let keys: Vec<&str> = entry
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["type", "command", "environment", "timeout"]);
    }

    #[test]
    fn the_timeout_is_the_installers_thirty_minutes_in_milliseconds() {
        assert_eq!(
            adapter().mcp_entry()["timeout"],
            json!(RAISED_TOOL_TIMEOUT_MS)
        );
        assert_eq!(RAISED_TOOL_TIMEOUT_MS, 1_800_000);
    }

    #[test]
    fn a_windows_path_survives_the_json_it_is_written_into() {
        // JSON escapes every backslash, and Kilo reads the array back as the path.
        let adapter = KiloCode::with(
            "C:\\Users\\someone\\.config\\kilo",
            "C:\\Users\\someone\\.vscode\\extensions",
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
            parsed["command"][0],
            json!("C:\\Users\\someone\\AppData\\Local\\Baton\\handoff-mcp.exe")
        );
    }

    #[test]
    fn the_file_of_each_scope() {
        let adapter = adapter();
        assert_eq!(
            adapter.config_file(&Scope::User),
            PathBuf::from("/home/someone/.config/kilo/kilo.json")
        );
        assert_eq!(
            adapter.config_file(&Scope::project("/work/project")),
            PathBuf::from("/work/project/kilo.json")
        );
    }

    #[test]
    fn the_folder_follows_the_variable_kilo_reads() {
        let home = Path::new("/home/someone");
        assert_eq!(
            config_dir_from(None, home),
            home.join(".config").join("kilo")
        );
        assert_eq!(
            config_dir_from(Some(OsString::from("/elsewhere/config")), home),
            PathBuf::from("/elsewhere/config").join("kilo")
        );
        // Blank is unset, as for our own variables.
        assert_eq!(
            config_dir_from(Some(OsString::from("   ")), home),
            home.join(".config").join("kilo")
        );
    }

    #[test]
    fn the_extension_is_found_by_its_folder_whatever_its_version() {
        let root = std::env::temp_dir().join(format!(
            "handoff-kilo-extensions-{}-{}",
            std::process::id(),
            crate::ids::new_session_ref()
        ));
        std::fs::create_dir_all(root.join("ms-python.python-2026.1.0")).expect("created");
        assert!(!extension_installed(&root));
        // A file of that name is not the extension; only its folder is.
        std::fs::write(root.join("kilocode.kilo-code-7.6.2.vsix"), b"").expect("written");
        assert!(!extension_installed(&root));
        std::fs::create_dir_all(root.join("kilocode.kilo-code-7.6.2-win32-x64")).expect("created");
        assert!(extension_installed(&root));
        assert!(!extension_installed(&root.join("missing")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_names_it_writes_survive_project_scope() {
        // Kilo strips nothing that was measured (T-081: a TOKEN-named variable arrived), but the
        // rule of every adapter costs nothing and the failure it prevents is invisible.
        let entry = adapter().mcp_entry();
        let environment = entry["environment"]
            .as_object()
            .expect("an environment object");
        for name in environment.keys() {
            assert!(super::super::survives_project_scope(name), "{name}");
        }
    }
}
