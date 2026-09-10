//! A crash file carries identifiers and nothing else (§7.14, TEL-01, R-19, §11.7).
//!
//! What it guards: a panic writes `crashes/<instant>.txt` with the last fifty log lines, and
//! the user may mail that file to us. `crash.rs` keeps a field only when its name is an
//! identifier, an instant, a size or a count, and its unit tests prove the filter; they
//! cannot prove that the file a real panic writes after real traffic holds nothing else. So a
//! child process (`child.rs`) does what the app does — `crash::install`, then the listener,
//! the registry, the store and the dispatch — and:
//!
//! 1. a server registers and opens a handoff whose spec carries a planted certain secret in
//!    every field the ingress scan of §5.5 covers (`forbidden.rs`); the user writes a note
//!    with one more in it and presses Done; the agent reports, with one more in the detail;
//! 2. one line is logged with a planted value in a field the crash ring must drop — the
//!    control, so that "the file does not hold it" is a statement about the filter and not
//!    about a line that never reached the ring;
//! 3. a thread panics, on purpose.
//!
//! The parent then reads the crash file and the child's stderr. The file must hold the panic,
//! the control line's message and the session's identifier, and neither a value of the
//! forbidden set nor the control value. The stderr — the production subscriber, every field
//! unfiltered — must hold the control value, which proves it shows what it is given, and no
//! value of the forbidden set: R-19's "stderr never logs values".

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::json;

use handoff_app_lib::channel::listener::{listen, ListenerConfig};
use handoff_app_lib::channel::token::Token;
use handoff_app_lib::channel::Dispatch;
use handoff_app_lib::crash;
use handoff_app_lib::log::{Db, Timestamp};
use handoff_app_lib::requests::{NoRequestObserver, Queue};
use handoff_app_lib::sessions::{NoObserver, Registry};
use handoff_app_lib::store::{self, NoRunbookSink, Store, UserAction};

use crate::fake_server::FakeServer;
use crate::support::{private_endpoint, TempDir};
use crate::{child, forbidden, report};

/// This test's own path, which is how the child is selected.
const TEST: &str = "crash_files::a_crash_file_after_real_traffic_carries_identifiers_only";

/// What the parent hands the child: the folder everything is written into.
const WORKSPACE: &str = "BATON_SECURITY_DIR";

/// The message of the panic the child causes, which is how its crash file is told apart.
const CONTROLLED_PANIC: &str = "the controlled panic of the security suite (T-053)";

/// The control value, logged in a field named `note`: the crash ring must drop it, the
/// stderr subscriber prints it.
const CONTROL: &str = "sk_live_CRASHCONTROL0000000001";

/// The message of the line that carries [`CONTROL`].
const CONTROL_MESSAGE: &str = "the security suite logs a value in a field the crash ring drops";

#[test]
fn a_crash_file_after_real_traffic_carries_identifiers_only() {
    if child::is_child() {
        crash_after_traffic();
        return;
    }
    let dir = TempDir::new("crash");
    // `debug` rather than the default `info`: every event the app can emit on this path is a
    // place a value could have been put, so the widest subscriber is the strictest reading.
    let output = child::run(
        TEST,
        &[
            (WORKSPACE, dir.path().as_os_str()),
            ("RUST_LOG", OsStr::new("debug")),
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let file = the_crash_file(&crash::crashes_dir(&dir.path().join("app-data")));

    let forbidden = forbidden::forbidden();
    let in_file = forbidden::leaks(&file, forbidden);
    let in_stderr = forbidden::leaks(&stderr, forbidden);

    // What the file has to hold for its silence to mean anything …
    let panic_recorded = file.contains(&format!("panic: {CONTROLLED_PANIC}"));
    let traffic_recorded = file.contains("a session registered") && file.contains("ses_");
    let control_line_recorded = file.contains(CONTROL_MESSAGE);
    let control_value_dropped = !file.contains(CONTROL);
    // … and what the tracing output has to hold for its silence to mean anything.
    let stderr_shows_fields = stderr.contains(CONTROL);

    report::record(
        "crash_files",
        json!({
            "status": report::status(
                panic_recorded
                    && traffic_recorded
                    && control_line_recorded
                    && control_value_dropped
                    && stderr_shows_fields
                    && in_file.is_empty()
                    && in_stderr.is_empty()
            ),
            "values_searched": forbidden.len(),
            "values_in_crash_file": in_file.len(),
            "values_in_tracing_output": in_stderr.len(),
            "recent_lines_carry_the_session": traffic_recorded,
            "control_value_dropped_from_crash_file": control_value_dropped,
            "control_value_in_tracing_output": stderr_shows_fields,
        }),
    );

    assert!(
        panic_recorded,
        "the crash file does not name the controlled panic:\n{file}"
    );
    assert!(
        traffic_recorded,
        "the crash file holds no line of the traffic before the panic, so the absence of \
         values in it proves nothing:\n{file}"
    );
    assert!(
        control_line_recorded,
        "the control line is not among the recent lines of the crash file:\n{file}"
    );
    assert!(
        stderr_shows_fields,
        "the tracing output does not show a field it was given, so reading it proves \
         nothing:\n{stderr}"
    );
    assert!(
        control_value_dropped,
        "the crash ring kept the value of a field it must drop (`note`):\n{file}"
    );
    assert!(
        in_file.is_empty(),
        "the crash file carries a value:\n{}\n\n{file}",
        in_file.join("\n")
    );
    assert!(
        in_stderr.is_empty(),
        "the tracing output carries a value (R-19):\n{}",
        in_stderr.join("\n")
    );
}

/// The one crash file the controlled panic wrote.
fn the_crash_file(dir: &Path) -> String {
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{}: {error}: the panic hook wrote nothing", dir.display()))
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter(|text| text.contains(CONTROLLED_PANIC))
        .collect();
    assert_eq!(
        files.len(),
        1,
        "{} crash files in {} name the controlled panic",
        files.len(),
        dir.display()
    );
    files.swap_remove(0)
}

/// The child: what `run()` installs, real traffic, the control line, and a panic.
fn crash_after_traffic() {
    let dir = PathBuf::from(
        std::env::var_os(WORKSPACE).expect("BATON_SECURITY_DIR is set by the parent"),
    );
    // What `run()` installs first: the stderr subscriber, the crash ring and the panic hook.
    let _recent = crash::install(dir.join("app-data"));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(traffic(&dir));

    // Last before the panic, so it is certainly among the fifty lines the file keeps.
    tracing::info!(note = CONTROL, "{CONTROL_MESSAGE}");
    let _ = std::thread::spawn(|| panic!("{CONTROLLED_PANIC}")).join();
    println!("{}", child::finished(TEST));
}

/// A session registers, opens a handoff carrying a planted secret in every field the
/// ingress scan covers, the user notes and finishes, the agent reports.
async fn traffic(dir: &Path) {
    let database = dir.join("handoff.sqlite");
    // The app's own wiring (`lib.rs`), with no window: two connections to one file, the
    // registry on the dispatch's, everything else on the store's.
    let registry_db = Db::open_at(&database).expect("the database opens");
    let store_db = Db::open_at(&database).expect("the database opens twice");
    let registry = Registry::open(&registry_db, Box::new(NoObserver)).expect("a registry");
    let queue = Arc::new(Queue::new(Box::new(NoRequestObserver)));
    let store = Store::load(
        store_db,
        Box::new(NoRunbookSink),
        Box::new(Arc::clone(&queue)),
    )
    .expect("a store");
    let (store, deliveries) = store::spawn(store);
    let endpoint = private_endpoint(dir);
    let token = Token::generate();
    let (handle, events) = listen(ListenerConfig::new(endpoint.clone(), token.clone()))
        .await
        .expect("the listener binds");
    let dispatch = Dispatch::new(
        registry_db,
        Arc::new(Mutex::new(registry)),
        store.clone(),
        handle.clone(),
        Arc::clone(&queue),
    );
    tokio::spawn(dispatch.run(events, deliveries));

    let mut server = FakeServer::register(&endpoint, &token).await;
    server
        .send(json!({
            "jsonrpc": "2.0", "id": 2, "method": "handoff.open",
            "params": {
                "call_id": "call_5ec0r1ty",
                "spec": {
                    "spec_version": 1,
                    "goal": format!("Rotate {} on the dashboard", forbidden::GOAL),
                    "where": format!("Stripe account {}", forbidden::WHERE),
                    "why_human": format!("only a person may read {}", forbidden::WHY_HUMAN),
                    "values": {
                        "api_key": forbidden::VALUE,
                        "backups": ["an ordinary item", forbidden::ITEM]
                    },
                    "steps": [
                        {
                            "text": format!("paste {} into .env", forbidden::STEP),
                            "values": ["api_key"],
                            "warning": format!("never commit {}", forbidden::WARNING)
                        }
                    ],
                    "verify": format!("call the API with {}", forbidden::VERIFY),
                    "lang": "en"
                },
                "secret_treated": [],
                "request_id": null
            }
        }))
        .await;
    let answer = server.expect("the handoff id").await;
    let id = answer["result"]["handoff_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the app answered the open with {answer}"))
        .to_owned();

    store
        .user(
            id.clone(),
            UserAction::Note(format!("pasted {} where it said", forbidden::NOTE)),
            Timestamp::now(),
        )
        .await
        .expect("the note is accepted");
    store
        .user(id.clone(), UserAction::Done, Timestamp::now())
        .await
        .expect("done is accepted");
    server.expect("the awaiting_verification event").await;
    server
        .send(json!({
            "jsonrpc": "2.0", "id": 3, "method": "handoff.verify",
            "params": { "handoff_id": id, "verify": {
                "ok": true,
                "detail": format!("the call with {} succeeded", forbidden::DETAIL)
            } }
        }))
        .await;
    server.expect("the verified outcome").await;

    handle.shutdown("the security suite is done").await;
}
