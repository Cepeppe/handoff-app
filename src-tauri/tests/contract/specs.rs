//! The spec fixtures (§4.2, §11.2 "Schema validation", row `both`).
//!
//! `fixtures/specs/valid` must pass, `fixtures/specs/invalid` must fail, and where the
//! failure is a schema failure the location has to be the one `<name>.expected.json`
//! records — the same location the server reports. One fixture is one mistake, so one
//! problem: a second problem here would mean the same mistake is being reported twice.
//!
//! Four of the invalid fixtures carry `"schema_valid": true`. Those break a **semantic**
//! rule of §4.2 — a blank field after trimming, a placeholder, a step citing a value the
//! spec does not declare — which the server applies after the schema (§5.4) and which the
//! app does not implement: the server is what answers agents (§2.4). What this suite pins
//! for them is that the schema *accepts* them, so the two sides agree about where the
//! boundary between the two stages runs.
//!
//! `code` is not compared for the same reason: `SPEC_INVALID` and
//! `SPEC_VERSION_UNSUPPORTED` belong to the server's error catalogue (§4.7.5). What is
//! compared is that both codes are still represented, so a fixture set that lost the
//! version case would be noticed here as well.

use std::path::Path;

use handoff_app_lib::format::schema::{validate, Document};
use handoff_app_lib::format::spec::HandoffSpec;
use serde_json::Value;

use crate::support::{assert_round_trips, fixture_dir, fixture_files, name_of, read_json};

/// What a `<name>.expected.json` records.
struct Expectation {
    path: String,
    code: String,
    schema_valid: bool,
}

fn expectation_for(fixture: &Path) -> Expectation {
    let stem = fixture
        .file_stem()
        .and_then(|name| name.to_str())
        .expect("a fixture name");
    let expected = fixture.with_file_name(format!("{stem}.expected.json"));
    let value = read_json(&expected);
    Expectation {
        path: value["path"].as_str().expect("a path").to_owned(),
        code: value["code"].as_str().expect("a code").to_owned(),
        schema_valid: value
            .get("schema_valid")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

#[test]
fn every_valid_spec_fixture_validates_and_round_trips() {
    let fixtures = fixture_files("fixtures/specs/valid", ".json");
    assert!(fixtures.len() >= 8, "the valid fixture set shrank");
    for fixture in fixtures {
        let name = name_of(&fixture);
        let document = read_json(&fixture);
        validate(Document::Spec, &document)
            .unwrap_or_else(|problems| panic!("{name} was refused by the schema: {problems:?}"));
        assert_round_trips::<HandoffSpec>(&document, &name);
    }
}

#[test]
fn every_invalid_spec_fixture_is_refused_where_the_expectation_says() {
    let mut codes: Vec<String> = Vec::new();
    let mut semantic_only: Vec<String> = Vec::new();
    let mut checked = 0_usize;

    for fixture in fixture_files("fixtures/specs/invalid", ".json") {
        let name = name_of(&fixture);
        if name.ends_with(".expected.json") {
            continue;
        }
        checked += 1;
        let document = read_json(&fixture);
        let expected = expectation_for(&fixture);
        codes.push(expected.code.clone());

        let outcome = validate(Document::Spec, &document);
        if expected.schema_valid {
            semantic_only.push(name.clone());
            outcome.unwrap_or_else(|problems| {
                panic!("{name} should pass the schema and fail a semantic rule: {problems:?}")
            });
            // It still has to parse: the semantic stage runs on a spec, not on a `Value`.
            serde_json::from_value::<HandoffSpec>(document)
                .unwrap_or_else(|error| panic!("{name} does not deserialise: {error}"));
            continue;
        }

        let problems = outcome
            .err()
            .unwrap_or_else(|| panic!("{name} should be refused by the schema"));
        assert_eq!(
            problems.iter().map(|p| p.path.clone()).collect::<Vec<_>>(),
            vec![expected.path.clone()],
            "{name} reports the wrong location"
        );
    }

    assert!(checked >= 30, "the invalid fixture set shrank to {checked}");
    assert_eq!(
        semantic_only,
        vec![
            "goal-blank.json",
            "placeholder-in-step-text.json",
            "placeholder-in-value.json",
            "step-values-unknown-key.json"
        ],
        "the set of fixtures that pass the schema and fail a semantic rule changed"
    );
    assert!(codes.iter().any(|code| code == "SPEC_INVALID"));
    assert!(codes.iter().any(|code| code == "SPEC_VERSION_UNSUPPORTED"));
}

#[test]
fn the_types_refuse_an_unknown_field_and_a_wrong_shape_on_their_own() {
    // `deny_unknown_fields` and the typed shapes are the second half of the deliverable:
    // an invalid fixture is caught by the schema *or* by deserialisation, and these five
    // are the ones a `Value` alone would carry into the store unnoticed. The others fail on
    // a limit serde knows nothing about, which is why the schema is what validates.
    for name in [
        "unknown-field-top-level",
        "unknown-field-in-step",
        "control-field-in-spec",
        "string-step",
        "value-wrong-type",
    ] {
        let document = read_json(&fixture_dir(&format!("fixtures/specs/invalid/{name}.json")));
        assert!(
            serde_json::from_value::<HandoffSpec>(document).is_err(),
            "{name} deserialised although its shape is not a spec"
        );
    }
}

#[test]
fn a_spec_that_declares_no_optional_field_keeps_it_absent() {
    // The round trip above would still pass if `null`s were written where the file has
    // nothing, as long as it happened on the way in and out alike. This says it directly,
    // on the fixture written for exactly that shape.
    let minimal = read_json(&fixture_dir("fixtures/specs/valid/minimal.json"));
    let parsed: HandoffSpec =
        serde_json::from_value(minimal.clone()).expect("the minimal fixture deserialises");
    let written = serde_json::to_value(&parsed).expect("it serialises");
    for absent in ["url", "secrets", "verify", "lang"] {
        assert!(
            written.get(absent).is_none(),
            "the minimal spec grew a `{absent}` on the way back out"
        );
    }
    assert_eq!(written, minimal);
}
