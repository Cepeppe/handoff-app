//! The outcome fixtures (§4.3, §11.2).
//!
//! One file per status of the design's table. Each has to validate against
//! `handoff-outcome.v1.schema.json` and to round-trip through [`Outcome`] field for field:
//! the app builds these objects (T-033) and the log stores them (LOG-02), so a field this
//! crate cannot carry is a field that would be lost between the two.
//!
//! The schema is the one that does not compile alone — its `draft_spec` is a `$ref` to the
//! absolute `$id` of the spec schema — so `runbook-match.json` also proves that the
//! registry of [`handoff_app_lib::format::schema`] resolves across files.

use std::collections::BTreeSet;

use handoff_app_lib::format::outcome::{Outcome, OutcomeStatus};
use handoff_app_lib::format::schema::{validate, Document};

use crate::support::{assert_round_trips, fixture_dir, fixture_files, name_of, read_json};

/// The fourteen statuses of the §4.3 table.
const STATUSES: [OutcomeStatus; 14] = [
    OutcomeStatus::InProgress,
    OutcomeStatus::Question,
    OutcomeStatus::Screenshot,
    OutcomeStatus::Deferred,
    OutcomeStatus::Parked,
    OutcomeStatus::AwaitingVerification,
    OutcomeStatus::ConfirmedByUser,
    OutcomeStatus::Verified,
    OutcomeStatus::Failed,
    OutcomeStatus::NotVerified,
    OutcomeStatus::Abandoned,
    OutcomeStatus::TransferredToOtherSession,
    OutcomeStatus::RunbookMatch,
    OutcomeStatus::TextMode,
];

#[test]
fn every_outcome_fixture_validates_and_round_trips() {
    for fixture in fixture_files("fixtures/outcomes", ".json") {
        let name = name_of(&fixture);
        let document = read_json(&fixture);
        validate(Document::Outcome, &document)
            .unwrap_or_else(|problems| panic!("{name} was refused by the schema: {problems:?}"));
        assert_round_trips::<Outcome>(&document, &name);
    }
}

#[test]
fn the_fixtures_cover_every_status_of_the_design_table() {
    let mut found: BTreeSet<String> = BTreeSet::new();
    for fixture in fixture_files("fixtures/outcomes", ".json") {
        let document = read_json(&fixture);
        found.insert(
            document["status"]
                .as_str()
                .unwrap_or_else(|| panic!("{} has no status", name_of(&fixture)))
                .to_owned(),
        );
    }
    let expected: BTreeSet<String> = STATUSES
        .iter()
        .map(|status| {
            serde_json::to_value(status)
                .expect("a status serialises")
                .as_str()
                .expect("as a string")
                .to_owned()
        })
        .collect();
    assert_eq!(found, expected);
}

#[test]
fn a_runbook_match_carries_a_draft_spec_across_the_schema_boundary() {
    // `handoff-outcome.v1.schema.json` refers to the absolute `$id` of the spec schema, so a
    // validator given the outcome file alone cannot resolve it. If the registry ever stops
    // holding both, this fixture is where it shows.
    let document = read_json(&fixture_dir("fixtures/outcomes/runbook-match.json"));
    validate(Document::Outcome, &document).expect("the published runbook_match validates");

    let outcome: Outcome = serde_json::from_value(document).expect("it deserialises");
    let first = outcome
        .runbooks
        .first()
        .expect("runbook_match carries at least one runbook");
    assert_eq!(first.draft_spec.spec_version, 1);
    assert!(!first.matched_words.is_empty());
    assert!(outcome.handoff_id.is_none(), "no handoff was opened");
    assert!(!outcome.is_final);
}

#[test]
fn a_secret_treated_entry_names_a_family_and_never_a_pattern_id() {
    // The vocabulary of §4.6. `stripe_secret_key` is a pattern id and must not appear here;
    // T-006 corrected a fixture that carried one.
    let document = read_json(&fixture_dir("fixtures/outcomes/awaiting-verification.json"));
    let outcome: Outcome = serde_json::from_value(document).expect("it deserialises");
    assert!(
        !outcome.secret_treated.is_empty(),
        "this fixture is the one that reports a secret"
    );
    for treated in &outcome.secret_treated {
        assert!(
            treated
                .kind
                .parse::<handoff_app_lib::redaction::certain::CertainSecretKind>()
                .is_ok(),
            "`{}` is not one of the six families",
            treated.kind
        );
        assert!(!treated.location.is_empty());
    }
}
