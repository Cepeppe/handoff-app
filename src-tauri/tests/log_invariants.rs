//! The log invariants (LOG-02, LOG-03, DET-04, §7.11).
//!
//! This suite asks the one question the log has to keep answering for ever: **is a value
//! the certain detector matched anywhere in the database?** It asks it from outside the
//! crate, through the public API, with [`log::test_support::dump_all_text`] — which reads
//! the table and column list from the file rather than from a list somebody maintains, so
//! a column added by a later migration is covered the day it exists.
//!
//! The values are unique sentinels: if one leaks, the failure says which field it came
//! from instead of "a secret was found".
//!
//! What is deliberately **not** asserted here: that `sends.text_as_sent` and
//! `events.payload_json` are masked. LOG-03 requires the send text to be "the full text
//! exactly as it left, after redaction" — it is the answer to "what did the agent actually
//! see", and a log that rewrote it would stop being that answer. The redaction happens
//! before the send, in the preview (PREV-01); this module guards the one place where a true
//! value would otherwise be written down, which is the spec and the state that travels with
//! it.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use handoff_app_lib::format::spec::{HandoffSpec, HandoffStep, SpecValue};
use handoff_app_lib::log::db::DATABASE_FILE_NAME;
use handoff_app_lib::log::handoffs::{HandoffRow, HandoffState};
use handoff_app_lib::log::test_support::dump_all_text;
use handoff_app_lib::log::{handoffs, sessions, Db, Timestamp};
use indexmap::IndexMap;

/// A sentinel that every build of the certain-secret pattern file matches as an `api_key`
/// (`stripe_secret_key`: `[sr]k_(?:live|test)_[0-9A-Za-z]{16,}`), and that no two callers
/// of this function share.
fn sentinel(index: u32) -> String {
    format!("sk_live_SENTINEL{index:016}")
}

fn tempdir() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "baton-log-invariants-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("a temporary directory");
    dir
}

/// A spec carrying a different sentinel in every field the ingress scan of §5.5 covers.
fn spec_of_sentinels() -> HandoffSpec {
    let mut values = IndexMap::new();
    values.insert("api_key".to_owned(), SpecValue::One(sentinel(1)));
    values.insert(
        "backups".to_owned(),
        SpecValue::Many(vec!["an ordinary item".to_owned(), sentinel(2)]),
    );
    HandoffSpec {
        spec_version: 1,
        goal: format!("rotate {} on the dashboard", sentinel(3)),
        r#where: format!("Stripe account {}", sentinel(4)),
        url: None,
        why_human: format!("only a person may read {}", sentinel(5)),
        values,
        secrets: None,
        steps: vec![HandoffStep {
            text: format!("paste {} into .env", sentinel(6)),
            url: None,
            values: Some(vec!["api_key".to_owned()]),
            warning: Some(format!("never commit {}", sentinel(7))),
        }],
        verify: Some(format!("call the API with {}", sentinel(8))),
        lang: Some("en".to_owned()),
    }
}

#[test]
fn no_sentinel_of_a_stored_handoff_appears_anywhere_in_the_database() {
    let db = Db::open_in_memory().expect("a database");
    sessions::register(&db, &session()).expect("a session");

    let mut row = handoff();
    row.session_ref = Some("ses_00000001".to_owned());
    row.spec = Some(spec_of_sentinels());
    // The state the store writes through carries the same spec (§7.4), and the request the
    // user typed may quote a value as well. Both are text the log cannot look inside, and
    // both are swept.
    row.state_json = format!(
        "{{\"spec\":{{\"values\":{{\"api_key\":\"{}\"}}}},\"undelivered\":[\"{}\"]}}",
        sentinel(1),
        sentinel(9)
    );
    row.request_text = Some(format!("rotate {} for me", sentinel(10)));
    handoffs::upsert(&db, &row).expect("a handoff");

    let dump = dump_all_text(&db).expect("a dump");
    for index in 1..=10 {
        assert!(
            !dump.contains(&sentinel(index)),
            "sentinel {index} reached the database:\n{dump}"
        );
    }
    // The masking has to have happened rather than the fields having been dropped: an
    // empty log would pass the assertion above.
    assert!(dump.contains("[treated as secret: api_key]"), "{dump}");
    assert!(
        dump.contains("paste [treated as secret: api_key] into .env"),
        "{dump}"
    );
    assert!(dump.contains("an ordinary item"), "{dump}");
}

#[test]
fn what_is_read_back_is_the_masked_spec_and_not_the_one_that_went_in() {
    // §5.5 keeps the true spec in memory for the copy button (DET-04); the database is the
    // one place it never is, so a restart cannot bring it back.
    let db = Db::open_in_memory().expect("a database");
    let mut row = handoff();
    row.spec = Some(spec_of_sentinels());
    handoffs::upsert(&db, &row).expect("a handoff");

    let read = handoffs::get(&db, "hf_0123456789")
        .expect("a read")
        .expect("a row");
    let spec = read.spec.expect("a spec");
    assert_eq!(
        spec.values["api_key"],
        SpecValue::One("[treated as secret: api_key]".to_owned())
    );
    assert_eq!(
        spec.values["backups"],
        SpecValue::Many(vec![
            "an ordinary item".to_owned(),
            "[treated as secret: api_key]".to_owned(),
        ])
    );
    assert_eq!(
        spec.steps[0].values.as_deref(),
        Some(["api_key".to_owned()].as_slice())
    );
}

#[test]
fn a_spec_with_nothing_to_hide_is_stored_exactly_as_it_arrived() {
    let db = Db::open_in_memory().expect("a database");
    let mut spec = spec_of_sentinels();
    spec.values.clear();
    spec.values.insert(
        "endpoint_url".to_owned(),
        SpecValue::One("https://example.test/hooks".to_owned()),
    );
    spec.goal = "create the webhook".to_owned();
    spec.r#where = "Stripe dashboard".to_owned();
    spec.why_human = "only a person can log in".to_owned();
    spec.steps[0].text = "open the dashboard".to_owned();
    spec.steps[0].warning = None;
    spec.verify = Some("the webhook fires".to_owned());

    let mut row = handoff();
    row.spec = Some(spec.clone());
    handoffs::upsert(&db, &row).expect("a handoff");

    let read = handoffs::get(&db, "hf_0123456789")
        .expect("a read")
        .expect("a row");
    assert_eq!(read.spec.expect("a spec"), spec);
}

#[test]
fn the_database_file_is_created_under_the_app_data_directory() {
    // The acceptance check of the task: `HANDOFF_APP_DATA_DIR` is the override tests use
    // (`TASKS.md` §0.4 item 4), and it is the only way a test may touch `open_app_data`
    // without writing into the real installation.
    let dir = tempdir();
    let nested = dir.join("not").join("created").join("yet");
    std::env::set_var("HANDOFF_APP_DATA_DIR", &nested);
    let db = Db::open_app_data().expect("a database in the app data directory");
    assert_eq!(db.path(), Some(nested.join(DATABASE_FILE_NAME).as_path()));
    assert!(nested.join(DATABASE_FILE_NAME).is_file());
    std::env::remove_var("HANDOFF_APP_DATA_DIR");
    drop(db);
    let _ = fs::remove_dir_all(&dir);
}

fn handoff() -> HandoffRow {
    HandoffRow {
        id: "hf_0123456789".to_owned(),
        created_at: Timestamp::parse("2026-09-08T11:00:00Z").expect("rfc 3339"),
        closed_at: None,
        session_ref: None,
        agent_id: Some("claude-code".to_owned()),
        client_name: Some("claude-code".to_owned()),
        project_dir: Some("C:\\projects\\baton".to_owned()),
        request_text: None,
        state: HandoffState::Active,
        final_state: None,
        spec: None,
        state_json: "{}".to_owned(),
        delivered_at: None,
        resumed_from_json: None,
        lang: Some("en".to_owned()),
    }
}

fn session() -> sessions::SessionRow {
    sessions::SessionRow {
        session_ref: "ses_00000001".to_owned(),
        agent_id: Some("claude-code".to_owned()),
        client_name: Some("claude-code".to_owned()),
        client_version: Some("2.1.263".to_owned()),
        pid_chain_json: "[4242,1212]".to_owned(),
        cwd: Some("C:\\projects\\baton".to_owned()),
        project_dir: Some("C:\\projects\\baton".to_owned()),
        claude_session_id: None,
        connected: true,
        first_seen: Timestamp::parse("2026-09-08T10:00:00Z").expect("rfc 3339"),
        last_seen: Timestamp::parse("2026-09-08T11:00:00Z").expect("rfc 3339"),
    }
}
