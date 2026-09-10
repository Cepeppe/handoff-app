//! The Codex installation adapter (§7.15, INST-01..08, ADPT-04, ADPT-06 item 1).
//!
//! One modification, one file, and the rule the Claude Code adapter keeps underneath it:
//! whatever is already in the file is still there afterwards, in the same order, spelled the
//! same way (INST-04). Codex's configuration is TOML that people write by hand and comment, so
//! the file is edited through [`super::toml`] rather than parsed and written out again.
//!
//! | File | Location | What |
//! |---|---|---|
//! | `~/.codex/config.toml` (under `$CODEX_HOME` when it is set; `.codex/config.toml` in a project) | `[mcp_servers.handoff]` | the stdio entry with the fixed launcher path, our two environment variables, the approval mode and the per-server `tool_timeout_sec` |
//!
//! Three facts measured against Codex 0.153.4 decide that shape (T-066, and
//! `docs/agent-facts.md` of `handoff-mcp`):
//!
//! - **There is no hook to register.** `codex exec` runs no end-of-turn hook in any placement,
//!   so the capability row says `stop_hook: false` and this adapter makes one modification
//!   where Claude Code's makes three. The consent screen has one line.
//! - **The approval mode is part of the entry.** Codex asks before every call to a tool that
//!   is not annotated read-only, and `codex exec` answers its own question with a refusal, so
//!   `handoff_to_user` and `handoff_verify` would never run. `default_tools_approval_mode =
//!   "approve"` lifts that for this server's tools and no other, and the consent line says so
//!   in words: it is a permission, and INST-01 is about showing what is granted.
//! - **The timeout is in seconds.** `tool_timeout_sec = 1800` is the thirty minutes of INST-03;
//!   `HANDOFF_TOOL_TIMEOUT_MS` mirrors it in milliseconds so the server heartbeats a minute
//!   before it (§5.6).
//!
//! **Project scope** writes `<project>/.codex/config.toml`. Codex reads that file only for a
//! project it trusts — measured with `codex mcp list` from inside the folder: an untrusted
//! project's servers are not listed, a trusted one's are — and the trust is the user's to give,
//! in Codex. Nothing here writes it.
//!
//! # Recognising ours
//!
//! By the fixed path in `command` (§7.15), with the test the Claude Code adapter uses,
//! [`fixed_path::is_ours`]: the entry names a file called `handoff-mcp`, wherever the bundle
//! has been moved since (FM-23).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use toml_edit::{Array, InlineTable, Item, Table};

use crate::channel::token;
use crate::paths;

use super::error::{InstallError, Result};
use super::fixed_path;
use super::toml::Document;
use super::{
    Description, Detection, InstallAdapter, Modification, Registration, Scope, ENV_HANDOFF_AGENT,
    ENV_HANDOFF_TOOL_TIMEOUT_MS, RAISED_TOOL_TIMEOUT_MS,
};

/// The agent id, shared with the server's capability table (INST-08, §5.6).
pub const AGENT_ID: &str = "codex";

/// The key our entry has under `mcp_servers` (§7.15).
pub const SERVER_KEY: &str = "handoff";

/// Where our entry is, from the root of the file.
pub const ENTRY_PATH: [&str; 2] = ["mcp_servers", SERVER_KEY];

/// `default_tools_approval_mode` of our entry: Baton's tools run without Codex asking each
/// time, and without it `codex exec` refuses them outright.
pub const APPROVAL_MODE: &str = "approve";

/// The variable Codex reads its home folder from.
pub const ENV_CODEX_HOME: &str = "CODEX_HOME";

/// The catalogue key of the one line of the consent screen.
pub const KEY_MCP_ENTRY: &str = "install.codex.mcpEntry";

/// The folder Codex keeps its configuration in, under the home or under a project.
const CODEX_FOLDER: &str = ".codex";

/// The configuration file inside it.
const CONFIG_FILE: &str = "config.toml";

/// The Codex adapter.
///
/// Its paths are fields for the reason the Claude Code adapter's are: a golden file points the
/// whole adapter at a temporary folder, and what it writes there is what it writes on a real
/// machine.
#[derive(Debug, Clone)]
pub struct Codex {
    /// Codex's home folder: `$CODEX_HOME`, else `~/.codex`.
    codex_home: PathBuf,
    /// The fixed launcher path written into the configuration (SRV-25).
    server: PathBuf,
    /// `~/.handoff/channel.token`, created by the first `apply` (INST-07, SRV-07).
    token_path: PathBuf,
}

impl Codex {
    /// The adapter for this machine and this build.
    ///
    /// # Errors
    ///
    /// [`InstallError::NoServer`] when the bundled server cannot be located.
    pub fn detected() -> Result<Self> {
        Ok(Self {
            codex_home: codex_home_from(std::env::var_os(ENV_CODEX_HOME), &paths::home_dir()),
            server: fixed_path::server_path()?,
            token_path: paths::token_path(),
        })
    }

    /// An adapter over explicit paths, for the golden files.
    #[must_use]
    pub fn with(
        codex_home: impl Into<PathBuf>,
        server: impl Into<PathBuf>,
        token: impl Into<PathBuf>,
    ) -> Self {
        Self {
            codex_home: codex_home.into(),
            server: server.into(),
            token_path: token.into(),
        }
    }

    /// The path this adapter registers.
    #[must_use]
    pub fn server(&self) -> &Path {
        &self.server
    }

    /// The file holding `mcp_servers`: `config.toml` in Codex's home, or in the project's
    /// `.codex/`.
    #[must_use]
    pub fn config_file(&self, scope: &Scope) -> PathBuf {
        match scope {
            Scope::User => self.codex_home.join(CONFIG_FILE),
            Scope::Project { path } => path.join(CODEX_FOLDER).join(CONFIG_FILE),
        }
    }

    /// Our entry, in the order a person reads it: what runs, with what, under which
    /// permission, for how long.
    ///
    /// `args` is written empty rather than left out, as it is for Claude Code: the fixed path
    /// is the whole command (SRV-19), and an explicit `[]` says so to whoever reads the file.
    #[must_use]
    pub fn mcp_entry(&self) -> Table {
        let mut env = InlineTable::new();
        env.insert(ENV_HANDOFF_AGENT, AGENT_ID.into());
        env.insert(
            ENV_HANDOFF_TOOL_TIMEOUT_MS,
            RAISED_TOOL_TIMEOUT_MS.to_string().into(),
        );

        let mut entry = Table::new();
        entry.insert(
            "command",
            toml_edit::value(self.server.display().to_string()),
        );
        entry.insert("args", toml_edit::value(Array::new()));
        entry.insert("env", toml_edit::value(env));
        entry.insert(
            "default_tools_approval_mode",
            toml_edit::value(APPROVAL_MODE),
        );
        entry.insert("tool_timeout_sec", toml_edit::value(tool_timeout_seconds()));
        entry
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

/// `$CODEX_HOME`, else `~/.codex`: Codex's own rule for where it keeps its configuration.
///
/// Blank counts as unset, the rule [`crate::paths`] applies to our own variables. A value that
/// is not Unicode counts as unset too: it is not a path a consent screen could name.
#[must_use]
pub fn codex_home_from(variable: Option<OsString>, home: &Path) -> PathBuf {
    variable
        .and_then(|value| value.into_string().ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map_or_else(|| home.join(CODEX_FOLDER), PathBuf::from)
}

/// `tool_timeout_sec`: the thirty minutes of INST-03, in the seconds Codex counts in.
fn tool_timeout_seconds() -> i64 {
    i64::try_from(RAISED_TOOL_TIMEOUT_MS / 1000).expect("thirty minutes fit in an i64")
}

/// The `command` of the entry under our key, whoever wrote it.
fn command_of(document: &Document) -> Option<&str> {
    document
        .get_at(&ENTRY_PATH)
        .and_then(Item::as_table_like)
        .and_then(|entry| entry.get("command"))
        .and_then(Item::as_str)
}

/// The name of a file, as the consent screen names it.
fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?")
        .to_owned()
}

/// Whether `codex` is on `PATH`.
///
/// No process is spawned, for the reason the Claude Code adapter gives: INST-05 runs the scan
/// at every launch. The native installer puts `codex.exe` there; an npm install leaves the
/// `.cmd` shim and an extensionless script beside it.
fn codex_on_path() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let names: &[&str] = if cfg!(windows) {
        &["codex.exe", "codex.cmd", "codex.bat", "codex"]
    } else {
        &["codex"]
    };
    std::env::split_paths(&path).any(|folder| names.iter().any(|name| folder.join(name).is_file()))
}

impl InstallAdapter for Codex {
    fn agent_id(&self) -> &'static str {
        AGENT_ID
    }

    fn detect(&self, scope: &Scope) -> Detection {
        let file = self.config_file(scope);
        // Codex's home folder counts as well as the file: Codex creates it at the first
        // login, before anybody has written a `config.toml`.
        let found = codex_on_path() || self.codex_home.is_dir() || file.exists();
        Detection {
            agent_id: AGENT_ID,
            found,
            config_files: vec![file],
            // Not read, for the reason `codex_on_path` gives.
            version: None,
        }
    }

    fn plan(&self, scope: &Scope) -> Result<Vec<Modification>> {
        let file = self.config_file(scope);
        let document = Document::read(&file)?;
        if !document.is_section_or_absent(&ENTRY_PATH[..1]) {
            return Err(InstallError::not_editable(
                &file,
                "`mcp_servers` is not written as `[mcp_servers.<name>]` sections, \
                 the one shape this adapter adds to",
            ));
        }

        let server = self.server.display().to_string();
        let minutes = (RAISED_TOOL_TIMEOUT_MS / 60_000).to_string();
        Ok(vec![Modification::toml(
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

    fn adapter() -> Codex {
        Codex::with(
            "/home/someone/.codex",
            "/apps/Baton/handoff-mcp",
            "/home/someone/.handoff/channel.token",
        )
    }

    #[test]
    fn the_entry_is_the_one_codex_was_measured_with() {
        // The five keys T-066 measured and `codex mcp get` read back: the fixed path, no
        // arguments, our two variables, the approval mode, and thirty minutes in seconds.
        assert_eq!(
            super::super::toml::render_at(&ENTRY_PATH, &Item::Table(adapter().mcp_entry())),
            concat!(
                "[mcp_servers.handoff]\n",
                "command = \"/apps/Baton/handoff-mcp\"\n",
                "args = []\n",
                "env = { HANDOFF_AGENT = \"codex\", HANDOFF_TOOL_TIMEOUT_MS = \"1800000\" }\n",
                "default_tools_approval_mode = \"approve\"\n",
                "tool_timeout_sec = 1800\n",
            )
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_path_is_written_so_that_codex_reads_its_backslashes() {
        // A literal string keeps every backslash as written, and is what `codex mcp get` read
        // back as the path (T-067's probe). `docs/consent-screen.md` prints this spelling.
        let adapter = Codex::with(
            "C:\\Users\\someone\\.codex",
            "C:\\Users\\someone\\AppData\\Local\\Baton\\handoff-mcp.exe",
            "C:\\Users\\someone\\.handoff\\channel.token",
        );
        let rendered =
            super::super::toml::render_at(&ENTRY_PATH, &Item::Table(adapter.mcp_entry()));
        assert!(
            rendered.contains(
                "command = 'C:\\Users\\someone\\AppData\\Local\\Baton\\handoff-mcp.exe'\n"
            ),
            "{rendered}"
        );
    }

    #[test]
    fn the_timeout_is_the_installers_thirty_minutes_in_seconds() {
        assert_eq!(tool_timeout_seconds() * 1000, 1_800_000);
    }

    #[test]
    fn the_file_of_each_scope() {
        let adapter = adapter();
        assert_eq!(
            adapter.config_file(&Scope::User),
            PathBuf::from("/home/someone/.codex/config.toml")
        );
        assert_eq!(
            adapter.config_file(&Scope::project("/work/project")),
            PathBuf::from("/work/project/.codex/config.toml")
        );
    }

    #[test]
    fn codex_home_follows_the_variable_codex_reads() {
        let home = Path::new("/home/someone");
        assert_eq!(codex_home_from(None, home), home.join(".codex"));
        assert_eq!(
            codex_home_from(Some(OsString::from("/elsewhere/codex")), home),
            PathBuf::from("/elsewhere/codex")
        );
        // Blank is unset, as for our own variables.
        assert_eq!(
            codex_home_from(Some(OsString::from("   ")), home),
            home.join(".codex")
        );
        assert_eq!(
            codex_home_from(Some(OsString::from(" /elsewhere/codex ")), home),
            PathBuf::from("/elsewhere/codex")
        );
    }

    #[test]
    fn the_names_it_writes_survive_project_scope() {
        // Codex strips nothing that was measured (T-066: a TOKEN-named variable arrived), but
        // the rule of every adapter costs nothing and the failure it prevents is invisible.
        let entry = adapter().mcp_entry();
        let env = entry
            .get("env")
            .and_then(Item::as_inline_table)
            .expect("an env table");
        for (name, _) in env.iter() {
            assert!(super::super::survives_project_scope(name), "{name}");
        }
    }
}
