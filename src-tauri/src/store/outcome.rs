//! Building the outcome the agent reads (§4.3, TOOL-13, DD-08, DD-15).
//!
//! One function per shape would be one function per status; instead there is one builder,
//! because §4.3 is explicit that **every field is present** with `null` or `[]` when it
//! does not apply, so an agent never branches on absence. What changes between statuses is
//! which of them carry a value, and that is a small table.
//!
//! # The instruction is the server's
//!
//! `instruction` is generated from `schemas/tool-contract.v1.md` and the session's
//! capability row decides its Stop-hook variant (§4.7.4), so the sentence an agent actually
//! reads is written by the server, which replaces whatever arrives over the channel. The
//! app still has to put a valid one in the field — the outcome schema requires a non-empty
//! string, and the log and the runbook writer read the object — so this module writes the
//! **gist texts of the §4.3 table**, with `<id>` as the only substitution. They are the
//! texts the golden channel fixtures carry, which is what lets `tests/fake-server` (T-034)
//! replay a sequence and compare it.
//!
//! # The pixels are not in here
//!
//! `screenshot.image_attached` is all the outcome says about an image; the bytes travel
//! beside it on `handoff.event` and on the `handoff.resume` snapshot (`DEVIATIONS.md`,
//! T-020). The app sets `image_attached` from the mode alone: whether an image block is
//! actually built is the server's decision, gated on `images_in_results`, and the preview
//! only ever offers "Send image" to a session whose row allows it (PREV-04, FM-05).

use indexmap::IndexMap;

use crate::format::outcome::{
    ContextStep, CurrentStep, Outcome, OutcomeContext, OutcomeNote, OutcomeStatus, ScreenshotInfo,
    ScreenshotMode,
};
use crate::format::schema::{validate, Document};
use crate::format::spec::SpecValue;
use crate::redaction::suspected::Exemptions;
use crate::redaction::typed::redact;

use super::handoff::{Handoff, ScreenshotPayload};

/// What `step_values` shows in place of a value the certain detector matched (§4.3,
/// CTX-01). Not the `[treated as secret: <kind>]` of §5.9: the context says *that* a value
/// is withheld, and the outcome's own `secret_treated` list is where the family is.
pub const CONTEXT_SECRET_PLACEHOLDER: &str = "[treated as secret]";

/// The version every outcome of this design carries.
pub const OUTCOME_VERSION: i64 = 1;

/// The fourteen statuses of §4.3, in the order the table prints them.
pub const ALL_STATUSES: [OutcomeStatus; 14] = [
    OutcomeStatus::InProgress,
    OutcomeStatus::Question,
    OutcomeStatus::Screenshot,
    OutcomeStatus::Deferred,
    OutcomeStatus::Parked,
    OutcomeStatus::AwaitingVerification,
    OutcomeStatus::ConfirmedByUser,
    OutcomeStatus::Verified,
    OutcomeStatus::Failed,
    OutcomeStatus::NotVerified,
    OutcomeStatus::Abandoned,
    OutcomeStatus::TransferredToOtherSession,
    OutcomeStatus::RunbookMatch,
    OutcomeStatus::TextMode,
];

/// Whether a status ends the handoff (§4.3, VER-01).
#[must_use]
pub fn is_final(status: OutcomeStatus) -> bool {
    matches!(
        status,
        OutcomeStatus::ConfirmedByUser
            | OutcomeStatus::Verified
            | OutcomeStatus::Failed
            | OutcomeStatus::NotVerified
            | OutcomeStatus::Abandoned
    )
}

/// The gist text of §4.3 for `status`, with `<id>` replaced by `handoff_id`.
///
/// `screenshot` is the one status whose sentence names the step and how the user sent it,
/// exactly as the §4.3 example and the golden fixtures write it; [`build`] passes those in.
#[must_use]
pub fn instruction_for(status: OutcomeStatus, handoff_id: Option<&str>) -> String {
    let id = handoff_id.unwrap_or("<id>");
    match status {
        OutcomeStatus::InProgress => format!(
            "The user is still working. Call handoff_to_user with resume={id} now to keep waiting."
        ),
        OutcomeStatus::Question | OutcomeStatus::Screenshot => ANSWER_ON_THE_STEP.to_owned(),
        OutcomeStatus::Deferred => format!(
            "Park this step, continue work that does not depend on it, then call \
             handoff_to_user with resume={id} before you conclude."
        ),
        OutcomeStatus::Parked => format!(
            "Do not resume now. The handoff stays in the overlay; mention {id} as pending in \
             your final summary."
        ),
        OutcomeStatus::AwaitingVerification => "Perform the verification below with your own \
             tools, then call handoff_verify with ok true, false or null and a detail. Never \
             read values listed in secrets."
            .to_owned(),
        OutcomeStatus::ConfirmedByUser => {
            "The handoff is complete and recorded as confirmed by the user.".to_owned()
        }
        OutcomeStatus::Verified => "Recorded as verified; a runbook was saved.".to_owned(),
        OutcomeStatus::Failed => "If you can correct it, call handoff_to_user with handoff_id \
             and replacement_steps that start from the actual error; otherwise tell the user."
            .to_owned(),
        OutcomeStatus::NotVerified => "Recorded as not verified. If you can still verify, call \
             handoff_verify; a late report is accepted."
            .to_owned(),
        OutcomeStatus::Abandoned => "The user abandoned this handoff. Do not retry the same \
             steps; ask the user how to proceed."
            .to_owned(),
        OutcomeStatus::TransferredToOtherSession => {
            format!("Another session took over {id}. Do nothing further with it.")
        }
        OutcomeStatus::RunbookMatch => "A runbook exists: start from here? Fill values_to_fill \
             and call handoff_to_user again with the completed spec and ignore_runbook=true."
            .to_owned(),
        OutcomeStatus::TextMode => "The overlay app is not running. Present the spec below to \
             the user in chat, walk them through the steps, and collect the result in chat. No \
             log or verified state exists in this mode."
            .to_owned(),
    }
}

/// The sentence `question` and `screenshot` share (§4.3).
const ANSWER_ON_THE_STEP: &str = "Answer on the current step: call handoff_to_user with \
     handoff_id and reply; add replacement_steps only if the remaining steps must change.";

/// The `screenshot` sentence, which names the step and how the user sent it.
fn screenshot_instruction(step: u32, mode: ScreenshotMode) -> String {
    let what = match mode {
        ScreenshotMode::Image => "an image",
        ScreenshotMode::Text => "extracted text",
    };
    format!("The user sent what they see at step {step} as {what}. Answer on this step: call handoff_to_user with handoff_id and reply; add replacement_steps only if the remaining steps must change.")
}

/// One note of the round as the agent reads it: the same words, with a certain match
/// replaced (§7.10, DET-01, PRIN-09).
///
/// A note is the one typed text of the app that is **not** redacted where it is typed. §7.10
/// scans "Ask, comments, edited OCR text" and RESP-02 calls a note a local annotation, so
/// `ui_bridge::commands::act` keeps what the user wrote — the step they read back afterwards
/// is their own sentence, which is the whole point of the button. But RESP-03 also reports
/// every note "all together in the final outcome", so a note *does* leave the machine, and
/// PRIN-09 allows nothing to leave unredacted. This is where it leaves, so this is where the
/// certain level is applied.
///
/// Only the certain level: DET-01 gives a suspected match to the user to decide on, the
/// sheet has no marking for a text it does not scan, and an outcome has nowhere to carry a
/// mark. The mask is [`crate::redaction::typed::mask_for`]'s — the one §7.10 gives to text
/// an agent reads, not the log's. No exemption applies: DET-03 exempts a spec value from the
/// **suspected** level and never from this one.
fn as_the_agent_reads_it(note: &OutcomeNote) -> OutcomeNote {
    OutcomeNote {
        step: note.step,
        text: redact(&note.text, &Exemptions::none()).text,
        at: note.at.clone(),
    }
}

/// The outcome for `status`, built from everything the handoff already knows.
///
/// `user_text` is the question, the comment beside a screenshot, or the reason typed with
/// Defer or Abandon; `screenshot` is present only for [`OutcomeStatus::Screenshot`].
///
/// # Panics
///
/// In a debug build, when the result does not validate against the vendored outcome schema.
/// Every input has been through the schema already — the spec at ingress, a replacement
/// step list at the continue — so a failure here is a defect of this builder and not of a
/// peer, and it is worth finding at the moment it is produced.
#[must_use]
pub fn build(
    handoff: &Handoff,
    status: OutcomeStatus,
    user_text: Option<String>,
    screenshot: Option<&ScreenshotPayload>,
) -> Outcome {
    let round = handoff.current_round();
    let step_index = handoff.cursor.step_index;
    let instruction = match (status, screenshot) {
        (OutcomeStatus::Screenshot, Some(payload)) => {
            screenshot_instruction(step_index, payload.mode)
        }
        _ => instruction_for(status, Some(&handoff.id)),
    };

    let wants_context = matches!(status, OutcomeStatus::Question | OutcomeStatus::Screenshot);
    let outcome = Outcome {
        outcome_version: OUTCOME_VERSION,
        handoff_id: Some(handoff.id.clone()),
        status,
        is_final: is_final(status),
        instruction,
        round: handoff.cursor.round.max(1),
        current_step: current_step(handoff),
        user_text,
        screenshot: screenshot.map(screenshot_info),
        context: if wants_context {
            context(handoff)
        } else {
            None
        },
        skipped_steps: round.map(|round| round.skipped.clone()).unwrap_or_default(),
        notes: round
            .map(|round| round.notes.iter().map(as_the_agent_reads_it).collect())
            .unwrap_or_default(),
        secret_treated: handoff.secret_treated.clone(),
        verify: round.and_then(|round| round.verify.clone()),
        deferral_count: handoff.deferral_count.min(2),
        resumed_from: handoff.resumed_from.clone(),
        app_reachable: true,
        already_delivered: false,
        runbooks: Vec::new(),
        spec_text: None,
    };
    debug_assert_valid(&outcome);
    outcome
}

/// The same outcome, marked as one an agent has already been given (TOOL-07).
#[must_use]
pub fn already_delivered(outcome: &Outcome) -> Outcome {
    let mut repeated = outcome.clone();
    repeated.already_delivered = true;
    repeated
}

/// The step the overlay's counter is showing.
fn current_step(handoff: &Handoff) -> Option<CurrentStep> {
    let step = handoff.current_step()?;
    Some(CurrentStep {
        index: handoff.cursor.step_index,
        total: handoff.step_count(),
        text: step.text.clone(),
    })
}

/// Where the user is, for the two statuses that ask the agent to answer on a step (CTX-01).
fn context(handoff: &Handoff) -> Option<OutcomeContext> {
    let spec = handoff.spec.as_ref()?;
    let step = handoff.current_step()?;
    let mut step_values = IndexMap::new();
    for name in step.values.iter().flatten() {
        // A step may only cite a declared value name (S3), so a name that is not in the map
        // cannot reach here from a validated spec; skipping it keeps a hand-built one from
        // producing an outcome with a value the agent cannot place.
        let Some(value) = spec.values.get(name) else {
            continue;
        };
        let shown = if handoff.is_secret_value(name) {
            SpecValue::One(CONTEXT_SECRET_PLACEHOLDER.to_owned())
        } else {
            value.clone()
        };
        step_values.insert(name.clone(), shown);
    }
    Some(OutcomeContext {
        goal: spec.goal.clone(),
        r#where: spec.r#where.clone(),
        step: ContextStep {
            index: handoff.cursor.step_index,
            total: handoff.step_count(),
            text: step.text.clone(),
            url: step.url.clone(),
            warning: step.warning.clone(),
        },
        step_values,
    })
}

/// What the outcome says about a screenshot: everything except the pixels.
fn screenshot_info(payload: &ScreenshotPayload) -> ScreenshotInfo {
    ScreenshotInfo {
        mode: payload.mode,
        text: match payload.mode {
            ScreenshotMode::Text => payload.text.clone(),
            ScreenshotMode::Image => None,
        },
        image_attached: matches!(payload.mode, ScreenshotMode::Image),
        // The schema requires a positive dimension. A capture that reported none is a
        // defect of the capture pipeline, and losing the user's message over it would be a
        // worse answer than sending it with the smallest dimension the schema allows.
        width: payload.width.max(1),
        height: payload.height.max(1),
        redactions: payload.redactions,
        ocr_engine: payload.ocr_engine.clone(),
    }
}

/// Refuses, in a debug build, an outcome the published schema would.
fn debug_assert_valid(outcome: &Outcome) {
    #[cfg(debug_assertions)]
    {
        let value = serde_json::to_value(outcome).expect("an outcome serialises");
        if let Err(problems) = validate(Document::Outcome, &value) {
            let where_ = problems
                .iter()
                .map(|problem| format!("{} ({})", problem.path, problem.keyword))
                .collect::<Vec<_>>()
                .join(", ");
            panic!("the store built an outcome the schema refuses: {where_}");
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = outcome;
        let _ = validate;
        let _ = Document::Outcome;
    }
}

/// Outcomes for the tests of the other modules of the store.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A minimal outcome that validates: what a test needs when it only cares that *an*
    /// outcome is there.
    pub(crate) fn any_outcome() -> Outcome {
        Outcome {
            outcome_version: OUTCOME_VERSION,
            handoff_id: Some("hf_0000000001".to_owned()),
            status: OutcomeStatus::Abandoned,
            is_final: true,
            instruction: instruction_for(OutcomeStatus::Abandoned, Some("hf_0000000001")),
            round: 1,
            current_step: None,
            user_text: None,
            screenshot: None,
            context: None,
            skipped_steps: Vec::new(),
            notes: Vec::new(),
            secret_treated: Vec::new(),
            verify: None,
            deferral_count: 0,
            resumed_from: None,
            app_reachable: true,
            already_delivered: false,
            runbooks: Vec::new(),
            spec_text: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::outcome::ResumedFrom;
    use crate::format::outcome::SecretTreated;
    use crate::format::spec::{HandoffSpec, HandoffStep};
    use crate::log::Timestamp;
    use crate::store::handoff::{Opener, Round};

    fn at(text: &str) -> Timestamp {
        Timestamp::parse(text).expect("rfc 3339")
    }

    fn spec() -> HandoffSpec {
        let mut values = IndexMap::new();
        values.insert(
            "endpoint_url".to_owned(),
            SpecValue::One("https://api.example.test/hook".to_owned()),
        );
        values.insert(
            "api_key".to_owned(),
            SpecValue::One("sk_live_0123456789abcdefghij".to_owned()),
        );
        HandoffSpec {
            spec_version: 1,
            goal: "Register the webhook".to_owned(),
            r#where: "Dashboard → Webhooks".to_owned(),
            url: None,
            why_human: "only a person can log in".to_owned(),
            values,
            secrets: None,
            steps: vec![
                HandoffStep {
                    text: "Paste the endpoint URL.".to_owned(),
                    url: Some("https://dashboard.example.test/hooks".to_owned()),
                    values: Some(vec!["endpoint_url".to_owned(), "api_key".to_owned()]),
                    warning: Some("do not press Delete".to_owned()),
                },
                HandoffStep {
                    text: "Save and copy the signing secret.".to_owned(),
                    url: None,
                    values: None,
                    warning: None,
                },
            ],
            verify: Some("the webhook fires".to_owned()),
            lang: Some("en".to_owned()),
        }
    }

    fn handoff() -> Handoff {
        Handoff::opened(
            "hf_7k3m9p2q4r".to_owned(),
            spec(),
            vec![SecretTreated {
                location: "values.api_key".to_owned(),
                kind: "api_key".to_owned(),
            }],
            &Opener {
                session_ref: "ses_00000001".to_owned(),
                agent_id: Some("claude-code".to_owned()),
                client_name: Some("claude-code".to_owned()),
                project_dir: Some("C:\\projects\\baton".to_owned()),
                label: ResumedFrom {
                    agent: "Claude Code".to_owned(),
                    project: "baton".to_owned(),
                },
            },
            &at("2026-09-08T11:00:00Z"),
        )
    }

    #[test]
    fn every_status_has_an_instruction_and_a_finality() {
        for status in ALL_STATUSES {
            let text = instruction_for(status, Some("hf_7k3m9p2q4r"));
            assert!(!text.is_empty(), "{status:?} has no instruction");
            assert!(
                !text.contains("<id>"),
                "{status:?} left a placeholder behind: {text}"
            );
        }
        assert!(is_final(OutcomeStatus::Verified));
        assert!(is_final(OutcomeStatus::Abandoned));
        assert!(!is_final(OutcomeStatus::Deferred));
        assert!(!is_final(OutcomeStatus::TransferredToOtherSession));
    }

    #[test]
    fn the_instructions_are_the_texts_the_golden_fixtures_carry() {
        // The five the channel goldens pin literally, so `fake-server` (T-034) can replay a
        // sequence and compare what the real app produced.
        assert_eq!(
            instruction_for(OutcomeStatus::Deferred, Some("hf_7k3m9p2q4r")),
            "Park this step, continue work that does not depend on it, then call \
             handoff_to_user with resume=hf_7k3m9p2q4r before you conclude."
        );
        assert_eq!(
            instruction_for(OutcomeStatus::Parked, Some("hf_7k3m9p2q4r")),
            "Do not resume now. The handoff stays in the overlay; mention hf_7k3m9p2q4r as \
             pending in your final summary."
        );
        assert_eq!(
            instruction_for(OutcomeStatus::Verified, None),
            "Recorded as verified; a runbook was saved."
        );
        assert_eq!(
            instruction_for(
                OutcomeStatus::TransferredToOtherSession,
                Some("hf_9p2r4k7m3t")
            ),
            "Another session took over hf_9p2r4k7m3t. Do nothing further with it."
        );
        assert_eq!(
            screenshot_instruction(2, ScreenshotMode::Text),
            "The user sent what they see at step 2 as extracted text. Answer on this step: \
             call handoff_to_user with handoff_id and reply; add replacement_steps only if \
             the remaining steps must change."
        );
    }

    #[test]
    fn a_note_reaches_the_agent_with_its_certain_matches_replaced() {
        // The one typed text `act` does not redact, because RESP-02 keeps a note local and
        // the user reads it back on their own step. RESP-03 still reports it in the final
        // outcome, so the mask is applied here, where it leaves (PRIN-09).
        let mut handoff = handoff();
        handoff
            .current_round_mut()
            .expect("a round")
            .notes
            .push(OutcomeNote {
                step: 1,
                text: "the field already held AKIAIOSFODNN7EXAMPLE".to_owned(),
                at: "2026-09-10T09:00:00.000Z".to_owned(),
            });

        let outcome = build(&handoff, OutcomeStatus::Verified, None, None);
        let note = outcome.notes.first().expect("the note travels");
        assert_eq!(note.text, "the field already held [REDACTED:api_key]");
        assert_eq!(note.step, 1);
        assert_eq!(note.at, "2026-09-10T09:00:00.000Z");
        // And the handoff still holds what the user wrote: the overlay draws this one.
        assert_eq!(
            handoff.current_round().expect("a round").notes[0].text,
            "the field already held AKIAIOSFODNN7EXAMPLE"
        );
    }

    #[test]
    fn an_ordinary_note_is_not_touched_on_its_way_out() {
        let mut handoff = handoff();
        handoff
            .current_round_mut()
            .expect("a round")
            .notes
            .push(OutcomeNote {
                step: 2,
                text: "the button is called Add destination now".to_owned(),
                at: "2026-09-10T09:01:00.000Z".to_owned(),
            });
        let outcome = build(&handoff, OutcomeStatus::Verified, None, None);
        assert_eq!(
            outcome.notes[0].text,
            "the button is called Add destination now"
        );
    }

    #[test]
    fn a_question_carries_the_context_of_its_step_with_secret_values_withheld() {
        let mut handoff = handoff();
        handoff
            .current_round_mut()
            .expect("a round")
            .skipped
            .push(2);
        let outcome = build(
            &handoff,
            OutcomeStatus::Question,
            Some("which button?".to_owned()),
            None,
        );

        assert_eq!(outcome.status, OutcomeStatus::Question);
        assert!(!outcome.is_final);
        assert_eq!(outcome.user_text.as_deref(), Some("which button?"));
        assert_eq!(outcome.skipped_steps, vec![2]);

        let context = outcome.context.expect("a question carries context");
        assert_eq!(context.goal, "Register the webhook");
        assert_eq!(context.step.index, 1);
        assert_eq!(context.step.total, 2);
        assert_eq!(context.step.warning.as_deref(), Some("do not press Delete"));
        assert_eq!(
            context.step_values.get("api_key"),
            Some(&SpecValue::One(CONTEXT_SECRET_PLACEHOLDER.to_owned()))
        );
        assert_eq!(
            context.step_values.get("endpoint_url"),
            Some(&SpecValue::One("https://api.example.test/hook".to_owned()))
        );
    }

    #[test]
    fn a_final_outcome_carries_no_context_and_says_so() {
        let mut handoff = handoff();
        handoff.state = crate::log::HandoffState::ConfirmedByUser;
        let outcome = build(&handoff, OutcomeStatus::ConfirmedByUser, None, None);
        assert!(outcome.is_final);
        assert!(outcome.context.is_none());
        assert!(outcome.screenshot.is_none());
        assert!(outcome.runbooks.is_empty());
        assert!(outcome.spec_text.is_none());
        assert!(outcome.app_reachable);
        assert!(!outcome.already_delivered);
        assert!(already_delivered(&outcome).already_delivered);
    }

    #[test]
    fn a_screenshot_reports_everything_but_the_pixels() {
        let handoff = handoff();
        let payload = ScreenshotPayload {
            mode: ScreenshotMode::Image,
            text: Some("this text is not sent in image mode".to_owned()),
            image_base64: Some("PIXELS".to_owned()),
            image_sha256: Some("0".repeat(64)),
            width: 2880,
            height: 1800,
            redactions: 2,
            redaction_boxes_json: Some("[]".to_owned()),
            ocr_engine: Some("vision".to_owned()),
            patterns_version: Some("1".to_owned()),
            comment: Some("I only see one option".to_owned()),
        };
        let outcome = build(
            &handoff,
            OutcomeStatus::Screenshot,
            payload.comment.clone(),
            Some(&payload),
        );
        let shot = outcome.screenshot.clone().expect("a screenshot");
        assert_eq!(shot.mode, ScreenshotMode::Image);
        assert!(shot.image_attached);
        assert_eq!(shot.text, None, "image mode sends no text");
        assert_eq!(shot.width, 2880);
        assert_eq!(shot.redactions, 2);
        let json = serde_json::to_string(&outcome).expect("serialisable");
        assert!(!json.contains("PIXELS"));
    }

    #[test]
    fn the_round_and_the_counters_come_from_the_current_round() {
        let mut handoff = handoff();
        handoff.rounds.push(Round::new(
            2,
            vec![HandoffStep {
                text: "start again from the error".to_owned(),
                url: None,
                values: None,
                warning: None,
            }],
            &at("2026-09-08T12:00:00Z"),
        ));
        handoff.cursor.round = 2;
        handoff.cursor.step_index = 1;

        let outcome = build(&handoff, OutcomeStatus::Question, None, None);
        assert_eq!(outcome.round, 2);
        let step = outcome.current_step.expect("a step");
        assert_eq!(step.index, 1);
        assert_eq!(step.total, 1);
        assert_eq!(step.text, "start again from the error");
    }
}
