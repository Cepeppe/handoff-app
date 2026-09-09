//! Typed text, before it leaves the machine (§7.10, DET-01, PRIN-09).
//!
//! "Typed text (Ask, comments, edited OCR text) runs through the same detectors before
//! send" (§7.10). This is the certain half of that sentence: the same patterns the ingress
//! scan and the screenshot pipeline use, applied to what the user wrote in an Ask, a Defer
//! or an Abandon sheet, so that the sheet can show the redacted text **before** the send
//! rather than after it (PREV-01's rule, applied to text the user typed).
//!
//! The mask is `[REDACTED:<kind>]`, which is the one §7.10 names for text the app sends.
//! It is deliberately not the log's `[treated as secret: <kind>]` ([`crate::log::redact`]):
//! that one is what a stored spec keeps, this one is what an agent reads.
//!
//! Only the **certain** level lives here. The suspected heuristics — long random strings,
//! tokens next to a `key`/`secret`/`password` label — are T-048's, and the sheet grows a
//! second, user-decidable level then; a certain match is never the user's to unlock (DET-01).
// TASK: T-048 — the suspected level over the same text.

use serde::Serialize;

use super::certain::{scan_text, CertainSecretKind};

/// What a redacted text looks like once a certain pattern has matched.
#[must_use]
pub fn mask_for(kind: CertainSecretKind) -> String {
    format!("[REDACTED:{}]", kind.as_str())
}

/// The typed text as it would be sent, and what was taken out of it.
///
/// It crosses into the webview, so it carries families and never the matched text (R-19):
/// the sheet says "an api_key was removed", the value itself is gone by then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Redacted {
    /// The text with every certain match replaced by its mask.
    pub text: String,
    /// The families that matched, in the order they appear, without repetition.
    pub kinds: Vec<String>,
}

impl Redacted {
    /// Whether anything was replaced.
    #[must_use]
    pub fn is_redacted(&self) -> bool {
        !self.kinds.is_empty()
    }
}

/// Runs the certain detector over `text` and replaces every match (§7.10).
#[must_use]
pub fn redact(text: &str) -> Redacted {
    let hits = scan_text(text);
    if hits.is_empty() {
        return Redacted {
            text: text.to_owned(),
            kinds: Vec::new(),
        };
    }

    let mut redacted = String::with_capacity(text.len());
    let mut kinds: Vec<String> = Vec::new();
    let mut cursor = 0_usize;
    for hit in hits {
        // `scan_text` returns non-overlapping hits ordered by position, so the spans are
        // walked once, left to right, with no bookkeeping.
        redacted.push_str(&text[cursor..hit.start]);
        redacted.push_str(&mask_for(hit.kind));
        let kind = hit.kind.as_str().to_owned();
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
        cursor = hit.end;
    }
    redacted.push_str(&text[cursor..]);

    Redacted {
        text: redacted,
        kinds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key of the AWS shape, which `patterns/certain-secrets.v1.json` calls an `api_key`.
    const AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

    #[test]
    fn ordinary_text_is_left_exactly_as_it_was() {
        let text = "The webhook did not fire. Step 2 says to click Save, and I did.";
        let redacted = redact(text);
        assert_eq!(redacted.text, text);
        assert!(redacted.kinds.is_empty());
        assert!(!redacted.is_redacted());
    }

    #[test]
    fn a_certain_secret_is_replaced_and_its_family_is_reported() {
        let redacted = redact(&format!("I pasted {AWS_KEY} and it was refused"));
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
        let redacted = redact(&format!("why does {AWS_KEY} not work?"));
        assert!(redacted.text.starts_with("why does "));
        assert!(redacted.text.ends_with(" not work?"));
    }

    #[test]
    fn several_matches_are_all_replaced_and_each_family_named_once() {
        let text = format!("first {AWS_KEY} then {AWS_KEY}");
        let redacted = redact(&text);
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
        let redacted = redact(&format!("però {AWS_KEY} → niente"));
        assert_eq!(redacted.text, "però [REDACTED:api_key] → niente");
    }
}
