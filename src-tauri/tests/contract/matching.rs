//! The matching fixtures (§4.5.3, RUN-07a, §11.2 "Runbook matching").
//!
//! `fixtures/matching/*.json` is the contract: the server and the app must produce the same
//! results, in the same order, with the same `matched_words`. Each case carries only the
//! four fields the rule reads; the expansion that fills the rest is written down in
//! `fixtures/matching/README.md` and is reproduced here exactly, because a filler that
//! varied between cases would make a case depend on something the rule never looks at.
//!
//! Every expanded runbook is validated against `handoff-runbook.v1.schema.json` before it
//! is matched, so no case can rest on a document the format would refuse.

use handoff_app_lib::format::runbook::Runbook;
use handoff_app_lib::format::schema::{validate, Document};
use handoff_app_lib::runbooks::matching::{
    match_runbooks, RunbookQuery, StoredRunbook, RUNBOOK_MATCH_MAX_RESULTS,
};
use serde::Deserialize;
use serde_json::json;

use crate::support::{fixture_files, name_of, read_json};

/// The four fields of a runbook the matching rule reads; everything else is filler.
#[derive(Debug, Clone, Deserialize)]
struct RunbookStub {
    id: String,
    #[serde(rename = "where")]
    where_: String,
    goal: String,
    last_verified_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Query {
    #[serde(rename = "where")]
    where_: String,
    goal: String,
    #[serde(default)]
    lang: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ExpectedMatch {
    id: String,
    matched_words: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MatchingCase {
    #[serde(rename = "case")]
    name: String,
    why: String,
    query: Query,
    runbooks: Vec<RunbookStub>,
    expected: Vec<ExpectedMatch>,
}

/// The expansion of `fixtures/matching/README.md`: the same filler for every stub, so a case
/// can only ever differ from another in the four fields it declares.
fn expand(stub: &RunbookStub) -> Runbook {
    serde_json::from_value(json!({
        "runbook_version": 1,
        "id": stub.id,
        "where": stub.where_,
        "goal": stub.goal,
        "why_human": "A person has to do this in the browser.",
        "url": null,
        "lang": null,
        "values": {},
        "secrets": {},
        "steps": [{ "text": "Do the thing.", "url": null, "values": [], "warning": null, "annotations": [] }],
        "verify": null,
        "trust": "verified",
        "last_verified_at": stub.last_verified_at,
        "last_run_failed_at": null,
        "runs": 1,
        "created_at": stub.last_verified_at,
        "updated_at": stub.last_verified_at,
        "origin": { "app": "handoff-app", "app_version": "1.0.0" }
    }))
    .expect("the expansion of a stub is a runbook")
}

fn cases() -> Vec<(String, MatchingCase)> {
    fixture_files("fixtures/matching", ".json")
        .into_iter()
        .map(|path| {
            let name = name_of(&path);
            let case: MatchingCase = serde_json::from_value(read_json(&path))
                .unwrap_or_else(|error| panic!("{name} is not a matching case: {error}"));
            (name, case)
        })
        .collect()
}

#[test]
fn every_case_is_named_after_its_file_and_says_what_it_pins() {
    let cases = cases();
    assert!(cases.len() >= 12, "the matching fixture set shrank");
    for (file, case) in &cases {
        assert_eq!(format!("{}.json", case.name), *file);
        assert!(!case.why.is_empty(), "{file} does not say what it pins");
    }
}

#[test]
fn every_stub_expands_into_a_runbook_the_schema_accepts() {
    for (file, case) in cases() {
        for stub in &case.runbooks {
            let expanded = serde_json::to_value(expand(stub)).expect("it serialises");
            validate(Document::Runbook, &expanded).unwrap_or_else(|problems| {
                panic!(
                    "{file}: the expansion of {} is refused: {problems:?}",
                    stub.id
                )
            });
        }
    }
}

#[test]
fn every_case_matches_exactly_what_the_fixture_expects() {
    for (file, case) in cases() {
        let stored: Vec<StoredRunbook> = case
            .runbooks
            .iter()
            .map(|stub| StoredRunbook {
                path: std::path::PathBuf::from(format!("{}.json", stub.id)),
                runbook: expand(stub),
            })
            .collect();
        let query = RunbookQuery {
            where_: case.query.where_.clone(),
            goal: case.query.goal.clone(),
            lang: case.query.lang.clone(),
        };

        let got: Vec<(String, Vec<String>)> = match_runbooks(&stored, &query)
            .into_iter()
            .map(|matched| {
                (
                    matched.stored.runbook.id.clone(),
                    matched.matched_words.clone(),
                )
            })
            .collect();
        let want: Vec<(String, Vec<String>)> = case
            .expected
            .iter()
            .map(|expected| (expected.id.clone(), expected.matched_words.clone()))
            .collect();

        assert_eq!(got, want, "{file} ({})", case.why);
    }
}

#[test]
fn the_cases_cover_the_families_the_rule_is_made_of() {
    let names = cases()
        .into_iter()
        .map(|(_, case)| case.name)
        .collect::<Vec<_>>()
        .join(" ");
    for family in [
        "arrow",
        "separator",
        "case-insensitive",
        "nfkc",
        "italian",
        "lang",
        "no-match",
        "ranking",
        "cap-at-five",
    ] {
        assert!(names.contains(family), "no case covers {family}");
    }
}

#[test]
fn the_cap_is_the_one_the_design_fixes() {
    assert_eq!(RUNBOOK_MATCH_MAX_RESULTS, 5);
    let (file, case) = cases()
        .into_iter()
        .find(|(_, case)| case.name == "cap-at-five-results")
        .expect("the cap case is shipped");
    assert!(
        case.runbooks.len() > RUNBOOK_MATCH_MAX_RESULTS,
        "{file} does not offer more runbooks than the cap"
    );
    assert_eq!(case.expected.len(), RUNBOOK_MATCH_MAX_RESULTS);
}
