//! The sequence actually executed (§4.5.1, RUN-02).
//!
//! "A recipe, not a diary": what a runbook records is the steps a person really performed,
//! in the order they performed them, with what happened on each of them attached as
//! annotations. The diary itself stays in the log.
//!
//! # Which steps are in it
//!
//! §4.5.1 writes the rule as "the round's steps minus the ones skipped in that round, minus
//! the steps that a later round replaced before they were confirmed". A step is confirmed,
//! skipped, or neither; and the two subtractions of that sentence leave exactly the
//! confirmed ones, because a step that is neither confirmed nor skipped was never executed:
//! either a correction round replaced it, or the round ended on it. So this module keeps
//! **the steps confirmed in their round**, which is the same set the sentence describes in
//! every case it names, and the right one in the case it does not (`Done` pressed before
//! the last step, which the store accepts, T-033).
//!
//! # Which annotations, and in which order
//!
//! Per step, in this order, because it is the order in which they happened:
//!
//! 1. `correction` on the **first** step of a round that followed another one — the reply
//!    the agent sent with the replacement steps (§4.5.1, F-08).
//! 2. the step's own `note`, `question` and `reply`, oldest first.
//! 3. `error` on the **last** step of a round the agent reported `ok: false` for.
//!
//! The questions and the replies are not in the handoff record: §7.4 keeps no field for
//! them, so they are read from the log through [`Exchanges`], the same pass the overlay
//! draws its history from.

use std::collections::BTreeMap;

use crate::format::outcome::{Annotation, AnnotationKind};
use crate::format::spec::HandoffStep;
use crate::store::handoff::{Handoff, Round};
use crate::store::Exchanges;

/// The longest an annotation's text may be (`handoff-runbook.v1.schema.json`).
///
/// A note is the user's and a reply is the agent's, and neither is bounded by the spec
/// schema, so the one that overruns is cut rather than allowed to refuse the whole file:
/// an annotation is the diary half of a runbook, and losing its tail costs no step.
const MAX_ANNOTATION_LENGTH: usize = 4000;

/// One step of the executed sequence, still carrying the values it was executed with.
///
/// The placeholders are put in afterwards ([`super::placeholders`]), because the
/// substitution needs the whole sequence at once to decide which step describes a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedStep {
    /// The round it was executed in, 1-based.
    pub round: u32,
    /// Its 1-based index inside that round.
    pub index: u32,
    /// The step as the spec (or the replacement) wrote it.
    pub step: HandoffStep,
    /// What happened on it, in the order it happened.
    pub annotations: Vec<Annotation>,
}

/// The sequence actually executed, oldest round first (§4.5.1).
///
/// Empty when no step was ever confirmed, which is a handoff with no recipe in it; the
/// writer refuses that rather than writing a runbook with no steps.
#[must_use]
pub fn executed(handoff: &Handoff, exchanges: &Exchanges) -> Vec<ExecutedStep> {
    let corrections = correction_replies(handoff, exchanges);
    let mut sequence: Vec<ExecutedStep> = Vec::new();

    for (position, round) in handoff.rounds.iter().enumerate() {
        let first = sequence.len();
        for (index, step) in confirmed_steps(round) {
            sequence.push(ExecutedStep {
                round: round.no,
                index,
                step: step.clone(),
                annotations: own_annotations(round, index, exchanges, &corrections),
            });
        }
        if sequence.len() == first {
            // A round nobody confirmed a step in contributes nothing to the recipe, and its
            // error and the correction that followed have no step to sit on. Both are still
            // in the log, which is where a diary belongs.
            continue;
        }
        if let Some(reply) = position
            .checked_sub(1)
            .and_then(|before| handoff.rounds.get(before))
            .and_then(|before| corrections.get(&before.no))
            .and_then(|at| exchanges.replies.get(*at))
        {
            insert_front(
                &mut sequence[first].annotations,
                annotation(AnnotationKind::Correction, &reply.text, round.no),
            );
        }
        if let Some(error) = error_of(round) {
            let last = sequence.len() - 1;
            sequence[last].annotations.push(error);
        }
    }
    sequence
}

/// The reply that carried the replacement steps, per round that was followed by another.
///
/// A `handoff.continue` that replaces the steps also carries a reply, and the store writes
/// it on the step the cursor was on before the round changed — so it is the **last** reply
/// of a round that has a round after it. It answers nothing the user asked: it is the
/// reason the correction round exists, so it becomes that round's `correction` annotation
/// and not a `reply` on the step it happens to sit on.
fn correction_replies(handoff: &Handoff, exchanges: &Exchanges) -> BTreeMap<u32, usize> {
    let mut found = BTreeMap::new();
    for pair in handoff.rounds.windows(2) {
        let Some(before) = pair.first() else { continue };
        if let Some((at, _)) = exchanges
            .replies
            .iter()
            .enumerate()
            .rfind(|(_, reply)| reply.round == before.no)
        {
            found.insert(before.no, at);
        }
    }
    found
}

/// The steps of `round` that were confirmed, with their 1-based indices, in step order.
fn confirmed_steps(round: &Round) -> impl Iterator<Item = (u32, &HandoffStep)> {
    round
        .steps
        .iter()
        .enumerate()
        .filter_map(move |(at, step)| {
            let index = u32::try_from(at).ok()?.checked_add(1)?;
            round.confirmed.contains(&index).then_some((index, step))
        })
}

/// The notes, questions and replies of one step, oldest first.
fn own_annotations(
    round: &Round,
    index: u32,
    exchanges: &Exchanges,
    corrections: &BTreeMap<u32, usize>,
) -> Vec<Annotation> {
    let correction = corrections.get(&round.no).copied();
    let mut found: Vec<(&str, Annotation)> = Vec::new();

    for note in round.notes.iter().filter(|note| note.step == index) {
        found.push((
            note.at.as_str(),
            annotation(AnnotationKind::Note, &note.text, round.no),
        ));
    }
    for question in &exchanges.questions {
        if question.round == round.no && question.step == index {
            found.push((
                question.at.as_str(),
                annotation(AnnotationKind::Question, &question.text, round.no),
            ));
        }
    }
    for (at, reply) in exchanges.replies.iter().enumerate() {
        if reply.round != round.no || reply.step != index {
            continue;
        }
        if correction == Some(at) {
            continue;
        }
        found.push((
            reply.at.as_str(),
            annotation(AnnotationKind::Reply, &reply.text, round.no),
        ));
    }

    // Stable, so two annotations written in the same millisecond keep the order above: what
    // the user wrote on the step, then what they asked, then what was answered.
    found.sort_by(|left, right| left.0.cmp(right.0));
    found
        .into_iter()
        .map(|(_, annotation)| annotation)
        .filter(|annotation| !annotation.text.is_empty())
        .collect()
}

/// The `error` annotation of a round the agent reported a failure for (VER-08).
fn error_of(round: &Round) -> Option<Annotation> {
    let report = round.verify.as_ref()?;
    if report.ok != Some(false) {
        return None;
    }
    let error = annotation(AnnotationKind::Error, report.detail.as_ref()?, round.no);
    (!error.text.is_empty()).then_some(error)
}

/// Puts an annotation first, unless it is empty.
fn insert_front(annotations: &mut Vec<Annotation>, annotation: Annotation) {
    if !annotation.text.is_empty() {
        annotations.insert(0, annotation);
    }
}

/// One annotation, with its text trimmed and cut to what the schema accepts.
fn annotation(kind: AnnotationKind, text: &str, round: u32) -> Annotation {
    Annotation {
        kind,
        text: clamp(text.trim(), MAX_ANNOTATION_LENGTH),
        round,
    }
}

/// `text` cut to `limit` **characters**, on a character boundary.
fn clamp(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    text.chars().take(limit).collect()
}
