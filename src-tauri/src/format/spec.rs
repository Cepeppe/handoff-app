//! The handoff spec, version 1 (§4.2, `schemas/handoff-spec.v1.schema.json`).
//!
//! These types describe a spec that passed the schema; the vendored schema stays the
//! contract and is what actually validates ([`crate::format::schema`]). Nothing here may
//! add a constraint the schema does not have, and nothing here may drop a field it does:
//! `deny_unknown_fields` mirrors the schema's `additionalProperties: false` object for
//! object, so an agent that mistypes `warnings` for `warning` is refused here as it is
//! there rather than having the field silently dropped (SPEC-06, SPEC-10).
//!
//! `values` and `secrets` are the two objects the schema leaves open, because their keys
//! belong to the user. They are ordered maps: the certain detector reports the locations of
//! a spec in document order (§5.5), and a sorted or hashed map would report a different
//! order from the server's for the same document.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// A value of the top-level `values` map: one string, or a list of strings.
///
/// Untagged, because the schema's `$defs/value` is an `anyOf` of exactly these two shapes
/// and the wire form carries no discriminator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SpecValue {
    /// A single value.
    One(String),
    /// A list of values, shown to the user as a list.
    Many(Vec<String>),
}

/// One step of a handoff. Always an object, never a string (SPEC-03).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffStep {
    /// What the person has to do, 1–2000 characters.
    pub text: String,
    /// A starting point for this step, restricted to the schemes of S5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Keys into the top-level `values`. Omitted, never empty, when the step uses none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    /// What the person should know before acting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// A handoff spec, version 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffSpec {
    /// `1` in this version; a higher one is answered with an update instruction (S1).
    pub spec_version: i64,
    /// What must be achieved.
    pub goal: String,
    /// Where to act. The matching rule normalises this to decide whether two handoffs
    /// happen in the same place (§4.5.3).
    ///
    /// `r#where` because `where` is a Rust keyword; serde carries the field as `where`.
    #[serde(rename = "where")]
    pub r#where: String,
    /// A starting point openable with one click.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Why a person performs this work.
    pub why_human: String,
    /// Values to use, taken from the project. Required, and may be empty.
    pub values: IndexMap<String, SpecValue>,
    /// Variable name to destination file, for values the user copies out of the external
    /// service. Names only: a value never appears here (PRIN-03).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<IndexMap<String, String>>,
    /// The ordered steps, shown one at a time.
    pub steps: Vec<HandoffStep>,
    /// The check the agent performs itself once the user is done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<String>,
    /// BCP-47 language tag of the texts; a hint for OCR and for the matching rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

/// The prefixes SPEC-07 allows a `url` to start with, in the order §4.2 prints them.
///
/// The schema is the contract (`^(https?://|ms-settings:|x-apple\.systempreferences:)`) and
/// the test below checks this list against it, one spec per scheme. It is written out here
/// because the overlay has to answer the same question about a string the schema never saw:
/// the target of an **Open** button, and the links auto-detected inside a step's text
/// (GUIDE-03, §7.6). Anything else is shown as plain text and is not clickable (SPEC-08).
pub const ALLOWED_URL_PREFIXES: [&str; 4] = [
    "http://",
    "https://",
    "ms-settings:",
    "x-apple.systempreferences:",
];

/// Whether `url` may be opened with one click (SPEC-07, GUIDE-03).
///
/// The comparison is case-sensitive, exactly as the schema's pattern is: a spec whose `url`
/// reads `HTTPS://` never passed validation, so a stored one cannot exist, and a string
/// found inside a step's text is offered as a link only in the spelling the schema would
/// have accepted.
#[must_use]
pub fn url_allowed(url: &str) -> bool {
    ALLOWED_URL_PREFIXES
        .iter()
        .any(|prefix| url.starts_with(prefix))
}

#[cfg(test)]
mod url_tests {
    use super::*;
    use crate::format::schema::{is_valid, Document};
    use serde_json::json;

    /// A minimal valid spec carrying `url`, so the schema itself answers the question.
    fn spec_with_url(url: &str) -> serde_json::Value {
        json!({
            "spec_version": 1,
            "goal": "Register the webhook",
            "where": "Dashboard → Webhooks",
            "url": url,
            "why_human": "only a person can log in",
            "values": {},
            "steps": [{ "text": "Open the page and add the endpoint." }]
        })
    }

    #[test]
    fn the_list_is_the_one_the_vendored_schema_enforces() {
        // Written out in this crate and pinned to the schema here, so a released change of
        // SPEC-07 shows up as a failing test rather than as an Open button that opens
        // something the format no longer allows.
        for prefix in ALLOWED_URL_PREFIXES {
            let url = format!("{prefix}example.test/path");
            assert!(url_allowed(&url), "{url} is refused by our own list");
            assert!(
                is_valid(Document::Spec, &spec_with_url(&url)),
                "{url} is refused by the vendored schema"
            );
        }
    }

    #[test]
    fn everything_else_is_plain_text() {
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "HTTPS://example.test",
            "data:text/html,<script>",
            "mailto:someone@example.test",
            "//example.test",
            "example.test",
            "",
        ] {
            assert!(!url_allowed(url), "{url} would have been clickable");
            assert!(
                !is_valid(Document::Spec, &spec_with_url(url)),
                "{url} is accepted by the vendored schema"
            );
        }
    }
}
