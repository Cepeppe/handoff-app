//! Golden files for the installation adapters (T-039, §7.15, INST-01..08).
//!
//! Every case here is a whole home folder: `tests/fixtures/install/<case>/in/` is copied
//! into a temporary directory, the adapter runs against it, and every file under
//! `<case>/out/` is compared **byte for byte** with what came out. That is the only form of
//! test that can hold the promises this task makes, because all of them are about bytes: the
//! key order of a file somebody else owns, the indentation it was written with, the hooks
//! the user already had, and `env.MCP_TOOL_TIMEOUT`, which the adapter must not touch at all
//! (T-026, Option B).
//!
//! The fixtures are written by hand and the code has to match them, never the other way
//! round. There is no "update the goldens" switch on purpose: a golden regenerated from the
//! implementation proves that the implementation equals itself.
//!
//! The server path in every fixture is the POSIX-looking `/apps/Baton/handoff-mcp`. It is
//! synthetic and it is spelled the same on both platforms — `Path::display` prints what it
//! was given — so one set of goldens covers Windows and macOS.

use std::fs;
use std::path::{Path, PathBuf};

use handoff_app_lib::install::claude_code::{ClaudeCode, AGENT_ID};
use handoff_app_lib::install::{
    self, survives_project_scope, Codex, Copilot, Cursor, InstallAdapter, InstallError,
    Modification, OpenCode, Registration, Scope,
};
use serde_json::Value;

/// The fixed launcher path every fixture registers.
const SERVER: &str = "/apps/Baton/handoff-mcp";

/// The path the fixtures of the "moved bundle" case still carry (FM-23).
const OLD_SERVER: &str = "/old/Baton/handoff-mcp";

// --------------------------------------------------------------------------- harness

/// A temporary home folder, removed when the guard is dropped.
///
/// The removal is best-effort: on Windows a file another process still holds cannot be
/// deleted, and a test that panicked over the real assertion should not then fail over its
/// own cleanup (the trap T-034 recorded).
struct TempHome(PathBuf);

impl TempHome {
    /// A copy of `fixtures/install/<case>/in/`, or an empty folder when there is none.
    fn from_case(case: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "handoff-install-{case}-{}-{}",
            std::process::id(),
            handoff_app_lib::ids::new_session_ref()
        ));
        fs::create_dir_all(&root).expect("the temporary home is created");
        let input = fixture_dir(case).join("in");
        if input.is_dir() {
            copy_tree(&input, &root);
        }
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn join(&self, relative: &str) -> PathBuf {
        let mut path = self.0.clone();
        for segment in relative.split('/') {
            path.push(segment);
        }
        path
    }

    /// The adapter under test: this home, the fixed server path, a token inside the home.
    fn adapter(&self) -> ClaudeCode {
        ClaudeCode::with(&self.0, SERVER, self.join(".handoff/channel.token"))
    }

    /// The Codex adapter over the same home: Codex's own folder is `<home>/.codex`.
    fn codex(&self) -> Codex {
        Codex::with(
            self.join(".codex"),
            SERVER,
            self.join(".handoff/channel.token"),
        )
    }

    /// The OpenCode adapter over the same home: its folder is `<home>/.config/opencode`.
    fn opencode(&self) -> OpenCode {
        OpenCode::with(
            self.join(".config/opencode"),
            SERVER,
            self.join(".handoff/channel.token"),
        )
    }

    /// The Cursor adapter over the same home: its folder is `<home>/.cursor`.
    fn cursor(&self) -> Cursor {
        Cursor::with(
            self.join(".cursor"),
            SERVER,
            self.join(".handoff/channel.token"),
        )
    }

    /// The GitHub Copilot adapter over the same home: the CLI's folder is `<home>/.copilot`,
    /// and VS Code's user folder is where Windows keeps it, `<home>/AppData/Roaming/Code/User`.
    fn copilot(&self) -> Copilot {
        Copilot::with(
            self.join(".copilot"),
            self.join("AppData/Roaming/Code/User"),
            SERVER,
            self.join(".handoff/channel.token"),
        )
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `tests/fixtures/install/<case>/`.
fn fixture_dir(case: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("install")
        .join(case)
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("the folder is created");
    for entry in fs::read_dir(from).expect("the fixture folder is readable") {
        let entry = entry.expect("the entry is readable");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("the fixture is copied");
        }
    }
}

/// The relative paths of every file under `dir`, sorted, with `/` as the separator.
fn files_under(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if entry.path().is_dir() {
                walk(&entry.path(), &relative, out);
            } else {
                out.push(relative);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out);
    out.sort();
    out
}

/// Compares the home against `fixtures/install/<case>/<side>/`, file by file, byte for byte.
fn assert_matches(home: &TempHome, case: &str, side: &str) {
    let expected_root = fixture_dir(case).join(side);
    let expected_files = files_under(&expected_root);
    assert!(
        !expected_files.is_empty(),
        "{} holds no golden files",
        expected_root.display()
    );

    for relative in expected_files {
        let expected = fs::read_to_string(expected_root.join(&relative))
            .unwrap_or_else(|error| panic!("the golden {relative} could not be read: {error}"))
            // `.gitattributes` checks these out with LF; a checkout that ignored it should
            // fail on the content, not on the line endings.
            .replace("\r\n", "\n");
        let actual = fs::read_to_string(home.join(&relative))
            .unwrap_or_else(|error| panic!("{relative} was not written: {error}"));
        assert_eq!(actual, expected, "{case}/{side}: {relative} differs");
    }
}

/// Runs the whole install: plan, then apply.
fn install(adapter: &impl InstallAdapter, scope: &Scope) -> Vec<Modification> {
    let plan = adapter.plan(scope).expect("the plan is made");
    adapter.apply(&plan).expect("the plan is applied");
    plan
}

/// The backups `apply` left beside `relative`.
fn backups_of(home: &TempHome, relative: &str) -> Vec<String> {
    let path = home.join(relative);
    let folder = path.parent().expect("the file has a folder").to_path_buf();
    let stem = format!(
        "{}.handoff-backup-",
        path.file_name()
            .and_then(|name| name.to_str())
            .expect("the file has a name")
    );
    let mut found: Vec<String> = fs::read_dir(&folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with(&stem))
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

// ------------------------------------------------------------------- the golden cases

#[test]
fn apply_on_a_machine_with_no_configuration_at_all() {
    let home = TempHome::from_case("empty");
    let plan = install(&home.adapter(), &Scope::User);

    // INST-02: three modifications, and on this machine all three really change something.
    assert_eq!(plan.len(), 3);
    assert!(plan.iter().all(|change| !change.is_noop()));
    assert!(plan.iter().all(|change| change.before.is_none()));

    assert_matches(&home, "empty", "out");

    // INST-07: the token file is created by the first apply.
    let token = home.join(".handoff/channel.token");
    assert!(token.is_file(), "the channel token was not created");
    assert_eq!(
        fs::read_to_string(&token).expect("it is readable").len(),
        64,
        "the token is not 64 hex characters"
    );

    // Nothing was there to back up.
    assert!(backups_of(&home, ".claude.json").is_empty());
}

#[test]
fn apply_on_a_machine_that_already_has_servers_hooks_and_an_env_block() {
    let home = TempHome::from_case("populated");
    install(&home.adapter(), &Scope::User);
    assert_matches(&home, "populated", "out");

    // §7.15: a backup per file that was edited, and only for files that existed.
    assert_eq!(backups_of(&home, ".claude.json").len(), 1);
    assert_eq!(backups_of(&home, ".claude/settings.json").len(), 1);
}

#[test]
fn the_existing_stop_hook_is_kept_and_ours_is_added_beside_it() {
    // INST-04 and A-21, read off the file rather than off the plan.
    let home = TempHome::from_case("populated");
    install(&home.adapter(), &Scope::User);

    let settings: Value = serde_json::from_str(
        &fs::read_to_string(home.join(".claude/settings.json")).expect("it was written"),
    )
    .expect("it is JSON");
    let stop = settings["hooks"]["Stop"]
        .as_array()
        .expect("Stop is an array");
    assert_eq!(stop.len(), 2, "the user's Stop hook was replaced");
    assert_eq!(stop[0]["hooks"][0]["command"], "./scripts/notify.sh");
    assert_eq!(
        stop[1]["hooks"][0]["command"],
        format!("\"{SERVER}\" hook stop")
    );
    // The other event the user had is untouched.
    assert_eq!(
        settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "./scripts/audit.sh"
    );
}

#[test]
fn a_foreign_mcp_tool_timeout_is_never_touched_and_never_mentioned() {
    // T-026, Option B: the global variable is not written, not restored, not removed, and
    // no line of the plan is about it.
    let home = TempHome::from_case("populated");
    let adapter = home.adapter();
    let plan = install(&adapter, &Scope::User);

    for change in &plan {
        assert!(
            !change.after.contains("MCP_TOOL_TIMEOUT"),
            "a modification writes MCP_TOOL_TIMEOUT"
        );
        assert!(
            !change.diff.contains("MCP_TOOL_TIMEOUT"),
            "a diff shown to the user mentions MCP_TOOL_TIMEOUT"
        );
        assert_ne!(change.path, vec!["env", "MCP_TOOL_TIMEOUT"]);
    }

    let after_apply = fs::read_to_string(home.join(".claude/settings.json")).expect("written");
    assert!(after_apply.contains("\"MCP_TOOL_TIMEOUT\": \"60000\""));

    adapter
        .uninstall(&Scope::User)
        .expect("the uninstall succeeds");
    let after_uninstall = fs::read_to_string(home.join(".claude/settings.json")).expect("written");
    assert!(after_uninstall.contains("\"MCP_TOOL_TIMEOUT\": \"60000\""));
}

#[test]
fn applying_twice_writes_nothing_the_second_time() {
    let home = TempHome::from_case("populated");
    let adapter = home.adapter();
    install(&adapter, &Scope::User);
    let after_first = fs::read_to_string(home.join(".claude.json")).expect("written");
    let settings_after_first =
        fs::read_to_string(home.join(".claude/settings.json")).expect("written");

    let second = adapter.plan(&Scope::User).expect("the plan is made");
    // Still three lines on the consent screen (INST-02), all of them no-ops now.
    assert_eq!(second.len(), 3);
    assert!(
        second.iter().all(Modification::is_noop),
        "a second plan still wants to change something"
    );

    adapter.apply(&second).expect("the plan is applied");
    assert_eq!(
        fs::read_to_string(home.join(".claude.json")).expect("written"),
        after_first
    );
    assert_eq!(
        fs::read_to_string(home.join(".claude/settings.json")).expect("written"),
        settings_after_first
    );
    // No second backup: a no-op plan does not touch the files at all.
    assert_eq!(backups_of(&home, ".claude.json").len(), 1);
    assert_eq!(backups_of(&home, ".claude/settings.json").len(), 1);
    assert_eq!(adapter.verify(&Scope::User), Registration::Registered);
}

#[test]
fn uninstall_leaves_the_files_exactly_as_they_were_found() {
    // INST-04, in its strongest form: install, uninstall, and the two files are the
    // fixture's own bytes again.
    let home = TempHome::from_case("populated");
    let adapter = home.adapter();
    install(&adapter, &Scope::User);
    adapter
        .uninstall(&Scope::User)
        .expect("the uninstall succeeds");

    assert_matches(&home, "populated", "in");
    assert_eq!(adapter.verify(&Scope::User), Registration::NotRegistered);
}

#[test]
fn uninstall_on_a_machine_where_we_were_the_only_entry_removes_our_keys_entirely() {
    let home = TempHome::from_case("empty");
    let adapter = home.adapter();
    install(&adapter, &Scope::User);
    adapter
        .uninstall(&Scope::User)
        .expect("the uninstall succeeds");

    // `hooks` and `mcpServers` were ours to create, so they go with our entries: what is
    // left is the empty object, not a shell of keys the user has to delete by hand.
    assert_eq!(
        fs::read_to_string(home.join(".claude.json")).expect("written"),
        "{}\n"
    );
    assert_eq!(
        fs::read_to_string(home.join(".claude/settings.json")).expect("written"),
        "{}\n"
    );
}

#[test]
fn project_scope_writes_the_project_files_and_not_the_home() {
    let home = TempHome::from_case("project");
    let project = home.path().to_path_buf();
    let adapter = ClaudeCode::with(
        home.path().join("elsewhere"),
        SERVER,
        home.join(".handoff/channel.token"),
    );
    let scope = Scope::project(&project);

    let plan = install(&adapter, &scope);
    assert_eq!(plan[0].file, project.join(".mcp.json"));
    assert_eq!(plan[1].file, project.join(".claude").join("settings.json"));

    assert_matches(&home, "project", "out");
    assert!(
        !home.join("elsewhere/.claude.json").exists(),
        "project scope wrote into the home folder"
    );
    assert_eq!(adapter.verify(&scope), Registration::Registered);
}

#[test]
fn a_moved_bundle_is_reported_as_a_path_mismatch_and_repaired_by_a_plan() {
    // FM-23: the registered path is ours and is not where we are.
    //
    // The fixture writes its hook commands **unquoted**, the spelling every machine
    // installed before the quoting fix of `fixed_path::hook_command` carries. Recognising
    // that spelling is what makes those machines repairable rather than orphaned, so this
    // case is the migration path as well as the moved bundle.
    let home = TempHome::from_case("moved");
    let adapter = home.adapter();

    assert_eq!(
        adapter.verify(&Scope::User),
        Registration::PathMismatch {
            registered: PathBuf::from(OLD_SERVER),
            current: PathBuf::from(SERVER),
        }
    );

    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    assert!(
        plan.iter().all(|change| !change.is_noop()),
        "the repair plan has nothing to do"
    );
    for change in &plan {
        assert!(
            change.diff.contains(OLD_SERVER) && change.diff.contains(SERVER),
            "the diff does not show the old path being replaced"
        );
    }
    adapter.apply(&plan).expect("the repair is applied");

    assert_eq!(adapter.verify(&Scope::User), Registration::Registered);
    let settings = fs::read_to_string(home.join(".claude/settings.json")).expect("written");
    assert!(
        !settings.contains(OLD_SERVER),
        "a hook still runs the old path"
    );
    // The repair replaced our group where it was; it did not add a second one, and the
    // command it left behind is the quoted spelling.
    let parsed: Value = serde_json::from_str(&settings).expect("it is JSON");
    assert_eq!(parsed["hooks"]["Stop"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        parsed["hooks"]["Stop"][0]["hooks"][0]["command"],
        format!("\"{SERVER}\" hook stop")
    );
}

#[test]
fn a_half_installed_machine_is_partial_and_names_what_is_missing() {
    let home = TempHome::from_case("empty");
    let adapter = home.adapter();
    let plan = adapter.plan(&Scope::User).expect("the plan is made");

    assert_eq!(adapter.verify(&Scope::User), Registration::NotRegistered);

    // Only the MCP entry, as an interrupted install would leave it.
    adapter.apply(&plan[..1]).expect("the entry is written");
    match adapter.verify(&Scope::User) {
        Registration::Partial { missing } => assert_eq!(
            missing,
            vec![
                "settings.json · hooks.Stop".to_owned(),
                "settings.json · hooks.SubagentStop".to_owned(),
            ]
        ),
        other => panic!("expected Partial, got {other:?}"),
    }
}

#[test]
fn the_env_block_carries_no_name_claude_code_would_strip() {
    // A-23: a stripped variable is invisible — the server would silently fall back to
    // `clientInfo` for its identity and to the table for its timeout.
    let home = TempHome::from_case("empty");
    let plan = home.adapter().plan(&Scope::User).expect("the plan is made");
    let entry: Value = serde_json::from_str(&plan[0].after).expect("the entry is JSON");
    let env = entry["env"]
        .as_object()
        .expect("the entry has an env block");

    assert_eq!(env.len(), 2, "the env block gained a variable");
    for name in env.keys() {
        assert!(
            survives_project_scope(name),
            "{name} is stripped in project scope (A-23)"
        );
    }
    assert_eq!(env["HANDOFF_AGENT"], AGENT_ID);
    assert_eq!(env["HANDOFF_TOOL_TIMEOUT_MS"], "1800000");
}

#[test]
fn a_plan_applied_against_a_file_that_moved_on_is_refused() {
    // INST-01: what the user consented to is what gets written, or nothing does.
    let home = TempHome::from_case("populated");
    let adapter = home.adapter();
    let plan = adapter.plan(&Scope::User).expect("the plan is made");

    fs::write(
        home.join(".claude.json"),
        "{\n  \"mcpServers\": {\n    \"handoff\": { \"command\": \"somewhere-else\" }\n  }\n}\n",
    )
    .expect("the file is rewritten");

    match adapter.apply(&plan) {
        Err(InstallError::Stale { location, .. }) => {
            assert_eq!(location, ".claude.json · mcpServers.handoff");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_configuration_file_that_is_not_json_is_never_written_to() {
    let home = TempHome::from_case("populated");
    let adapter = home.adapter();
    let broken = "{ this is not json\n";
    fs::write(home.join(".claude.json"), broken).expect("the file is rewritten");

    assert!(matches!(
        adapter.plan(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert!(matches!(
        adapter.uninstall(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert_eq!(
        fs::read_to_string(home.join(".claude.json")).expect("it is still there"),
        broken
    );
}

#[test]
fn detect_finds_the_agent_by_its_configuration_and_names_the_files_of_the_scope() {
    let home = TempHome::from_case("populated");
    let adapter = home.adapter();
    let detection = adapter.detect(&Scope::User);

    assert_eq!(detection.agent_id, AGENT_ID);
    assert!(detection.found, "an existing ~/.claude.json is the agent");
    assert_eq!(
        detection.config_files,
        vec![
            home.join(".claude.json"),
            home.join(".claude/settings.json")
        ]
    );
    // INST-05 runs this at every launch: no subprocess, so no version.
    assert_eq!(detection.version, None);
}

// -------------------------------------------------------------- the consent path (T-040)

#[test]
fn the_consent_path_registers_exactly_what_it_showed_and_then_has_nothing_left_to_do() {
    // The whole of F-13's first half, as onboarding walks it: the screen asks for a plan,
    // shows three modifications on two lines, and applies the plan it showed. The files it
    // produces are the same goldens a bare `apply` produces — the consent screen adds a
    // question, not a second way of writing.
    let home = TempHome::from_case("empty");
    let adapter = home.adapter();

    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    let shown = install::digest(&plan);
    assert_eq!(plan.len(), 3, "INST-02 counts three modifications");

    let lines = adapter.consent_lines(&plan);
    assert_eq!(lines.len(), 2, "the two hooks are one line (INST-02)");
    assert!(lines.iter().all(|line| !line.is_noop));

    adapter.apply(&plan).expect("the plan is applied");
    assert_matches(&home, "empty", "out");

    // A second visit to the same screen: three modifications still, every one of them a
    // no-op, so the screen can say "already in order" and `apply` writes nothing at all.
    let again = adapter.plan(&Scope::User).expect("the plan is made again");
    assert_eq!(again.len(), 3);
    assert!(again.iter().all(Modification::is_noop));
    assert!(adapter
        .consent_lines(&again)
        .iter()
        .all(|line| line.is_noop));
    assert_ne!(
        install::digest(&again),
        shown,
        "a plan over a registered machine is not the plan over an empty one"
    );
}

#[test]
fn the_digest_of_a_plan_changes_the_moment_the_file_does() {
    // The other half of INST-01. `apply` refuses a stale plan, but the consent screen cannot
    // hand a plan back through a webview and be believed, so it hands back this fingerprint
    // and the command re-plans: a file that moved on hashes differently, and the screen asks
    // again rather than writing something nobody saw.
    let home = TempHome::from_case("populated");
    let adapter = home.adapter();
    let shown = install::digest(&adapter.plan(&Scope::User).expect("the plan is made"));

    // A key we do not touch is not part of what was consented to, and moving it leaves the
    // fingerprint alone: the screen would be asking again about a change the user never saw.
    fs::write(
        home.join(".claude.json"),
        "{\n  \"mcpServers\": {\n    \"other\": { \"command\": \"somewhere-else\" }\n  }\n}\n",
    )
    .expect("the file is rewritten");
    assert_eq!(
        install::digest(&adapter.plan(&Scope::User).expect("the plan is made again")),
        shown
    );

    // One of ours, on the other hand, is exactly what the screen showed.
    fs::write(
        home.join(".claude.json"),
        "{\n  \"mcpServers\": {\n    \"handoff\": { \"command\": \"somewhere-else\" }\n  }\n}\n",
    )
    .expect("the file is rewritten");
    assert_ne!(
        install::digest(&adapter.plan(&Scope::User).expect("the plan is made again")),
        shown
    );
}

#[test]
fn a_moved_bundle_is_named_by_the_launch_check_and_repaired_by_the_consent_path() {
    // FM-23 end to end: §7.2's `check_registered_paths` finds it, the repair is the ordinary
    // plan, and afterwards the registration is `Registered` again. The check itself reads the
    // *real* home, so what is exercised here is the adapter half of it, over the fixture.
    let home = TempHome::from_case("moved");
    let adapter = home.adapter();

    let Registration::PathMismatch {
        registered,
        current,
    } = adapter.verify(&Scope::User)
    else {
        panic!("the moved bundle was not reported as a path mismatch");
    };
    assert_eq!(registered, PathBuf::from(OLD_SERVER));
    assert_eq!(current, PathBuf::from(SERVER));

    let plan = adapter.plan(&Scope::User).expect("the repair plan is made");
    assert!(
        plan.iter().any(|change| !change.is_noop()),
        "a repair with nothing to repair"
    );
    adapter.apply(&plan).expect("the repair is applied");
    assert_eq!(adapter.verify(&Scope::User), Registration::Registered);
}

#[test]
fn a_four_space_settings_file_stays_a_four_space_settings_file() {
    // INST-04 as a user reads it: `git diff` after the install shows our hooks and nothing
    // else, because the file is written back with the indentation it was found with.
    let home = TempHome::from_case("empty");
    let settings = home.join(".claude/settings.json");
    fs::create_dir_all(settings.parent().expect("it has a folder")).expect("the folder is made");
    fs::write(
        &settings,
        "{\n    \"env\": {\n        \"A\": \"b\"\n    }\n}\n",
    )
    .expect("the file is written");

    install(&home.adapter(), &Scope::User);

    let written = fs::read_to_string(&settings).expect("written");
    assert!(
        written.starts_with("{\n    \"env\": {\n        \"A\": \"b\"\n    },\n    \"hooks\": {"),
        "the file was reindented:\n{written}"
    );
}

// ------------------------------------------------------------- the Codex adapter (T-067)
//
// The same promises over a TOML file: `<home>/.codex/config.toml`, one `[mcp_servers.handoff]`
// section, and every comment, server and profile the user had still there, spelled as they
// spelled it.

/// Codex's configuration inside a temporary home.
const CODEX_CONFIG: &str = ".codex/config.toml";

#[test]
fn codex_on_a_machine_with_no_configuration_at_all() {
    let home = TempHome::from_case("codex-empty");
    let plan = install(&home.codex(), &Scope::User);

    // One modification and no hook: `codex exec` runs none (T-066).
    assert_eq!(plan.len(), 1);
    assert!(plan[0].before.is_none() && !plan[0].is_noop());
    assert_matches(&home, "codex-empty", "out");

    // INST-07 holds for every adapter: the first apply creates the token.
    assert!(
        home.join(".handoff/channel.token").is_file(),
        "the channel token was not created"
    );
    assert!(backups_of(&home, CODEX_CONFIG).is_empty());
}

#[test]
fn codex_keeps_every_comment_server_and_profile_the_user_had() {
    let home = TempHome::from_case("codex-populated");
    install(&home.codex(), &Scope::User);
    assert_matches(&home, "codex-populated", "out");
    assert_eq!(backups_of(&home, CODEX_CONFIG).len(), 1);
}

#[test]
fn codex_uninstall_gives_the_file_back_byte_for_byte() {
    let home = TempHome::from_case("codex-populated");
    let adapter = home.codex();
    install(&adapter, &Scope::User);
    adapter
        .uninstall(&Scope::User)
        .expect("the uninstall succeeds");

    assert_matches(&home, "codex-populated", "in");
    assert_eq!(adapter.verify(&Scope::User), Registration::NotRegistered);
}

#[test]
fn codex_applying_twice_writes_nothing_the_second_time() {
    let home = TempHome::from_case("codex-populated");
    let adapter = home.codex();
    install(&adapter, &Scope::User);
    let after_first = fs::read_to_string(home.join(CODEX_CONFIG)).expect("written");

    let second = adapter.plan(&Scope::User).expect("the plan is made");
    assert_eq!(second.len(), 1);
    assert!(
        second[0].is_noop(),
        "a second plan still wants to change something:\n{}",
        second[0].diff
    );
    adapter.apply(&second).expect("the plan is applied");

    assert_eq!(
        fs::read_to_string(home.join(CODEX_CONFIG)).expect("written"),
        after_first
    );
    assert_eq!(backups_of(&home, CODEX_CONFIG).len(), 1);
    assert_eq!(adapter.verify(&Scope::User), Registration::Registered);
}

#[test]
fn codex_uninstall_where_ours_was_the_only_server_leaves_nothing_of_ours() {
    let home = TempHome::from_case("codex-empty");
    let adapter = home.codex();
    install(&adapter, &Scope::User);
    adapter
        .uninstall(&Scope::User)
        .expect("the uninstall succeeds");

    // `mcp_servers` was ours to create and had no header of its own, so it goes with our
    // section: an empty file is Codex's "no configuration".
    assert_eq!(
        fs::read_to_string(home.join(CODEX_CONFIG)).expect("written"),
        ""
    );
}

#[test]
fn codex_project_scope_writes_the_projects_file_and_not_codexs_home() {
    let home = TempHome::from_case("codex-project");
    let project = home.path().to_path_buf();
    let adapter = Codex::with(
        home.join("elsewhere/.codex"),
        SERVER,
        home.join(".handoff/channel.token"),
    );
    let scope = Scope::project(&project);

    let plan = install(&adapter, &scope);
    assert_eq!(plan[0].file, project.join(".codex").join("config.toml"));
    assert_matches(&home, "codex-project", "out");
    assert!(
        !home.join("elsewhere/.codex/config.toml").exists(),
        "project scope wrote into Codex's home"
    );
    assert_eq!(adapter.verify(&scope), Registration::Registered);
}

#[test]
fn codex_a_moved_bundle_is_a_path_mismatch_repaired_where_the_section_stands() {
    // FM-23: the section is ours and names the old path. The repair rewrites it in place,
    // with the comment above its header, and changes nothing else.
    let home = TempHome::from_case("codex-moved");
    let adapter = home.codex();

    assert_eq!(
        adapter.verify(&Scope::User),
        Registration::PathMismatch {
            registered: PathBuf::from(OLD_SERVER),
            current: PathBuf::from(SERVER),
        }
    );
    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    assert!(
        plan[0].diff.contains(OLD_SERVER) && plan[0].diff.contains(SERVER),
        "the diff does not show the old path being replaced:\n{}",
        plan[0].diff
    );
    adapter.apply(&plan).expect("the repair is applied");

    assert_matches(&home, "codex-moved", "out");
    assert_eq!(adapter.verify(&Scope::User), Registration::Registered);
}

#[test]
fn codex_an_entry_somebody_wrote_by_hand_is_theirs_until_the_plan_shows_it_replaced() {
    // `handoff-mcp`'s install-without-app page tells a person to write `[mcp_servers.handoff]`
    // with `npx`. Its command is not our fixed path, so it is not our registration, an
    // uninstall leaves it alone, and a plan replaces it only with its old lines in the diff.
    let home = TempHome::from_case("codex-empty");
    let file = home.join(CODEX_CONFIG);
    fs::create_dir_all(file.parent().expect("it has a folder")).expect("the folder is made");
    let theirs =
        "[mcp_servers.handoff]\ncommand = \"npx\"\nargs = [\"-y\", \"baton-handoff-mcp\"]\n";
    fs::write(&file, theirs).expect("the file is written");
    let adapter = home.codex();

    assert_eq!(adapter.verify(&Scope::User), Registration::NotRegistered);
    adapter.uninstall(&Scope::User).expect("nothing to remove");
    assert_eq!(fs::read_to_string(&file).expect("it is there"), theirs);

    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    assert!(
        plan[0].diff.contains("-command = \"npx\""),
        "the diff hides what it replaces:\n{}",
        plan[0].diff
    );
}

#[test]
fn codex_a_plan_applied_against_a_file_that_moved_on_is_refused() {
    let home = TempHome::from_case("codex-populated");
    let adapter = home.codex();
    let plan = adapter.plan(&Scope::User).expect("the plan is made");

    fs::write(
        home.join(CODEX_CONFIG),
        "[mcp_servers.handoff]\ncommand = \"somewhere-else\"\n",
    )
    .expect("the file is rewritten");

    match adapter.apply(&plan) {
        Err(InstallError::Stale { location, .. }) => {
            assert_eq!(location, "config.toml · mcp_servers.handoff");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn codex_a_configuration_that_is_not_toml_is_never_written_to() {
    let home = TempHome::from_case("codex-populated");
    let adapter = home.codex();
    let broken = "[mcp_servers.handoff\ncommand = \n";
    fs::write(home.join(CODEX_CONFIG), broken).expect("the file is rewritten");

    assert!(matches!(
        adapter.plan(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert!(matches!(
        adapter.uninstall(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert_eq!(
        fs::read_to_string(home.join(CODEX_CONFIG)).expect("it is still there"),
        broken
    );
}

#[test]
fn codex_an_inline_mcp_servers_table_is_refused_rather_than_rewritten() {
    // Valid TOML, and the user's own spelling: adding a `[mcp_servers.handoff]` section to it
    // would mean turning their inline table into sections first.
    let home = TempHome::from_case("codex-empty");
    let file = home.join(CODEX_CONFIG);
    fs::create_dir_all(file.parent().expect("it has a folder")).expect("the folder is made");
    let inline = "mcp_servers = { other = { command = \"npx\" } }\n";
    fs::write(&file, inline).expect("the file is written");

    assert!(matches!(
        home.codex().plan(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert_eq!(fs::read_to_string(&file).expect("it is there"), inline);
}

#[test]
fn codex_the_consent_path_shows_one_line_with_the_permission_behind_show() {
    let home = TempHome::from_case("codex-empty");
    let adapter = home.codex();
    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    let shown = install::digest(&plan);

    let lines = adapter.consent_lines(&plan);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].description.key, "install.codex.mcpEntry");
    assert_eq!(
        lines[0].description.args.get("minutes").map(String::as_str),
        Some("30")
    );
    for granted in [
        "default_tools_approval_mode = \"approve\"",
        "tool_timeout_sec = 1800",
    ] {
        assert!(
            lines[0].diff.contains(granted),
            "Show does not reveal {granted}:\n{}",
            lines[0].diff
        );
    }

    adapter.apply(&plan).expect("the plan is applied");
    assert_matches(&home, "codex-empty", "out");

    let again = adapter.plan(&Scope::User).expect("the plan is made again");
    assert!(again.iter().all(Modification::is_noop));
    assert_ne!(install::digest(&again), shown);
}

#[test]
fn codex_detect_names_the_file_of_the_scope_and_finds_codex_by_its_folder() {
    let home = TempHome::from_case("codex-populated");
    let detection = home.codex().detect(&Scope::User);

    assert_eq!(detection.agent_id, "codex");
    // `.codex/` exists in the fixture, which is enough on a runner with no `codex` on PATH.
    assert!(detection.found);
    assert_eq!(detection.config_files, vec![home.join(CODEX_CONFIG)]);
    assert_eq!(detection.version, None);
}

#[test]
fn claude_code_and_codex_in_one_home_do_not_touch_each_others_files() {
    // One machine, two agents: each adapter writes its own files and nothing of the other's,
    // so Claude Code's files are exactly its own goldens and Codex's are exactly its.
    let home = TempHome::from_case("populated");
    install(&home.adapter(), &Scope::User);
    install(&home.codex(), &Scope::User);

    assert_matches(&home, "populated", "out");
    let codex_golden = fs::read_to_string(
        fixture_dir("codex-empty")
            .join("out")
            .join(".codex")
            .join("config.toml"),
    )
    .expect("the golden is readable")
    .replace("\r\n", "\n");
    assert_eq!(
        fs::read_to_string(home.join(CODEX_CONFIG)).expect("written"),
        codex_golden
    );
}

// ---------------------------------------------------------- the OpenCode adapter (T-074)
//
// The Claude Code promises over OpenCode's JSON: `<home>/.config/opencode/opencode.json`, one
// `mcp.handoff` entry, and every server, setting and key the user had still there, in order.

/// OpenCode's configuration inside a temporary home.
const OPENCODE_CONFIG: &str = ".config/opencode/opencode.json";

#[test]
fn opencode_on_a_machine_with_no_configuration_at_all() {
    let home = TempHome::from_case("opencode-empty");
    let plan = install(&home.opencode(), &Scope::User);

    // One modification and no hook: OpenCode has none to register (T-074).
    assert_eq!(plan.len(), 1);
    assert!(plan[0].before.is_none() && !plan[0].is_noop());
    assert_matches(&home, "opencode-empty", "out");

    // INST-07 holds for every adapter: the first apply creates the token.
    assert!(
        home.join(".handoff/channel.token").is_file(),
        "the channel token was not created"
    );
    assert!(backups_of(&home, OPENCODE_CONFIG).is_empty());
}

#[test]
fn opencode_keeps_every_server_and_setting_the_user_had() {
    let home = TempHome::from_case("opencode-populated");
    install(&home.opencode(), &Scope::User);
    assert_matches(&home, "opencode-populated", "out");
    assert_eq!(backups_of(&home, OPENCODE_CONFIG).len(), 1);
}

#[test]
fn opencode_uninstall_gives_the_file_back_byte_for_byte() {
    let home = TempHome::from_case("opencode-populated");
    let adapter = home.opencode();
    install(&adapter, &Scope::User);
    adapter
        .uninstall(&Scope::User)
        .expect("the uninstall succeeds");

    assert_matches(&home, "opencode-populated", "in");
    assert_eq!(adapter.verify(&Scope::User), Registration::NotRegistered);
}

#[test]
fn opencode_applying_twice_writes_nothing_the_second_time() {
    let home = TempHome::from_case("opencode-populated");
    let adapter = home.opencode();
    install(&adapter, &Scope::User);
    let after_first = fs::read_to_string(home.join(OPENCODE_CONFIG)).expect("written");

    let second = adapter.plan(&Scope::User).expect("the plan is made");
    assert_eq!(second.len(), 1);
    assert!(
        second[0].is_noop(),
        "a second plan still wants to change something:\n{}",
        second[0].diff
    );
    adapter.apply(&second).expect("the plan is applied");

    assert_eq!(
        fs::read_to_string(home.join(OPENCODE_CONFIG)).expect("written"),
        after_first
    );
    assert_eq!(backups_of(&home, OPENCODE_CONFIG).len(), 1);
    assert_eq!(adapter.verify(&Scope::User), Registration::Registered);
}

#[test]
fn opencode_uninstall_where_ours_was_the_only_server_leaves_the_empty_object() {
    let home = TempHome::from_case("opencode-empty");
    let adapter = home.opencode();
    install(&adapter, &Scope::User);
    adapter
        .uninstall(&Scope::User)
        .expect("the uninstall succeeds");

    // `mcp` was ours to create, so it goes with our entry, as for Claude Code.
    assert_eq!(
        fs::read_to_string(home.join(OPENCODE_CONFIG)).expect("written"),
        "{}\n"
    );
}

#[test]
fn opencode_project_scope_writes_the_projects_file_and_not_the_global_one() {
    let home = TempHome::from_case("opencode-project");
    let project = home.path().to_path_buf();
    let adapter = OpenCode::with(
        home.join("elsewhere/opencode"),
        SERVER,
        home.join(".handoff/channel.token"),
    );
    let scope = Scope::project(&project);

    let plan = install(&adapter, &scope);
    assert_eq!(plan[0].file, project.join("opencode.json"));
    assert_matches(&home, "opencode-project", "out");
    assert!(
        !home.join("elsewhere/opencode/opencode.json").exists(),
        "project scope wrote into OpenCode's global folder"
    );
    assert_eq!(adapter.verify(&scope), Registration::Registered);
}

#[test]
fn opencode_a_moved_bundle_is_a_path_mismatch_repaired_where_the_entry_stands() {
    // FM-23: the entry is ours and names the old path. The repair rewrites it in place, before
    // the user's other server, and changes nothing else.
    let home = TempHome::from_case("opencode-moved");
    let adapter = home.opencode();

    assert_eq!(
        adapter.verify(&Scope::User),
        Registration::PathMismatch {
            registered: PathBuf::from(OLD_SERVER),
            current: PathBuf::from(SERVER),
        }
    );
    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    assert!(
        plan[0].diff.contains(OLD_SERVER) && plan[0].diff.contains(SERVER),
        "the diff does not show the old path being replaced:\n{}",
        plan[0].diff
    );
    adapter.apply(&plan).expect("the repair is applied");

    assert_matches(&home, "opencode-moved", "out");
    assert_eq!(adapter.verify(&Scope::User), Registration::Registered);
}

#[test]
fn opencode_an_entry_somebody_wrote_by_hand_is_theirs_until_the_plan_shows_it_replaced() {
    // `handoff-mcp`'s install-without-app page tells a person to write `mcp.handoff` with
    // `npx`. Its command is not our fixed path, so it is not our registration, an uninstall
    // leaves it alone, and a plan replaces it only with its old lines in the diff.
    let home = TempHome::from_case("opencode-empty");
    let file = home.join(OPENCODE_CONFIG);
    fs::create_dir_all(file.parent().expect("it has a folder")).expect("the folder is made");
    let theirs = concat!(
        "{\n",
        "  \"mcp\": {\n",
        "    \"handoff\": {\n",
        "      \"type\": \"local\",\n",
        "      \"command\": [\n",
        "        \"npx\",\n",
        "        \"-y\",\n",
        "        \"baton-handoff-mcp\"\n",
        "      ]\n",
        "    }\n",
        "  }\n",
        "}\n",
    );
    fs::write(&file, theirs).expect("the file is written");
    let adapter = home.opencode();

    assert_eq!(adapter.verify(&Scope::User), Registration::NotRegistered);
    adapter.uninstall(&Scope::User).expect("nothing to remove");
    assert_eq!(fs::read_to_string(&file).expect("it is there"), theirs);

    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    assert!(
        plan[0].diff.contains("\"npx\""),
        "the diff hides what it replaces:\n{}",
        plan[0].diff
    );
}

#[test]
fn opencode_a_plan_applied_against_a_file_that_moved_on_is_refused() {
    let home = TempHome::from_case("opencode-populated");
    let adapter = home.opencode();
    let plan = adapter.plan(&Scope::User).expect("the plan is made");

    fs::write(
        home.join(OPENCODE_CONFIG),
        "{\n  \"mcp\": {\n    \"handoff\": { \"type\": \"local\", \"command\": [\"elsewhere\"] }\n  }\n}\n",
    )
    .expect("the file is rewritten");

    match adapter.apply(&plan) {
        Err(InstallError::Stale { location, .. }) => {
            assert_eq!(location, "opencode.json · mcp.handoff");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn opencode_a_file_with_comments_is_refused_rather_than_rewritten() {
    // OpenCode reads JSON with comments, and a JSON parse would lose them (INST-04): the file
    // is the user's, so the adapter refuses it and never writes a byte into it.
    let home = TempHome::from_case("opencode-populated");
    let adapter = home.opencode();
    let commented = "{\n  // the model I use\n  \"model\": \"openrouter/vendor/model\"\n}\n";
    fs::write(home.join(OPENCODE_CONFIG), commented).expect("the file is rewritten");

    assert!(matches!(
        adapter.plan(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert!(matches!(
        adapter.uninstall(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert_eq!(
        fs::read_to_string(home.join(OPENCODE_CONFIG)).expect("it is still there"),
        commented
    );
}

#[test]
fn opencode_an_mcp_that_is_not_an_object_is_refused_rather_than_replaced() {
    // Valid JSON that OpenCode itself would not accept: writing our entry would replace the
    // user's value, so the adapter refuses and leaves it for the user to see.
    let home = TempHome::from_case("opencode-empty");
    let file = home.join(OPENCODE_CONFIG);
    fs::create_dir_all(file.parent().expect("it has a folder")).expect("the folder is made");
    let odd = "{\n  \"mcp\": []\n}\n";
    fs::write(&file, odd).expect("the file is written");

    assert!(matches!(
        home.opencode().plan(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert_eq!(fs::read_to_string(&file).expect("it is there"), odd);
}

#[test]
fn opencode_the_consent_path_shows_one_line_with_the_timeout_behind_show() {
    let home = TempHome::from_case("opencode-empty");
    let adapter = home.opencode();
    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    let shown = install::digest(&plan);

    let lines = adapter.consent_lines(&plan);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].description.key, "install.opencode.mcpEntry");
    assert_eq!(
        lines[0].description.args.get("minutes").map(String::as_str),
        Some("30")
    );
    for written in ["\"timeout\": 1800000", "\"HANDOFF_AGENT\": \"opencode\""] {
        assert!(
            lines[0].diff.contains(written),
            "Show does not reveal {written}:\n{}",
            lines[0].diff
        );
    }

    adapter.apply(&plan).expect("the plan is applied");
    assert_matches(&home, "opencode-empty", "out");

    let again = adapter.plan(&Scope::User).expect("the plan is made again");
    assert!(again.iter().all(Modification::is_noop));
    assert_ne!(install::digest(&again), shown);
}

#[test]
fn opencode_detect_names_the_file_of_the_scope_and_finds_opencode_by_its_folder() {
    let home = TempHome::from_case("opencode-populated");
    let detection = home.opencode().detect(&Scope::User);

    assert_eq!(detection.agent_id, "opencode");
    // `.config/opencode/` exists in the fixture, which is enough on a runner with no
    // `opencode` on PATH.
    assert!(detection.found);
    assert_eq!(detection.config_files, vec![home.join(OPENCODE_CONFIG)]);
    assert_eq!(detection.version, None);
}

// ------------------------------------------------------------ the Cursor adapter (T-070)
//
// The Claude Code promises over Cursor's JSON: `<home>/.cursor/mcp.json`, one
// `mcpServers.handoff` entry, and every server the user had still there, in order.

/// Cursor's configuration inside a temporary home.
const CURSOR_CONFIG: &str = ".cursor/mcp.json";

#[test]
fn cursor_on_a_machine_with_no_configuration_at_all() {
    let home = TempHome::from_case("cursor-empty");
    let plan = install(&home.cursor(), &Scope::User);

    // One modification and no hook: none of Cursor's reaches ours (T-069).
    assert_eq!(plan.len(), 1);
    assert!(plan[0].before.is_none() && !plan[0].is_noop());
    assert_matches(&home, "cursor-empty", "out");

    // INST-07 holds for every adapter: the first apply creates the token.
    assert!(
        home.join(".handoff/channel.token").is_file(),
        "the channel token was not created"
    );
    assert!(backups_of(&home, CURSOR_CONFIG).is_empty());
}

#[test]
fn cursor_keeps_every_server_the_user_had() {
    let home = TempHome::from_case("cursor-populated");
    install(&home.cursor(), &Scope::User);
    assert_matches(&home, "cursor-populated", "out");
    assert_eq!(backups_of(&home, CURSOR_CONFIG).len(), 1);
}

#[test]
fn cursor_uninstall_gives_the_file_back_byte_for_byte() {
    let home = TempHome::from_case("cursor-populated");
    let adapter = home.cursor();
    install(&adapter, &Scope::User);
    adapter
        .uninstall(&Scope::User)
        .expect("the uninstall succeeds");

    assert_matches(&home, "cursor-populated", "in");
    assert_eq!(adapter.verify(&Scope::User), Registration::NotRegistered);
}

#[test]
fn cursor_applying_twice_writes_nothing_the_second_time() {
    let home = TempHome::from_case("cursor-populated");
    let adapter = home.cursor();
    install(&adapter, &Scope::User);
    let after_first = fs::read_to_string(home.join(CURSOR_CONFIG)).expect("written");

    let second = adapter.plan(&Scope::User).expect("the plan is made");
    assert_eq!(second.len(), 1);
    assert!(
        second[0].is_noop(),
        "a second plan still wants to change something:\n{}",
        second[0].diff
    );
    adapter.apply(&second).expect("the plan is applied");

    assert_eq!(
        fs::read_to_string(home.join(CURSOR_CONFIG)).expect("written"),
        after_first
    );
    assert_eq!(backups_of(&home, CURSOR_CONFIG).len(), 1);
    assert_eq!(adapter.verify(&Scope::User), Registration::Registered);
}

#[test]
fn cursor_uninstall_where_ours_was_the_only_server_leaves_the_empty_object() {
    let home = TempHome::from_case("cursor-empty");
    let adapter = home.cursor();
    install(&adapter, &Scope::User);
    adapter
        .uninstall(&Scope::User)
        .expect("the uninstall succeeds");

    // `mcpServers` was ours to create, so it goes with our entry, as for Claude Code.
    assert_eq!(
        fs::read_to_string(home.join(CURSOR_CONFIG)).expect("written"),
        "{}\n"
    );
}

#[test]
fn cursor_project_scope_writes_the_projects_file_and_not_the_users() {
    let home = TempHome::from_case("cursor-project");
    let project = home.path().to_path_buf();
    let adapter = Cursor::with(
        home.join("elsewhere/.cursor"),
        SERVER,
        home.join(".handoff/channel.token"),
    );
    let scope = Scope::project(&project);

    let plan = install(&adapter, &scope);
    assert_eq!(plan[0].file, project.join(".cursor").join("mcp.json"));
    assert_matches(&home, "cursor-project", "out");
    assert!(
        !home.join("elsewhere/.cursor/mcp.json").exists(),
        "project scope wrote into the user's Cursor folder"
    );
    assert_eq!(adapter.verify(&scope), Registration::Registered);
}

#[test]
fn cursor_a_moved_bundle_is_a_path_mismatch_repaired_where_the_entry_stands() {
    // FM-23: the entry is ours and names the old path. The repair rewrites it in place, before
    // the user's other server, and changes nothing else.
    let home = TempHome::from_case("cursor-moved");
    let adapter = home.cursor();

    assert_eq!(
        adapter.verify(&Scope::User),
        Registration::PathMismatch {
            registered: PathBuf::from(OLD_SERVER),
            current: PathBuf::from(SERVER),
        }
    );
    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    assert!(
        plan[0].diff.contains(OLD_SERVER) && plan[0].diff.contains(SERVER),
        "the diff does not show the old path being replaced:\n{}",
        plan[0].diff
    );
    adapter.apply(&plan).expect("the repair is applied");

    assert_matches(&home, "cursor-moved", "out");
    assert_eq!(adapter.verify(&Scope::User), Registration::Registered);
}

#[test]
fn cursor_an_entry_somebody_wrote_by_hand_is_theirs_until_the_plan_shows_it_replaced() {
    // `handoff-mcp`'s install-without-app page tells a person to write `mcpServers.handoff`
    // with `npx`. Its command is not our fixed path, so it is not our registration, an
    // uninstall leaves it alone, and a plan replaces it only with its old lines in the diff.
    let home = TempHome::from_case("cursor-empty");
    let file = home.join(CURSOR_CONFIG);
    fs::create_dir_all(file.parent().expect("it has a folder")).expect("the folder is made");
    let theirs = concat!(
        "{\n",
        "  \"mcpServers\": {\n",
        "    \"handoff\": {\n",
        "      \"command\": \"npx\",\n",
        "      \"args\": [\n",
        "        \"-y\",\n",
        "        \"baton-handoff-mcp\"\n",
        "      ],\n",
        "      \"env\": {\n",
        "        \"HANDOFF_AGENT\": \"cursor\"\n",
        "      }\n",
        "    }\n",
        "  }\n",
        "}\n",
    );
    fs::write(&file, theirs).expect("the file is written");
    let adapter = home.cursor();

    assert_eq!(adapter.verify(&Scope::User), Registration::NotRegistered);
    adapter.uninstall(&Scope::User).expect("nothing to remove");
    assert_eq!(fs::read_to_string(&file).expect("it is there"), theirs);

    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    assert!(
        plan[0].diff.contains("\"npx\""),
        "the diff hides what it replaces:\n{}",
        plan[0].diff
    );
}

#[test]
fn cursor_a_plan_applied_against_a_file_that_moved_on_is_refused() {
    let home = TempHome::from_case("cursor-populated");
    let adapter = home.cursor();
    let plan = adapter.plan(&Scope::User).expect("the plan is made");

    fs::write(
        home.join(CURSOR_CONFIG),
        "{\n  \"mcpServers\": {\n    \"handoff\": { \"command\": \"elsewhere\" }\n  }\n}\n",
    )
    .expect("the file is rewritten");

    match adapter.apply(&plan) {
        Err(InstallError::Stale { location, .. }) => {
            assert_eq!(location, "mcp.json · mcpServers.handoff");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn cursor_a_file_with_comments_is_refused_rather_than_rewritten() {
    // A JSON parse would lose the comments (INST-04): the file is the user's, so the adapter
    // refuses it and never writes a byte into it.
    let home = TempHome::from_case("cursor-populated");
    let adapter = home.cursor();
    let commented = "{\n  // the servers I use\n  \"mcpServers\": {}\n}\n";
    fs::write(home.join(CURSOR_CONFIG), commented).expect("the file is rewritten");

    assert!(matches!(
        adapter.plan(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert!(matches!(
        adapter.uninstall(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert_eq!(
        fs::read_to_string(home.join(CURSOR_CONFIG)).expect("it is still there"),
        commented
    );
}

#[test]
fn cursor_mcp_servers_that_are_not_an_object_are_refused_rather_than_replaced() {
    // Valid JSON that Cursor itself would not accept: writing our entry would replace the
    // user's value, so the adapter refuses and leaves it for the user to see.
    let home = TempHome::from_case("cursor-empty");
    let file = home.join(CURSOR_CONFIG);
    fs::create_dir_all(file.parent().expect("it has a folder")).expect("the folder is made");
    let odd = "{\n  \"mcpServers\": []\n}\n";
    fs::write(&file, odd).expect("the file is written");

    assert!(matches!(
        home.cursor().plan(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert_eq!(fs::read_to_string(&file).expect("it is there"), odd);
}

#[test]
fn cursor_the_consent_path_shows_one_line_that_grants_and_raises_nothing() {
    let home = TempHome::from_case("cursor-empty");
    let adapter = home.cursor();
    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    let shown = install::digest(&plan);

    let lines = adapter.consent_lines(&plan);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].description.key, "install.cursor.mcpEntry");
    assert_eq!(
        lines[0].description.args.get("file").map(String::as_str),
        Some("mcp.json")
    );
    assert!(
        !lines[0].description.args.contains_key("minutes"),
        "Cursor reads no timeout from an entry (T-069)"
    );
    assert!(
        lines[0].diff.contains("\"HANDOFF_AGENT\": \"cursor\""),
        "Show does not reveal the agent:\n{}",
        lines[0].diff
    );
    for absent in ["timeout", "HANDOFF_TOOL_TIMEOUT_MS", "Mcp("] {
        assert!(
            !lines[0].diff.contains(absent),
            "Show reveals {absent}:\n{}",
            lines[0].diff
        );
    }

    adapter.apply(&plan).expect("the plan is applied");
    assert_matches(&home, "cursor-empty", "out");

    let again = adapter.plan(&Scope::User).expect("the plan is made again");
    assert!(again.iter().all(Modification::is_noop));
    assert_ne!(install::digest(&again), shown);
}

#[test]
fn cursor_detect_names_the_file_of_the_scope_and_finds_cursor_by_its_folder() {
    let home = TempHome::from_case("cursor-populated");
    let detection = home.cursor().detect(&Scope::User);

    assert_eq!(detection.agent_id, "cursor");
    // `.cursor/` exists in the fixture, which is enough on a runner with no Cursor on PATH.
    assert!(detection.found);
    assert_eq!(detection.config_files, vec![home.join(CURSOR_CONFIG)]);
    assert_eq!(detection.version, None);
}

#[test]
fn five_agents_in_one_home_do_not_touch_each_others_files() {
    // One machine, five agents: each adapter writes its own files and nothing of the others'.
    let home = TempHome::from_case("populated");
    install(&home.adapter(), &Scope::User);
    install(&home.codex(), &Scope::User);
    install(&home.cursor(), &Scope::User);
    install(&home.copilot(), &Scope::User);
    install(&home.opencode(), &Scope::User);

    assert_matches(&home, "populated", "out");
    for (case, relative) in [
        ("opencode-empty", OPENCODE_CONFIG),
        ("cursor-empty", CURSOR_CONFIG),
        ("copilot-empty", COPILOT_CLI_CONFIG),
        ("copilot-empty", COPILOT_VSCODE_CONFIG),
    ] {
        let golden = fs::read_to_string(fixture_dir(case).join("out").join(relative))
            .expect("the golden is readable")
            .replace("\r\n", "\n");
        assert_eq!(
            fs::read_to_string(home.join(relative)).expect("written"),
            golden,
            "{relative}"
        );
    }
}

// ------------------------------------------------------ the GitHub Copilot adapter (T-072)
//
// Two files, one per surface: the Copilot CLI's `mcp-config.json` under `mcpServers`, and VS
// Code's `mcp.json` under `servers`, each keeping every server the user had, in order.

/// The CLI's configuration inside a temporary home.
const COPILOT_CLI_CONFIG: &str = ".copilot/mcp-config.json";

/// VS Code's user configuration inside a temporary home.
const COPILOT_VSCODE_CONFIG: &str = "AppData/Roaming/Code/User/mcp.json";

#[test]
fn copilot_on_a_machine_with_no_configuration_at_all() {
    let home = TempHome::from_case("copilot-empty");
    let plan = install(&home.copilot(), &Scope::User);

    // Two modifications, one per surface, and no hook: none of Copilot's answers ours (T-072).
    assert_eq!(plan.len(), 2);
    assert!(plan
        .iter()
        .all(|modification| modification.before.is_none() && !modification.is_noop()));
    assert_matches(&home, "copilot-empty", "out");
    assert!(
        home.join(".handoff/channel.token").is_file(),
        "the channel token was not created"
    );
    assert!(backups_of(&home, COPILOT_CLI_CONFIG).is_empty());
    assert!(backups_of(&home, COPILOT_VSCODE_CONFIG).is_empty());
}

#[test]
fn copilot_keeps_every_server_the_user_had_in_both_files() {
    let home = TempHome::from_case("copilot-populated");
    install(&home.copilot(), &Scope::User);
    assert_matches(&home, "copilot-populated", "out");
    assert_eq!(backups_of(&home, COPILOT_CLI_CONFIG).len(), 1);
    assert_eq!(backups_of(&home, COPILOT_VSCODE_CONFIG).len(), 1);
}

#[test]
fn copilot_uninstall_gives_both_files_back_byte_for_byte() {
    let home = TempHome::from_case("copilot-populated");
    let adapter = home.copilot();
    install(&adapter, &Scope::User);
    adapter
        .uninstall(&Scope::User)
        .expect("the uninstall succeeds");

    assert_matches(&home, "copilot-populated", "in");
    assert_eq!(adapter.verify(&Scope::User), Registration::NotRegistered);
}

#[test]
fn copilot_applying_twice_writes_nothing_the_second_time() {
    let home = TempHome::from_case("copilot-populated");
    let adapter = home.copilot();
    install(&adapter, &Scope::User);
    let cli = fs::read_to_string(home.join(COPILOT_CLI_CONFIG)).expect("written");
    let vscode = fs::read_to_string(home.join(COPILOT_VSCODE_CONFIG)).expect("written");

    let second = adapter.plan(&Scope::User).expect("the plan is made");
    assert_eq!(second.len(), 2);
    assert!(
        second.iter().all(Modification::is_noop),
        "a second plan still wants to change something"
    );
    adapter.apply(&second).expect("the plan is applied");

    assert_eq!(
        fs::read_to_string(home.join(COPILOT_CLI_CONFIG)).expect("written"),
        cli
    );
    assert_eq!(
        fs::read_to_string(home.join(COPILOT_VSCODE_CONFIG)).expect("written"),
        vscode
    );
    assert_eq!(backups_of(&home, COPILOT_CLI_CONFIG).len(), 1);
    assert_eq!(backups_of(&home, COPILOT_VSCODE_CONFIG).len(), 1);
    assert_eq!(adapter.verify(&Scope::User), Registration::Registered);
}

#[test]
fn copilot_uninstall_where_ours_was_the_only_server_leaves_the_empty_objects() {
    let home = TempHome::from_case("copilot-empty");
    let adapter = home.copilot();
    install(&adapter, &Scope::User);
    adapter
        .uninstall(&Scope::User)
        .expect("the uninstall succeeds");

    // `mcpServers` and `servers` were ours to create, so each goes with our entry.
    for relative in [COPILOT_CLI_CONFIG, COPILOT_VSCODE_CONFIG] {
        assert_eq!(
            fs::read_to_string(home.join(relative)).expect("written"),
            "{}\n",
            "{relative}"
        );
    }
}

#[test]
fn copilot_project_scope_writes_the_projects_files_and_not_the_users() {
    let home = TempHome::from_case("copilot-project");
    let project = home.path().to_path_buf();
    let adapter = Copilot::with(
        home.join("elsewhere/.copilot"),
        home.join("elsewhere/Code/User"),
        SERVER,
        home.join(".handoff/channel.token"),
    );
    let scope = Scope::project(&project);

    let plan = install(&adapter, &scope);
    assert_eq!(plan[0].file, project.join(".github").join("mcp.json"));
    assert_eq!(plan[1].file, project.join(".vscode").join("mcp.json"));
    assert_matches(&home, "copilot-project", "out");
    assert!(
        !home.join("elsewhere/.copilot/mcp-config.json").exists()
            && !home.join("elsewhere/Code/User/mcp.json").exists(),
        "project scope wrote into the user's files"
    );
    // The CLI reads a project's `.mcp.json` too, and that one is Claude Code's.
    assert!(!project.join(".mcp.json").exists());
    assert_eq!(adapter.verify(&scope), Registration::Registered);
}

#[test]
fn copilot_a_moved_bundle_is_a_path_mismatch_repaired_where_the_entries_stand() {
    // FM-23: both entries are ours and name the old path. The repair rewrites each in place and
    // changes nothing else.
    let home = TempHome::from_case("copilot-moved");
    let adapter = home.copilot();

    assert_eq!(
        adapter.verify(&Scope::User),
        Registration::PathMismatch {
            registered: PathBuf::from(OLD_SERVER),
            current: PathBuf::from(SERVER),
        }
    );
    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    for modification in &plan {
        assert!(
            modification.diff.contains(OLD_SERVER) && modification.diff.contains(SERVER),
            "the diff does not show the old path being replaced:\n{}",
            modification.diff
        );
    }
    adapter.apply(&plan).expect("the repair is applied");

    assert_matches(&home, "copilot-moved", "out");
    assert_eq!(adapter.verify(&Scope::User), Registration::Registered);
}

#[test]
fn copilot_one_surface_registered_is_partly_registered() {
    // The CLI's entry is there and VS Code's is not: the repair offer names the missing one.
    let home = TempHome::from_case("copilot-empty");
    let adapter = home.copilot();
    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    adapter
        .apply(&plan[..1])
        .expect("the CLI's entry is applied");

    assert_eq!(
        adapter.verify(&Scope::User),
        Registration::Partial {
            missing: vec!["mcp.json · servers.handoff".to_owned()],
        }
    );
}

#[test]
fn copilot_a_vscode_file_with_comments_is_refused_and_nothing_is_written() {
    // A JSON parse would lose the comments (INST-04): the file is the user's, so the adapter
    // refuses the whole plan and writes neither file.
    let home = TempHome::from_case("copilot-populated");
    let adapter = home.copilot();
    let commented = "{\n  // the servers I use\n  \"servers\": {}\n}\n";
    fs::write(home.join(COPILOT_VSCODE_CONFIG), commented).expect("the file is rewritten");
    let cli_before = fs::read_to_string(home.join(COPILOT_CLI_CONFIG)).expect("it is there");

    assert!(matches!(
        adapter.plan(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert!(matches!(
        adapter.uninstall(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert_eq!(
        fs::read_to_string(home.join(COPILOT_VSCODE_CONFIG)).expect("it is still there"),
        commented
    );
    assert_eq!(
        fs::read_to_string(home.join(COPILOT_CLI_CONFIG)).expect("it is still there"),
        cli_before
    );
}

#[test]
fn copilot_servers_that_are_not_an_object_are_refused_rather_than_replaced() {
    let home = TempHome::from_case("copilot-empty");
    let file = home.join(COPILOT_CLI_CONFIG);
    fs::create_dir_all(file.parent().expect("it has a folder")).expect("the folder is made");
    let odd = "{\n  \"mcpServers\": []\n}\n";
    fs::write(&file, odd).expect("the file is written");

    assert!(matches!(
        home.copilot().plan(&Scope::User),
        Err(InstallError::Malformed { .. })
    ));
    assert_eq!(fs::read_to_string(&file).expect("it is there"), odd);
}

#[test]
fn copilot_the_consent_screen_shows_one_line_per_surface() {
    let home = TempHome::from_case("copilot-empty");
    let adapter = home.copilot();
    let plan = adapter.plan(&Scope::User).expect("the plan is made");
    let lines = adapter.consent_lines(&plan);

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].description.key, "install.copilot.cliEntry");
    assert_eq!(
        lines[0].description.args.get("minutes").map(String::as_str),
        Some("30")
    );
    assert_eq!(lines[1].description.key, "install.copilot.vscodeEntry");
    assert!(!lines[1].description.args.contains_key("minutes"));
    for line in &lines {
        assert!(!line.diff.to_lowercase().contains("hook"), "{}", line.diff);
    }
}

#[test]
fn copilot_detect_names_both_files_of_the_scope() {
    let home = TempHome::from_case("copilot-populated");
    let detection = home.copilot().detect(&Scope::User);

    assert_eq!(detection.agent_id, "copilot");
    // `.copilot/` exists in the fixture, which is enough on a runner with neither surface.
    assert!(detection.found);
    assert_eq!(
        detection.config_files,
        vec![
            home.join(COPILOT_CLI_CONFIG),
            home.join(COPILOT_VSCODE_CONFIG)
        ]
    );
    assert_eq!(detection.version, None);
}
