//! Identifiers against the certain patterns (§4.1, §11.2 "no generated id matches any
//! pattern").
//!
//! `hf_` is deliberately the prefix of a Hugging Face access token, whose pattern requires
//! thirty-four characters after it. Ten can never reach that, but the claim is worth
//! checking rather than reasoning about: an alphabet, a length or a pattern could change,
//! and the day one of them does, a handoff id would start being masked out of the very text
//! that names it.

use handoff_app_lib::ids::{
    new_handoff_id, new_request_id, new_runbook_id, new_session_ref, CALL_ID_RE, HANDOFF_ID_RE,
    ID_ALPHABET, REQUEST_ID_RE, RUNBOOK_ID_RE, SESSION_REF_RE,
};
use handoff_app_lib::redaction::certain::scan_text;
use regex::Regex;

/// The design's own count (§11.2). Enough that a one-in-a-thousand shape would show.
const HOW_MANY: usize = 10_000;

fn none_of_ten_thousand_looks_like_a_secret(make: fn() -> String, shape: &Regex, what: &str) {
    let mut broken: Vec<String> = Vec::new();
    for _ in 0..HOW_MANY {
        let id = make();
        if !shape.is_match(&id) || !scan_text(&id).is_empty() {
            broken.push(id);
        }
    }
    assert!(
        broken.is_empty(),
        "{what}: {} of {HOW_MANY} broke their shape or looked like a secret ({broken:?})",
        broken.len()
    );
}

#[test]
fn ten_thousand_handoff_ids_never_match_a_certain_pattern() {
    none_of_ten_thousand_looks_like_a_secret(new_handoff_id, &HANDOFF_ID_RE, "handoff id");
}

#[test]
fn ten_thousand_request_ids_never_match_a_certain_pattern() {
    none_of_ten_thousand_looks_like_a_secret(new_request_id, &REQUEST_ID_RE, "request id");
}

#[test]
fn ten_thousand_session_refs_never_match_a_certain_pattern() {
    none_of_ten_thousand_looks_like_a_secret(new_session_ref, &SESSION_REF_RE, "session ref");
}

#[test]
fn ten_thousand_runbook_ids_never_match_a_certain_pattern() {
    none_of_ten_thousand_looks_like_a_secret(new_runbook_id, &RUNBOOK_ID_RE, "runbook id");
}

#[test]
fn the_alphabet_is_the_crockford_one_and_the_shapes_are_the_designs() {
    assert_eq!(ID_ALPHABET.len(), 32);
    for forbidden in *b"ilou" {
        assert!(!ID_ALPHABET.contains(&forbidden));
    }
    // A Hugging Face token is `hf_` plus thirty-four; a handoff id is `hf_` plus ten, and
    // that is the whole reason the two cannot collide.
    let token = format!("hf_{}", "a".repeat(34));
    assert!(scan_text("hf_0123456789").is_empty());
    assert!(!scan_text(&token).is_empty());
    assert!(!HANDOFF_ID_RE.is_match(&token));
    assert!(CALL_ID_RE.is_match("call_0123456z"));
}
