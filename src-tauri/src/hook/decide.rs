//! The decision itself: the pseudocode of §7.5, with the DD-25 extension of OI-10.
//!
//! ```text
//! decide(hook_peer):
//!   if hook.stop_hook_active: return neutral
//!   session = bind(hook_peer) or return neutral
//!   items = []
//!   for h in handoffs of that session:
//!       if h.state in {deferred, parked} and no attached call:       Deferred
//!       if h.state == awaiting_verification and no attached call:    Unreported
//!       if h.state == active and h.undelivered and no call:          PendingEvent   (DD-25)
//!   for r in user_requests where r.session in {session, unassigned}: Request
//!   items = items.filter(i => not blocked_before(session, i.key))
//!   if items.empty: return neutral
//!   record blocked(session, item.key); return block(reason = render(items, max 3, English))
//! ```
//!
//! # Four things the pseudocode leaves to the implementation
//!
//! - **The cap comes before the record.** §7.5 filters, then says "record blocked for each"
//!   and renders "max 3". Recording an item the reason never named would silence it for the
//!   rest of the session without anyone having been told about it — the fourth request of a
//!   turn would simply be lost, against OPEN-06. So the list is cut to [`MAX_ITEMS`] first
//!   and only those are recorded; the rest are still there at the next end of turn.
//! - **The session that owns a handoff is the one that opened it.** §7.5 also names "the
//!   session of the attached call", but all three handoff conditions require that there is
//!   no attached call, so that half of the disjunction can never fire. Recorded in
//!   `DEVIATIONS.md`.
//! - **A parked handoff is not asked for back.** §8.1 gives `parked` to the user alone and
//!   RESP-07 tells the agent to cite it in its final summary, so the item is the same one
//!   (one block per handoff per session, whichever deferral produced it) and the sentence is
//!   the one RESP-07 asks for.
//! - **The reason is English**, whatever the UI language: §7.5 fixes it, and the reader is
//!   the agent. A queued request is rendered by `requests::text` in English for the same
//!   reason, while the clipboard renders the same entry in the user's language (OPEN-05).

use crate::format::channel::HookInput;
use crate::i18n::Language;
use crate::log::{hook_blocks, Db, HandoffState, Result, Timestamp};
use crate::requests::queue::{Queue, UserRequest};
use crate::requests::text::render_for;
use crate::sessions::HookBinding;
use crate::store::HandoffSnapshot;

/// How many items one block reason may carry (§7.5).
pub const MAX_ITEMS: usize = 3;

/// How much of a goal a reason quotes, in characters.
///
/// A goal is up to 300 characters (§4.2) and a block reason is read by an agent that has a
/// context to spend; three of them at full length would be a paragraph, and the goal is
/// there to say *which* handoff, not to restate it. Code points, not bytes: the cut has to
/// land on a character on both platforms and in any language.
const GOAL_LIMIT: usize = 80;

/// What a hook is answered with (§6.3 `hook.stop`, F-10).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Decision {
    /// Whether the agent is asked to keep going.
    pub block: bool,
    /// The instruction it should act on. Present only when `block` (the channel schema
    /// requires one only then).
    pub reason: Option<String>,
    /// The sessions a hook could not be told apart between (FM-22, SRV-18). The answer is
    /// neutral and the overlay asks the user which tab this is, the next time they interact.
    pub needs_session_picker: Vec<String>,
}

impl Decision {
    /// The answer to everything that is not worth blocking for.
    #[must_use]
    pub fn neutral() -> Self {
        Self::default()
    }

    /// Neutral, and the UI has a question to ask (FM-22).
    #[must_use]
    fn ambiguous(candidates: Vec<String>) -> Self {
        Self {
            block: false,
            reason: None,
            needs_session_picker: candidates,
        }
    }
}

/// One thing worth stopping an agent for (§7.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A handoff the user deferred, or parked by deferring twice (RESP-06, RESP-07).
    Deferred {
        /// Which handoff.
        handoff_id: String,
        /// Its goal, as the reason quotes it.
        goal: Option<String>,
        /// Deferred twice: the user resumes it, the agent only cites it (RESP-07).
        parked: bool,
    },
    /// A handoff waiting for the verification report the agent owes it (VER-07).
    Unreported {
        /// Which handoff.
        handoff_id: String,
        /// Its goal, as the reason quotes it.
        goal: Option<String>,
    },
    /// An active handoff with an answer nobody was listening for (OI-10, DD-25, FM-06).
    PendingEvent {
        /// Which handoff.
        handoff_id: String,
        /// Its goal, as the reason quotes it.
        goal: Option<String>,
    },
    /// Something the user queued for this session (OPEN-06, OPEN-04a, FM-31).
    Request(UserRequest),
}

impl Item {
    /// The key SRV-12 counts by: one block per item per session.
    ///
    /// A deferral and a park share it on purpose — they are the same handoff in the same
    /// situation, one deferral later — so an agent is stopped once for it and not twice.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Deferred { handoff_id, .. } => format!("deferred:{handoff_id}"),
            Self::Unreported { handoff_id, .. } => format!("unreported:{handoff_id}"),
            Self::PendingEvent { handoff_id, .. } => format!("pending_event:{handoff_id}"),
            Self::Request(request) => format!("request:{}", request.id),
        }
    }

    /// The queued request this item is, when it is one: the hook's answer delivers it
    /// (OPEN-06), so it is marked as reached by the Stop hook and given to this session.
    #[must_use]
    pub fn delivered_request(&self) -> Option<&str> {
        match self {
            Self::Request(request) => Some(request.id.as_str()),
            _ => None,
        }
    }

    /// The line the agent reads, in English (§7.5).
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Deferred {
                handoff_id,
                goal,
                parked: false,
            } => format!(
                "{} is deferred and the user is waiting: call handoff_to_user with \
                 resume={handoff_id} to pick it up.",
                names(handoff_id, goal.as_deref())
            ),
            Self::Deferred {
                handoff_id,
                goal,
                parked: true,
            } => format!(
                "{} was deferred twice and waits for the user in the overlay: mention it in \
                 your final summary and do not resume it.",
                names(handoff_id, goal.as_deref())
            ),
            Self::Unreported { handoff_id, goal } => format!(
                "{} is awaiting your verification report: perform its verify and call \
                 handoff_verify.",
                names(handoff_id, goal.as_deref())
            ),
            Self::PendingEvent { handoff_id, goal } => format!(
                "{} has an answer the user sent while nothing was listening: call \
                 handoff_to_user with resume={handoff_id} to collect it.",
                names(handoff_id, goal.as_deref())
            ),
            Self::Request(request) => render_for(Language::En, request),
        }
    }
}

/// `Handoff hf_… (its goal)`, or `Handoff hf_…` when there is no spec yet.
fn names(handoff_id: &str, goal: Option<&str>) -> String {
    match goal.map(truncate) {
        Some(goal) => format!("Handoff {handoff_id} ({goal})"),
        None => format!("Handoff {handoff_id}"),
    }
}

/// The goal as a reason quotes it: at most [`GOAL_LIMIT`] characters, then an ellipsis.
fn truncate(goal: &str) -> String {
    let mut characters = goal.chars();
    let kept: String = characters.by_ref().take(GOAL_LIMIT).collect();
    if characters.next().is_none() {
        kept
    } else {
        format!("{kept}…")
    }
}

/// Answers one `hook.stop` (§7.5, F-10, SRV-12, SRV-13).
///
/// The whole decision writes once, through `log::hook_blocks::commit_decision`: the counters
/// and the requests this answer delivered are one fact.
///
/// # Errors
///
/// [`crate::log::StoreError::Persistence`] when the queue cannot be read or the decision
/// cannot be recorded. The caller answers neutrally: a safety net that cannot write must not
/// hold the agent (PRIN-10, FM-33).
pub fn decide(
    db: &Db,
    queue: &Queue,
    binding: &HookBinding,
    hook: &HookInput,
    handoffs: &[HandoffSnapshot],
    now: &Timestamp,
) -> Result<Decision> {
    // The agent's own guard against a hook loop. The server exits on it before it connects
    // (§5.11), so this is the second line of the same defence and the one that holds for an
    // adapter whose hook does not check (ADPT-08).
    if hook.stop_hook_active {
        return Ok(Decision::neutral());
    }
    let session_ref = match binding {
        HookBinding::Bound(session_ref) => session_ref.as_str(),
        HookBinding::Ambiguous(candidates) => return Ok(Decision::ambiguous(candidates.clone())),
        HookBinding::None => return Ok(Decision::neutral()),
    };

    let requests = queue.open_for_session(db, session_ref)?;
    let mut items = items_for(session_ref, handoffs, &requests);
    items.retain(|item| !hook_blocks::blocked_before(db, session_ref, &item.key()).unwrap_or(true));
    items.truncate(MAX_ITEMS);
    if items.is_empty() {
        return Ok(Decision::neutral());
    }

    let entries: Vec<(String, Option<String>)> = items
        .iter()
        .map(|item| (item.key(), item.delivered_request().map(ToOwned::to_owned)))
        .collect();
    // The write decides what may be said: SRV-12 is the primary key, so an item another hook
    // of the same session recorded between the filter above and this line comes back missing
    // and is dropped from the reason rather than said twice.
    let recorded = hook_blocks::commit_decision(db, session_ref, &entries, now)?;
    items.retain(|item| recorded.contains(&item.key()));
    if items.is_empty() {
        return Ok(Decision::neutral());
    }

    let reason = render(&items);
    tracing::info!(
        session_ref,
        items = items.len(),
        "a hook was blocked at the end of a turn"
    );
    Ok(Decision {
        block: true,
        reason: Some(reason),
        needs_session_picker: Vec::new(),
    })
}

/// Everything a session is worth stopping for, in the order §7.5 collects it: the handoffs
/// as the overlay lists them, then the queued requests, oldest first.
#[must_use]
pub fn items_for(
    session_ref: &str,
    handoffs: &[HandoffSnapshot],
    requests: &[UserRequest],
) -> Vec<Item> {
    let mut items = Vec::new();
    for handoff in handoffs {
        if handoff.session_ref.as_deref() != Some(session_ref) || handoff.call_attached {
            continue;
        }
        let goal = handoff.goal.clone();
        match handoff.state {
            HandoffState::Deferred | HandoffState::Parked => items.push(Item::Deferred {
                handoff_id: handoff.id.clone(),
                goal,
                parked: handoff.state == HandoffState::Parked,
            }),
            HandoffState::AwaitingVerification => items.push(Item::Unreported {
                handoff_id: handoff.id.clone(),
                goal,
            }),
            HandoffState::Active if handoff.undelivered > 0 => items.push(Item::PendingEvent {
                handoff_id: handoff.id.clone(),
                goal,
            }),
            _ => {}
        }
    }
    items.extend(requests.iter().cloned().map(Item::Request));
    items
}

/// The items as one reason: one line each, in the order they were collected.
#[must_use]
pub fn render(items: &[Item]) -> String {
    items
        .iter()
        .map(Item::reason)
        .collect::<Vec<String>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::channel::HookEventName;
    use crate::log::testing::{at, session};
    use crate::log::{sessions, HandoffState};
    use crate::requests::queue::NoRequestObserver;

    const SESSION: &str = "ses_00000001";
    const OTHER: &str = "ses_00000002";

    fn hook(stop_hook_active: bool) -> HookInput {
        HookInput {
            session_id: "0b1e7c94-6f3a-4d21-9f0c-2ab5e8d17c43".to_owned(),
            hook_event_name: HookEventName::Stop,
            stop_hook_active,
            agent_id: None,
            agent_type: None,
        }
    }

    fn tab(id: &str, state: HandoffState) -> HandoffSnapshot {
        HandoffSnapshot {
            id: id.to_owned(),
            state,
            round: 1,
            step_index: 1,
            step_total: 4,
            steps: Vec::new(),
            confirmed: Vec::new(),
            skipped: Vec::new(),
            notes: Vec::new(),
            deferral_count: 0,
            pending_question: None,
            undelivered: 0,
            call_attached: false,
            session_ref: Some(SESSION.to_owned()),
            opener_label: None,
            project_dir: None,
            goal: Some("Register the Stripe webhook for payment events".to_owned()),
            location: None,
            url: None,
            lang: None,
            values: indexmap::IndexMap::new(),
            secrets: None,
            secret_treated: Vec::new(),
            history: Vec::new(),
            verify_report: None,
            verify: None,
            request_text: None,
            linked_request_id: None,
            resumed_from: None,
            final_outcome: None,
            orphan: false,
            runbook_proposal: None,
            created_at: at("2026-09-08T11:00:00Z"),
            closed_at: None,
        }
    }

    fn app() -> (Db, Queue) {
        let db = Db::open_in_memory().expect("a database");
        sessions::register(&db, &session(SESSION)).expect("a session");
        sessions::register(&db, &session(OTHER)).expect("another session");
        (db, Queue::new(Box::new(NoRequestObserver)))
    }

    fn bound() -> HookBinding {
        HookBinding::Bound(SESSION.to_owned())
    }

    fn now() -> Timestamp {
        at("2026-09-08T12:00:00Z")
    }

    // ---------------------------------------------------------------- the F-10 table

    #[test]
    fn f10_row_1_stop_hook_active_is_neutral_whatever_is_waiting() {
        let (db, queue) = app();
        let tabs = [tab("hf_0123456789", HandoffState::Deferred)];
        let decision =
            decide(&db, &queue, &bound(), &hook(true), &tabs, &now()).expect("an answer");
        assert_eq!(decision, Decision::neutral());
        // And nothing was recorded, so the next hook still has something to say.
        assert!(hook_blocks::list_for_session(&db, SESSION)
            .expect("the blocks")
            .is_empty());
    }

    #[test]
    fn f10_row_2_an_unbound_or_ambiguous_hook_is_neutral() {
        let (db, queue) = app();
        let tabs = [tab("hf_0123456789", HandoffState::Deferred)];

        let none = decide(&db, &queue, &HookBinding::None, &hook(false), &tabs, &now())
            .expect("an answer");
        assert_eq!(none, Decision::neutral());

        let candidates = vec![SESSION.to_owned(), OTHER.to_owned()];
        let ambiguous = decide(
            &db,
            &queue,
            &HookBinding::Ambiguous(candidates.clone()),
            &hook(false),
            &tabs,
            &now(),
        )
        .expect("an answer");
        assert!(!ambiguous.block);
        assert_eq!(ambiguous.reason, None);
        assert_eq!(
            ambiguous.needs_session_picker, candidates,
            "FM-22: the overlay asks which session this was"
        );
    }

    #[test]
    fn f10_row_3_a_bound_session_with_nothing_waiting_is_neutral() {
        let (db, queue) = app();
        let mut elsewhere = tab("hf_0123456789", HandoffState::Deferred);
        elsewhere.session_ref = Some(OTHER.to_owned());
        let mut listening = tab("hf_0000000001", HandoffState::Deferred);
        listening.call_attached = true;
        let quiet = tab("hf_0000000002", HandoffState::Active);

        let decision = decide(
            &db,
            &queue,
            &bound(),
            &hook(false),
            &[elsewhere, listening, quiet],
            &now(),
        )
        .expect("an answer");
        assert_eq!(decision, Decision::neutral());
    }

    #[test]
    fn f10_row_4_a_bound_session_with_an_item_is_blocked_with_a_reason() {
        let (db, queue) = app();
        let tabs = [tab("hf_0123456789", HandoffState::AwaitingVerification)];
        let decision =
            decide(&db, &queue, &bound(), &hook(false), &tabs, &now()).expect("an answer");
        assert!(decision.block);
        assert_eq!(
            decision.reason.as_deref(),
            Some(
                "Handoff hf_0123456789 (Register the Stripe webhook for payment events) is \
                 awaiting your verification report: perform its verify and call handoff_verify."
            ),
            "the sentence `fixtures/channel/f10-hook-block.jsonl` carries"
        );
    }

    // Row 5 of the table — the app unreachable or over the budget — is the hook's own
    // (§5.11): nothing of this module runs, and `handoff-mcp` exits neutrally.

    // ------------------------------------------------------- the once-per-item rule

    #[test]
    fn the_same_item_blocks_once_and_the_second_hook_of_the_session_is_neutral() {
        let (db, queue) = app();
        let tabs = [tab("hf_0123456789", HandoffState::Deferred)];
        assert!(
            decide(&db, &queue, &bound(), &hook(false), &tabs, &now())
                .expect("an answer")
                .block
        );
        let second = decide(&db, &queue, &bound(), &hook(false), &tabs, &now()).expect("an answer");
        assert_eq!(second, Decision::neutral(), "SRV-12: at most once");
        assert_eq!(
            hook_blocks::list_for_session(&db, SESSION).expect("the blocks"),
            ["deferred:hf_0123456789"]
        );
    }

    #[test]
    fn a_second_handoff_is_a_second_block() {
        let (db, queue) = app();
        let first = [tab("hf_0123456789", HandoffState::Deferred)];
        assert!(
            decide(&db, &queue, &bound(), &hook(false), &first, &now())
                .expect("an answer")
                .block
        );

        let both = [
            tab("hf_0123456789", HandoffState::Deferred),
            tab("hf_0000000001", HandoffState::Deferred),
        ];
        let decision =
            decide(&db, &queue, &bound(), &hook(false), &both, &now()).expect("an answer");
        assert!(decision.block);
        let reason = decision.reason.expect("a reason");
        assert!(reason.contains("hf_0000000001"), "{reason}");
        assert!(
            !reason.contains("hf_0123456789"),
            "the one already blocked for is not repeated: {reason}"
        );
    }

    #[test]
    fn a_park_is_the_same_item_as_the_deferral_it_grew_from() {
        // SRV-12 counts per handoff, and RESP-07 already tells the agent to stop resuming.
        let (db, queue) = app();
        let deferred = [tab("hf_0123456789", HandoffState::Deferred)];
        assert!(
            decide(&db, &queue, &bound(), &hook(false), &deferred, &now())
                .expect("an answer")
                .block
        );
        let parked = [tab("hf_0123456789", HandoffState::Parked)];
        assert_eq!(
            decide(&db, &queue, &bound(), &hook(false), &parked, &now()).expect("an answer"),
            Decision::neutral()
        );
    }

    #[test]
    fn another_session_of_the_same_handoff_is_blocked_on_its_own_count() {
        let (db, queue) = app();
        let tabs = [tab("hf_0123456789", HandoffState::Deferred)];
        assert!(
            decide(&db, &queue, &bound(), &hook(false), &tabs, &now())
                .expect("an answer")
                .block
        );

        let mut theirs = tab("hf_0123456789", HandoffState::Deferred);
        theirs.session_ref = Some(OTHER.to_owned());
        assert!(
            decide(
                &db,
                &queue,
                &HookBinding::Bound(OTHER.to_owned()),
                &hook(false),
                &[theirs],
                &now()
            )
            .expect("an answer")
            .block
        );
    }

    // -------------------------------------------------------------------- the items

    #[test]
    fn oi_10_an_active_handoff_with_an_undelivered_answer_blocks_once() {
        // DD-25, confirmed by the owner in T-001 D4: the case FM-06 leaves open, where a
        // heartbeat returned and the agent never resumed.
        let (db, queue) = app();
        let mut waiting = tab("hf_0123456789", HandoffState::Active);
        waiting.undelivered = 1;
        let decision =
            decide(&db, &queue, &bound(), &hook(false), &[waiting], &now()).expect("an answer");
        assert!(decision.block);
        let reason = decision.reason.expect("a reason");
        assert!(reason.contains("resume=hf_0123456789"), "{reason}");
        assert_eq!(
            hook_blocks::list_for_session(&db, SESSION).expect("the blocks"),
            ["pending_event:hf_0123456789"]
        );
    }

    #[test]
    fn an_active_handoff_with_nothing_undelivered_is_not_an_item() {
        let (db, queue) = app();
        let tabs = [tab("hf_0123456789", HandoffState::Active)];
        assert_eq!(
            decide(&db, &queue, &bound(), &hook(false), &tabs, &now()).expect("an answer"),
            Decision::neutral()
        );
    }

    #[test]
    fn a_final_handoff_is_never_an_item() {
        let (db, queue) = app();
        for state in [
            HandoffState::Verified,
            HandoffState::Failed,
            HandoffState::NotVerified,
            HandoffState::ConfirmedByUser,
            HandoffState::Abandoned,
            HandoffState::AwaitingSpec,
        ] {
            let tabs = [tab("hf_0123456789", state)];
            assert_eq!(
                decide(&db, &queue, &bound(), &hook(false), &tabs, &now()).expect("an answer"),
                Decision::neutral(),
                "{state:?} is not something to stop an agent for"
            );
        }
    }

    #[test]
    fn a_queued_request_is_delivered_by_the_block_and_assigned_to_the_session() {
        // OPEN-06 and OPEN-04a: the hook is the safety net, and it also decides where an
        // unassigned request went.
        let (db, queue) = app();
        let id = queue
            .create(
                &db,
                "I'm about to create the API key on Stripe",
                None,
                &now(),
            )
            .expect("a request");

        let decision = decide(&db, &queue, &bound(), &hook(false), &[], &now()).expect("an answer");
        assert!(decision.block);
        let reason = decision.reason.expect("a reason");
        assert!(reason.contains(&format!("request_id={id}")), "{reason}");
        assert!(
            reason.contains("I'm about to create the API key on Stripe"),
            "{reason}"
        );

        let row = queue.get(&db, &id).expect("a read").expect("a row");
        assert_eq!(
            row.delivered_via,
            Some(crate::log::user_requests::DeliveredVia::StopHook)
        );
        assert_eq!(row.session_ref.as_deref(), Some(SESSION));

        // Once told, this session is not told again.
        assert_eq!(
            decide(&db, &queue, &bound(), &hook(false), &[], &now()).expect("an answer"),
            Decision::neutral()
        );
    }

    #[test]
    fn a_queued_resume_asks_the_agent_to_come_back_to_its_handoff() {
        // FM-31: the user picked a parked handoff up and the agent is elsewhere.
        let (db, queue) = app();
        crate::log::handoffs::upsert(&db, &crate::log::testing::handoff("hf_0123456789"))
            .expect("the handoff it is about");
        queue
            .queue_resume_request(&db, "hf_0123456789", Some(SESSION), &now())
            .expect("a resume request");
        let decision = decide(&db, &queue, &bound(), &hook(false), &[], &now()).expect("an answer");
        let reason = decision.reason.expect("a reason");
        assert!(reason.contains("resume=hf_0123456789"), "{reason}");
    }

    // ------------------------------------------------------------------- the reason

    #[test]
    fn a_reason_carries_at_most_three_items_and_keeps_the_rest_for_the_next_turn() {
        let (db, queue) = app();
        let tabs: Vec<HandoffSnapshot> = [
            "hf_0000000001",
            "hf_0000000002",
            "hf_0000000003",
            "hf_0000000004",
        ]
        .into_iter()
        .map(|id| tab(id, HandoffState::Deferred))
        .collect();

        let first = decide(&db, &queue, &bound(), &hook(false), &tabs, &now()).expect("an answer");
        let reason = first.reason.expect("a reason");
        assert_eq!(reason.lines().count(), MAX_ITEMS);
        assert!(!reason.contains("hf_0000000004"), "{reason}");
        assert_eq!(
            hook_blocks::list_for_session(&db, SESSION)
                .expect("the blocks")
                .len(),
            MAX_ITEMS,
            "an item nobody was told about must not be recorded as told"
        );

        let second = decide(&db, &queue, &bound(), &hook(false), &tabs, &now()).expect("an answer");
        let reason = second.reason.expect("a reason");
        assert_eq!(reason.lines().count(), 1);
        assert!(reason.contains("hf_0000000004"), "{reason}");
    }

    #[test]
    fn every_item_names_its_handoff_its_goal_and_what_to_do() {
        let goal = "Register the Stripe webhook for payment events";
        let cases = [
            Item::Deferred {
                handoff_id: "hf_0123456789".to_owned(),
                goal: Some(goal.to_owned()),
                parked: false,
            },
            Item::Deferred {
                handoff_id: "hf_0123456789".to_owned(),
                goal: Some(goal.to_owned()),
                parked: true,
            },
            Item::Unreported {
                handoff_id: "hf_0123456789".to_owned(),
                goal: Some(goal.to_owned()),
            },
            Item::PendingEvent {
                handoff_id: "hf_0123456789".to_owned(),
                goal: Some(goal.to_owned()),
            },
        ];
        let mut sentences = Vec::new();
        for item in &cases {
            let reason = item.reason();
            assert!(reason.contains("hf_0123456789"), "{reason}");
            assert!(reason.contains(goal), "{reason}");
            assert!(reason.ends_with('.'), "{reason}");
            sentences.push(reason);
        }
        sentences.sort();
        sentences.dedup();
        assert_eq!(sentences.len(), cases.len(), "each item says its own thing");
    }

    #[test]
    fn a_parked_handoff_is_not_asked_to_be_resumed() {
        // RESP-07 and §8.1: `parked` belongs to the user; the agent cites it and stops.
        let reason = Item::Deferred {
            handoff_id: "hf_0123456789".to_owned(),
            goal: None,
            parked: true,
        }
        .reason();
        assert!(!reason.contains("resume="), "{reason}");
        assert!(reason.contains("final summary"), "{reason}");
    }

    #[test]
    fn a_long_goal_is_cut_on_a_character_and_a_handoff_without_one_is_named_alone() {
        let long = "é".repeat(GOAL_LIMIT + 20);
        let reason = Item::Unreported {
            handoff_id: "hf_0123456789".to_owned(),
            goal: Some(long),
        }
        .reason();
        let quoted: String = reason
            .chars()
            .skip_while(|c| *c != '(')
            .skip(1)
            .take_while(|c| *c != ')')
            .collect();
        assert_eq!(quoted.chars().count(), GOAL_LIMIT + 1, "{quoted}");
        assert!(quoted.ends_with('…'));

        let no_goal = Item::Unreported {
            handoff_id: "hf_0123456789".to_owned(),
            goal: None,
        }
        .reason();
        assert!(
            no_goal.starts_with("Handoff hf_0123456789 is awaiting"),
            "{no_goal}"
        );
    }

    #[test]
    fn the_items_of_a_session_come_before_its_queue_and_in_the_order_of_the_tabs() {
        let older = tab("hf_0000000001", HandoffState::Deferred);
        let newer = tab("hf_0000000002", HandoffState::AwaitingVerification);
        let request = crate::log::testing::request("hf_0000000003");
        let items = items_for(SESSION, &[older, newer], std::slice::from_ref(&request));
        assert_eq!(
            items.iter().map(Item::key).collect::<Vec<String>>(),
            [
                "deferred:hf_0000000001",
                "unreported:hf_0000000002",
                "request:hf_0000000003"
            ]
        );
    }
}
