//! The round trip of RUN-10: the app writes a runbook, the server finds it.
//!
//! Creating a runbook is the app's (§2.4, RUN-10) and reading one is the server's, so the
//! only way to know the two agree is to write a file with the real writer and then hand it
//! to the real `handoff-mcp`. That is what this does: it drives the store through a verified
//! handoff, points the vendored binary at the folder with `HANDOFF_HOME`, and reads what
//! `runbooks search` prints.
//!
//! What it proves that a unit test cannot: the file parses under the **server's** reader and
//! its schema, the matching rule of §4.5.3 agrees across the two implementations on the same
//! `where` and `goal`, and `matched_words` is the list the app expects — the three ways the
//! two sides could drift apart without either suite noticing (§3.4).
//!
//! # When there is no binary
//!
//! `server.lock.json` pins no darwin asset while macOS is deferred (`TASKS.md` §0.4 item 7),
//! so the macOS CI leg fetches the format material alone (`fetch-server --format-only`) and
//! has nothing to spawn. The test then reports why and passes: a job that cannot run it must
//! not fail on it, and the Windows leg — where the binary is always there — does run it.

use std::path::{Path, PathBuf};
use std::process::Command;

use handoff_app_lib::format::outcome::ResumedFrom;
use handoff_app_lib::format::spec::{HandoffSpec, HandoffStep, SpecValue};
use handoff_app_lib::log::sessions::{self, SessionRow};
use handoff_app_lib::log::{Db, Timestamp};
use handoff_app_lib::runbooks::RunbookWriter;
use handoff_app_lib::store::handoff::{Call, Opener};
use handoff_app_lib::store::{NoRequests, OpenParams, Store};
use serde_json::Value;

const SESSION: &str = "ses_00000001";
const WHERE: &str = "Stripe Dashboard → Developers → Webhooks";
const GOAL: &str = "Register the webhook for payment events";

#[test]
fn a_runbook_this_app_wrote_is_found_by_the_vendored_server() {
    let Some(server) = vendored_server() else {
        eprintln!(
            "skipped: no vendored handoff-mcp binary for this platform \
             (`node scripts/fetch-server.mjs`; the macOS leg fetches the format alone)"
        );
        return;
    };

    let home = TempHome::new();
    write_a_runbook(&home);

    let names = home.runbook_names();
    assert_eq!(names.len(), 1, "the writer produced one file: {names:?}");

    let found = search(&server, &home, WHERE, GOAL);
    let runbooks = found
        .get("runbooks")
        .and_then(Value::as_array)
        .expect("the server prints an object with a runbooks array");
    assert_eq!(runbooks.len(), 1, "the server found it: {found}");

    let runbook = &runbooks[0];
    assert_eq!(
        runbook.get("path").and_then(Value::as_str).map(|path| {
            Path::new(path)
                .file_name()
                .expect("a file name")
                .to_string_lossy()
                .into_owned()
        }),
        Some(names[0].clone()),
        "the path it reports is the file the writer wrote: {runbook}"
    );
    assert_eq!(
        runbook.get("trust").and_then(Value::as_str),
        Some("verified")
    );

    // §4.5.3: the shared words, in the order they first appear in the *query's* goal, with
    // the stop-words and the tokens shorter than three code points dropped.
    let matched: Vec<&str> = runbook
        .get("matched_words")
        .and_then(Value::as_array)
        .expect("matched_words")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        matched,
        vec!["register", "webhook", "payment", "events"],
        "the two implementations of §4.5.3 agree: {runbook}"
    );

    // And the draft spec it converts the file into carries the placeholder as a name to
    // fill, never a value (RUN-05, §4.5.4).
    let draft = runbook.get("draft_spec").expect("a draft spec");
    assert_eq!(
        draft
            .pointer("/values/endpoint_url")
            .and_then(Value::as_str),
        Some(""),
        "the value is a name to fill: {draft}"
    );
    let printed = serde_json::to_string(&found).expect("the answer serialises");
    assert!(
        !printed.contains("https://api.example.test/hook"),
        "no value of the handoff reached the runbook: {printed}"
    );

    // A different place is a different runbook, whatever the goal says (§4.5.3).
    let elsewhere = search(&server, &home, "Some other console", GOAL);
    assert_eq!(
        elsewhere
            .get("runbooks")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0),
        "{elsewhere}"
    );
}

/// Drives the store through one verified handoff, with the writer over `<home>/runbooks/`.
fn write_a_runbook(home: &TempHome) {
    let db = Db::open_at(home.0.join("handoff.sqlite")).expect("a database");
    sessions::register(&db, &session()).expect("a session");
    let mut store = Store::load(
        db,
        Box::new(RunbookWriter::at(home.0.join("runbooks"))),
        Box::new(NoRequests),
    )
    .expect("an empty store");

    let now = Timestamp::parse("2026-09-09T11:00:00Z").expect("rfc 3339");
    let id = store
        .open(
            OpenParams {
                spec: spec(),
                secret_treated: Vec::new(),
                request_id: None,
                opener: opener(),
                call: Call {
                    conn_id: 1,
                    call_id: "call_00000001".to_owned(),
                    session_ref: Some(SESSION.to_owned()),
                },
            },
            &now,
        )
        .expect("the open is accepted")
        .handoff_id;
    store.confirm(&id, &now).expect("step 1");
    store.confirm(&id, &now).expect("step 2");
    store.done(&id, &now).expect("done");
    store
        .verify(&id, Some(true), Some("it fires".to_owned()), &now)
        .expect("the report is accepted");
}

/// `handoff-mcp runbooks search`, over `home`.
fn search(server: &Path, home: &TempHome, where_: &str, goal: &str) -> Value {
    let output = Command::new(server)
        .args([
            "runbooks", "search", "--where", where_, "--goal", goal, "--lang", "en",
        ])
        .env("HANDOFF_HOME", &home.0)
        .output()
        .expect("the vendored server runs");
    assert!(
        output.status.success(),
        "runbooks search exited {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    // A file it had to skip is named on stderr and is not fatal (`> Note from T-016`), so a
    // writer defect shows up as an empty list plus this line rather than as a failure.
    let warnings = String::from_utf8_lossy(&output.stderr);
    assert!(
        warnings.trim().is_empty(),
        "the server skipped a file it should have read: {warnings}"
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "the answer is JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// The vendored `handoff-mcp` for this platform, when the lock pins one.
fn vendored_server() -> Option<PathBuf> {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "win32-x64",
        ("macos", "x86_64") => "darwin-x64",
        ("macos", "aarch64") => "darwin-arm64",
        _ => return None,
    };
    let name = if cfg!(windows) {
        "handoff-mcp.exe"
    } else {
        "handoff-mcp"
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("vendor")
        .join("handoff-mcp")
        .join("bin")
        .join(platform)
        .join(name);
    path.is_file().then_some(path)
}

/// A temporary `HANDOFF_HOME` of this test binary's own, removed when it is dropped.
struct TempHome(PathBuf);

impl TempHome {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "handoff-roundtrip-{}-{}",
            std::process::id(),
            handoff_app_lib::ids::new_session_ref()
        ));
        std::fs::create_dir_all(&dir).expect("the temporary home is created");
        Self(dir)
    }

    fn runbook_names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.0.join("runbooks"))
            .expect("the runbook folder exists")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".json"))
            .collect();
        names.sort();
        names
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn session() -> SessionRow {
    SessionRow {
        session_ref: SESSION.to_owned(),
        agent_id: Some("claude-code".to_owned()),
        client_name: Some("claude-code".to_owned()),
        client_version: Some("2.1.266".to_owned()),
        pid_chain_json: "[4242,1212]".to_owned(),
        cwd: Some("C:/projects/baton".to_owned()),
        project_dir: Some("C:/projects/baton".to_owned()),
        claude_session_id: None,
        connected: true,
        first_seen: Timestamp::parse("2026-09-09T10:00:00Z").expect("rfc 3339"),
        last_seen: Timestamp::parse("2026-09-09T11:00:00Z").expect("rfc 3339"),
    }
}

fn opener() -> Opener {
    Opener {
        session_ref: SESSION.to_owned(),
        agent_id: Some("claude-code".to_owned()),
        client_name: Some("claude-code".to_owned()),
        project_dir: Some("C:/projects/baton".to_owned()),
        label: ResumedFrom {
            agent: "Claude Code".to_owned(),
            project: "baton".to_owned(),
        },
    }
}

fn spec() -> HandoffSpec {
    let mut values = indexmap::IndexMap::new();
    values.insert(
        "endpoint_url".to_owned(),
        SpecValue::One("https://api.example.test/hook".to_owned()),
    );
    HandoffSpec {
        spec_version: 1,
        goal: GOAL.to_owned(),
        r#where: WHERE.to_owned(),
        url: None,
        why_human: "Requires access to the production Stripe account.".to_owned(),
        values,
        secrets: None,
        steps: vec![
            HandoffStep {
                text: "Open Developers and then Webhooks.".to_owned(),
                url: None,
                values: None,
                warning: None,
            },
            HandoffStep {
                text: "Paste https://api.example.test/hook as the endpoint URL.".to_owned(),
                url: None,
                values: Some(vec!["endpoint_url".to_owned()]),
                warning: None,
            },
        ],
        verify: Some("A test event reaches https://api.example.test/hook.".to_owned()),
        lang: Some("en".to_owned()),
    }
}
