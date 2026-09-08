//! The one thing the log is not allowed to store (LOG-02, DET-04, FM-15).
//!
//! The spec that reaches the app is the **true** spec: §5.5 is explicit that the server
//! does not mask it, because the copy button has to copy the real value. What may never be
//! written down is the same value: "log and runbook keep a placeholder" (DET-04, §7.6), and
//! §7.11 stores `spec_json` with "secret-treated values replaced by placeholders".
//!
//! # What the placeholder is
//!
//! `[treated as secret: <kind>]`, the mask of §5.9 — the same text the server renders in
//! text mode, and the one the vendored `docs/text-mode.md` calls the mask used "everywhere
//! else". `kind` is the family (`api_key`, `token`, …), never the pattern id, so the log
//! does not record which vendor issued the secret. §7.11 says "placeholders" and never says
//! which; this is the choice, and `DEVIATIONS.md` records it.
//!
//! # Where it is applied
//!
//! Only the matched span is replaced, not the field (the rule the server settled in T-014):
//! a step reading "paste sk_live_… into .env" keeps its instruction. The scan is this
//! crate's own [`crate::redaction::certain`], not the `secret_treated` list the server
//! sent — that list carries locations and families but no spans, and re-running the same
//! patterns is also the last defence §4.5.2 asks the runbook writer for.
//!
//! Beside the spec, [`sweep`] removes the same literals from any other text of the same
//! row. `state_json` is opaque to the log and would otherwise carry the values the spec no
//! longer does, which would make the masking of `spec_json` decorative.

use crate::format::spec::{HandoffSpec, HandoffStep, SpecValue};
use crate::redaction::certain::scan_text;

/// What a secret-treated value looks like once it is written down.
#[must_use]
pub fn mask_for(kind: &str) -> String {
    format!("[treated as secret: {kind}]")
}

/// The masked text, and every literal that was masked out of it.
fn mask_text(text: &str) -> (String, Vec<String>) {
    let hits = scan_text(text);
    if hits.is_empty() {
        return (text.to_owned(), Vec::new());
    }
    let mut masked = String::with_capacity(text.len());
    let mut literals = Vec::with_capacity(hits.len());
    let mut cursor = 0_usize;
    for hit in hits {
        // `scan_text` returns non-overlapping hits ordered by position, so the spans can be
        // walked once, left to right, with no bookkeeping.
        masked.push_str(&text[cursor..hit.start]);
        masked.push_str(&mask_for(hit.kind.as_str()));
        literals.push(text[hit.start..hit.end].to_owned());
        cursor = hit.end;
    }
    masked.push_str(&text[cursor..]);
    (masked, literals)
}

/// The spec as the log stores it, and every literal that was taken out of it.
///
/// The fields scanned are the fields of §5.5, which is what [`crate::redaction::certain`]
/// already walks: `values` (array items included), `goal`, `where`, `why_human`, `verify`,
/// and each step's `text` and `warning`. `url` and `steps[].url` are not among them.
#[must_use]
pub fn mask_spec(spec: &HandoffSpec) -> (HandoffSpec, Vec<String>) {
    let mut literals: Vec<String> = Vec::new();
    let mut masked = spec.clone();

    for value in masked.values.values_mut() {
        match value {
            SpecValue::One(text) => {
                let (replaced, found) = mask_text(text);
                *text = replaced;
                literals.extend(found);
            }
            SpecValue::Many(items) => {
                for item in items.iter_mut() {
                    let (replaced, found) = mask_text(item);
                    *item = replaced;
                    literals.extend(found);
                }
            }
        }
    }
    mask_field(&mut masked.goal, &mut literals);
    mask_field(&mut masked.r#where, &mut literals);
    mask_field(&mut masked.why_human, &mut literals);
    if let Some(verify) = masked.verify.as_mut() {
        mask_field(verify, &mut literals);
    }
    for step in &mut masked.steps {
        mask_step(step, &mut literals);
    }

    literals.sort_unstable();
    literals.dedup();
    (masked, literals)
}

fn mask_field(field: &mut String, literals: &mut Vec<String>) {
    let (replaced, found) = mask_text(field);
    *field = replaced;
    literals.extend(found);
}

fn mask_step(step: &mut HandoffStep, literals: &mut Vec<String>) {
    mask_field(&mut step.text, literals);
    if let Some(warning) = step.warning.as_mut() {
        mask_field(warning, literals);
    }
}

/// `text` with every one of `literals` replaced by the mask of its family.
///
/// Used on the columns the log cannot look inside — `state_json`, `request_text` — where a
/// value the spec declared may appear again. The scan runs on the result rather than on the
/// literals list alone, so a secret that only ever appeared outside the spec is caught too.
#[must_use]
pub fn sweep(text: &str, literals: &[String]) -> String {
    let mut swept = text.to_owned();
    // Longest first: a literal that contains another must be replaced before it, or the
    // shorter replacement would cut the longer one in half and leave its tail behind.
    let mut ordered: Vec<&String> = literals.iter().collect();
    ordered.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    for literal in ordered {
        if !swept.contains(literal.as_str()) {
            continue;
        }
        let kind = scan_text(literal)
            .first()
            .map_or_else(|| "api_key".to_owned(), |hit| hit.kind.as_str().to_owned());
        swept = swept.replace(literal.as_str(), &mask_for(&kind));
    }
    let (swept, _) = mask_text(&swept);
    swept
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;

    /// A value that every certain pattern file since T-006 matches as `api_key`.
    const STRIPE_KEY: &str = "sk_live_0123456789abcdefghij";

    fn spec_with(value: &str, step_text: &str) -> HandoffSpec {
        let mut values = IndexMap::new();
        values.insert("api_key".to_owned(), SpecValue::One(value.to_owned()));
        values.insert(
            "list".to_owned(),
            SpecValue::Many(vec!["harmless".to_owned(), value.to_owned()]),
        );
        HandoffSpec {
            spec_version: 1,
            goal: "Create the key".to_owned(),
            r#where: "Stripe dashboard".to_owned(),
            url: None,
            why_human: "Only a person can log in".to_owned(),
            values,
            secrets: None,
            steps: vec![HandoffStep {
                text: step_text.to_owned(),
                url: None,
                values: Some(vec!["api_key".to_owned()]),
                warning: Some(format!("do not commit {value}")),
            }],
            verify: Some("check the key works".to_owned()),
            lang: None,
        }
    }

    #[test]
    fn the_mask_names_the_family_and_not_the_pattern() {
        assert_eq!(mask_for("api_key"), "[treated as secret: api_key]");
    }

    #[test]
    fn a_value_that_is_a_secret_becomes_the_mask_alone() {
        let (masked, literals) = mask_spec(&spec_with(STRIPE_KEY, "open the dashboard"));
        assert_eq!(
            masked.values["api_key"],
            SpecValue::One("[treated as secret: api_key]".to_owned())
        );
        assert_eq!(literals, vec![STRIPE_KEY.to_owned()]);
    }

    #[test]
    fn a_secret_inside_a_sentence_loses_the_span_and_keeps_the_sentence() {
        let (masked, _) = mask_spec(&spec_with(
            STRIPE_KEY,
            &format!("paste {STRIPE_KEY} into .env"),
        ));
        assert_eq!(
            masked.steps[0].text,
            "paste [treated as secret: api_key] into .env"
        );
        assert_eq!(
            masked.steps[0].warning.as_deref(),
            Some("do not commit [treated as secret: api_key]")
        );
    }

    #[test]
    fn every_field_of_the_ingress_scan_is_masked_and_url_is_not() {
        let mut spec = spec_with(STRIPE_KEY, "open the dashboard");
        spec.goal = format!("rotate {STRIPE_KEY}");
        spec.r#where = format!("account {STRIPE_KEY}");
        spec.why_human = format!("only a person may see {STRIPE_KEY}");
        spec.verify = Some(format!("call the api with {STRIPE_KEY}"));
        spec.url = Some(format!("https://example.test/{STRIPE_KEY}"));
        let (masked, _) = mask_spec(&spec);
        for field in [&masked.goal, &masked.r#where, &masked.why_human] {
            assert!(!field.contains(STRIPE_KEY), "{field}");
        }
        assert!(!masked.verify.expect("a verify").contains(STRIPE_KEY));
        // §5.5 does not scan `url`, and the app must not disagree with the server about
        // which fields were scanned.
        assert_eq!(
            masked.url.as_deref(),
            Some(format!("https://example.test/{STRIPE_KEY}").as_str())
        );
    }

    #[test]
    fn a_list_item_is_masked_and_its_neighbours_are_not() {
        let (masked, _) = mask_spec(&spec_with(STRIPE_KEY, "open the dashboard"));
        assert_eq!(
            masked.values["list"],
            SpecValue::Many(vec![
                "harmless".to_owned(),
                "[treated as secret: api_key]".to_owned(),
            ])
        );
    }

    #[test]
    fn a_spec_with_no_secret_comes_back_unchanged() {
        let spec = spec_with("an ordinary value", "open the dashboard");
        let (masked, literals) = mask_spec(&spec);
        assert_eq!(masked, spec);
        assert!(literals.is_empty());
    }

    #[test]
    fn the_sweep_removes_a_literal_from_a_text_the_log_cannot_read() {
        let state = format!("{{\"spec\":{{\"values\":{{\"api_key\":\"{STRIPE_KEY}\"}}}}}}");
        let swept = sweep(&state, &[STRIPE_KEY.to_owned()]);
        assert!(!swept.contains(STRIPE_KEY));
        assert!(swept.contains("[treated as secret: api_key]"));
    }

    #[test]
    fn the_sweep_also_catches_a_secret_no_literal_named() {
        // The literals list comes from the spec; a value that only ever appeared in the
        // state has to be caught by the patterns themselves, or the sweep would be a list
        // lookup pretending to be a guard.
        let other = "sk_test_zyxwvutsrqponmlkjihg";
        let swept = sweep(other, &[STRIPE_KEY.to_owned()]);
        assert_eq!(swept, "[treated as secret: api_key]");
    }

    #[test]
    fn a_literal_that_contains_another_is_replaced_first() {
        let long = format!("{STRIPE_KEY}0123456789");
        let swept = sweep(&long, &[STRIPE_KEY.to_owned(), long.clone()]);
        assert_eq!(swept, "[treated as secret: api_key]");
    }
}
