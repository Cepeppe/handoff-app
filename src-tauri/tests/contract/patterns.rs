//! The certain-secret pattern file and its corpora (§4.1, §4.6, §11.2 "Certain detector").
//!
//! This is the **first** compilation of `patterns/certain-secrets.v1.json` with the Rust
//! `regex` crate: until now the file was only ever compiled by JavaScript (a note under
//! T-029 says so, from T-006). A failure here is a defect of the file, not of this port.
//!
//! What is checked is what a compiler cannot: that the file stays inside the subset the two
//! engines share, that every family of §4.6 is present in the documented order, that the
//! two corpora keep recall at 1.0 with zero false positives, and that no identifier this
//! project generates can ever be read as a secret. The server's
//! `test/contract/patterns.test.ts` asserts the same things on the other side; the
//! syntactic guard below is its mirror.

use std::collections::HashSet;

use handoff_app_lib::format::patterns::{pattern_file, patterns_version};
use handoff_app_lib::redaction::certain::{scan_text, CertainSecretKind, CERTAIN_SECRET_KINDS};
use regex::Regex;

use crate::support::{fixture_dir, read_to_string};

/// The families of §4.6, in the order the file must keep.
const EXPECTED_IDS: [&str; 16] = [
    "private_key_block",
    "aws_access_key_id",
    "stripe_secret_key",
    "stripe_webhook_secret",
    "github_token",
    "slack_token",
    "slack_webhook_url",
    "google_api_key",
    "anthropic_api_key",
    "openai_api_key",
    "gitlab_pat",
    "npm_token",
    "sendgrid_api_key",
    "huggingface_token",
    "digitalocean_token",
    "jwt",
];

/// Constructs one engine has and the other does not, or has with other semantics.
///
/// `\b` is on the list because JavaScript in `u` mode defines it over ASCII word characters
/// and the Rust crate over Unicode ones, so the same pattern would cut differently on the
/// two sides next to a non-ASCII character (T-006).
const FORBIDDEN: [(&str, &str); 8] = [
    (r"\(\?=", "look-ahead"),
    (r"\(\?!", "negative look-ahead"),
    (r"\(\?<", "look-behind or named group"),
    (r"\(\?>", "atomic group"),
    (r"\(\?[imsuxU)-]", "inline flags"),
    (r"\\[1-9]", "back-reference"),
    (r"[*+?}]\+", "possessive quantifier"),
    (r"\\[bB]", "word boundary"),
];

fn corpus(name: &str) -> String {
    read_to_string(&fixture_dir(&format!("fixtures/secrets/{name}")))
}

/// Corpus lines that carry a secret: blank lines and `#` headers are not part of it.
fn positive_lines() -> Vec<String> {
    corpus("positive.txt")
        .lines()
        .map(|line| line.trim_end().to_owned())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

#[test]
fn the_file_is_version_one_and_carries_the_notes_that_explain_the_subset() {
    let file = pattern_file();
    assert_eq!(file.patterns_version, 1);
    assert_eq!(patterns_version(), file.patterns_version);
    assert!(file.notes["regex_subset"].contains("no look-behind"));
    assert!(file.notes["word_boundaries"].contains(r"\b"));
    assert!(file.notes["policy"]
        .to_lowercase()
        .contains("precision first"));
    for (name, text) in &file.notes {
        assert!(!text.trim().is_empty(), "the note `{name}` is empty");
    }
}

#[test]
fn it_holds_every_family_of_the_design_in_the_documented_order() {
    let ids: Vec<&str> = pattern_file()
        .patterns
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(ids, EXPECTED_IDS);
    // The more specific prefix has to win: an Anthropic key also satisfies the wider
    // OpenAI shape, and file order is what resolves the overlap.
    let anthropic = ids.iter().position(|id| *id == "anthropic_api_key");
    let openai = ids.iter().position(|id| *id == "openai_api_key");
    assert!(anthropic < openai);
}

#[test]
fn every_pattern_has_a_family_a_description_and_both_kinds_of_example() {
    for entry in &pattern_file().patterns {
        let kind: CertainSecretKind = entry
            .kind
            .parse()
            .unwrap_or_else(|error| panic!("{}: {error}", entry.id));
        assert!(CERTAIN_SECRET_KINDS.contains(&kind));
        assert!(
            !entry.description.is_empty(),
            "{} has no description",
            entry.id
        );
        assert!(
            !entry.tests.match_.is_empty(),
            "{} has no example",
            entry.id
        );
        assert!(
            !entry.tests.no_match.is_empty(),
            "{} has no counter-example",
            entry.id
        );
    }
}

#[test]
fn every_pattern_compiles_with_the_rust_regex_crate() {
    for entry in &pattern_file().patterns {
        Regex::new(&entry.regex)
            .unwrap_or_else(|error| panic!("{} does not compile: {error}", entry.id));
    }
}

#[test]
fn every_pattern_stays_inside_the_subset_the_two_engines_share() {
    for entry in &pattern_file().patterns {
        for (probe, name) in FORBIDDEN {
            let probe = Regex::new(probe).expect("the guard compiles");
            assert!(
                !probe.is_match(&entry.regex),
                "{} uses {name}, which does not travel between the two engines",
                entry.id
            );
        }
    }
}

#[test]
fn every_pattern_matches_its_own_examples_and_rejects_its_counter_examples() {
    for entry in &pattern_file().patterns {
        let regex = Regex::new(&entry.regex).expect("it compiles");
        for sample in &entry.tests.match_ {
            assert!(
                regex.is_match(sample),
                "{} should match its own example",
                entry.id
            );
        }
        for sample in &entry.tests.no_match {
            assert!(
                !regex.is_match(sample),
                "{} should not match {sample}",
                entry.id
            );
        }
    }
}

#[test]
fn the_positive_corpus_is_matched_line_by_line() {
    let lines = positive_lines();
    assert!(
        lines.len() >= 60,
        "the positive corpus shrank to {}",
        lines.len()
    );
    let missed: Vec<&String> = lines
        .iter()
        .filter(|line| scan_text(line).is_empty())
        .collect();
    assert!(missed.is_empty(), "recall is not 1.0; missed {missed:?}");
}

#[test]
fn every_pattern_is_covered_by_at_least_one_positive_line() {
    let lines = positive_lines();
    let uncovered: Vec<&str> = pattern_file()
        .patterns
        .iter()
        .filter(|entry| {
            let regex = Regex::new(&entry.regex).expect("it compiles");
            !lines.iter().any(|line| regex.is_match(line))
        })
        .map(|entry| entry.id.as_str())
        .collect();
    assert!(
        uncovered.is_empty(),
        "no positive line covers {uncovered:?}"
    );
}

#[test]
fn the_negative_corpus_produces_no_match_at_all() {
    let text = corpus("negative.txt");
    let lines = text.lines().filter(|line| !line.is_empty()).count();
    assert!(lines >= 200, "the negative corpus shrank to {lines} lines");
    // The excerpt is only built to make a failure readable; nothing is printed on success,
    // and nothing here is logged (R-19).
    let hits: Vec<String> = scan_text(&text)
        .into_iter()
        .map(|hit| format!("{} at {}..{}", hit.kind, hit.start, hit.end))
        .collect();
    assert!(hits.is_empty(), "the negative corpus produced {hits:?}");
}

#[test]
fn the_stop_word_lists_are_lowercase_and_carry_the_excerpt_of_the_design() {
    let file = pattern_file();
    for tag in ["en", "it"] {
        let words = &file.stop_words[tag];
        assert!(words.len() >= 40, "{tag} ships only {} words", words.len());
        let unique: HashSet<&String> = words.iter().collect();
        assert_eq!(unique.len(), words.len(), "{tag} repeats a word");
        for word in words {
            assert_eq!(*word, word.to_lowercase(), "{word} is not lowercase");
            assert!(
                word.chars().all(|character| character.is_ascii_lowercase()),
                "{word} is not a plain word"
            );
        }
    }
    for word in ["the", "and", "for", "with", "into", "from", "that", "this"] {
        assert!(file.stop_words["en"].iter().any(|entry| entry == word));
    }
    for word in [
        "il", "lo", "la", "gli", "per", "con", "del", "della", "che", "una", "uno",
    ] {
        assert!(file.stop_words["it"].iter().any(|entry| entry == word));
    }
}
