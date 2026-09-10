//! Detection, redaction geometry and burn-in (§7.10, DET-01..04, PREV-01..05).
//!
//! Two detectors over the OCR result: the **certain** patterns, compiled from the pattern
//! file of the pinned server artifact (never a copy of it, §3.4), and the **suspected**
//! heuristics that belong to the app alone. What they find becomes geometry, the geometry
//! is burned into the pixels before anything leaves the machine, and the preview the user
//! must confirm shows the burned image (PRIN-09).
//!
//! - [`certain`] — the public patterns of §4.6, over a spec at ingress and over a capture.
//! - [`suspected`] — the four heuristics of §7.10 and the exemption list of DET-03.
//! - [`boxes`] — what they find, as rectangles, plus the [`boxes::RedactionPlan`] the user
//!   edits in the preview.
//! - [`burn`] — crop, downscale, rescale, expand, fill, encode (CAP-05, CAP-06), and the
//!   same decision applied to the text pane.
//! - [`typed`] — the two detectors over what the user writes in a sheet, before it is sent.
//!
//! # Where a match becomes a mask, and which mask
//!
//! Three texts stand for a secret in this application and they are not interchangeable:
//!
//! | Mask | Where | Written by |
//! |---|---|---|
//! | `[treated as secret: <kind>]` | the log and a runbook, for a spec value | [`crate::log::redact`] |
//! | `[REDACTED:<kind>]` | text an agent reads | [`typed::mask_for`] |
//! | [`burn::SUSPECTED_MASK`] | text an agent reads, suspected level | [`burn::redact_text`] |
//!
//! # Who may lift what (DET-01)
//!
//! A certain match is never the user's to unlock, in a preview or in a sheet: the patterns
//! are precision-first by policy (§4.6) and a false positive there is a defect of the
//! pattern file, not a decision to hand to somebody in a hurry. A suspected match is
//! exactly the opposite — it is a heuristic, R-07 is the risk of it being wrong too often,
//! and DET-01 says the user decides. In the preview that decision is a click on the box; in
//! a sheet it is the user's own hands on their own sentence, so the sheet marks the words
//! and sends what was typed.

pub mod boxes;
pub mod burn;
pub mod certain;
pub mod suspected;
pub mod typed;

/// Replaces the given spans of `text`, which must be sorted and must not overlap.
///
/// Shared by the two masking paths so that they cannot disagree about what "replace the
/// span" means at a character boundary: the offsets are byte offsets, and a slice on the
/// wrong one panics rather than fails.
pub(crate) fn splice(text: &str, replacements: &[(usize, usize, String)]) -> String {
    if replacements.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0_usize;
    for (start, end, mask) in replacements {
        if *start < cursor {
            continue;
        }
        out.push_str(&text[cursor..*start]);
        out.push_str(mask);
        cursor = *end;
    }
    out.push_str(&text[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splicing_keeps_what_is_between_the_spans() {
        let replaced = splice(
            "one two three",
            &[(0, 3, "[a]".to_owned()), (8, 13, "[b]".to_owned())],
        );
        assert_eq!(replaced, "[a] two [b]");
    }

    #[test]
    fn splicing_nothing_returns_the_text() {
        assert_eq!(splice("untouched", &[]), "untouched");
    }
}
