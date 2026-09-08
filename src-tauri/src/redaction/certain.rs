//! The certain detector (§4.6, §5.5, DET-02, DET-04, SPEC-13).
//!
//! The patterns are public and belong to the server; the app applies them itself because a
//! screenshot never reaches the server before it has been redacted (§2.4, last row). Open →
//! closed reuse is allowed; the reverse is not.
//!
//! Two callers, one rule. [`scan_text`] is what the redaction pipeline runs over an OCR
//! result (T-048), and [`scan_spec`] reproduces the server's ingress scan field for field,
//! so the app can check that what it received in `secret_treated` is what the same patterns
//! find in the same spec.
//!
//! What leaves this module is a location and a family, never the matched text (R-19).
//!
//! # Two facts to keep in mind
//!
//! - **`kind` is the family, never the pattern id.** The vocabulary is `private_key`,
//!   `api_key`, `token`, `webhook_secret`, `webhook_url`, `jwt`. Printing the id would tell
//!   the reader which vendor issued the secret, which is the one fact masking exists to
//!   withhold.
//! - **A span is a byte offset here and a UTF-16 code unit in the server.** The two agree
//!   on the ASCII corpora and not in general; nothing crosses the channel that carries a
//!   span, so this only matters to whoever compares the two by hand.

use std::str::FromStr;
use std::sync::LazyLock;

use regex::Regex;

use crate::format::paths::{child_path, Segment};
use crate::format::patterns::pattern_file;
use crate::format::spec::{HandoffSpec, SpecValue};

pub use crate::format::patterns::patterns_version;

/// The coarse family reported in an outcome's `secret_treated` and kept by the log (§4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CertainSecretKind {
    /// A PEM private key.
    PrivateKey,
    /// An API key.
    ApiKey,
    /// A personal or machine token.
    Token,
    /// A webhook signing secret.
    WebhookSecret,
    /// A webhook URL that is itself the credential.
    WebhookUrl,
    /// A JSON Web Token.
    Jwt,
}

/// The six families, in the order the pattern file's notes list them.
pub const CERTAIN_SECRET_KINDS: [CertainSecretKind; 6] = [
    CertainSecretKind::PrivateKey,
    CertainSecretKind::ApiKey,
    CertainSecretKind::Token,
    CertainSecretKind::WebhookSecret,
    CertainSecretKind::WebhookUrl,
    CertainSecretKind::Jwt,
];

impl CertainSecretKind {
    /// The wire name, which is what an outcome and the log carry.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrivateKey => "private_key",
            Self::ApiKey => "api_key",
            Self::Token => "token",
            Self::WebhookSecret => "webhook_secret",
            Self::WebhookUrl => "webhook_url",
            Self::Jwt => "jwt",
        }
    }
}

impl std::fmt::Display for CertainSecretKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A `kind` outside the six-value vocabulary is a defect of the pinned pattern file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownKind(pub String);

impl std::fmt::Display for UnknownKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "unknown certain-secret kind `{}`", self.0)
    }
}

impl std::error::Error for UnknownKind {}

impl FromStr for CertainSecretKind {
    type Err = UnknownKind;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        CERTAIN_SECRET_KINDS
            .into_iter()
            .find(|kind| kind.as_str() == value)
            .ok_or_else(|| UnknownKind(value.to_owned()))
    }
}

/// One match: which family, and where it was found. Never what it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretMatch {
    /// The family of the pattern that matched.
    pub kind: CertainSecretKind,
    /// Byte offset of the first byte of the match.
    pub start: usize,
    /// Byte offset one past its last byte.
    pub end: usize,
}

/// One certain secret found in a spec: where it is, and which family it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretTreated {
    /// Display path of the string it was found in, e.g. `values.api_key`.
    pub location: String,
    /// The family, never the pattern id.
    pub kind: CertainSecretKind,
}

/// A hit with the span it occupies inside its string. App-internal: what travels in an
/// outcome is [`SecretTreated`], which the schema pins to exactly two keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretSpan {
    /// Display path of the string it was found in.
    pub location: String,
    /// The family.
    pub kind: CertainSecretKind,
    /// Byte offset of the first byte of the match.
    pub start: usize,
    /// Byte offset one past its last byte.
    pub end: usize,
}

struct CompiledPattern {
    id: &'static str,
    kind: CertainSecretKind,
    regex: Regex,
}

/// Compiled once, in the order of the file: the list is fixed at build time and a scan must
/// not pay for compilation.
///
/// A pattern that does not compile, or a `kind` outside the vocabulary, is a defect of the
/// pinned artifact and there is nothing a caller could do about it, so this panics with the
/// pattern named.
static COMPILED: LazyLock<Vec<CompiledPattern>> = LazyLock::new(|| {
    pattern_file()
        .patterns
        .iter()
        .map(|entry| {
            let kind = entry
                .kind
                .parse()
                .unwrap_or_else(|error| panic!("pattern {}: {error}", entry.id));
            let regex = Regex::new(&entry.regex).unwrap_or_else(|error| {
                panic!(
                    "pattern {} does not compile with the regex crate: {error}",
                    entry.id
                )
            });
            CompiledPattern {
                // The parsed file is a `LazyLock` static, so its strings are `'static`.
                id: entry.id.as_str(),
                kind,
                regex,
            }
        })
        .collect()
});

/// The pattern ids, in file order, for the contract tests and for `doctor`-style reports.
#[must_use]
pub fn pattern_ids() -> Vec<&'static str> {
    COMPILED.iter().map(|pattern| pattern.id).collect()
}

/// Every certain secret in `text`, ordered by position.
///
/// Patterns are applied in file order and a match overlapping one already found is dropped,
/// so a value is reported once under its most specific family. That is what makes the file
/// order meaningful: `anthropic_api_key` precedes `openai_api_key` because an Anthropic key
/// also satisfies the wider OpenAI shape.
#[must_use]
pub fn scan_text(text: &str) -> Vec<SecretMatch> {
    let mut found: Vec<SecretMatch> = Vec::new();
    for pattern in COMPILED.iter() {
        for hit in pattern.regex.find_iter(text) {
            let (start, end) = (hit.start(), hit.end());
            if start == end {
                // No pattern can match the empty string; the guard says so rather than
                // leaving a zero-width hit to be masked into an infinite loop later.
                continue;
            }
            if found
                .iter()
                .any(|other| start < other.end && other.start < end)
            {
                continue;
            }
            found.push(SecretMatch {
                kind: pattern.kind,
                start,
                end,
            });
        }
    }
    found.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| left.end.cmp(&right.end))
    });
    found
}

/// Every hit in one string, tagged with the location that string has in the spec.
fn spans_in(location: &str, text: &str) -> Vec<SecretSpan> {
    scan_text(text)
        .into_iter()
        .map(|hit| SecretSpan {
            location: location.to_owned(),
            kind: hit.kind,
            start: hit.start,
            end: hit.end,
        })
        .collect()
}

/// One entry of `values`: a string, or a list whose items are located by index.
fn value_spans(key: &str, value: &SpecValue) -> Vec<SecretSpan> {
    let path = child_path("values", Segment::Field(key));
    match value {
        SpecValue::One(text) => spans_in(&path, text),
        SpecValue::Many(items) => items
            .iter()
            .enumerate()
            .flat_map(|(index, item)| spans_in(&child_path(&path, Segment::Index(index)), item))
            .collect(),
    }
}

/// Every certain secret in a spec, with its span, in the field order of §5.5.
///
/// `url` and `steps[].url` are deliberately not scanned: the design lists the fields and
/// those two are not among them, and a url is already confined to three schemes (S5). A
/// Slack webhook URL sent as a *value* or written into a step text is caught, which is
/// where it is put in practice.
///
/// The order is the server's, and it is part of the contract: `values` in document order,
/// then `goal`, `where`, `why_human`, `verify`, then each step's `text` and `warning`.
#[must_use]
pub fn scan_spec_spans(spec: &HandoffSpec) -> Vec<SecretSpan> {
    let mut found: Vec<SecretSpan> = Vec::new();
    for (key, value) in &spec.values {
        found.extend(value_spans(key, value));
    }
    found.extend(spans_in("goal", &spec.goal));
    found.extend(spans_in("where", &spec.r#where));
    found.extend(spans_in("why_human", &spec.why_human));
    if let Some(verify) = &spec.verify {
        found.extend(spans_in("verify", verify));
    }
    for (index, step) in spec.steps.iter().enumerate() {
        let at = child_path("steps", Segment::Index(index));
        found.extend(spans_in(
            &child_path(&at, Segment::Field("text")),
            &step.text,
        ));
        if let Some(warning) = &step.warning {
            found.extend(spans_in(
                &child_path(&at, Segment::Field("warning")),
                warning,
            ));
        }
    }
    found
}

/// Every certain secret in a spec, as an outcome reports it (§4.3 `secret_treated`).
#[must_use]
pub fn scan_spec(spec: &HandoffSpec) -> Vec<SecretTreated> {
    scan_spec_spans(spec)
        .into_iter()
        .map(|span| SecretTreated {
            location: span.location,
            kind: span.kind,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_family_and_the_span_of_a_secret_inside_ordinary_text() {
        let secret = "AKIAIOSFODNN7EXAMPLE";
        let text = format!("The access key is {secret} and it was rotated today.");
        let start = text.find(secret).expect("the secret is in the text");
        assert_eq!(
            scan_text(&text),
            vec![SecretMatch {
                kind: CertainSecretKind::ApiKey,
                start,
                end: start + secret.len(),
            }]
        );
    }

    #[test]
    fn reports_a_value_once_under_the_most_specific_family() {
        // The Anthropic key also satisfies the wider OpenAI shape; file order resolves it.
        let matches = scan_text("sk-ant-api03-A1b2C3d4E5f6G7h8I9j0KlMnOpQrStUvWxYz");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].start, 0);
    }

    #[test]
    fn returns_the_matches_in_reading_order() {
        let text = "first hf_QzWxEcRvTyBnUmIoLpAsDfGhJkZxCv3456 then glpat-A1b2C3d4E5f6G7h8I9j0";
        let matches = scan_text(text);
        assert_eq!(
            matches.iter().map(|hit| hit.kind).collect::<Vec<_>>(),
            vec![CertainSecretKind::Token, CertainSecretKind::Token]
        );
        assert_eq!(
            matches.iter().map(|hit| hit.start).collect::<Vec<_>>(),
            vec![
                text.find("hf_").expect("present"),
                text.find("glpat-").expect("present")
            ]
        );
    }

    #[test]
    fn finds_nothing_in_empty_or_ordinary_text() {
        assert!(scan_text("").is_empty());
        assert!(scan_text("Open the settings page and copy the publishable key.").is_empty());
    }

    #[test]
    fn a_kind_round_trips_through_its_wire_name() {
        for kind in CERTAIN_SECRET_KINDS {
            assert_eq!(kind.as_str().parse::<CertainSecretKind>(), Ok(kind));
        }
        assert!("stripe_secret_key".parse::<CertainSecretKind>().is_err());
    }
}
