//! The ingress scan over a whole spec (§5.5, DET-04, SPEC-13).
//!
//! `scan_spec` has a reference implementation to match, `src/secrets/ingress.ts` in the
//! server, and what has to agree is the **set of locations and their order**: `values` in
//! document order with array items indexed, then `goal`, `where`, `why_human`, `verify`,
//! then each step's `text` and `warning`. `url` and `steps[].url` are deliberately not
//! scanned — §5.5 lists the fields and those two are not among them, and a url is already
//! confined to three schemes (S5).
//!
//! The spec used here is built from the vendored valid fixture, with secrets planted in
//! every scanned field, so the case exercises the real shape rather than a hand-written
//! object that could drift from the format.

use handoff_app_lib::format::spec::HandoffSpec;
use handoff_app_lib::redaction::certain::{scan_spec, scan_spec_spans, CertainSecretKind};
use serde_json::json;

use crate::support::{fixture_dir, read_json};

/// Synthetic shapes, one per family used below. None has ever been valid anywhere.
const STRIPE: &str = "sk_live_A1b2C3d4E5f6G7h8I9j0";
const WHSEC: &str = "whsec_KpQ8vN2mR7tYxW4bZ1cD5eF9gH3jL6nM";
const SLACK_URL: &str =
    "https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXX";
const AWS: &str = "AKIAIOSFODNN7EXAMPLE";

fn spec_with_secrets_everywhere() -> HandoffSpec {
    serde_json::from_value(json!({
        "spec_version": 1,
        "goal": format!("Rotate {AWS} in the dashboard"),
        "where": format!("Dashboard {WHSEC}"),
        "url": "https://dashboard.stripe.com/webhooks",
        "why_human": format!("Only a person can read {STRIPE} from the screen."),
        "values": {
            "api_key": STRIPE,
            "endpoints": [SLACK_URL, "https://example.com/ok"],
            "plain": "nothing to see here"
        },
        "steps": [
            { "text": format!("Paste {STRIPE} into .env"), "url": SLACK_URL,
              "warning": format!("Do not commit {WHSEC}") },
            { "text": "Save and close." }
        ],
        "verify": format!("Check that {AWS} no longer works."),
        "lang": "en"
    }))
    .expect("the spec is well formed")
}

#[test]
fn the_locations_are_the_ones_the_server_produces_in_the_order_it_produces_them() {
    let found = scan_spec(&spec_with_secrets_everywhere());
    let locations: Vec<&str> = found
        .iter()
        .map(|treated| treated.location.as_str())
        .collect();
    assert_eq!(
        locations,
        vec![
            "values.api_key",
            "values.endpoints[0]",
            "goal",
            "where",
            "why_human",
            "verify",
            "steps[0].text",
            "steps[0].warning",
        ]
    );
}

#[test]
fn the_family_is_reported_and_never_the_pattern_id() {
    let found = scan_spec(&spec_with_secrets_everywhere());
    let kinds: Vec<CertainSecretKind> = found.iter().map(|treated| treated.kind).collect();
    assert_eq!(
        kinds,
        vec![
            CertainSecretKind::ApiKey,        // values.api_key, a Stripe key
            CertainSecretKind::WebhookUrl,    // values.endpoints[0], a Slack webhook
            CertainSecretKind::ApiKey,        // goal, an AWS key id
            CertainSecretKind::WebhookSecret, // where
            CertainSecretKind::ApiKey,        // why_human
            CertainSecretKind::ApiKey,        // verify
            CertainSecretKind::ApiKey,        // steps[0].text
            CertainSecretKind::WebhookSecret, // steps[0].warning
        ]
    );
}

#[test]
fn a_url_field_is_not_scanned_but_the_same_url_in_a_value_is() {
    // §5.5 lists the fields, and `url` and `steps[].url` are not among them: a Slack
    // webhook URL sent as the spec's `url` reports nothing, and the same string sent as a
    // value reports `webhook_url`. T-048 is where a URL on screen gets caught.
    let spec = spec_with_secrets_everywhere();
    let found = scan_spec(&spec);
    assert!(found.iter().all(|treated| treated.location != "url"));
    assert!(found
        .iter()
        .all(|treated| treated.location != "steps[0].url"));
    assert!(found
        .iter()
        .any(|treated| treated.location == "values.endpoints[0]"
            && treated.kind == CertainSecretKind::WebhookUrl));
}

#[test]
fn a_span_covers_the_secret_and_not_the_sentence_around_it() {
    // Masking replaces the matched span, not the whole field: a step reading "paste … into
    // .env" keeps its instruction and loses only the secret (§5.9).
    let spec = spec_with_secrets_everywhere();
    let span = scan_spec_spans(&spec)
        .into_iter()
        .find(|span| span.location == "steps[0].text")
        .expect("the step carries a secret");
    let text = &spec.steps[0].text;
    assert_eq!(&text[span.start..span.end], STRIPE);
    assert!(span.start > 0, "the sentence before the secret is kept");
}

#[test]
fn a_vendored_spec_fixture_with_no_secret_reports_nothing() {
    let document = read_json(&fixture_dir("fixtures/specs/valid/stripe-webhook.json"));
    let spec: HandoffSpec = serde_json::from_value(document).expect("it deserialises");
    assert_eq!(scan_spec(&spec), Vec::new());
}
