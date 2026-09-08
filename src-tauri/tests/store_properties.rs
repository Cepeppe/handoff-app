//! The store, under random sequences of actions (§11.2, §8.1, NFR-12).
//!
//! The unit tests beside the code ask "does this transition do what §8.1 says". This suite
//! asks the other question: whatever order a user and an agent do things in, do the five
//! invariants of §11.2 still hold?
//!
//! - **At most one attached call** per handoff (TOOL-03).
//! - **The undelivered queue is FIFO** (DD-12, §5.7): the only two things that may happen to
//!   it are an outcome appended at the back and the oldest taken from the front.
//! - **Final states are reached only through the tool**: `verified` and `failed` never
//!   appear unless `handoff_verify` was accepted (VER-02, PRIN-08).
//! - **`verified` never without a report** (VER-02).
//! - **The counters reset per round** (VER-09): the first look at a new round finds no
//!   confirmations, no skips and no notes.
//!
//! Two more that cost nothing here and would be expensive to find later: **a refusal changes
//! nothing** (every guard runs before the first mutation, which is what makes FM-28's
//! promise true for the ordinary refusals as well), and **every outcome that leaves the
//! store validates against the vendored schema**, which is the acceptance criterion of this
//! task asserted on random input rather than on the fixtures.

use std::collections::VecDeque;

use handoff_app_lib::format::channel::DetachReason;
use handoff_app_lib::format::outcome::{Outcome, ResumedFrom, ScreenshotMode};
use handoff_app_lib::format::schema::{validate, Document};
use handoff_app_lib::format::spec::{HandoffSpec, HandoffStep, SpecValue};
use handoff_app_lib::log::sessions::{self, SessionRow};
use handoff_app_lib::log::{Db, HandoffState, Timestamp};
use handoff_app_lib::store::actor::{OpenParams, Store};
use handoff_app_lib::store::handoff::{Call, Opener, ScreenshotPayload};
use handoff_app_lib::store::Refusal;
use indexmap::IndexMap;
use proptest::prelude::*;

const OPENER: &str = "ses_00000001";
const OTHER: &str = "ses_00000002";

/// One thing a user or an agent can do to a handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Confirm,
    Skip,
    Note,
    Ask,
    Screenshot,
    Defer,
    Abandon,
    Done,
    ResumeFromOverlay,
    CloseOrphan,
    Detach,
    /// A resume from the opening session (`false`) or from another one (`true`).
    Resume(bool),
    Reply,
    ReplyWithReplacementSteps,
    Verify(Option<bool>),
    VerifyingTimeout,
    SessionDisconnected,
}

fn an_action() -> impl Strategy<Value = Action> {
    prop_oneof![
        // The ordinary rhythm of a handoff is the user walking the steps, so those weigh
        // more: a sequence made mostly of resumes and disconnects would exercise the guards
        // and never the machine.
        6 => Just(Action::Confirm),
        3 => Just(Action::Skip),
        3 => Just(Action::Note),
        4 => Just(Action::Ask),
        3 => Just(Action::Screenshot),
        3 => Just(Action::Defer),
        1 => Just(Action::Abandon),
        4 => Just(Action::Done),
        2 => Just(Action::ResumeFromOverlay),
        1 => Just(Action::CloseOrphan),
        3 => Just(Action::Detach),
        3 => any::<bool>().prop_map(Action::Resume),
        3 => Just(Action::Reply),
        2 => Just(Action::ReplyWithReplacementSteps),
        4 => prop_oneof![Just(Some(true)), Just(Some(false)), Just(None)].prop_map(Action::Verify),
        2 => Just(Action::VerifyingTimeout),
        1 => Just(Action::SessionDisconnected),
    ]
}

/// A spec with `steps` steps, and a `verify` when asked for.
fn spec(steps: usize, verify: bool) -> HandoffSpec {
    let mut values = IndexMap::new();
    values.insert(
        "endpoint_url".to_owned(),
        SpecValue::One("https://api.example.test/hook".to_owned()),
    );
    HandoffSpec {
        spec_version: 1,
        goal: "Register the webhook".to_owned(),
        r#where: "Dashboard, then Webhooks".to_owned(),
        url: None,
        why_human: "only a person can log in".to_owned(),
        values,
        secrets: None,
        steps: (1..=steps)
            .map(|index| HandoffStep {
                text: format!("step {index}"),
                url: None,
                values: Some(vec!["endpoint_url".to_owned()]),
                warning: None,
            })
            .collect(),
        verify: verify.then(|| "the webhook fires".to_owned()),
        lang: Some("en".to_owned()),
    }
}

fn session_row(session_ref: &str) -> SessionRow {
    SessionRow {
        session_ref: session_ref.to_owned(),
        agent_id: Some("claude-code".to_owned()),
        client_name: Some("claude-code".to_owned()),
        client_version: Some("2.1.263".to_owned()),
        pid_chain_json: "[4242]".to_owned(),
        cwd: Some("C:/projects/baton".to_owned()),
        project_dir: Some("C:/projects/baton".to_owned()),
        claude_session_id: None,
        connected: true,
        first_seen: at(0),
        last_seen: at(0),
    }
}

fn opener(session_ref: &str) -> Opener {
    Opener {
        session_ref: session_ref.to_owned(),
        agent_id: Some("claude-code".to_owned()),
        client_name: Some("claude-code".to_owned()),
        project_dir: Some("C:/projects/baton".to_owned()),
        label: ResumedFrom {
            agent: "Claude Code".to_owned(),
            project: "baton".to_owned(),
        },
    }
}

/// The instant `minutes` minutes after a fixed origin, so a run never depends on the clock.
fn at(minutes: i64) -> Timestamp {
    Timestamp::from_millis(1_788_000_000_000 + minutes * 60_000)
}

/// What the harness remembers between actions, so the invariants can be checked against
/// history rather than against a single snapshot.
struct Seen {
    queue: VecDeque<String>,
    round: u32,
    verified_through_the_tool: bool,
}

/// Runs one sequence and asserts every invariant after every action.
fn run(actions: &[Action], steps: usize, with_verify: bool) -> Result<(), TestCaseError> {
    let db = Db::open_in_memory().expect("a database");
    for reference in [OPENER, OTHER] {
        sessions::register(&db, &session_row(reference)).expect("a session");
    }
    let mut store = Store::new(db);
    let id = store
        .open(
            OpenParams {
                spec: spec(steps, with_verify),
                secret_treated: Vec::new(),
                request_id: None,
                opener: opener(OPENER),
                call: Call {
                    conn_id: 1,
                    call_id: "call_00000000".to_owned(),
                    session_ref: Some(OPENER.to_owned()),
                },
            },
            &at(0),
        )
        .expect("the open is accepted")
        .handoff_id;

    let mut seen = Seen {
        queue: VecDeque::new(),
        round: 1,
        verified_through_the_tool: false,
    };

    for (index, action) in actions.iter().enumerate() {
        // Twenty minutes per action, so a sequence of a couple of dozen crosses the
        // thirty-minute verification window of VER-06 several times without ever reaching
        // the seven days after which a late report stops being accepted (DD-16).
        let now = at((i64::try_from(index).unwrap_or(0) + 1) * 20);
        let call_id = format!("call_{index:08}");
        // Half the detaches name the call that is really attached and half name one that is
        // not, which is the race DD-24 leaves open between a heartbeat and an outcome.
        let attached = attached_call_id(&store, &id).unwrap_or_else(|| call_id.clone());
        let before = store.snapshot(&id, &now).expect("the handoff is there");

        let refused = match *action {
            Action::Confirm => store.confirm(&id, &now).err(),
            Action::Skip => store.skip(&id, &now).err(),
            Action::Note => store.note(&id, "a note", &now).err(),
            Action::Ask => store.ask(&id, "a question", &now).err(),
            Action::Screenshot => store.screenshot(&id, &a_screenshot(), &now).err(),
            Action::Defer => store.defer(&id, None, &now).err(),
            Action::Abandon => store.abandon(&id, None, &now).err(),
            Action::Done => store.done(&id, &now).err(),
            Action::ResumeFromOverlay => store.resume_from_overlay(&id, &now).err(),
            Action::CloseOrphan => store.close_orphan(&id, &now).err(),
            Action::Detach => store
                .detach_call(&id, &attached, DetachReason::Heartbeat, &now)
                .err(),
            Action::Resume(elsewhere) => {
                store.resume(&id, &a_call(&call_id, elsewhere), &now).err()
            }
            Action::Reply => store
                .continue_handoff(&id, &a_call(&call_id, false), "an answer", None, &now)
                .err(),
            Action::ReplyWithReplacementSteps => store
                .continue_handoff(
                    &id,
                    &a_call(&call_id, false),
                    "start again",
                    Some(spec(2, false).steps),
                    &now,
                )
                .err(),
            Action::Verify(ok) => store
                .verify(&id, ok, Some("what I ran".to_owned()), &now)
                .err(),
            Action::VerifyingTimeout => store.verifying_timeout(&id, &now).err(),
            Action::SessionDisconnected => store.session_disconnected(OPENER, &now).err(),
        };

        if matches!(*action, Action::Verify(_)) && refused.is_none() {
            seen.verified_through_the_tool = true;
        }

        let after = store
            .snapshot(&id, &now)
            .expect("the handoff is still there");
        if let Some(refusal) = refused {
            prop_assert!(
                !matches!(refusal, Refusal::Persistence(_)),
                "an in-memory database refused a write: {refusal}"
            );
            prop_assert_eq!(
                &before,
                &after,
                "a refused action changed the handoff: {:?} -> {}",
                action,
                refusal
            );
        }

        for delivery in store.take_deliveries() {
            assert_schema_valid(&delivery.outcome)?;
            prop_assert_eq!(
                &delivery.handoff_id,
                &id,
                "an outcome was addressed to another handoff"
            );
        }

        let handoff = store.get(&id).expect("the handoff is still there");

        // At most one attached call (TOOL-03): the type says so, and the snapshot is what a
        // view would read, so both are checked.
        prop_assert_eq!(
            handoff.attached_call.is_some(),
            after.call_attached,
            "the snapshot and the record disagree about the attached call"
        );

        // The queue is FIFO (DD-12): either it grew at the back, or its head was taken.
        let queue: VecDeque<String> = handoff
            .undelivered
            .iter()
            .map(|queued| fingerprint(&queued.outcome))
            .collect();
        prop_assert!(
            fifo_step(&seen.queue, &queue),
            "the undelivered queue was not treated as FIFO: {:?} -> {:?} after {:?}",
            seen.queue,
            queue,
            action
        );
        seen.queue = queue;

        // The counters restart with a round (VER-09).
        if after.round != seen.round {
            prop_assert!(
                after.confirmed.is_empty() && after.skipped.is_empty() && after.notes.is_empty(),
                "round {} started with counters from round {}",
                after.round,
                seen.round
            );
            seen.round = after.round;
        }
        for index in after.confirmed.iter().chain(after.skipped.iter()) {
            prop_assert!(
                *index >= 1 && *index <= after.step_total,
                "a counter points outside the round"
            );
        }

        // `verified` and `failed` are only ever reached through the tool (VER-02).
        if matches!(after.state, HandoffState::Verified | HandoffState::Failed) {
            prop_assert!(
                seen.verified_through_the_tool,
                "{:?} was reached without a report",
                after.state
            );
            prop_assert!(
                handoff
                    .last_round()
                    .and_then(|round| round.verify.as_ref())
                    .is_some(),
                "{:?} carries no verification report",
                after.state
            );
        }

        // A final state always has the outcome an agent will come for (SRV-23).
        if after.state.is_final() {
            prop_assert!(after.final_outcome.is_some());
            prop_assert!(after.closed_at.is_some());
        }
        prop_assert!(
            after.deferral_count <= 2,
            "RESP-07 caps the deferrals at two"
        );
    }

    prop_assert_eq!(store.list_for_ui(&at(1_000)).len(), 1);
    Ok(())
}

fn a_screenshot() -> ScreenshotPayload {
    ScreenshotPayload {
        mode: ScreenshotMode::Text,
        text: Some("what the page says".to_owned()),
        image_base64: None,
        image_sha256: None,
        width: 1600,
        height: 900,
        redactions: 0,
        redaction_boxes_json: Some("[]".to_owned()),
        ocr_engine: Some("tesseract".to_owned()),
        patterns_version: Some("1".to_owned()),
        comment: Some("is this right?".to_owned()),
    }
}

fn a_call(call_id: &str, elsewhere: bool) -> Call {
    Call {
        conn_id: if elsewhere { 9 } else { 1 },
        call_id: call_id.to_owned(),
        session_ref: Some(if elsewhere { OTHER } else { OPENER }.to_owned()),
    }
}

/// The call currently attached, so a detach sometimes names the right one.
fn attached_call_id(store: &Store, id: &str) -> Option<String> {
    store
        .get(id)
        .and_then(|handoff| handoff.attached_call.as_ref())
        .map(|call| call.call_id.clone())
}

/// Enough of an outcome to tell two queued ones apart.
fn fingerprint(outcome: &Outcome) -> String {
    format!(
        "{:?}|{}|{}",
        outcome.status,
        outcome.round,
        outcome.user_text.as_deref().unwrap_or("")
    )
}

/// Whether `after` is `before` with at most one item appended at the back, or with its head
/// removed, or unchanged: the three things a FIFO queue may do in one step.
fn fifo_step(before: &VecDeque<String>, after: &VecDeque<String>) -> bool {
    if after.len() == before.len() + 1 {
        return after.iter().take(before.len()).eq(before.iter());
    }
    if after.len() + 1 == before.len() {
        return after.iter().eq(before.iter().skip(1));
    }
    // A final state clears nothing and adds nothing; anything else would be a reordering.
    after == before
}

fn assert_schema_valid(outcome: &Outcome) -> Result<(), TestCaseError> {
    let value = serde_json::to_value(outcome).expect("an outcome serialises");
    match validate(Document::Outcome, &value) {
        Ok(()) => Ok(()),
        Err(problems) => {
            let where_ = problems
                .iter()
                .map(|problem| format!("{} ({})", problem.path, problem.keyword))
                .collect::<Vec<_>>()
                .join(", ");
            Err(TestCaseError::fail(format!(
                "an outcome the schema refuses: {where_}"
            )))
        }
    }
}

proptest! {
    // The acceptance of T-033 asks for at least a thousand cases in CI, and the sequences
    // are short enough that a thousand of them run in a few seconds.
    #![proptest_config(ProptestConfig { cases: 1024, max_shrink_iters: 4096, ..ProptestConfig::default() })]

    /// Any sequence of user and agent actions, against a spec that asks for a verification.
    #[test]
    fn the_invariants_hold_under_any_sequence_when_a_verification_is_asked_for(
        actions in prop::collection::vec(an_action(), 1..24),
        steps in 1_usize..5,
    ) {
        run(&actions, steps, true)?;
    }

    /// The same, against a spec whose result only the user can confirm (PRIN-08).
    #[test]
    fn the_invariants_hold_under_any_sequence_when_the_user_is_the_only_judge(
        actions in prop::collection::vec(an_action(), 1..24),
        steps in 1_usize..5,
    ) {
        run(&actions, steps, false)?;
    }
}
