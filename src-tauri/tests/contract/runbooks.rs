//! The runbook fixtures (§4.5, §11.2).
//!
//! The app is what writes these files (T-044) and the server is what reads them, so both
//! halves matter here: a valid runbook has to survive a round trip through [`Runbook`]
//! unchanged, and an invalid one has to be refused — by the schema, by
//! `deny_unknown_fields`, or by the typed shapes.
//!
//! `last-verified-at-not-a-date-time.json` is the fixture that fails only if `format` is
//! asserted rather than annotated, which is what the validator is configured for.

use handoff_app_lib::format::runbook::Runbook;
use handoff_app_lib::format::schema::{validate, Document};

use crate::support::{assert_round_trips, fixture_files, name_of, read_json};

#[test]
fn every_valid_runbook_fixture_validates_and_round_trips() {
    let fixtures = fixture_files("fixtures/runbooks/valid", ".json");
    assert!(fixtures.len() >= 3, "the valid runbook fixture set shrank");
    for fixture in fixtures {
        let name = name_of(&fixture);
        let document = read_json(&fixture);
        validate(Document::Runbook, &document)
            .unwrap_or_else(|problems| panic!("{name} was refused by the schema: {problems:?}"));
        assert_round_trips::<Runbook>(&document, &name);
    }
}

#[test]
fn every_invalid_runbook_fixture_is_refused() {
    let fixtures = fixture_files("fixtures/runbooks/invalid", ".json");
    assert!(
        fixtures.len() >= 8,
        "the invalid runbook fixture set shrank"
    );
    for fixture in fixtures {
        let name = name_of(&fixture);
        let document = read_json(&fixture);
        let by_schema = validate(Document::Runbook, &document).is_err();
        let by_serde = serde_json::from_value::<Runbook>(document).is_err();
        assert!(
            by_schema || by_serde,
            "{name} was accepted by both the schema and the types"
        );
        // Every one of these is a structural mistake, so the schema has to catch it: serde
        // alone would not notice `runs: 0` or a bad id.
        assert!(by_schema, "{name} was accepted by the schema");
    }
}

#[test]
fn a_runbook_keeps_its_nulls_rather_than_dropping_them() {
    // Every field of a runbook is required, so `url: null` is not the same document as a
    // runbook with no `url`. The minimal fixture is the one that carries the nulls.
    let document = read_json(&crate::support::fixture_dir(
        "fixtures/runbooks/valid/minimal-confirmed-by-user.json",
    ));
    let parsed: Runbook = serde_json::from_value(document.clone()).expect("it deserialises");
    let written = serde_json::to_value(&parsed).expect("it serialises");
    for nullable in ["url", "lang", "verify", "last_run_failed_at"] {
        assert!(
            written.get(nullable).is_some(),
            "the runbook lost its `{nullable}` on the way back out"
        );
    }
    assert_eq!(written, document);
}
