//! The suspected detector (§7.10, DET-01, DET-03, R-06, R-07).
//!
//! The certain patterns are the server's and they are public ([`super::certain`]); this is
//! the half that belongs to the app alone. It has no list of vendors and no prefixes: it
//! looks at the **shape** of a token and at the **words next to it**, which is what DET-01
//! calls "long random strings; fields labelled key, secret, token, password".
//!
//! # The four rules of §7.10
//!
//! A token is suspected when any of these holds, and they are tried in this order so that
//! the reason a user is shown is the most specific one:
//!
//! 1. [`SuspectedReason::Hex`] — 32 characters or more, all hexadecimal.
//! 2. [`SuspectedReason::Base64`] — 24 characters or more from the base64 alphabet (the
//!    URL-safe one included), mixing letters and digits ([`is_mixed`]).
//! 3. [`SuspectedReason::Entropy`] — 20 characters or more, Shannon entropy above
//!    [`ENTROPY_BITS_PER_CHAR`] bits per character, mixing letters and digits.
//! 4. [`SuspectedReason::Label`] — a value-shaped token on a line that carries a label
//!    matching `key|secret|token|password|passwd|pwd|bearer|api`, or on the line under it.
//!
//! Rules 1 to 3 are properties of the string and need no context. Rule 4 is the one that
//! catches a short password, which no entropy threshold can, and it is also the one that
//! can be wrong: it is confined to **value-shaped** tokens ([`is_value_shaped`]) so that
//! "Open the API key page" does not redact four ordinary words. R-07 is that failure, and
//! its cost — a user who unlocks everything out of habit — is worse than a single miss on
//! a line the certain detector already reads.
//!
//! # The exemption (DET-03)
//!
//! "Only values that passed the certain check at ingress are exempt from suspected-level
//! redaction in screenshots." So a spec value the agent sent — an endpoint id, an account
//! number, a random-looking reference the user has to type — is not flagged, because the
//! agent put it on the screen on purpose and the user needs to read it. A value the certain
//! detector **did** match is never exempt: it stays redacted at the certain level, where the
//! user cannot unlock it (DET-01).
//!
//! The exemption is built from the spec itself ([`Exemptions::of_spec`]) rather than from
//! the `secret_treated` list the server sends: scanning the strings here answers the question
//! from the very values it exempts, whatever that list says.
//!
//! # What never leaves this module
//!
//! A [`SuspectedMatch`] carries a span and a reason, never the matched text (R-19). The
//! caller that draws a box takes the span; the caller that logs takes the count.

use std::collections::BTreeSet;

use crate::format::spec::{HandoffSpec, SpecValue};

use super::certain::scan_text as scan_certain;

/// The Shannon entropy, in bits per character, a long token has to beat (§7.10).
pub const ENTROPY_BITS_PER_CHAR: f64 = 3.5;

/// The length from which a token is judged on its entropy (§7.10).
pub const ENTROPY_MIN_CHARS: usize = 20;

/// The length from which a run of hexadecimal digits is suspected on its own (§7.10).
pub const HEX_MIN_CHARS: usize = 32;

/// The length from which a base64-looking token is suspected on its own (§7.10).
pub const BASE64_MIN_CHARS: usize = 24;

/// The length from which a token next to a label is worth looking at (§7.10, R-07).
///
/// Below it the rule would flag ordinary short words on any line that says "key", which is
/// the false-positive spiral R-07 describes. A password shorter than this is missed, and
/// that is the direction the design chooses: the user is looking at the preview.
pub const LABEL_MIN_CHARS: usize = 8;

/// The words that make a token a label (§7.10, DET-01).
const LABEL_WORDS: [&str; 8] = [
    "key", "secret", "token", "password", "passwd", "pwd", "bearer", "api",
];

/// Why a token is suspected. The order of the variants is the order the rules are tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SuspectedReason {
    /// A long run of hexadecimal digits.
    Hex,
    /// A long token from the base64 alphabet.
    Base64,
    /// A long token whose characters are spread out enough to be random.
    Entropy,
    /// A value-shaped token beside a `key`/`secret`/`token`/`password` label.
    Label,
}

impl SuspectedReason {
    /// The name the view and the log use. Not a user-visible sentence: the catalogue key
    /// for that lives in `src/locales/`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hex => "hex",
            Self::Base64 => "base64",
            Self::Entropy => "entropy",
            Self::Label => "label",
        }
    }
}

impl std::fmt::Display for SuspectedReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One token the suspected detector picked out: where it is, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuspectedMatch {
    /// Byte offset of the first byte of the token.
    pub start: usize,
    /// Byte offset one past its last byte.
    pub end: usize,
    /// The rule that fired.
    pub reason: SuspectedReason,
}

/// The spec values that are **not** to be flagged (DET-03).
///
/// Built once per handoff and carried by the [`super::boxes::RedactionPlan`], so that every
/// pass over a capture and over the text pane asks the same question of the same set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exemptions {
    /// The exempt strings, and the tokens they are made of.
    allowed: BTreeSet<String>,
}

impl Exemptions {
    /// Nothing is exempt: a capture taken with no handoff in front of it.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// The exemption list of a spec (DET-03).
    ///
    /// Every value string — an array's items one by one — is exempt **unless** the certain
    /// detector matches inside it. The whole string is exempt and so is each of its tokens,
    /// because OCR reads a line and a value may be one word of it.
    #[must_use]
    pub fn of_spec(spec: &HandoffSpec) -> Self {
        let mut exemptions = Self::default();
        for value in spec.values.values() {
            match value {
                SpecValue::One(text) => exemptions.allow(text),
                SpecValue::Many(items) => {
                    for item in items {
                        exemptions.allow(item);
                    }
                }
            }
        }
        exemptions
    }

    /// The exemption list of a set of plain strings, which is what the tests drive.
    #[must_use]
    pub fn of_values<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut exemptions = Self::default();
        for value in values {
            exemptions.allow(value.as_ref());
        }
        exemptions
    }

    /// Adds one value, unless the certain detector matches inside it (DET-03).
    fn allow(&mut self, value: &str) {
        let value = value.trim();
        if value.is_empty() || !scan_certain(value).is_empty() {
            return;
        }
        self.allowed.insert(value.to_owned());
        for token in tokens_of(value) {
            self.allowed.insert(token.text.to_owned());
        }
    }

    /// Whether this exact token was declared by the spec and passed the certain check.
    #[must_use]
    pub fn covers(&self, token: &str) -> bool {
        self.allowed.contains(token)
    }

    /// How many strings are exempt. For the log line, never their content.
    #[must_use]
    pub fn len(&self) -> usize {
        self.allowed.len()
    }

    /// Whether nothing is exempt.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty()
    }
}

/// One token of a text, with its span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Token<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

/// Whether a character may be part of a token.
///
/// Alphanumerics and the separators a credential is written with. A comma, a quote, a
/// bracket or a semicolon ends a token, so `"sk_live_abc",` does not carry its punctuation
/// into the entropy count and a box does not cover a line's closing bracket.
fn is_token_char(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || is_separator(character)
        || (!character.is_ascii() && character.is_alphanumeric())
}

/// The characters that may sit **inside** a token but never at either end of one.
fn is_separator(character: char) -> bool {
    matches!(character, '_' | '-' | '.' | '+' | '/' | '=' | '~')
}

/// The tokens of a text, in reading order.
fn tokens_of(text: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;
    for (index, character) in text.char_indices() {
        if is_token_char(character) {
            start.get_or_insert(index);
        } else if let Some(from) = start.take() {
            tokens.push(Token {
                text: &text[from..index],
                start: from,
                end: index,
            });
        }
    }
    if let Some(from) = start {
        tokens.push(Token {
            text: &text[from..],
            start: from,
            end: text.len(),
        });
    }
    tokens.into_iter().filter_map(trim_separators).collect()
}

/// Drops the leading and trailing separators of a token.
///
/// `(sk_live_x)` has already lost its brackets; `-- name` and `page.` have not, and a
/// trailing full stop inside an entropy count is a character the writer never typed.
fn trim_separators(token: Token<'_>) -> Option<Token<'_>> {
    let leading = token.text.len() - token.text.trim_start_matches(is_separator).len();
    let trimmed = token.text.trim_matches(is_separator);
    if trimmed.is_empty() {
        return None;
    }
    Some(Token {
        text: trimmed,
        start: token.start + leading,
        end: token.start + leading + trimmed.len(),
    })
}

/// Whether the token carries the mix §7.10 asks for: **a letter and a digit**.
///
/// "Mixed character classes" has to be given a reading, and this is the narrow one. Read
/// widely — any two of lowercase, uppercase, digit and punctuation — every long URL and
/// every deep file path on the screen is flagged: `dashboard.stripe.com/settings/webhooks`
/// is 38 characters of two "classes" and its Shannon entropy is close to four bits, so the
/// wide reading redacts the address bar of the page the user is being guided through. That
/// is R-07 arriving on the first screenshot.
///
/// What the narrow reading gives up is an unlabelled random string of letters alone, which
/// is neither what a key looks like (every family of §4.6 mixes digits in) nor unprotected:
/// the label rule catches it wherever the screen says what it is.
#[must_use]
pub fn is_mixed(text: &str) -> bool {
    text.chars().any(char::is_alphabetic)
        && text.chars().any(|character| character.is_ascii_digit())
}

/// The Shannon entropy of a string, in bits per character.
///
/// Counted over its own character frequencies, which is the measure §7.10 names and the
/// only one computable from the string alone. A 20-character token cannot exceed
/// `log2(20) ≈ 4.32`, so the 3.5 threshold asks for characters that mostly differ.
#[must_use]
pub fn entropy_bits_per_char(text: &str) -> f64 {
    let mut counts: Vec<(char, u32)> = Vec::new();
    let mut total = 0_u32;
    for character in text.chars() {
        total += 1;
        match counts.iter_mut().find(|(seen, _)| *seen == character) {
            Some((_, count)) => *count += 1,
            None => counts.push((character, 1)),
        }
    }
    if total == 0 {
        return 0.0;
    }
    let total = f64::from(total);
    -counts
        .into_iter()
        .map(|(_, count)| {
            let probability = f64::from(count) / total;
            probability * probability.log2()
        })
        .sum::<f64>()
}

/// Whether every character is a hexadecimal digit.
fn is_hex(text: &str) -> bool {
    text.chars().all(|character| character.is_ascii_hexdigit())
}

/// Whether every character belongs to the base64 alphabet, URL-safe variant included.
fn is_base64ish(text: &str) -> bool {
    text.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=' | '-' | '_')
    })
}

/// The alphabetic words a token is built from, lowercased, for the label test.
///
/// Splits on anything that is not a letter and on a lower-to-upper transition, so `api_key`,
/// `apiKey`, `X-API-KEY` and `API KEY` all yield `api` and `key`. A trailing `s` is dropped,
/// so `Passwords` is a label too.
fn words_of(token: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for character in token.chars() {
        if character.is_alphabetic() {
            if previous_lower && character.is_uppercase() && !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_lower = character.is_lowercase();
            current.extend(character.to_lowercase());
        } else {
            previous_lower = false;
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
        .into_iter()
        .map(|word| match word.strip_suffix('s') {
            Some(singular) if singular.len() >= 3 => singular.to_owned(),
            _ => word,
        })
        .collect()
}

/// Whether a token is one of the words that label a credential field (§7.10).
#[must_use]
pub fn is_label(token: &str) -> bool {
    words_of(token)
        .iter()
        .any(|word| LABEL_WORDS.contains(&word.as_str()))
}

/// Whether a token could be the **value** of a labelled field, rather than more prose.
///
/// Eight characters or more, and one of the three marks that separate a credential from an
/// English word: a digit, a separator inside the token, or an uppercase letter that follows
/// a lowercase one. The last one is deliberately not "any uppercase": `Dashboard` and
/// `Settings` are capitalised words, `A1b2C3` and `myPassword` are not.
#[must_use]
pub fn is_value_shaped(token: &str) -> bool {
    if token.chars().count() < LABEL_MIN_CHARS {
        return false;
    }
    if token.chars().any(|character| character.is_ascii_digit()) {
        return true;
    }
    if token.chars().any(is_separator) {
        return true;
    }
    let mut seen_lower = false;
    for character in token.chars() {
        if character.is_lowercase() {
            seen_lower = true;
        } else if seen_lower && character.is_uppercase() {
            return true;
        }
    }
    false
}

/// The rule a token satisfies on its own, without looking at its line.
fn shape_reason(token: &str) -> Option<SuspectedReason> {
    let length = token.chars().count();
    if length >= HEX_MIN_CHARS && is_hex(token) {
        return Some(SuspectedReason::Hex);
    }
    if length >= BASE64_MIN_CHARS && is_base64ish(token) && is_mixed(token) {
        return Some(SuspectedReason::Base64);
    }
    if length >= ENTROPY_MIN_CHARS
        && is_mixed(token)
        && entropy_bits_per_char(token) > ENTROPY_BITS_PER_CHAR
    {
        return Some(SuspectedReason::Entropy);
    }
    None
}

/// Every suspected token of one line, given whether the line before it carried a label.
///
/// The answer's second half is whether the line below should be treated as this line's
/// value, which is what "the line below" means in §7.10. It is true only when the line has
/// a label **and no value of its own**: a dashboard puts the value either beside the label
/// or under it, never both, and reading the row under `Access key ID   AKIA…` as the key's
/// value would flag the date on it. R-07 is that failure repeated on every page.
fn scan_line(line: &str, exempt: &Exemptions, label_above: bool) -> (Vec<SuspectedMatch>, bool) {
    let tokens = tokens_of(line);
    let has_label = tokens.iter().any(|token| is_label(token.text));
    let labelled = has_label || label_above;
    let mut found = Vec::new();
    let mut carried = false;
    for token in &tokens {
        if exempt.covers(token.text) {
            continue;
        }
        let is_value = !is_label(token.text) && is_value_shaped(token.text);
        carried |= is_value;
        let reason = shape_reason(token.text)
            .or_else(|| (labelled && is_value).then_some(SuspectedReason::Label));
        if let Some(reason) = reason {
            found.push(SuspectedMatch {
                start: token.start,
                end: token.end,
                reason,
            });
        }
    }
    (found, has_label && !carried)
}

/// Every suspected token of `text`, in reading order (§7.10).
///
/// A line break is what "the line below" is measured in, so a caller that has OCR blocks
/// joins them with `\n` in reading order and gets the adjacency rule for free.
#[must_use]
pub fn scan_text(text: &str, exempt: &Exemptions) -> Vec<SuspectedMatch> {
    let mut found = Vec::new();
    let mut label_above = false;
    let mut offset = 0_usize;
    for line in text.split_inclusive('\n') {
        let (matches, has_label) = scan_line(line, exempt, label_above);
        found.extend(matches.into_iter().map(|hit| SuspectedMatch {
            start: hit.start + offset,
            end: hit.end + offset,
            reason: hit.reason,
        }));
        label_above = has_label;
        offset += line.len();
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_hex_string_is_suspected_on_its_shape_alone() {
        let text = "digest 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08 ok";
        let found = scan_text(text, &Exemptions::none());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].reason, SuspectedReason::Hex);
        assert_eq!(found[0].end - found[0].start, 64);
    }

    #[test]
    fn a_base64_looking_token_is_suspected_and_an_ordinary_long_word_is_not() {
        let found = scan_text(
            "value dGhpc0lzQVNlY3JldFZhbHVlMTIzNA== and internationalisations",
            &Exemptions::none(),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].reason, SuspectedReason::Base64);
    }

    #[test]
    fn a_long_address_is_not_a_long_random_string() {
        // R-07 in its most ordinary shape: the page the user is being guided through.
        for ordinary in [
            "dashboard.stripe.com/settings/webhooks",
            "Users/alice/project/config.json",
            "internationalisation-guidelines",
        ] {
            assert!(!is_mixed(ordinary), "{ordinary} counts as mixed");
            assert!(
                scan_text(ordinary, &Exemptions::none()).is_empty(),
                "{ordinary} was flagged"
            );
        }
    }

    #[test]
    fn entropy_separates_a_random_string_from_a_repetitive_one() {
        let random = "Zq7Z4tR2xL9pV0mW3kBn";
        assert!(entropy_bits_per_char(random) > ENTROPY_BITS_PER_CHAR);
        assert!(entropy_bits_per_char("aaaaaaaaaaAAAAAAAAAA") < ENTROPY_BITS_PER_CHAR);
        let found = scan_text(
            &format!("id {random} and aaaaaaaaaaAAAAAAAAAA"),
            &Exemptions::none(),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].reason, SuspectedReason::Entropy);
    }

    #[test]
    fn a_short_password_is_caught_by_its_label_and_nothing_else_is() {
        let found = scan_text("Password  hunter2-tango", &Exemptions::none());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].reason, SuspectedReason::Label);
        assert_eq!(
            &"Password  hunter2-tango"[found[0].start..found[0].end],
            "hunter2-tango"
        );
    }

    #[test]
    fn a_label_with_a_value_beside_it_does_not_claim_the_line_below() {
        // The row under a filled one is another row, not the continuation of this one.
        let text = "Access key ID  AKIA-not-a-real-one\nCreated  2026-08-14\n";
        let found = scan_text(text, &Exemptions::none());
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].start..found[0].end], "AKIA-not-a-real-one");
    }

    #[test]
    fn a_label_reaches_the_line_below_it_and_no_further() {
        let text = "API key\nrk-alpha-42-bravo\nordinary-words-here\n";
        let found = scan_text(text, &Exemptions::none());
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].start..found[0].end], "rk-alpha-42-bravo");
    }

    #[test]
    fn prose_about_a_key_does_not_redact_the_sentence() {
        // R-07: the rule that annoys users into unlocking everything is this one.
        let found = scan_text(
            "Open the API key page in the dashboard and press Reveal.",
            &Exemptions::none(),
        );
        assert!(found.is_empty(), "flagged {found:?}");
    }

    #[test]
    fn label_words_are_matched_at_a_boundary_and_not_inside_another_word() {
        assert!(is_label("key"));
        assert!(is_label("api_key"));
        assert!(is_label("apiKey"));
        assert!(is_label("X-API-KEY"));
        assert!(is_label("Passwords"));
        assert!(is_label("Bearer"));
        assert!(!is_label("monkey"));
        assert!(!is_label("keyboard"));
        assert!(!is_label("tokenizer"));
        assert!(!is_label("turkey"));
    }

    #[test]
    fn a_capitalised_word_is_not_value_shaped_but_a_credential_is() {
        assert!(!is_value_shaped("Dashboard"));
        assert!(!is_value_shaped("Settings"));
        assert!(!is_value_shaped("short1"));
        assert!(is_value_shaped("hunter2-tango"));
        assert!(is_value_shaped("myPassword"));
        assert!(is_value_shaped("alpha-bravo"));
    }

    #[test]
    fn a_spec_value_that_passed_the_certain_check_is_exempt() {
        let exempt = Exemptions::of_values(["we_1P9xTz2eZvKYlo2C0Sd8h4kL"]);
        let text = "Endpoint  we_1P9xTz2eZvKYlo2C0Sd8h4kL";
        assert!(scan_text(text, &exempt).is_empty());
        assert_eq!(scan_text(text, &Exemptions::none()).len(), 1);
    }

    #[test]
    fn a_value_the_certain_detector_matched_is_never_exempt() {
        // DET-03's second half: the exemption is for values that *passed* the check.
        let exempt = Exemptions::of_values(["AKIAIOSFODNN7EXAMPLE"]);
        assert!(exempt.is_empty());
        assert!(!exempt.covers("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn an_exempt_value_of_several_words_exempts_each_of_them() {
        let exempt = Exemptions::of_values(["order 4711-A9Z2-QQ01-9931"]);
        assert!(exempt.covers("4711-A9Z2-QQ01-9931"));
        assert!(exempt.covers("order 4711-A9Z2-QQ01-9931"));
    }

    #[test]
    fn punctuation_is_not_part_of_a_token() {
        let text = "the id is (Zq7Z4tR2xL9pV0mW3kBn), copied.";
        let found = scan_text(text, &Exemptions::none());
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].start..found[0].end], "Zq7Z4tR2xL9pV0mW3kBn");
    }

    #[test]
    fn spans_are_on_character_boundaries_in_a_line_with_accents() {
        let text = "però la password è alpha-bravo-99";
        let found = scan_text(text, &Exemptions::none());
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].start..found[0].end], "alpha-bravo-99");
    }
}
