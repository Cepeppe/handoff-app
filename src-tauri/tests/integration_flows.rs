//! The flows of §9, driven headlessly: a real listener, a real store, no UI (§11.3).
//!
//! Everything below runs the app's own code — the listener of T-031, the registry of T-032,
//! the store of T-033 and the dispatch of T-034 — against `fake-server`, the double that
//! plays `handoff-mcp` over a real named pipe or Unix socket. The user is played by calling
//! the store's own API, which is what the overlay will call (§7.6) and what the e2e
//! automation channel will drive (DD-33, T-043).
//!
//! # What "message for message" means here, and where it stops
//!
//! Each flow is replayed from `vendor/handoff-mcp/format/fixtures/channel/*.jsonl` through
//! [`Replay`], and what crossed the socket is compared with the fixture modulo identifiers
//! and instants (`fake-server/golden.rs`). Where a comparison ignores a field, the ignore
//! is attached to the exact message and carries its reason. Three kinds of reason occur:
//!
//! - **it belongs to whichever peer is driving** — the channel token, the app's own
//!   `app_version`, the `project` label of a `resumed_from` (the app writes the folder's
//!   name, OPEN-02, where the fixture wrote a whole path);
//! - **the fixture is illustrative there and contradicts itself** — `notes` and
//!   `skipped_steps` are per round (§4.3, §7.4), and three fixtures show a final outcome
//!   with both emptied one message after the same round reported them full;
//! - **the fixture shows a later moment of the same field's life** — `already_delivered` is
//!   true in `f11` for an outcome this flow collects for the first time (TOOL-07).
//!
//! Two fixtures are replayed only in part, and the tests say which lines and why: `f07`
//! needs the user-request queue of T-038 and `f10` the hook decision of T-035.

#[path = "fake-server/mod.rs"]
mod fake_server;

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

use handoff_app_lib::channel::listener::{listen, ChannelHandle, ListenerConfig};
use handoff_app_lib::channel::token::Token;
use handoff_app_lib::channel::{Dispatch, Endpoint};
use handoff_app_lib::format::outcome::ScreenshotMode;
use handoff_app_lib::ids;
use handoff_app_lib::log::{Db, HandoffState, Timestamp};
use handoff_app_lib::sessions::{NoObserver, Registry};
use handoff_app_lib::store::{
    self, HandoffSnapshot, NoResumeRequests, NoRunbookSink, ScreenshotPayload, Store, StoreHandle,
    UserAction,
};

use fake_server::golden::{Golden, GoldenLine, Replay};
use fake_server::{FakeServer, PATIENCE};

// ---------------------------------------------------------------------------------------
// The app, headless
// ---------------------------------------------------------------------------------------

/// A running app: the listener, the registry, the store and the dispatch that joins them.
struct App {
    handle: ChannelHandle,
    store: StoreHandle,
    endpoint: Endpoint,
    token: Token,
    /// Holds the database, and the socket where there is one. Removed on drop, unless a
    /// second app is going to be started over it.
    dir: PathBuf,
    removes_the_directory: bool,
}

impl App {
    /// A whole app on a private endpoint and a private database.
    async fn start() -> Self {
        Self::start_in(temp_dir(), |_| {}).await
    }

    /// The same, with the listener's timings shortened, or restarted over an existing
    /// database (FM-13).
    async fn start_in(dir: PathBuf, tweak: impl FnOnce(&mut ListenerConfig)) -> Self {
        let database = dir.join("handoff.sqlite");
        // Two connections to one file, as `log::db` documents: the registry writes
        // `sessions` from the dispatch task, the store writes everything else from its own.
        let registry_db = Db::open_at(&database).expect("the database opens");
        let store_db = Db::open_at(&database).expect("the database opens twice");
        let registry = Registry::open(&registry_db, Box::new(NoObserver)).expect("a registry");
        let store = Store::load(
            store_db,
            Box::new(NoRunbookSink),
            Box::new(NoResumeRequests),
        )
        .expect("a store");
        let (store, deliveries) = store::spawn(store);

        let endpoint = private_endpoint(&dir);
        let token = Token::generate();
        let mut config = ListenerConfig::new(endpoint.clone(), token.clone());
        tweak(&mut config);
        let (handle, events) = listen(config).await.expect("the listener binds");

        let dispatch = Dispatch::new(registry_db, registry, store.clone(), handle.clone());
        tokio::spawn(dispatch.run(events, deliveries));

        Self {
            handle,
            store,
            endpoint,
            token,
            dir,
            removes_the_directory: true,
        }
    }

    /// Leaves the directory behind when this app is dropped.
    ///
    /// The restart of FM-13 is two apps over one database, and the first one's `Drop` would
    /// otherwise take the file with it. It does on macOS, where a temporary directory whose
    /// files are still open is removed without complaint, and it does not on Windows, where
    /// the removal fails and the test passes for the wrong reason.
    fn keep_the_directory(&mut self) {
        self.removes_the_directory = false;
    }

    /// A `fake-server` connected and registered as a session.
    async fn session(&self) -> FakeServer {
        FakeServer::register(&self.endpoint, &self.token).await
    }

    /// One of the actions a person takes in the overlay.
    async fn user(&self, handoff_id: &str, action: UserAction) {
        self.store
            .user(handoff_id.to_owned(), action, Timestamp::now())
            .await
            .expect("the overlay action is accepted");
    }

    /// The tab as the overlay would draw it.
    async fn tab(&self, handoff_id: &str) -> HandoffSnapshot {
        self.store
            .snapshot(handoff_id.to_owned(), Timestamp::now())
            .await
            .expect("the handoff is in the store")
    }

    /// Waits for a condition of the app's own state.
    async fn wait_for(
        &self,
        what: &str,
        mut ready: impl FnMut(&HandoffSnapshot) -> bool,
        id: &str,
    ) {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            if ready(&self.tab(id).await) {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{what} never happened"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// The `sessions` row of the registry, read back from the database.
    fn session_row(&self, session_ref: &str) -> handoff_app_lib::log::sessions::SessionRow {
        let db = Db::open_at(self.dir.join("handoff.sqlite")).expect("the database opens");
        handoff_app_lib::log::sessions::get(&db, session_ref)
            .expect("the row is readable")
            .expect("the session was registered")
    }

    async fn stop(&self) {
        self.handle.shutdown("the test ended").await;
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Best effort: the store's connection may still hold the file open on Windows, and
        // a temporary directory left behind is not worth failing a green test over.
        if self.removes_the_directory {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// An endpoint of this test's own, so that tests run in parallel and never touch the real
/// one.
fn private_endpoint(dir: &std::path::Path) -> Endpoint {
    let unique = short_id();
    if cfg!(windows) {
        Endpoint::Pipe {
            name: format!(r"\\.\pipe\hf-{}-{unique}", std::process::id()),
        }
    } else {
        // One socket per app, not one per directory: the restart of FM-13 puts two apps in
        // one directory and the second would meet the first's file. Short, because
        // `sun_path` is 104 bytes and a macOS temporary directory is already fifty of them
        // (§5.8, FM-12).
        Endpoint::Unix {
            path: dir.join(format!("{unique}.sock")),
            pointer: None,
        }
    }
}

/// Eight characters, unique enough for one test's directory or socket.
fn short_id() -> String {
    ids::new_session_ref()
        .strip_prefix("ses_")
        .expect("a session ref is prefixed")
        .to_owned()
}

// ---------------------------------------------------------------------------------------
// Reading a fixture, so that nothing is transcribed
// ---------------------------------------------------------------------------------------

/// The `user_text` an outcome of the fixture carries: what the person typed.
fn user_text(line: &GoldenLine) -> String {
    line.message["params"]["outcome"]["user_text"]
        .as_str()
        .or_else(|| line.message["result"]["outcome"]["user_text"].as_str())
        .expect("the fixture's outcome carries a user text")
        .to_owned()
}

/// The first note of an outcome of the fixture.
fn note_text(line: &GoldenLine) -> String {
    line.message["params"]["outcome"]["notes"][0]["text"]
        .as_str()
        .expect("the fixture's outcome carries a note")
        .to_owned()
}

/// The screenshot the fixture shows, as the preview would hand it to the store.
fn screenshot_payload(line: &GoldenLine) -> ScreenshotPayload {
    let shot = &line.message["params"]["outcome"]["screenshot"];
    assert_eq!(shot["mode"], json!("text"), "this fixture sends text");
    ScreenshotPayload {
        mode: ScreenshotMode::Text,
        text: Some(
            shot["text"]
                .as_str()
                .expect("the extracted text")
                .to_owned(),
        ),
        image_base64: None,
        image_sha256: None,
        width: u32::try_from(shot["width"].as_u64().expect("a width")).expect("a width"),
        height: u32::try_from(shot["height"].as_u64().expect("a height")).expect("a height"),
        redactions: u32::try_from(shot["redactions"].as_u64().expect("a count")).expect("a count"),
        redaction_boxes_json: None,
        ocr_engine: Some(shot["ocr_engine"].as_str().expect("the engine").to_owned()),
        patterns_version: None,
        comment: Some(user_text(line)),
    }
}

/// The handoff id a fixture invented, read from the answer to its `handoff.open`.
fn fixture_handoff_id(golden: &Golden) -> String {
    golden
        .lines
        .iter()
        .find_map(|line| line.message["result"]["handoff_id"].as_str())
        .expect("the fixture opens a handoff")
        .to_owned()
}

/// The token a fixture's `hello` carries. Aliased to the live one so a replay authenticates.
fn fixture_token(fixture: &str) -> String {
    fake_server::hello_of(fixture)["params"]["token"]
        .as_str()
        .expect("a hello carries a token")
        .to_owned()
}

/// The `handoff_id` the app answered a `handoff.open` with.
fn opened(answer: &Value) -> String {
    answer["result"]["handoff_id"]
        .as_str()
        .expect("the app answered with a handoff id")
        .to_owned()
}

/// The four steps of the fixtures' spec, walked as the fixtures' outcomes report them: a
/// note on the first step, the first two confirmed, the third skipped, done on the fourth.
async fn walk_the_four_steps(app: &App, id: &str, note: &str) {
    app.user(id, UserAction::Note(note.to_owned())).await;
    app.user(id, UserAction::Confirm).await;
    app.user(id, UserAction::Confirm).await;
    app.user(id, UserAction::Skip).await;
    app.user(id, UserAction::Done).await;
}

/// The keys a comparison ignores because their value belongs to the app and not to the flow.
const THE_APPS_OWN: [&str; 2] = ["app_version", "token"];

/// The keys three fixtures contradict themselves on: `notes` and `skipped_steps` are per
/// round (§4.3 "current round", §7.4), and a fixture that reports them full in one message
/// and empty in the next — for the same round — describes no implementation.
const THE_FIXTURES_EMPTIED_COUNTERS: [&str; 2] = ["notes", "skipped_steps"];

// ---------------------------------------------------------------------------------------
// F-01 · Session start and registration
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn f01_a_session_registers_pings_both_ways_and_is_told_the_app_is_leaving() {
    // The fixture's own ping interval is 30 s (§6.3); shortened here so the app's half of
    // the exchange happens inside a test rather than inside a coffee break.
    let app = App::start_in(temp_dir(), |config| {
        config.ping_interval = Duration::from_millis(150);
    })
    .await;
    let golden = Golden::load("f01-register");
    let reason = golden
        .lines
        .iter()
        .find_map(|line| line.message["params"]["reason"].as_str())
        .expect("the fixture ends with an app.shutdown")
        .to_owned();

    let mut fake = FakeServer::connect(&app.endpoint).await;
    let mut replay = Replay::new(golden);
    replay.alias(&fixture_token("f01-register"), app.token.as_str());

    replay.send_next(&mut fake).await; // hello
    replay.expect_next(&mut fake).await; // the session_ref
    replay.send_next(&mut fake).await; // the server pings
    replay.expect_next(&mut fake).await; // {}
    replay.expect_next(&mut fake).await; // the app pings after its silence budget
    replay.send_next(&mut fake).await; // {}

    app.handle.shutdown(&reason).await;
    replay.expect_next(&mut fake).await; // app.shutdown
    fake.expect_closed().await;

    replay.assert_finished();
    replay.assert_transcript(&fake, &THE_APPS_OWN);
}

#[tokio::test]
async fn a_peer_with_the_wrong_token_is_refused_exactly_as_the_fixture_says() {
    // The fixture's token is deliberately not this installation's, so nothing is aliased
    // and nothing is ignored: this sequence is reproducible to the byte.
    let app = App::start().await;
    let mut fake = FakeServer::connect(&app.endpoint).await;
    let mut replay = Replay::new(Golden::load("auth-failed"));

    replay.send_next(&mut fake).await;
    replay.expect_next(&mut fake).await;
    fake.expect_closed().await;

    replay.assert_finished();
    replay.assert_transcript(&fake, &[]);
}

#[tokio::test]
async fn a_peer_speaking_another_protocol_version_is_told_which_one_this_app_speaks() {
    // FM-11: the version is checked before the token, so a peer wrong about both is told to
    // update rather than sent looking for a broken installation.
    let app = App::start().await;
    let mut fake = FakeServer::connect(&app.endpoint).await;
    let mut replay = Replay::new(Golden::load("protocol-mismatch"));

    replay.send_next(&mut fake).await;
    replay.expect_next(&mut fake).await;
    fake.expect_closed().await;

    replay.assert_finished();
    replay.assert_transcript(&fake, &[]);
}

// ---------------------------------------------------------------------------------------
// F-02 · Agent-opened handoff, happy path to verified
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn f02_an_opened_handoff_is_guided_reported_and_verified() {
    let app = App::start().await;
    let mut fake = app.session().await;
    fake.mark();

    let golden = Golden::load("f02-happy-path");
    let note = note_text(&golden.lines[2]);
    let mut replay = Replay::new(golden);

    replay.send_next(&mut fake).await; // handoff.open
    let id = opened(&replay.expect_next(&mut fake).await);

    walk_the_four_steps(&app, &id, &note).await;
    replay.expect_next(&mut fake).await; // handoff.event awaiting_verification

    replay.send_next(&mut fake).await; // handoff.verify ok:true
    replay.expect_next(&mut fake).await; // the verified outcome

    replay.assert_finished();
    // Everything up to the report is reproducible exactly, counters included.
    replay.assert_transcript_range(&fake, 0..3, &[], "up to the verification request");
    // The fixture empties `notes` and `skipped_steps` on the final outcome although the
    // same round reported them one message earlier; §4.3 says they are the round's.
    replay.assert_transcript_range(
        &fake,
        3..5,
        &THE_FIXTURES_EMPTIED_COUNTERS,
        "the final outcome",
    );

    assert_eq!(app.tab(&id).await.state, HandoffState::Verified);
    app.stop().await;
}

// ---------------------------------------------------------------------------------------
// F-04 · Ask and screenshot round trip
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn f04_a_question_and_a_screenshot_are_answered_on_the_step_they_were_raised_on() {
    let app = App::start().await;
    let mut fake = app.session().await;
    fake.mark();

    let golden = Golden::load("f04-ask-reply");
    let question = user_text(&golden.lines[2]);
    let screenshot = screenshot_payload(&golden.lines[5]);
    let mut replay = Replay::new(golden);

    replay.send_next(&mut fake).await; // handoff.open
    let id = opened(&replay.expect_next(&mut fake).await);

    // The fixture raises both on step 2.
    app.user(&id, UserAction::Confirm).await;
    app.user(&id, UserAction::Ask(question)).await;
    replay.expect_next(&mut fake).await; // handoff.event question

    replay.send_next(&mut fake).await; // handoff.continue with the reply
    replay.expect_next(&mut fake).await; // {ok: true}

    app.user(&id, UserAction::Screenshot(Box::new(screenshot)))
        .await;
    replay.expect_next(&mut fake).await; // handoff.event screenshot

    replay.send_next(&mut fake).await; // handoff.continue
    replay.expect_next(&mut fake).await; // {ok: true}

    replay.assert_finished();
    replay.assert_transcript_range(&fake, 0..5, &[], "the question and its reply");
    // The fixture's screenshot outcome carries a note on step 1 that its own question
    // outcome, one message earlier and on the same round, does not.
    replay.assert_transcript_range(&fake, 5..8, &["notes"], "the screenshot and its reply");

    let tab = app.tab(&id).await;
    assert_eq!(tab.state, HandoffState::Active);
    assert!(tab.pending_question.is_none(), "the reply cleared it");
    app.stop().await;
}

// ---------------------------------------------------------------------------------------
// F-05 · Defer, resume, second deferral, resume from the overlay
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn f05_a_deferred_handoff_comes_back_active_and_a_second_deferral_parks_it() {
    let app = App::start().await;
    let mut fake = app.session().await;
    fake.mark();

    let golden = Golden::load("f05-defer-park");
    let first = user_text(&golden.lines[2]);
    let second = user_text(&golden.lines[5]);
    let mut replay = Replay::new(golden);

    replay.send_next(&mut fake).await; // handoff.open
    let id = opened(&replay.expect_next(&mut fake).await);

    app.user(&id, UserAction::Confirm).await;
    app.user(&id, UserAction::Defer(Some(first))).await;
    replay.expect_next(&mut fake).await; // handoff.event deferred

    replay.send_next(&mut fake).await; // handoff.resume with a new call
    replay.expect_next(&mut fake).await; // {state: active} — §8.1, the agent came back

    app.user(&id, UserAction::Defer(Some(second))).await;
    replay.expect_next(&mut fake).await; // handoff.event parked

    replay.assert_finished();
    replay.assert_transcript(&fake, &[]);

    assert_eq!(app.tab(&id).await.state, HandoffState::Parked);

    // RESP-07 and FM-31: a parked handoff is picked up by the user, not by the agent. The
    // request that brings the agent back is T-035's queue; what belongs here is the state.
    app.user(&id, UserAction::ResumeFromOverlay).await;
    assert_eq!(app.tab(&id).await.state, HandoffState::Active);
    app.stop().await;
}

// ---------------------------------------------------------------------------------------
// F-06 · Heartbeat and resume
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn f06_a_detached_call_is_replaced_by_the_one_that_resumes() {
    let app = App::start().await;
    let mut fake = app.session().await;
    fake.mark();

    let golden = Golden::load("f06-heartbeat-resume");
    let note = note_text(&golden.lines[5]);
    let mut replay = Replay::new(golden);

    replay.send_next(&mut fake).await; // handoff.open
    let id = opened(&replay.expect_next(&mut fake).await);

    replay.send_next(&mut fake).await; // handoff.detach_call, heartbeat
    app.wait_for("the detach", |tab| !tab.call_attached, &id)
        .await;

    replay.send_next(&mut fake).await; // handoff.resume with the new call
    replay.expect_next(&mut fake).await; // {state: active}

    walk_the_four_steps(&app, &id, &note).await;
    replay.expect_next(&mut fake).await; // handoff.event, on the call that resumed

    replay.assert_finished();
    replay.assert_transcript(&fake, &[]);
    app.stop().await;
}

// ---------------------------------------------------------------------------------------
// F-08 · Failed verification and correction round
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn f08_a_failed_report_is_corrected_by_replacement_steps_in_a_new_round() {
    let app = App::start().await;
    let mut fake = app.session().await;
    fake.mark();

    let golden = Golden::load("f08-failed-correction");
    let note = note_text(&golden.lines[2]);
    let mut replay = Replay::new(golden);

    replay.send_next(&mut fake).await; // handoff.open
    let id = opened(&replay.expect_next(&mut fake).await);

    walk_the_four_steps(&app, &id, &note).await;
    replay.expect_next(&mut fake).await; // handoff.event awaiting_verification

    replay.send_next(&mut fake).await; // handoff.verify ok:false
    replay.expect_next(&mut fake).await; // the failed outcome

    replay.send_next(&mut fake).await; // handoff.continue with replacement_steps
    replay.expect_next(&mut fake).await; // {ok: true}

    // VER-09: round 2 restarts the counter, two steps this time.
    let tab = app.tab(&id).await;
    assert_eq!((tab.round, tab.step_index, tab.step_total), (2, 1, 2));
    app.user(&id, UserAction::Confirm).await;
    app.user(&id, UserAction::Done).await;
    replay.expect_next(&mut fake).await; // handoff.event awaiting_verification, round 2

    replay.send_next(&mut fake).await; // handoff.verify ok:true
    replay.expect_next(&mut fake).await; // the verified outcome

    replay.assert_finished();
    replay.assert_transcript_range(&fake, 0..4, &[], "the first round and its report");
    replay.assert_transcript_range(
        &fake,
        4..5,
        &THE_FIXTURES_EMPTIED_COUNTERS,
        "the failed outcome",
    );
    replay.assert_transcript_range(&fake, 5..7, &[], "the correction");
    // The fixture carries round 1's note into round 2; the counters are per round (§7.4).
    replay.assert_transcript_range(&fake, 7..8, &["notes"], "the second round's report");
    replay.assert_transcript_range(&fake, 8..10, &[], "the verified outcome");

    assert_eq!(app.tab(&id).await.state, HandoffState::Verified);
    app.stop().await;
}

// ---------------------------------------------------------------------------------------
// F-11 · Detach, disconnect, transfer
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn f11_a_resume_from_another_session_takes_the_call_over() {
    // FM-25 and the first six lines of the fixture, all of which belong to session 1: open,
    // cancel, resume, and the `transferred_to_other_session` that session 2's resume causes.
    let app = App::start().await;
    let mut one = app.session().await;
    one.mark();

    let mut replay = Replay::new(Golden::load("f11-transfer").subset(&[0, 1, 2, 3, 4, 5]));

    replay.send_next(&mut one).await; // handoff.open
    let id = opened(&replay.expect_next(&mut one).await);

    // The fixture's transferred outcome reports step 2, so the user has confirmed one step.
    app.user(&id, UserAction::Confirm).await;
    replay.send_next(&mut one).await; // handoff.detach_call, cancelled
    app.wait_for("the cancellation", |tab| !tab.call_attached, &id)
        .await;
    replay.send_next(&mut one).await; // handoff.resume, same session
    replay.expect_next(&mut one).await; // {state: active}

    // Session 2 resumes the same handoff: TOOL-08 lets any session of the installation do
    // it, and the call that was listening is told (FM-25).
    let mut two = app.session().await;
    two.mark();
    two.send(json!({
        "jsonrpc": "2.0", "id": 40, "method": "handoff.resume",
        "params": { "call_id": "call_8t4r6m1p", "handoff_id": id }
    }))
    .await;
    let taken_over = two.expect("the snapshot session 2 resumed into").await;
    assert_eq!(taken_over["result"]["state"], json!("active"));
    assert_eq!(
        taken_over["result"]["resumed_from"]["agent"],
        json!("Claude Code"),
        "TOOL-08: the outcome says which session opened it"
    );

    replay.expect_next(&mut one).await; // handoff.event transferred_to_other_session
    replay.assert_finished();
    replay.assert_transcript(&one, &[]);
    app.stop().await;
}

#[tokio::test]
async fn f11_a_session_that_leaves_ends_the_verification_and_a_later_report_is_late() {
    // FM-08, VER-06, DD-16 and FM-26, and the last five lines of the fixture, all of which
    // belong to session 2. Session 1 opens the handoff and goes away without reporting.
    let app = App::start().await;
    let golden = Golden::load("f11-transfer");
    let fixture_id = fixture_handoff_id(&golden);

    let id = {
        let mut one = app.session().await;
        one.mark();
        let mut opening = Replay::new(golden.subset(&[0, 1]));
        opening.send_next(&mut one).await;
        let id = opened(&opening.expect_next(&mut one).await);
        opening.assert_finished();
        opening.assert_transcript(&one, &[]);

        app.user(&id, UserAction::Confirm).await;
        app.user(&id, UserAction::Confirm).await;
        app.user(&id, UserAction::Confirm).await;
        app.user(&id, UserAction::Done).await;
        app.wait_for(
            "the verification window opening",
            |tab| tab.state == HandoffState::AwaitingVerification,
            &id,
        )
        .await;
        id
        // `one` is dropped here: the socket closes and the session is gone.
    };
    app.wait_for(
        "the pessimistic close of VER-06",
        |tab| tab.state == HandoffState::NotVerified,
        &id,
    )
    .await;

    let mut two = app.session().await;
    two.mark();
    let mut replay = Replay::new(golden.subset(&[6, 7, 8, 9, 10]));
    replay.alias(&fixture_id, &id);

    replay.send_next(&mut two).await; // handoff.resume from the new session
    replay.expect_next(&mut two).await; // the not_verified outcome, with resumed_from
    replay.send_next(&mut two).await; // handoff.verify ok:true, late
    replay.expect_next(&mut two).await; // the verified outcome
    replay.send_next(&mut two).await; // session.bye
    two.expect_closed().await;

    replay.assert_finished();
    // `already_delivered`: the fixture shows a second collection (TOOL-07); this flow is the
    // first, because the outcome was queued by a disconnect nobody was listening to.
    // `project`: the app writes the project folder's name (OPEN-02), the fixture a path.
    replay.assert_transcript(&two, &["already_delivered", "project"]);

    assert_eq!(app.tab(&id).await.state, HandoffState::Verified);
    app.stop().await;
}

// ---------------------------------------------------------------------------------------
// The two fixtures that are replayed in part
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn f07_a_spec_naming_a_request_takes_that_request_s_id() {
    // DD-13, and the first two lines of the fixture. The rest of F-07 — the shortcut, the
    // request queue, the clipboard and the tab that was already waiting — is T-038's, and
    // the fixture's `confirmed_by_user` needs a spec without `verify`, which this one has.
    let app = App::start().await;
    let mut fake = app.session().await;
    fake.mark();

    let golden = Golden::load("f07-user-request");
    let mut replay = Replay::new(golden.subset(&[0, 1]));
    replay.send_next(&mut fake).await;
    replay.expect_next(&mut fake).await;

    replay.assert_finished();
    replay.assert_transcript(&fake, &[]);
    app.stop().await;
}

#[tokio::test]
async fn f10_a_hook_is_bound_answered_once_and_closed() {
    // §6.2 and the first three lines of the fixture. The answer is neutral: which handoffs
    // are worth blocking for is T-035, and the fixture's fourth line is its block.
    let app = App::start().await;
    let _session = app.session().await;

    let mut hook = FakeServer::connect(&app.endpoint).await;
    let golden = Golden::load("f10-hook-block");
    let mut replay = Replay::new(golden.subset(&[0, 1, 2]));
    replay.alias(&fixture_token("f10-hook-block"), app.token.as_str());

    replay.send_next(&mut hook).await; // hello, role hook
    let welcome = replay.expect_next(&mut hook).await;
    assert_eq!(
        welcome["result"]["session_ref"],
        Value::Null,
        "a hook registers no session"
    );
    replay.send_next(&mut hook).await; // hook.stop

    let decision = hook.expect("the hook decision").await;
    assert_eq!(
        decision["result"]["block"],
        json!(false),
        "neutral until T-035"
    );
    assert_eq!(decision["result"].get("reason"), None);
    // §6.2: one question, one answer, then the connection goes — and the close is the
    // dispatch's to ask for, not the listener's.
    hook.expect_closed().await;

    replay.assert_finished();
    replay.assert_transcript_range(&hook, 0..3, &THE_APPS_OWN, "the hook's handshake");
    app.stop().await;
}

// ---------------------------------------------------------------------------------------
// Failure modes with no fixture of their own
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn fm32_a_reply_with_nothing_pending_is_refused_as_not_waiting() {
    let app = App::start().await;
    let mut fake = app.session().await;
    fake.mark();

    let mut replay = Replay::new(Golden::load("f02-happy-path").subset(&[0, 1]));
    replay.send_next(&mut fake).await;
    let id = opened(&replay.expect_next(&mut fake).await);

    fake.send(json!({
        "jsonrpc": "2.0", "id": 50, "method": "handoff.continue",
        "params": { "call_id": "call_2q7m8r1t", "handoff_id": id, "reply": "nobody asked" }
    }))
    .await;
    let refusal = fake.expect("the refusal").await;
    assert_eq!(refusal["error"]["code"], json!(-32011));
    assert_eq!(refusal["error"]["message"], json!("not_waiting"));
    app.stop().await;
}

#[tokio::test]
async fn a_continue_citing_a_value_the_spec_never_declared_names_the_keys_it_refused() {
    let app = App::start().await;
    let mut fake = app.session().await;
    fake.mark();

    let mut replay = Replay::new(Golden::load("f02-happy-path").subset(&[0, 1]));
    replay.send_next(&mut fake).await;
    let id = opened(&replay.expect_next(&mut fake).await);

    fake.send(json!({
        "jsonrpc": "2.0", "id": 51, "method": "handoff.continue",
        "params": {
            "call_id": "call_2q7m8r1t", "handoff_id": id, "reply": "start again",
            "replacement_steps": [{ "text": "Paste it", "values": ["nowhere_declared"] }]
        }
    }))
    .await;
    let refusal = fake.expect("the refusal").await;
    assert_eq!(refusal["error"]["code"], json!(-32010));
    assert_eq!(refusal["error"]["message"], json!("unknown_value_key"));
    assert_eq!(
        refusal["error"]["data"]["keys"],
        json!(["nowhere_declared"]),
        "the peer is told which name it invented"
    );
    app.stop().await;
}

#[tokio::test]
async fn an_unknown_handoff_is_not_found() {
    let app = App::start().await;
    let mut fake = app.session().await;
    fake.mark();

    fake.send(json!({
        "jsonrpc": "2.0", "id": 52, "method": "handoff.resume",
        "params": { "call_id": "call_2q7m8r1t", "handoff_id": "hf_0000000000" }
    }))
    .await;
    let refusal = fake.expect("the refusal").await;
    assert_eq!(refusal["error"]["code"], json!(-32014));
    assert_eq!(refusal["error"]["message"], json!("not_found"));
    app.stop().await;
}

#[tokio::test]
async fn fm13_a_restarted_app_gives_a_reconnecting_session_its_handoff_back() {
    // The app crashed or was restarted mid-handoff: the state comes back from SQLite and a
    // server that reconnects re-attaches with `handoff.resume` (NFR-12, FM-13).
    let dir = temp_dir();
    let id = {
        let mut app = App::start_in(dir.clone(), |_| {}).await;
        app.keep_the_directory();
        let mut fake = app.session().await;
        fake.mark();
        let mut replay = Replay::new(Golden::load("f02-happy-path").subset(&[0, 1]));
        replay.send_next(&mut fake).await;
        let id = opened(&replay.expect_next(&mut fake).await);
        app.user(&id, UserAction::Confirm).await;
        app.user(&id, UserAction::Note("half way".to_owned())).await;
        app.stop().await;
        id
    };

    // A second app over the same database. The endpoint is a fresh one: what FM-13 is about
    // is the state surviving, and a pipe name freed by a task that has just been told to
    // stop is a race this test would only be measuring.
    let app = App::start_in(dir, |_| {}).await;
    let mut fake = app.session().await;
    fake.mark();
    fake.send(json!({
        "jsonrpc": "2.0", "id": 60, "method": "handoff.resume",
        "params": { "call_id": "call_5w3n9k2v", "handoff_id": id }
    }))
    .await;
    let snapshot = fake.expect("the snapshot of the restored handoff").await;
    assert_eq!(snapshot["result"]["state"], json!("active"));
    assert_eq!(snapshot["result"]["outcome"], Value::Null, "nothing queued");

    // The call is attached again: the next interrupting action reaches it (SRV-21).
    let tab = app.tab(&id).await;
    assert_eq!(tab.step_index, 2, "the cursor survived the restart");
    assert_eq!(tab.notes.len(), 1, "and so did the note");
    app.user(&id, UserAction::Ask("still here?".to_owned()))
        .await;
    let event = fake.expect("the question").await;
    assert_eq!(event["method"], json!("handoff.event"));
    assert_eq!(event["params"]["call_id"], json!("call_5w3n9k2v"));
    app.stop().await;
}

#[tokio::test]
async fn an_outcome_goes_to_the_connection_its_call_arrived_on_and_to_no_other() {
    // The defect this task exists to avoid: two sessions of the same installation, and an
    // outcome delivered to the one that is not waiting for it.
    let app = App::start().await;
    let mut one = app.session().await;
    let mut two = app.session().await;
    one.mark();
    two.mark();

    let mut replay = Replay::new(Golden::load("f02-happy-path").subset(&[0, 1]));
    replay.send_next(&mut one).await;
    let id = opened(&replay.expect_next(&mut one).await);

    app.user(&id, UserAction::Ask("which button?".to_owned()))
        .await;
    let event = one
        .expect("the question, on the connection that opened it")
        .await;
    assert_eq!(event["method"], json!("handoff.event"));

    // Nothing reached the other session. A ping proves the connection is alive and that the
    // next thing on it is the answer to the ping, not somebody else's outcome.
    two.send(json!({ "jsonrpc": "2.0", "id": 70, "method": "ping", "params": {} }))
        .await;
    let answer = two.expect("the pong").await;
    assert_eq!(answer["id"], json!(70));
    assert_eq!(answer["result"], json!({}));
    app.stop().await;
}

#[tokio::test]
async fn traffic_moves_a_session_s_last_sign_of_life_and_leaving_marks_it_disconnected() {
    // §8.3 counts "a message, a ping, or the disconnection itself"; `Registry::touch` is
    // what records the first of the three and nothing called it before this task.
    let app = App::start().await;
    let mut fake = app.session().await;
    let session_ref = fake.session_ref.clone().expect("a registration");
    let registered = app.session_row(&session_ref);
    assert!(registered.connected);

    // `Timestamp` has millisecond resolution, so the two instants have to be able to differ.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let mut replay = Replay::new(Golden::load("f02-happy-path").subset(&[0, 1]));
    replay.send_next(&mut fake).await;
    let id = opened(&replay.expect_next(&mut fake).await);
    let after_traffic = app.session_row(&session_ref);
    assert!(
        after_traffic.last_seen > registered.last_seen,
        "a message on the connection is a sign of life"
    );

    // SRV-21, SRV-22: the connection goes, the session is marked, the handoff is not.
    drop(fake);
    app.wait_for("the detach", |tab| !tab.call_attached, &id)
        .await;
    assert_eq!(app.tab(&id).await.state, HandoffState::Active);
    assert!(!app.session_row(&session_ref).connected);
    app.stop().await;
}

/// A directory of this test's own.
fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hf-{}-{}", std::process::id(), short_id()));
    std::fs::create_dir_all(&dir).expect("the temporary directory is created");
    dir
}
