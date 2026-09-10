//! Typed text, before it leaves the machine (§7.10, DET-01, PRIN-09).
//!
//! "Typed text (Ask, comments, edited OCR text) runs through the same detectors before
//! send" (§7.10). Both levels are here, and DET-01 gives them two different treatments,
//! which is the one thing to understand about this file:
//!
//! - a **certain** match is "redacted automatically and shown redacted in the preview": the
//!   span becomes `[REDACTED:<kind>]` and there is no way for the user to put it back;
//! - a **suspected** match is "highlighted with the warning *may contain a secret*; the
//!   user decides": the words are marked, and they are sent as they were written.
//!
//! The asymmetry with the screenshot pipeline is deliberate and it is the same rule seen
//! from two sides. A capture is the machine's reading of a screen the user did not compose,
//! so a flagged box is drawn by default and unlocking it is one click (PREV-02). A sheet
//! holds a sentence the user has just written, character by character, in the window they
//! are looking at: marking the words *is* handing them the decision, and quietly rewriting
//! their sentence would take it away. Nothing is hidden either way — the sheet shows what
//! the agent will read before the button is pressed.
//!
//! The exemption list of DET-03 applies here too: a value the agent itself sent in the spec
//! is not a suspicion, or every Ask that quotes an endpoint id would carry a warning.

use serde::Serialize;

use super::certain::{scan_text as scan_certain, CertainSecretKind};
use super::splice;
use super::suspected::{scan_text as scan_suspected, Exemptions};

/// What a redacted text looks like once a certain pattern has matched.
#[must_use]
pub fn mask_for(kind: CertainSecretKind) -> String {
    format!("[REDACTED:{}]", kind.as_str())
}

/// One run of the text the sheet draws, marked or not (DET-01).
///
/// The sheet renders the runs in order, giving the marked ones the warning style, which is
/// what "highlighted" means for a text a `<textarea>` cannot style in place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextSegment {
    /// The characters of this run.
    pub text: String,
    /// Whether the suspected detector picked this run out.
    pub suspected: bool,
}

/// The typed text as it would be sent, and what the two detectors made of it.
///
/// It crosses into the webview. The certain half carries families and never the matched
/// text (R-19); the suspected half carries the user's own words, which the webview already
/// has — they are what is in the box they are typing into.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Redacted {
    /// The text with every certain match replaced by its mask. This is what is sent.
    pub text: String,
    /// The certain families that matched, in the order they appear, without repetition.
    pub kinds: Vec<String>,
    /// The suspected rules that fired, in order, without repetition. Never the words.
    pub reasons: Vec<String>,
    /// `text`, cut into runs so that the sheet can mark the suspected ones.
    pub segments: Vec<TextSegment>,
}

impl Redacted {
    /// Whether a certain match was replaced.
    #[must_use]
    pub fn is_redacted(&self) -> bool {
        !self.kinds.is_empty()
    }

    /// Whether the suspected detector has something to warn about.
    #[must_use]
    pub fn is_suspected(&self) -> bool {
        !self.reasons.is_empty()
    }
}

/// Where each mask ended up in the spliced text.
///
/// `replacements` is what [`splice`] was given, sorted and non-overlapping, so walking it
/// once is enough: every span before a mask keeps its length, and the mask's own length is
/// what moves the rest along.
fn spans_of_masks(replacements: &[(usize, usize, String)]) -> Vec<(usize, usize)> {
    let mut spans = Vec::with_capacity(replacements.len());
    let mut source = 0_usize;
    let mut target = 0_usize;
    for (start, end, mask) in replacements {
        target += start.saturating_sub(source);
        spans.push((target, target + mask.len()));
        target += mask.len();
        source = *end;
    }
    spans
}

/// Runs both detectors over `text` and applies each level's treatment (§7.10, DET-01).
#[must_use]
pub fn redact(text: &str, exempt: &Exemptions) -> Redacted {
    let hits = scan_certain(text);
    let mut kinds: Vec<String> = Vec::new();
    let replacements: Vec<(usize, usize, String)> = hits
        .iter()
        .map(|hit| {
            let kind = hit.kind.as_str().to_owned();
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
            (hit.start, hit.end, mask_for(hit.kind))
        })
        .collect();
    let masked = splice(text, &replacements);
    let mask_spans = spans_of_masks(&replacements);

    // The suspected pass runs on the **masked** text, so that its spans are offsets into
    // what is actually sent; the masks themselves are then skipped, or a text made only of
    // them would be reported as suspicious in its own right.
    let mut reasons: Vec<String> = Vec::new();
    let mut segments: Vec<TextSegment> = Vec::new();
    let mut cursor = 0_usize;
    for hit in scan_suspected(&masked, exempt) {
        if mask_spans
            .iter()
            .any(|(start, end)| hit.start < *end && *start < hit.end)
        {
            continue;
        }
        let reason = hit.reason.as_str().to_owned();
        if !reasons.contains(&reason) {
            reasons.push(reason);
        }
        if hit.start > cursor {
            segments.push(TextSegment {
                text: masked[cursor..hit.start].to_owned(),
                suspected: false,
            });
        }
        segments.push(TextSegment {
            text: masked[hit.start..hit.end].to_owned(),
            suspected: true,
        });
        cursor = hit.end;
    }
    if cursor < masked.len() {
        segments.push(TextSegment {
            text: masked[cursor..].to_owned(),
            suspected: false,
        });
    }

    Redacted {
        text: masked,
        kinds,
        reasons,
        segments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key of the AWS shape, which `patterns/certain-secrets.v1.json` calls an `api_key`.
    const AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

    fn plain(text: &str) -> Redacted {
        redact(text, &Exemptions::none())
    }

    #[test]
    fn ordinary_text_is_left_exactly_as_it_was() {
        let text = "The webhook did not fire. Step 2 says to click Save, and I did.";
        let redacted = plain(text);
        assert_eq!(redacted.text, text);
        assert!(redacted.kinds.is_empty());
        assert!(redacted.reasons.is_empty());
        assert!(!redacted.is_redacted());
        assert!(!redacted.is_suspected());
        assert_eq!(
            redacted.segments,
            vec![TextSegment {
                text: text.to_owned(),
                suspected: false
            }]
        );
    }

    #[test]
    fn a_certain_secret_is_replaced_and_its_family_is_reported() {
        let redacted = plain(&format!("I pasted {AWS_KEY} and it was refused"));
        assert_eq!(
            redacted.text,
            "I pasted [REDACTED:api_key] and it was refused"
        );
        assert_eq!(redacted.kinds, vec!["api_key".to_owned()]);
        assert!(redacted.is_redacted());
        assert!(
            !redacted.text.contains(AWS_KEY),
            "the value survived its own redaction"
        );
    }

    #[test]
    fn only_the_span_is_replaced_so_the_sentence_survives() {
        // The rule the server settled in T-014 and the log follows: what is masked is the
        // match, never the field, or a question would lose the question.
        let redacted = plain(&format!("why does {AWS_KEY} not work?"));
        assert!(redacted.text.starts_with("why does "));
        assert!(redacted.text.ends_with(" not work?"));
    }

    #[test]
    fn several_matches_are_all_replaced_and_each_family_named_once() {
        let text = format!("first {AWS_KEY} then {AWS_KEY}");
        let redacted = plain(&text);
        assert_eq!(
            redacted.text,
            "first [REDACTED:api_key] then [REDACTED:api_key]"
        );
        assert_eq!(redacted.kinds, vec!["api_key".to_owned()]);
    }

    #[test]
    fn the_mask_is_the_one_of_the_section_and_not_the_logs() {
        assert_eq!(mask_for(CertainSecretKind::Jwt), "[REDACTED:jwt]");
        assert_ne!(
            mask_for(CertainSecretKind::ApiKey),
            crate::log::redact::mask_for("api_key"),
            "the sent mask and the stored mask are two different texts"
        );
    }

    #[test]
    fn text_around_a_multi_byte_character_is_cut_on_character_boundaries() {
        // The spans are byte offsets; a slice on the wrong boundary would panic rather than
        // fail, and a user writing in Italian or pasting a → is the ordinary case.
        let redacted = plain(&format!("però {AWS_KEY} → niente"));
        assert_eq!(redacted.text, "però [REDACTED:api_key] → niente");
    }

    #[test]
    fn a_suspected_token_is_marked_and_still_sent() {
        // DET-01: the user decides. The words reach the agent; the sheet says which ones
        // the detector is unsure about.
        let redacted = plain("the password is hunter2-tango");
        assert_eq!(redacted.text, "the password is hunter2-tango");
        assert_eq!(redacted.reasons, vec!["label".to_owned()]);
        assert!(redacted.is_suspected());
        assert_eq!(
            redacted.segments,
            vec![
                TextSegment {
                    text: "the password is ".to_owned(),
                    suspected: false
                },
                TextSegment {
                    text: "hunter2-tango".to_owned(),
                    suspected: true
                },
            ]
        );
    }

    #[test]
    fn a_spec_value_the_agent_sent_is_not_a_suspicion() {
        let text = "I cannot find we_1P9xTz2eZvKYlo2C0Sd8h4kL on the page";
        assert!(plain(text).is_suspected());
        let exempt = Exemptions::of_values(["we_1P9xTz2eZvKYlo2C0Sd8h4kL"]);
        assert!(!redact(text, &exempt).is_suspected());
    }

    #[test]
    fn a_mask_is_never_itself_flagged_as_a_long_random_token() {
        let redacted = plain(&format!("{AWS_KEY} {AWS_KEY} {AWS_KEY}"));
        assert!(redacted.reasons.is_empty(), "{:?}", redacted.reasons);
    }
}
