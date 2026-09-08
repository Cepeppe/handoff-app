//! Identifier generation and shapes (§4.1).
//!
//! Every identifier of the system is a short prefix plus lowercase Crockford base32, five
//! random bits per character: `hf_` for a handoff (and for a user-opened request, which
//! keeps its id when the spec arrives, DD-13), `ses_` for a channel registration, `rb_` for
//! a runbook. The alphabet drops `i`, `l`, `o` and `u`, so an id read aloud or copied out
//! of a chat cannot turn into a different id.
//!
//! The app mints `hf_`, `ses_` and `rb_`; `call_` belongs to the server, which is the only
//! peer that opens a call, and its shape is here so the app can recognise one.
//!
//! The `hf_` prefix is deliberately shared with Hugging Face access tokens, whose certain
//! pattern requires thirty-four characters after the prefix: ten can never reach it, and
//! `tests/contract/ids.rs` asserts that no generated id of any shape matches any
//! certain-secret pattern.

use std::sync::LazyLock;

use rand::{rng, RngExt};
use regex::Regex;

/// Crockford base32, lowercase: the ten digits and the twenty-two letters that are left.
pub const ID_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// The shapes of §4.1, anchored to the whole string.
///
/// `\A` and `\z` rather than `^` and `$`: the design writes these shapes with `^…$`, which
/// in JavaScript without the `m` flag anchors to the whole string, while the Rust `regex`
/// crate lets `$` match before a trailing newline as well. These two anchors are the ones
/// that mean the same thing on both sides.
pub static HANDOFF_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| compile(r"\Ahf_[0-9a-hjkmnp-tv-z]{10}\z"));
/// A user-opened request has the shape of the handoff it becomes (DD-13).
pub static REQUEST_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| compile(r"\Ahf_[0-9a-hjkmnp-tv-z]{10}\z"));
/// A channel registration; never shown to an agent.
pub static SESSION_REF_RE: LazyLock<Regex> =
    LazyLock::new(|| compile(r"\Ases_[0-9a-hjkmnp-tv-z]{8}\z"));
/// One blocking call; minted by the server, recognised here.
pub static CALL_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| compile(r"\Acall_[0-9a-hjkmnp-tv-z]{8}\z"));
/// A runbook; part of its file name (DD-17).
pub static RUNBOOK_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| compile(r"\Arb_[0-9a-hjkmnp-tv-z]{10}\z"));

fn compile(shape: &str) -> Regex {
    Regex::new(shape).expect("an identifier shape does not compile")
}

/// `length` characters of the alphabet after `prefix`.
///
/// The alphabet has 32 entries and 32 divides 256, so masking the low five bits of a random
/// byte leaves every character equally likely: no rejection loop and no modulo bias.
fn random_id(prefix: &str, length: usize) -> String {
    let mut bytes = vec![0_u8; length];
    rng().fill(bytes.as_mut_slice());
    let mut id = String::with_capacity(prefix.len() + length);
    id.push_str(prefix);
    for byte in bytes {
        id.push(char::from(ID_ALPHABET[usize::from(byte & 31)]));
    }
    id
}

/// A handoff id: `hf_` plus ten characters, fifty random bits.
#[must_use]
pub fn new_handoff_id() -> String {
    random_id("hf_", 10)
}

/// The id of a user-opened request. Same shape as a handoff id on purpose: when the agent
/// sends a spec carrying this `request_id`, the handoff takes the id over (DD-13).
#[must_use]
pub fn new_request_id() -> String {
    new_handoff_id()
}

/// A channel session reference: `ses_` plus eight characters. Never shown to an agent.
#[must_use]
pub fn new_session_ref() -> String {
    random_id("ses_", 8)
}

/// A runbook id: `rb_` plus ten characters. Part of the runbook file name.
#[must_use]
pub fn new_runbook_id() -> String {
    random_id("rb_", 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alphabet_is_crockford_without_i_l_o_and_u() {
        assert_eq!(ID_ALPHABET.len(), 32);
        let unique: std::collections::HashSet<u8> = ID_ALPHABET.iter().copied().collect();
        assert_eq!(unique.len(), 32);
        for forbidden in *b"ilou" {
            assert!(!ID_ALPHABET.contains(&forbidden));
        }
    }

    #[test]
    fn every_generator_produces_the_shape_the_design_writes() {
        assert!(HANDOFF_ID_RE.is_match(&new_handoff_id()));
        assert!(REQUEST_ID_RE.is_match(&new_request_id()));
        assert!(SESSION_REF_RE.is_match(&new_session_ref()));
        assert!(RUNBOOK_ID_RE.is_match(&new_runbook_id()));
    }

    #[test]
    fn the_shapes_are_anchored_to_the_whole_string() {
        assert!(!HANDOFF_ID_RE.is_match("hf_0123456789\n"));
        assert!(!HANDOFF_ID_RE.is_match(" hf_0123456789"));
        assert!(!HANDOFF_ID_RE.is_match("hf_0123456789x"));
        // `i`, `l`, `o` and `u` are not in the alphabet and not in the shape.
        assert!(!HANDOFF_ID_RE.is_match("hf_iiiiiiiiii"));
    }

    #[test]
    fn a_call_id_is_recognised_though_the_app_never_mints_one() {
        assert!(CALL_ID_RE.is_match("call_4m7q2t9x"));
        assert!(!CALL_ID_RE.is_match("call_4m7q2t9"));
    }
}
