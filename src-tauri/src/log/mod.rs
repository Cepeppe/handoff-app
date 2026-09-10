//! The local log (§7.11, LOG-01..05, NET-01).
//!
//! SQLite schema, migrations, queries, export and deletion, in the app data directory. It
//! records what was sent, when and to which session, with hashes rather than pixels, and
//! it is the only place a past handoff can be read from. Nothing here ever leaves the
//! machine.
//!
//! # It is also where the state lives (DD-31)
//!
//! The name says "log" and the table list says more: `handoffs.state_json` is the store's
//! own state, in the same database and therefore in the same transaction as the row that
//! records the transition. That is DD-31, and it is what makes NFR-12 ("an app restart, an
//! interrupted call, a dead server all resume from the same state") a property of the file
//! rather than of a careful ordering of writes.
//!
//! # The two rules that shape this module
//!
//! - **No SQL outside `log/`.** Every table is reached through the typed functions of a
//!   module named after it; [`db::Db`] hands its connection out only within this module.
//!   The rest of the crate passes rows, never statements, so a schema change is a change
//!   here and a compile error everywhere it matters.
//! - **No secret-treated value is ever written.** LOG-02 and DET-04 say the log keeps a
//!   placeholder; [`redact`] applies it and [`handoffs::upsert`] is where it happens, so a
//!   caller cannot forget. `tests/security/log_invariants.rs` is the check, and
//!   [`test_support::dump_all_text`] is what it checks with.
//!
//! # Reading order
//!
//! [`db`] opens the file and migrates it; [`time`] is the one shape an instant has;
//! [`redact`] is the masking; then one module per table; [`transitions`] commits the rows
//! of one move of the state machine together (DD-31); [`maintenance`] is "delete
//! everything" and "export everything" (LOG-04).

pub mod db;
pub mod error;
pub mod events;
pub mod handoffs;
pub mod hook_blocks;
pub mod maintenance;
pub mod network_events;
pub mod redact;
pub mod rounds;
pub mod sends;
pub mod sessions;
pub mod settings;
pub mod test_support;
pub mod time;
pub mod transitions;
pub mod user_requests;

pub use db::Db;
pub use error::{PersistenceCause, Result, StoreError};
pub use handoffs::{HandoffRow, HandoffState};
pub use time::Timestamp;
pub use transitions::Transition;

/// Fixtures shared by the unit tests of this module.
///
/// One place to build a valid row of each table, so a test says what it is about instead of
/// restating fourteen columns. Everything here is deliberately boring; a test that needs an
/// interesting value sets it after calling.
#[cfg(test)]
pub(crate) mod testing {
    use std::fs;
    use std::path::{Path, PathBuf};

    use indexmap::IndexMap;

    use super::events::{EventKind, EventRow};
    use super::handoffs::{HandoffRow, HandoffState};
    use super::network_events::NetworkEventRow;
    use super::rounds::RoundRow;
    use super::sends::{SendKind, SendRow};
    use super::sessions::SessionRow;
    use super::user_requests::UserRequestRow;
    use super::{events, handoffs, hook_blocks, network_events, rounds, sends, sessions};
    use super::{Db, Timestamp};
    use crate::format::spec::{HandoffSpec, HandoffStep, SpecValue};

    /// A value the certain-secret patterns match as `api_key`, used wherever a test needs a
    /// secret that is unmistakably one.
    pub(crate) const STRIPE_KEY: &str = "sk_live_0123456789abcdefghij";

    /// A fixed instant, so a test never depends on the clock.
    pub(crate) fn at(text: &str) -> Timestamp {
        Timestamp::parse(text).expect("rfc 3339")
    }

    pub(crate) fn handoff(id: &str) -> HandoffRow {
        HandoffRow {
            id: id.to_owned(),
            created_at: at("2026-09-08T11:00:00Z"),
            closed_at: None,
            session_ref: None,
            agent_id: Some("claude-code".to_owned()),
            client_name: Some("claude-code".to_owned()),
            project_dir: Some("C:\\projects\\baton".to_owned()),
            request_text: None,
            state: HandoffState::Active,
            final_state: None,
            spec: None,
            state_json: "{\"cursor\":{\"round\":1,\"step_index\":1}}".to_owned(),
            delivered_at: None,
            resumed_from_json: None,
            lang: Some("en".to_owned()),
        }
    }

    pub(crate) fn round(handoff_id: &str, no: i64) -> RoundRow {
        RoundRow {
            handoff_id: handoff_id.to_owned(),
            no,
            steps_json: "[{\"text\":\"open the dashboard\"}]".to_owned(),
            started_at: at("2026-09-08T11:00:00Z"),
            ended_at: None,
            verify_ok: None,
            verify_detail: None,
            verify_reported_at: None,
            verify_late: false,
        }
    }

    pub(crate) fn event(handoff_id: &str, kind: EventKind) -> EventRow {
        EventRow {
            id: 0,
            handoff_id: handoff_id.to_owned(),
            round: Some(1),
            at: at("2026-09-08T11:00:00Z"),
            kind,
            step_index: None,
            payload_json: Some("{}".to_owned()),
        }
    }

    pub(crate) fn send(handoff_id: &str, kind: SendKind) -> SendRow {
        SendRow {
            id: 0,
            handoff_id: handoff_id.to_owned(),
            at: at("2026-09-08T11:00:00Z"),
            kind,
            text_as_sent: Some("the dashboard shows the key".to_owned()),
            image_sha256: Some("0".repeat(64)),
            image_w: Some(1600),
            image_h: Some(900),
            redaction_boxes_json: Some("[]".to_owned()),
            ocr_engine: Some("ocrs".to_owned()),
            patterns_version: Some("1".to_owned()),
        }
    }

    pub(crate) fn session(session_ref: &str) -> SessionRow {
        SessionRow {
            session_ref: session_ref.to_owned(),
            agent_id: Some("claude-code".to_owned()),
            client_name: Some("claude-code".to_owned()),
            client_version: Some("2.1.263".to_owned()),
            pid_chain_json: "[4242,1212]".to_owned(),
            cwd: Some("C:\\projects\\baton".to_owned()),
            project_dir: Some("C:\\projects\\baton".to_owned()),
            claude_session_id: None,
            connected: true,
            first_seen: at("2026-09-08T10:00:00Z"),
            last_seen: at("2026-09-08T11:00:00Z"),
        }
    }

    pub(crate) fn request(id: &str) -> UserRequestRow {
        UserRequestRow {
            id: id.to_owned(),
            session_ref: None,
            text: "I'm about to create the API key on Stripe".to_owned(),
            created_at: at("2026-09-08T11:00:00Z"),
            delivered_via: None,
            linked_handoff_id: None,
            about_handoff_id: None,
        }
    }

    /// A spec whose values, goal and step text all carry [`STRIPE_KEY`].
    pub(crate) fn spec_with_secret() -> HandoffSpec {
        let mut values = IndexMap::new();
        values.insert("api_key".to_owned(), SpecValue::One(STRIPE_KEY.to_owned()));
        HandoffSpec {
            spec_version: 1,
            goal: format!("rotate {STRIPE_KEY}"),
            r#where: "Stripe dashboard".to_owned(),
            url: None,
            why_human: "only a person can log in".to_owned(),
            values,
            secrets: None,
            steps: vec![HandoffStep {
                text: format!("paste {STRIPE_KEY} into .env"),
                url: None,
                values: Some(vec!["api_key".to_owned()]),
                warning: None,
            }],
            verify: Some("the webhook fires".to_owned()),
            lang: Some("en".to_owned()),
        }
    }

    /// An in-memory database with one of everything, and two handoffs so a test that
    /// deletes one has a control.
    pub(crate) fn populated() -> Db {
        let db = Db::open_in_memory().expect("a database");
        sessions::register(&db, &session("ses_00000001")).expect("a session");
        for id in ["hf_0000000001", "hf_0000000002"] {
            let mut row = handoff(id);
            row.session_ref = Some("ses_00000001".to_owned());
            handoffs::upsert(&db, &row).expect("a handoff");
            rounds::upsert(&db, &round(id, 1)).expect("a round");
            events::append(&db, &event(id, EventKind::Confirm)).expect("an event");
            sends::append(&db, &send(id, SendKind::Question)).expect("a send");
        }
        super::user_requests::upsert(&db, &request("hf_0000000003")).expect("a request");
        hook_blocks::record(
            &db,
            "ses_00000001",
            "deferred:hf_0000000001",
            &at("2026-09-08T11:30:00Z"),
        )
        .expect("a hook block");
        network_events::append(
            &db,
            &NetworkEventRow {
                id: 0,
                at: at("2026-09-08T11:00:00Z"),
                domain: "updates.example.test".to_owned(),
                bytes_sent: 128,
                purpose: "update check".to_owned(),
            },
        )
        .expect("a network event");
        db
    }

    /// A private temporary directory. The crate has no dependency for one, and the process
    /// id plus a counter keep two tests of the same run apart.
    pub(crate) fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "baton-log-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("a temporary directory");
        dir
    }

    /// Removes a temporary directory, ignoring a file the platform still holds open.
    pub(crate) fn clean(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }
}
