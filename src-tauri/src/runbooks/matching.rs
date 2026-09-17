//! The matching rule and its ranking (§4.5.3, RUN-07a).
//!
//! A runbook matches when its `where` normalises to the same string as the query's and the
//! two goals share at least one word beyond stop-words. Nothing else is consulted: not the
//! steps, not the url, not the trust label. The shared words come back as `matched_words`
//! so that the agent — and the person reading the outcome — can see exactly why. The rule
//! is deterministic and explainable: no fuzzy similarity, no stemming, no model.
//!
//! The server implements the same rule (`src/runbooks/normalize.ts` and `match.ts`), and
//! `fixtures/matching/*.json` is the contract both are held to. Three choices §4.5.3 does
//! not make are parity-critical (T-016):
//!
//! 1. **A third ranking key, `id` ascending.** The design stops after shared-token count
//!    and `last_verified_at`, which leaves two runbooks verified in the same instant with
//!    the same number of shared words in the order the directory listing produced. With a
//!    cap of five results that is not cosmetic: an undefined tie decides which runbook is
//!    shown and which is dropped.
//! 2. **Whitespace is an explicit character class, not `\s`.** JavaScript in `u` mode
//!    includes U+FEFF and not U+0085; the Rust `regex` crate includes U+0085 and not
//!    U+FEFF. A `where` carrying a byte-order mark would otherwise normalise to two
//!    different strings on the two sides.
//! 3. **A token's length is counted in code points.** `String.length` is UTF-16 units in
//!    JavaScript and `str::len` is bytes in Rust; only code points are the same unit on
//!    both sides.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::format::patterns::stop_words;
use crate::format::runbook::Runbook;

/// At most this many results, whatever the ranking (§4.1, `RUNBOOK_MATCH_MAX_RESULTS`).
pub const RUNBOOK_MATCH_MAX_RESULTS: usize = 5;

/// Tokens shorter than this carry no meaning worth matching on (§4.5.3).
pub const MIN_GOAL_TOKEN_LENGTH: usize = 3;

/// Whitespace, as the union of what JavaScript and Rust each call whitespace, so that the
/// two implementations agree by construction. NFKC has already folded most of the exotic
/// spaces into U+0020 by the time this class is applied; the rest are listed anyway.
const WHITESPACE: &str = "\\t\\n\\x0B\\x0C\\r\\x20\\u{0085}\\u{00a0}\\u{1680}\\u{2000}-\\u{200a}\\u{2028}\\u{2029}\\u{202f}\\u{205f}\\u{3000}\\u{feff}";

/// The separator characters §4.5.3 lists, in its order: `→ > » / \ | – — - : , ; .`
///
/// Written with escapes for the four non-ASCII ones so the source cannot be misread, and
/// with the hyphen escaped so it can never be taken for a range.
const SEPARATORS: &str = "\\u{2192}>\\u{00bb}/\\\\|\\u{2013}\\u{2014}\\-:,;.";

/// A run of whitespace or of separators, which collapses to a single space.
static SEPARATOR_RUN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!("[{WHITESPACE}{SEPARATORS}]+"))
        .expect("the separator class of the matching rule does not compile")
});

/// Anything that is neither a letter nor a number, which is where a goal is cut.
static NON_ALPHANUMERIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[^\p{L}\p{N}]+")
        .expect("the token separator of the matching rule does not compile")
});

/// A runbook as it was read from `~/.handoff/runbooks/`, with the file it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRunbook {
    /// Where the file is; it travels to the agent in `runbooks[].path`.
    pub path: PathBuf,
    /// What it says.
    pub runbook: Runbook,
}

/// What the query asks: the same three inputs `handoff_runbooks` takes (§4.7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunbookQuery {
    /// The place the new handoff happens in.
    pub where_: String,
    /// What it is trying to achieve.
    pub goal: String,
    /// The BCP-47 tag of the texts, which selects the stop-word list.
    pub lang: Option<String>,
}

/// One runbook that matched, with the words that made it match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedRunbook<'a> {
    /// The runbook and its file.
    pub stored: &'a StoredRunbook,
    /// The shared words, in the order they first appear in the **query's** goal.
    pub matched_words: Vec<String>,
}

/// `where` reduced to the form two specs must share to be the same place (§4.5.3): NFKC,
/// lowercase, every run of whitespace or of the separator characters to one space, trimmed.
///
/// "Stripe Dashboard → Developers → Webhooks" and "stripe dashboard / developers / webhooks"
/// both become "stripe dashboard developers webhooks".
#[must_use]
pub fn normalize_where(where_: &str) -> String {
    let folded: String = where_.nfkc().collect::<String>().to_lowercase();
    SEPARATOR_RUN.replace_all(&folded, " ").trim().to_owned()
}

/// The words of a goal that matching looks at (§4.5.3): NFKC, lowercase, cut on everything
/// that is not a letter or a number, tokens shorter than [`MIN_GOAL_TOKEN_LENGTH`] dropped,
/// stop-words dropped.
///
/// Returned distinct and in order of first appearance, because the intersection is a set —
/// a word repeated in a goal must not count twice — and because `matched_words` is read by
/// a person, who follows it best in the order they wrote it.
#[must_use]
pub fn tokens(goal: &str, lang: Option<&str>) -> Vec<String> {
    let stop = stop_words(lang);
    let folded: String = goal.nfkc().collect::<String>().to_lowercase();
    let mut found: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for token in NON_ALPHANUMERIC.split(&folded) {
        if token.is_empty() || seen.contains(token) {
            continue;
        }
        // Code points, deliberately, and not grapheme clusters: this is what the server
        // counts with `Array.from(token).length`, and the two implementations have to drop
        // the same tokens.
        if token.chars().count() < MIN_GOAL_TOKEN_LENGTH {
            continue;
        }
        if stop.contains(token) {
            continue;
        }
        seen.insert(token.to_owned());
        found.push(token.to_owned());
    }
    found
}

/// Whether a runbook matches, and on which words.
///
/// `None` when the places differ or the goals share nothing beyond stop-words.
#[must_use]
pub fn matches(runbook: &Runbook, query: &RunbookQuery) -> Option<Vec<String>> {
    if normalize_where(&runbook.r#where) != normalize_where(&query.where_) {
        return None;
    }
    let lang = query.lang.as_deref();
    let theirs: HashSet<String> = tokens(&runbook.goal, lang).into_iter().collect();
    let shared: Vec<String> = tokens(&query.goal, lang)
        .into_iter()
        .filter(|word| theirs.contains(word))
        .collect();
    if shared.is_empty() {
        None
    } else {
        Some(shared)
    }
}

/// `last_verified_at` as an instant, so two timestamps written with different offsets or
/// different fractional precision still compare correctly.
///
/// The schema has already checked the format; a value that still fails to parse ranks last
/// rather than poisoning the comparison, because `None` orders below every `Some`.
fn verified_at(runbook: &Runbook) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&runbook.last_verified_at)
        .ok()
        .map(|instant| instant.with_timezone(&Utc))
}

/// The matching runbooks, ranked and capped.
///
/// `runbooks` is whatever the reader handed over; this function does no I/O, so the safety
/// net — which never fails on a runbook problem — and the tool, which reports an unreadable
/// folder, can share it after deciding what an unreadable folder means.
///
/// Ranking: shared-token count descending, then `last_verified_at` descending, then `id`
/// ascending in byte order (ids are ASCII, and the order must not depend on a locale).
#[must_use]
pub fn match_runbooks<'a>(
    runbooks: &'a [StoredRunbook],
    query: &RunbookQuery,
) -> Vec<MatchedRunbook<'a>> {
    let mut matched: Vec<MatchedRunbook<'a>> = runbooks
        .iter()
        .filter_map(|stored| {
            matches(&stored.runbook, query).map(|matched_words| MatchedRunbook {
                stored,
                matched_words,
            })
        })
        .collect();

    matched.sort_by(|left, right| {
        right
            .matched_words
            .len()
            .cmp(&left.matched_words.len())
            .then_with(|| {
                verified_at(&right.stored.runbook).cmp(&verified_at(&left.stored.runbook))
            })
            .then_with(|| left.stored.runbook.id.cmp(&right.stored.runbook.id))
    });

    matched.truncate(RUNBOOK_MATCH_MAX_RESULTS);
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrows_and_separators_of_the_design_all_collapse() {
        let expected = "stripe dashboard developers webhooks";
        for written in [
            "Stripe Dashboard → Developers → Webhooks",
            "stripe dashboard / developers / webhooks",
            "Stripe Dashboard » Developers » Webhooks",
            "Stripe Dashboard \\ Developers \\ Webhooks",
            "Stripe Dashboard | Developers | Webhooks",
            "Stripe Dashboard – Developers — Webhooks",
            "Stripe Dashboard: Developers, Developers; Webhooks",
            "Stripe Dashboard > Developers > Webhooks.",
        ] {
            let normalised = normalize_where(written);
            assert!(
                normalised.starts_with("stripe dashboard developers"),
                "{written} normalised to {normalised}"
            );
        }
        assert_eq!(
            normalize_where("  Stripe Dashboard → Developers → Webhooks  "),
            expected
        );
    }

    #[test]
    fn a_byte_order_mark_is_whitespace_on_both_sides() {
        // JavaScript's `\s` matches U+FEFF and Rust's does not; the explicit class is why
        // the two implementations agree here.
        assert_eq!(
            normalize_where("stripe\u{feff}dashboard"),
            "stripe dashboard"
        );
        // And U+0085, which Rust's `\s` matches and JavaScript's does not.
        assert_eq!(
            normalize_where("stripe\u{0085}dashboard"),
            "stripe dashboard"
        );
    }

    #[test]
    fn short_words_and_stop_words_are_dropped() {
        let found = tokens("Set up the Stripe webhook", Some("en"));
        assert_eq!(found, vec!["set", "stripe", "webhook"]);
    }

    #[test]
    fn a_word_repeated_in_a_goal_is_counted_once_and_kept_in_order() {
        let found = tokens("webhook for the webhook events", Some("en"));
        assert_eq!(found, vec!["webhook", "events"]);
    }

    #[test]
    fn a_token_shorter_than_three_code_points_is_dropped_whatever_its_bytes() {
        // Two code points, six bytes: `str::len` would keep it, `chars().count()` drops it.
        assert!(tokens("\u{4e2d}\u{6587} webhook", None).contains(&"webhook".to_owned()));
        assert!(!tokens("\u{4e2d}\u{6587} webhook", None).contains(&"\u{4e2d}\u{6587}".to_owned()));
    }
}
