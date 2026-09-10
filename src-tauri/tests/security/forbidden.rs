//! The forbidden set: the values no database row, log line or crash file may ever hold
//! (§11.2 "Log invariants", §11.7 "log-never-contains-values", R-19).
//!
//! One loader for every Rust suite that asks the question, so that the suites cannot drift
//! apart on what "a value" is. Two sources, both of them certain secrets:
//!
//! - **the planted sentinels** below, one per field of a spec the ingress scan of §5.5 covers
//!   and one per free-text place a flow can reach (the state, a request, a note, a
//!   verification detail). Each matches `stripe_secret_key` (§4.6:
//!   `[sr]k_(?:live|test)_[0-9A-Za-z]{16,}`), so the certain detector sees every one of them,
//!   and each is unique, so a failure names the field it was planted in;
//! - **the positive corpus of the pinned format**, `fixtures/secrets/positive.txt`: "any
//!   fixture secret", in the words of §11.2. No flow plants these, so over a database or a
//!   crash file they guard the day one does rather than find something today.
//!
//! Who reads it: `tests/integration_flows.rs` after every flow, and the security suite over
//! the database, the tracing output and the crash files. The e2e suite applies the same rule
//! to the live database in `tests/e2e/db.ts` (`leaks`), with secrets it generates per run
//! because they travel through a model's prompt.
//!
//! Shared by more than one test binary through `#[path]`, each of which uses part of it,
//! hence the `dead_code` allowance.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Planted in `values.api_key`.
pub const VALUE: &str = "sk_live_SENTINELVALUE000000001";
/// Planted as the second item of a list value.
pub const ITEM: &str = "sk_live_SENTINELITEM0000000002";
/// Planted in `goal`.
pub const GOAL: &str = "sk_live_SENTINELGOAL0000000003";
/// Planted in `where`.
pub const WHERE: &str = "sk_live_SENTINELWHERE000000004";
/// Planted in `why_human`.
pub const WHY_HUMAN: &str = "sk_live_SENTINELWHYHUMAN000005";
/// Planted in a step's text.
pub const STEP: &str = "sk_live_SENTINELSTEP0000000006";
/// Planted in a step's warning.
pub const WARNING: &str = "sk_live_SENTINELWARNING0000007";
/// Planted in `verify`.
pub const VERIFY: &str = "sk_live_SENTINELVERIFY00000008";
/// Planted in the state the store writes through (§7.4).
pub const STATE: &str = "sk_live_SENTINELSTATE000000009";
/// Planted in the text of a user request (§7.7).
pub const REQUEST: &str = "sk_live_SENTINELREQUEST0000010";
/// Planted in a note the user typed on a step (RESP-02).
pub const NOTE: &str = "sk_live_SENTINELNOTE0000000011";
/// Planted in the detail of a verification report (§4.4).
pub const DETAIL: &str = "sk_live_SENTINELDETAIL00000012";

/// Every planted sentinel, with the place it is planted in.
pub const PLANTED: [(&str, &str); 12] = [
    ("values.api_key", VALUE),
    ("an item of a list value", ITEM),
    ("goal", GOAL),
    ("where", WHERE),
    ("why_human", WHY_HUMAN),
    ("a step's text", STEP),
    ("a step's warning", WARNING),
    ("verify", VERIFY),
    ("the store's state", STATE),
    ("a user request", REQUEST),
    ("a note", NOTE),
    ("a verification detail", DETAIL),
];

/// The shortest value this check accepts. A short or common string would be found by
/// accident, or would make "absent" mean nothing; `tests/e2e/db.ts` refuses the same.
const SHORTEST: usize = 12;

/// `fixtures/secrets/positive.txt` of the pinned format, as `fetch-server` unpacked it.
#[must_use]
pub fn corpus_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("vendor")
        .join("handoff-mcp")
        .join("format")
        .join("fixtures")
        .join("secrets")
        .join("positive.txt")
}

/// Every secret of the positive corpus: its lines, less the empty ones and the comments.
///
/// # Panics
///
/// When the file is missing — `vendor/` is filled by `node scripts/fetch-server.mjs` — or
/// holds too few lines to be the corpus.
#[must_use]
pub fn corpus_secrets() -> Vec<String> {
    let path = corpus_file();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error} (run `node scripts/fetch-server.mjs`)",
            path.display()
        )
    });
    let secrets: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect();
    assert!(
        secrets.len() >= 20,
        "{} holds {} secrets, which is not the corpus",
        path.display(),
        secrets.len()
    );
    secrets
}

static FORBIDDEN: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut all: Vec<String> = PLANTED
        .iter()
        .map(|(_, value)| (*value).to_owned())
        .collect();
    all.extend(corpus_secrets());
    for needle in &all {
        assert!(
            needle.len() >= SHORTEST,
            "{needle:?} is too short to be looked for: it would be found by accident"
        );
    }
    all
});

/// The whole forbidden set: the planted sentinels, then the corpus.
#[must_use]
pub fn forbidden() -> &'static [String] {
    &FORBIDDEN
}

/// Every needle found in `text`, each with where it was planted and a few characters around
/// the place it was found, which is what makes a failure readable.
#[must_use]
pub fn leaks(text: &str, needles: &[String]) -> Vec<String> {
    needles
        .iter()
        .filter_map(|needle| {
            text.find(needle.as_str()).map(|at| {
                let planted = PLANTED
                    .iter()
                    .find(|(_, value)| *value == needle.as_str())
                    .map_or("the secret corpus", |(place, _)| place);
                format!(
                    "{needle} (from {planted}) at byte {at}: …{}…",
                    excerpt(text, at, needle.len())
                )
            })
        })
        .collect()
}

/// Fails, naming every leak, when any needle is in `text`.
///
/// # Panics
///
/// When one is.
pub fn assert_absent(what: &str, text: &str, needles: &[String]) {
    let found = leaks(text, needles);
    assert!(
        found.is_empty(),
        "{what} holds a value it must never hold:\n{}",
        found.join("\n")
    );
}

/// Forty bytes either side of a match, widened to character boundaries.
fn excerpt(text: &str, at: usize, len: usize) -> &str {
    let mut start = at.saturating_sub(40);
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + len + 40).min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    &text[start..end]
}
