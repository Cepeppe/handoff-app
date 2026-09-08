//! The public certain-secret pattern file (§4.6, DD-20), as the app reads it.
//!
//! `patterns/certain-secrets.v1.json` belongs to the server and reaches the app through the
//! pinned release artifact (§3.4). Neither side may add, remove or edit a pattern locally,
//! so the file is embedded verbatim at build time and parsed once: what the app treats as a
//! secret in a screenshot is exactly what the server treated as a secret at ingress.
//!
//! Two consumers read it, and this module is the only place that knows where it lives:
//! [`crate::redaction::certain`] compiles the regexes, and [`crate::runbooks::matching`]
//! takes the stop-word lists the matching rule drops (§4.5.3).

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use serde::Deserialize;

/// The file itself, from the pinned artifact. `build.rs` fails with a readable message when
/// `vendor/` has not been filled.
const CERTAIN_SECRETS: &str =
    include_str!("../../../vendor/handoff-mcp/format/patterns/certain-secrets.v1.json");

/// One pattern of the file.
#[derive(Debug, Clone, Deserialize)]
pub struct PatternEntry {
    /// The pattern's id, e.g. `stripe_secret_key`. Internal: it names the vendor and never
    /// travels in an outcome or a log record.
    pub id: String,
    /// The coarse family, e.g. `api_key`. This is what `secret_treated` reports.
    pub kind: String,
    /// The regex, in the subset JavaScript and the Rust `regex` crate share.
    pub regex: String,
    /// What the pattern recognises, for a person reading the file.
    pub description: String,
    /// The examples the file carries with it.
    pub tests: PatternTests,
}

/// The examples a pattern must match and must not match.
#[derive(Debug, Clone, Deserialize)]
pub struct PatternTests {
    /// Values the pattern must find.
    #[serde(rename = "match")]
    pub match_: Vec<String>,
    /// Values it must not.
    pub no_match: Vec<String>,
}

/// The whole file.
#[derive(Debug, Clone, Deserialize)]
pub struct PatternFile {
    /// The version of the file, reported so a mismatch between the two sides is visible.
    pub patterns_version: u32,
    /// The documentation the file carries: why the subset is what it is, why `\b` is
    /// absent, why the order matters. Read by the contract tests, not by the code.
    #[serde(rename = "_notes")]
    pub notes: HashMap<String, String>,
    /// The patterns, in the order they are applied.
    pub patterns: Vec<PatternEntry>,
    /// The stop-word lists of the matching rule, one per language tag.
    pub stop_words: HashMap<String, Vec<String>>,
}

/// Parsed once, at first use. A file that does not parse is a defect of the pinned
/// artifact, not of a caller, so this panics rather than making every caller carry a
/// `Result` for something that cannot be recovered from at run time.
static FILE: LazyLock<PatternFile> = LazyLock::new(|| {
    serde_json::from_str(CERTAIN_SECRETS)
        .expect("the vendored patterns/certain-secrets.v1.json does not parse")
});

/// The pattern file of the pinned release.
#[must_use]
pub fn pattern_file() -> &'static PatternFile {
    &FILE
}

/// The version of the pattern file (§4.6).
#[must_use]
pub fn patterns_version() -> u32 {
    FILE.patterns_version
}

/// The union of every stop-word list, which is what a language we ship no list for gets,
/// and what an absent `lang` gets (§4.5.3).
static STOP_WORDS_UNION: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    FILE.stop_words
        .values()
        .flatten()
        .map(String::as_str)
        .collect()
});

/// Per-language sets, built once so the matcher does not rebuild them per runbook.
static STOP_WORDS: LazyLock<HashMap<&'static str, HashSet<&'static str>>> = LazyLock::new(|| {
    FILE.stop_words
        .iter()
        .map(|(tag, words)| (tag.as_str(), words.iter().map(String::as_str).collect()))
        .collect()
});

/// The stop-word set for a language tag: its own list if the file ships one, else the
/// union of all of them.
///
/// Only the primary subtag is compared, lowercased, so `en-GB` and `EN` both select `en`
/// and a tag we ship no list for behaves exactly like an absent one.
#[must_use]
pub fn stop_words(lang: Option<&str>) -> &'static HashSet<&'static str> {
    let Some(tag) = lang.and_then(primary_subtag) else {
        return &STOP_WORDS_UNION;
    };
    STOP_WORDS.get(tag.as_str()).unwrap_or(&STOP_WORDS_UNION)
}

/// The primary subtag of a BCP-47 tag, lowercased; `None` when there is none.
fn primary_subtag(lang: &str) -> Option<String> {
    let tag = lang.split('-').next().unwrap_or_default().to_lowercase();
    if tag.is_empty() {
        None
    } else {
        Some(tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vendored_file_parses_and_is_version_one() {
        assert_eq!(patterns_version(), 1);
        assert!(!pattern_file().patterns.is_empty());
    }

    #[test]
    fn a_shipped_language_gets_its_own_list() {
        let english = stop_words(Some("en"));
        assert!(english.contains("the"));
        assert!(!english.contains("della"));
    }

    #[test]
    fn a_region_subtag_selects_its_language() {
        assert_eq!(stop_words(Some("it-IT")), stop_words(Some("it")));
        assert_eq!(stop_words(Some("EN")), stop_words(Some("en")));
    }

    #[test]
    fn an_absent_or_unshipped_language_gets_the_union() {
        let union = stop_words(None);
        assert!(union.contains("the"));
        assert!(union.contains("della"));
        assert_eq!(stop_words(Some("fr")), union);
        assert_eq!(stop_words(Some("")), union);
    }
}
