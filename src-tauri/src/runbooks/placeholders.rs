//! Placeholders (§4.5.2, RUN-04, RUN-05).
//!
//! A runbook holds value **names**, never values. This module is what makes that true: it
//! walks the executed sequence, replaces every literal value by `{{name}}`, describes each
//! name by the step it appeared in, and hands the result to the writer.
//!
//! # Three things happen to a text, in this order
//!
//! 1. **Substitution.** The values are matched longest first, as exact literals, in one
//!    left-to-right pass. An array value has each of its items substituted with the same
//!    `{{name}}`. The pass is left to right and never re-reads what it has written, so a
//!    value that happens to be a substring of another value's *name* cannot end up nested
//!    inside a placeholder it did not produce.
//! 2. **Masking.** A certain secret that is not a declared value — one the ingress detector
//!    found inside a step's text, a warning or the `verify` (§5.5) — has no name to be
//!    replaced by, so it gets the mask of §5.9, `[treated as secret: <kind>]`, exactly as
//!    the log writes it (`log::redact`, T-030). `DEVIATIONS.md` records why: §4.5.2 covers
//!    the value case and nothing else, and the alternative — refusing to write the runbook
//!    at all — would lose the recipe of every handoff whose steps merely mention a key.
//! 3. **The last defence.** The certain detector runs once more over the finished document
//!    (`writer`), and a match aborts the write. After steps 1 and 2 it must find nothing:
//!    it is there to catch a defect in this module, not to do its work.
//!
//! # A secret-treated value
//!
//! Its literal is substituted like any other, so it never reaches the file, and its
//! description is the fixed sentence `[treated as secret at ingress]` rather than the step
//! it appeared in — a description is a sentence about the value, and this one must say
//! nothing about it (DET-04).
//!
//! # After a restart
//!
//! `handoffs.spec_json` and `state_json` come back with every secret-treated literal already
//! replaced by the same mask (LOG-02, T-030), in the spec **and** in the round's steps. The
//! substitution then matches the mask instead of the value, which is the same string in both
//! places and therefore produces the same placeholders. Nothing leaks either way; what is
//! lost after a restart is the true value, which no runbook was ever going to carry.

use indexmap::IndexMap;

use crate::format::runbook::{RunbookStep, RunbookValue};
use crate::format::spec::{HandoffSpec, SpecValue};
use crate::log::redact;

use super::sequence::ExecutedStep;

/// What a value whose literal was treated as a secret at ingress is described as (§4.5.2).
pub const SECRET_DESCRIPTION: &str = "[treated as secret at ingress]";

/// `text` with every certain secret in it masked, and nothing else changed.
///
/// The three fields a runbook copies verbatim — `where`, `goal`, `why_human` — are scanned
/// at ingress like every other text of §5.5 and carry no value name to be replaced by, so
/// they get the same mask a step's text gets. Without it a spec that named a key in its
/// `why_human` would abort its own runbook at the last defence, and RUN-01 promises one for
/// every verified handoff.
#[must_use]
pub fn mask(text: &str) -> String {
    redact::sweep(text, &[])
}

/// The runbook's three placeholdered parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholdered {
    /// The executed steps, with placeholders and their annotations.
    pub steps: Vec<RunbookStep>,
    /// The spec's `verify`, with placeholders, or `None` when the spec had none.
    pub verify: Option<String>,
    /// Every value name the spec declared, with its description.
    pub values: IndexMap<String, RunbookValue>,
}

/// One value name and one literal that stands for it. An array value has one entry per item.
#[derive(Debug, Clone)]
struct Literal {
    name: String,
    text: String,
}

/// Substitutes the values of `spec` throughout `executed` and describes each of them.
///
/// `is_secret` answers "was this value name treated as a secret at ingress" — the handoff's
/// own question ([`crate::store::Handoff::is_secret_value`]), asked as a closure so this
/// module needs no handoff.
#[must_use]
pub fn apply(
    spec: &HandoffSpec,
    executed: &[ExecutedStep],
    is_secret: &dyn Fn(&str) -> bool,
) -> Placeholdered {
    let literals = literals_of(spec);

    let mut steps: Vec<RunbookStep> = Vec::with_capacity(executed.len());
    // name -> the finished text of the first step that used it, which becomes its
    // description (§4.5, `values` row).
    let mut described: IndexMap<String, String> = IndexMap::new();

    for step in executed {
        let (text, in_text) = substitute(&step.step.text, &literals);
        let warning = step
            .step
            .warning
            .as_ref()
            .map(|warning| substitute(warning, &literals));
        let mut used: Vec<String> = in_text.clone();
        if let Some((_, in_warning)) = warning.as_ref() {
            for name in in_warning {
                if !used.contains(name) {
                    used.push(name.clone());
                }
            }
        }
        for name in &in_text {
            described
                .entry(name.clone())
                .or_insert_with(|| text.clone());
        }
        steps.push(RunbookStep {
            text,
            url: step.step.url.clone(),
            values: used,
            warning: warning.map(|(warning, _)| warning),
            annotations: step.annotations.clone(),
        });
    }

    let verify = spec
        .verify
        .as_ref()
        .map(|verify| substitute(verify, &literals).0);

    let mut values: IndexMap<String, RunbookValue> = IndexMap::new();
    for name in spec.values.keys() {
        let description = if is_secret(name) {
            Some(SECRET_DESCRIPTION.to_owned())
        } else {
            described.get(name).cloned()
        };
        values.insert(name.clone(), RunbookValue { description });
    }

    Placeholdered {
        steps,
        verify,
        values,
    }
}

/// Every literal the spec declares, longest first.
///
/// Longest first is §4.5.2's own rule and it is what makes the single pass correct: at any
/// position the first literal that matches is the longest one that could, so a value that
/// is a prefix of another never cuts it short.
fn literals_of(spec: &HandoffSpec) -> Vec<Literal> {
    let mut literals: Vec<Literal> = Vec::new();
    for (name, value) in &spec.values {
        match value {
            SpecValue::One(text) => literals.push(Literal {
                name: name.clone(),
                text: text.clone(),
            }),
            SpecValue::Many(items) => literals.extend(items.iter().map(|item| Literal {
                name: name.clone(),
                text: item.clone(),
            })),
        }
    }
    // An empty literal would match everywhere and consume nothing; S6 already refuses one,
    // and dropping it here means this pass cannot loop whatever it is handed.
    literals.retain(|literal| !literal.text.is_empty());
    literals.sort_by(|left, right| {
        right
            .text
            .len()
            .cmp(&left.text.len())
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.text.cmp(&right.text))
    });
    literals
}

/// `text` with every value replaced by its placeholder and every remaining certain secret
/// masked, and the names that were substituted, in order of first appearance.
fn substitute(text: &str, literals: &[Literal]) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut used: Vec<String> = Vec::new();
    let mut cursor = 0_usize;

    while cursor < text.len() {
        let rest = &text[cursor..];
        if let Some(literal) = literals
            .iter()
            .find(|literal| rest.starts_with(literal.text.as_str()))
        {
            out.push_str("{{");
            out.push_str(&literal.name);
            out.push_str("}}");
            if !used.contains(&literal.name) {
                used.push(literal.name.clone());
            }
            cursor += literal.text.len();
            continue;
        }
        // One character at a time, so `cursor` is always on a boundary and a literal can
        // only ever be matched where one begins.
        let next = rest.chars().next().unwrap_or_default();
        out.push(next);
        cursor += next.len_utf8();
    }

    // Whatever the ingress detector found outside a declared value has no name; the mask of
    // §5.9 is what the log writes for the same string, and `sweep` with no literals is
    // exactly "mask every certain match in this text".
    (mask(&out), used)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::outcome::Annotation;
    use crate::format::spec::HandoffStep;

    fn spec_with(
        values: &[(&str, SpecValue)],
        steps: &[&str],
        verify: Option<&str>,
    ) -> HandoffSpec {
        let mut map = IndexMap::new();
        for (name, value) in values {
            map.insert((*name).to_owned(), value.clone());
        }
        HandoffSpec {
            spec_version: 1,
            goal: "Register the webhook".to_owned(),
            r#where: "Dashboard → Webhooks".to_owned(),
            url: None,
            why_human: "only a person can log in".to_owned(),
            values: map,
            secrets: None,
            steps: steps
                .iter()
                .map(|text| HandoffStep {
                    text: (*text).to_owned(),
                    url: None,
                    values: None,
                    warning: None,
                })
                .collect(),
            verify: verify.map(ToOwned::to_owned),
            lang: Some("en".to_owned()),
        }
    }

    fn executed_of(spec: &HandoffSpec) -> Vec<ExecutedStep> {
        spec.steps
            .iter()
            .enumerate()
            .map(|(at, step)| ExecutedStep {
                round: 1,
                index: u32::try_from(at + 1).expect("a small index"),
                step: step.clone(),
                annotations: Vec::<Annotation>::new(),
            })
            .collect()
    }

    fn never_secret(_: &str) -> bool {
        false
    }

    #[test]
    fn a_value_that_is_a_prefix_of_another_does_not_cut_it_short() {
        let spec = spec_with(
            &[
                ("short", SpecValue::One("acme".to_owned())),
                ("long", SpecValue::One("acme-production".to_owned())),
            ],
            &["Open acme-production, not acme."],
            None,
        );
        let out = apply(&spec, &executed_of(&spec), &never_secret);
        assert_eq!(out.steps[0].text, "Open {{long}}, not {{short}}.");
        assert_eq!(out.steps[0].values, vec!["long", "short"]);
    }

    #[test]
    fn every_item_of_an_array_value_becomes_the_same_placeholder() {
        let spec = spec_with(
            &[(
                "events",
                SpecValue::Many(vec![
                    "payment_intent.succeeded".to_owned(),
                    "charge.refunded".to_owned(),
                ]),
            )],
            &["Select payment_intent.succeeded and charge.refunded."],
            None,
        );
        let out = apply(&spec, &executed_of(&spec), &never_secret);
        assert_eq!(out.steps[0].text, "Select {{events}} and {{events}}.");
        assert_eq!(out.steps[0].values, vec!["events"]);
    }

    #[test]
    fn a_value_that_appears_in_no_step_keeps_its_name_and_loses_its_description() {
        let spec = spec_with(
            &[("unused", SpecValue::One("nowhere-to-be-seen".to_owned()))],
            &["Open the dashboard."],
            None,
        );
        let out = apply(&spec, &executed_of(&spec), &never_secret);
        assert!(out.values.contains_key("unused"));
        assert_eq!(out.values["unused"].description, None);
    }

    #[test]
    fn the_description_is_the_first_step_that_used_the_value() {
        let spec = spec_with(
            &[("token", SpecValue::One("abcdef".to_owned()))],
            &[
                "Open the page.",
                "Paste abcdef in the field.",
                "Paste abcdef again.",
            ],
            None,
        );
        let out = apply(&spec, &executed_of(&spec), &never_secret);
        assert_eq!(
            out.values["token"].description.as_deref(),
            Some("Paste {{token}} in the field.")
        );
    }

    #[test]
    fn a_secret_treated_value_is_described_by_the_fixed_sentence_and_never_by_its_step() {
        let spec = spec_with(
            &[(
                "api_key",
                SpecValue::One("sk_live_0123456789abcdef".to_owned()),
            )],
            &["Paste sk_live_0123456789abcdef into the field."],
            None,
        );
        let out = apply(&spec, &executed_of(&spec), &|name| name == "api_key");
        assert_eq!(out.steps[0].text, "Paste {{api_key}} into the field.");
        assert_eq!(
            out.values["api_key"].description.as_deref(),
            Some(SECRET_DESCRIPTION)
        );
    }

    #[test]
    fn a_certain_secret_that_is_not_a_value_is_masked_and_not_written() {
        let spec = spec_with(
            &[("banner", SpecValue::One("Maintenance tonight".to_owned()))],
            &["Sign in with sk_live_0123456789abcdef and set Maintenance tonight."],
            None,
        );
        let out = apply(&spec, &executed_of(&spec), &never_secret);
        assert_eq!(
            out.steps[0].text,
            "Sign in with [treated as secret: api_key] and set {{banner}}."
        );
    }

    #[test]
    fn the_verify_text_is_placeholdered_too() {
        let spec = spec_with(
            &[(
                "endpoint",
                SpecValue::One("https://example.test/hook".to_owned()),
            )],
            &["Open the page."],
            Some("Send a test event to https://example.test/hook."),
        );
        let out = apply(&spec, &executed_of(&spec), &never_secret);
        assert_eq!(
            out.verify.as_deref(),
            Some("Send a test event to {{endpoint}}.")
        );
    }

    #[test]
    fn a_value_whose_literal_is_a_substring_of_another_values_name_is_not_nested() {
        // `{{banner_text}}` contains "banner", and a second replacement pass over the
        // written text would produce `{{{{banner}}_text}}`.
        let spec = spec_with(
            &[
                (
                    "banner_text",
                    SpecValue::One("Maintenance tonight".to_owned()),
                ),
                ("plain", SpecValue::One("banner".to_owned())),
            ],
            &["Set Maintenance tonight."],
            None,
        );
        let out = apply(&spec, &executed_of(&spec), &never_secret);
        assert_eq!(out.steps[0].text, "Set {{banner_text}}.");
    }

    #[test]
    fn a_warning_is_substituted_and_its_names_reach_the_steps_values() {
        let mut spec = spec_with(
            &[("target", SpecValue::One("production".to_owned()))],
            &["Open the console."],
            None,
        );
        spec.steps[0].warning = Some("This is production.".to_owned());
        let out = apply(&spec, &executed_of(&spec), &never_secret);
        assert_eq!(out.steps[0].warning.as_deref(), Some("This is {{target}}."));
        assert_eq!(out.steps[0].values, vec!["target"]);
    }
}
