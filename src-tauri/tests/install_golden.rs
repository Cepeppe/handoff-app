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
    survives_project_scope, InstallAdapter, InstallError, Modification, Registration, Scope,
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
fn install(adapter: &ClaudeCode, scope: &Scope) -> Vec<Modification> {
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
