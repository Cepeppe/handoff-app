//! Schema validation of every shared document (§4.2, §4.3, §4.5, §6, §11.2).
//!
//! The four schemas of the pinned release — spec, outcome, runbook and the internal channel
//! protocol — are embedded verbatim at build time (§3.4: the app never carries a divergent
//! copy) and compiled once into JSON Schema 2020-12 validators.
//!
//! # Why all four are registered together
//!
//! `handoff-outcome.v1.schema.json` does **not** compile on its own: its
//! `$defs/runbook_match/properties/draft_spec` is a `$ref` to the absolute `$id` of the
//! spec schema, and the channel schema references both public schemas by relative path.
//! Every schema is therefore put into one `referencing` registry under its own `$id`, and
//! each validator is built against that registry. Nothing is fetched: the crate is compiled
//! with `default-features = false` precisely so that it cannot reach the network.
//!
//! # Why the errors are collapsed
//!
//! One mistake must be reported as one problem, at one location, or the server and the app
//! name different fields for the same file. `<name>.expected.json` beside every invalid spec
//! fixture is the contract, and `tests/contract/specs.rs` holds both sides to it.
//!
//! Two shapes of the spec schema report a single mistake through a nested structure:
//!
//! - `anyOf` — `$defs/value` is "a string or a list of strings". A branch that failed
//!   because the *type* did not match is not the branch the author meant, so it is dropped;
//!   what is left is the branch they did mean, and its errors carry the real location
//!   (`values.events[0]`, not `values.events`). When no branch matched the type — a value
//!   that is a number — nothing survives and the `anyOf` keeps its own location.
//! - `propertyNames` — a key that breaks the name rule. The error already sits at the
//!   object (`values`), which is where the server reports it too, so the nested failure of
//!   the name itself is not descended into.
//!
//! The rule and its reasoning are the server's, in `src/format/render-errors.ts`; this is
//! the same rule written against a validator that nests where ajv flattens.

use std::sync::LazyLock;

use jsonschema::error::ValidationErrorKind;
use jsonschema::{Draft, Registry, ValidationError, Validator};
use serde_json::Value;

use super::paths::display_path;

/// The vendored schemas, embedded at build time.
const SPEC_SCHEMA_JSON: &str =
    include_str!("../../../vendor/handoff-mcp/format/schemas/handoff-spec.v1.schema.json");
const OUTCOME_SCHEMA_JSON: &str =
    include_str!("../../../vendor/handoff-mcp/format/schemas/handoff-outcome.v1.schema.json");
const RUNBOOK_SCHEMA_JSON: &str =
    include_str!("../../../vendor/handoff-mcp/format/schemas/handoff-runbook.v1.schema.json");
const CHANNEL_SCHEMA_JSON: &str =
    include_str!("../../../vendor/handoff-mcp/format/protocol/channel/channel.v1.schema.json");

/// The `$id` of each schema, which is also the URI the others reference it by. Read from
/// the files themselves rather than retyped, so a released change of `$id` cannot be missed
/// here; a test asserts they are what the design publishes.
fn schema_id(schema: &Value) -> String {
    schema
        .get("$id")
        .and_then(Value::as_str)
        .expect("a vendored schema has no $id")
        .to_owned()
}

struct Schemas {
    spec: Value,
    outcome: Value,
    runbook: Value,
    channel: Value,
}

static SCHEMAS: LazyLock<Schemas> = LazyLock::new(|| Schemas {
    spec: parse(SPEC_SCHEMA_JSON, "handoff-spec.v1.schema.json"),
    outcome: parse(OUTCOME_SCHEMA_JSON, "handoff-outcome.v1.schema.json"),
    runbook: parse(RUNBOOK_SCHEMA_JSON, "handoff-runbook.v1.schema.json"),
    channel: parse(CHANNEL_SCHEMA_JSON, "channel.v1.schema.json"),
});

fn parse(source: &str, name: &str) -> Value {
    serde_json::from_str(source).unwrap_or_else(|error| {
        panic!("the vendored {name} does not parse: {error}");
    })
}

/// The four compiled validators.
struct Validators {
    spec: Validator,
    outcome: Validator,
    runbook: Validator,
    channel: Validator,
}

/// Built once. A schema that fails to compile is a defect of the pinned artifact, and there
/// is nothing a caller could do about it at run time, so this panics with the schema named.
static VALIDATORS: LazyLock<Validators> = LazyLock::new(|| {
    let schemas = &*SCHEMAS;

    // The channel schema references the two public schemas by relative path, and the
    // outcome references the spec by absolute `$id`. Registering all four under their own
    // `$id` resolves both without anything being retrieved.
    let registry = Registry::new()
        .draft(Draft::Draft202012)
        .extend([
            (schema_id(&schemas.spec), schemas.spec.clone()),
            (schema_id(&schemas.outcome), schemas.outcome.clone()),
            (schema_id(&schemas.runbook), schemas.runbook.clone()),
            (schema_id(&schemas.channel), schemas.channel.clone()),
        ])
        .expect("the vendored schemas do not form a registry")
        .prepare()
        .expect("the vendored schemas do not form a registry");

    // `should_validate_formats` on purpose: in 2020-12 `format` is an annotation by
    // default, and the server asserts it (ajv-formats). A `last_verified_at` that is not a
    // date-time has to be refused on both sides — `fixtures/runbooks/invalid` has the case.
    let build = |schema: &Value, name: &str| {
        jsonschema::options()
            .with_registry(&registry)
            .should_validate_formats(true)
            .build(schema)
            .unwrap_or_else(|error| panic!("the vendored {name} does not compile: {error}"))
    };

    Validators {
        spec: build(&schemas.spec, "handoff-spec.v1.schema.json"),
        outcome: build(&schemas.outcome, "handoff-outcome.v1.schema.json"),
        runbook: build(&schemas.runbook, "handoff-runbook.v1.schema.json"),
        channel: build(&schemas.channel, "channel.v1.schema.json"),
    }
});

/// One thing that is wrong with a document, at one location.
///
/// `path` is the display notation of §4.7.5 and is what the fixtures pin. `keyword` is the
/// schema keyword that refused the value, kept so a caller can tell "unknown field" from
/// "too long" without parsing a sentence; the reader-facing texts of the error catalogue
/// are the server's, because the server is what answers agents (§2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Where, e.g. `steps[0].text`.
    pub path: String,
    /// Which schema keyword refused it, e.g. `maxLength`.
    pub keyword: String,
}

/// Which document a validator is for, for the messages and for the tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Document {
    /// A handoff spec.
    Spec,
    /// An outcome.
    Outcome,
    /// A runbook.
    Runbook,
    /// One line of the internal channel.
    ChannelMessage,
}

impl Document {
    fn validator(self) -> &'static Validator {
        let validators = &*VALIDATORS;
        match self {
            Self::Spec => &validators.spec,
            Self::Outcome => &validators.outcome,
            Self::Runbook => &validators.runbook,
            Self::ChannelMessage => &validators.channel,
        }
    }
}

/// Validates `instance` against one of the four schemas.
///
/// The problems come back collapsed and sorted by location, so two runs over the same
/// document report the same list.
///
/// # Errors
///
/// Every location the schema refused, at most one problem per mistake.
pub fn validate(document: Document, instance: &Value) -> Result<(), Vec<Problem>> {
    let mut problems: Vec<Problem> = Vec::new();
    for error in document.validator().iter_errors(instance) {
        collect(&error, "", &mut problems);
    }
    if problems.is_empty() {
        return Ok(());
    }
    problems.sort_by(|left, right| left.path.cmp(&right.path));
    problems.dedup();
    Err(problems)
}

/// True when `instance` satisfies the schema. The cheap form, for a hot path that only
/// needs a yes or a no.
#[must_use]
pub fn is_valid(document: Document, instance: &Value) -> bool {
    document.validator().is_valid(instance)
}

/// Turns one validation error into problems, descending only where a nested structure
/// describes a single mistake in a more precise place than its parent.
fn collect(error: &ValidationError<'_>, root: &str, problems: &mut Vec<Problem>) {
    let path = display_path(error.instance_path(), root);
    match error.kind() {
        // A key that breaks the name rule. The error already sits at the object, which is
        // where the server reports it as well; the nested failure of the name itself adds
        // no location.
        ValidationErrorKind::PropertyNames { .. } => problems.push(Problem {
            path,
            keyword: "propertyNames".to_owned(),
        }),

        // Every unexpected key is its own problem, named where it sits: an agent that
        // mistyped two fields has to see both (SPEC-06).
        ValidationErrorKind::AdditionalProperties { unexpected } => {
            for field in unexpected {
                problems.push(Problem {
                    path: super::paths::child_path(&path, super::paths::Segment::Field(field)),
                    keyword: "additionalProperties".to_owned(),
                });
            }
        }

        // A missing field is reported at the field, not at the object holding it.
        ValidationErrorKind::Required { property } => {
            let name = property.as_str().unwrap_or_default();
            problems.push(Problem {
                path: super::paths::child_path(&path, super::paths::Segment::Field(name)),
                keyword: "required".to_owned(),
            });
        }

        // The one place the document has to be read more precisely than the keyword.
        ValidationErrorKind::AnyOf { context } => {
            collect_any_of(error, context, root, problems);
        }

        other => problems.push(Problem {
            path,
            keyword: other.keyword().to_owned(),
        }),
    }
}

/// The `anyOf` rule: keep the branch whose type matched, or the `anyOf` itself.
fn collect_any_of(
    error: &ValidationError<'_>,
    context: &[Vec<ValidationError<'static>>],
    root: &str,
    problems: &mut Vec<Problem>,
) {
    let at = error.instance_path();
    let survivors: Vec<&Vec<ValidationError<'static>>> = context
        .iter()
        .filter(|branch| {
            // A branch that failed on the type of the value itself is not the shape the
            // author meant; a branch that failed deeper inside it is.
            !branch.iter().any(|inner| {
                matches!(inner.kind(), ValidationErrorKind::Type { .. })
                    && inner.instance_path() == at
            })
        })
        .collect();

    if survivors.is_empty() {
        problems.push(Problem {
            path: display_path(at, root),
            keyword: "anyOf".to_owned(),
        });
        return;
    }
    for branch in survivors {
        for inner in branch {
            collect(inner, root, problems);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The `$id`s the design publishes (§4.2, §4.3, §4.5, §6.1).
    const IDS: [&str; 4] = [
        "https://raw.githubusercontent.com/Cepeppe/handoff-mcp/main/schemas/handoff-spec.v1.schema.json",
        "https://raw.githubusercontent.com/Cepeppe/handoff-mcp/main/schemas/handoff-outcome.v1.schema.json",
        "https://raw.githubusercontent.com/Cepeppe/handoff-mcp/main/schemas/handoff-runbook.v1.schema.json",
        "https://raw.githubusercontent.com/Cepeppe/handoff-mcp/main/protocol/channel/channel.v1.schema.json",
    ];

    fn minimal_spec() -> Value {
        json!({
            "spec_version": 1,
            "goal": "Register the webhook",
            "where": "Dashboard",
            "why_human": "Only a person can sign in.",
            "values": {},
            "steps": [{ "text": "Open the page." }]
        })
    }

    #[test]
    fn the_four_schemas_compile_and_keep_their_published_ids() {
        let schemas = &*SCHEMAS;
        let ids = [
            schema_id(&schemas.spec),
            schema_id(&schemas.outcome),
            schema_id(&schemas.runbook),
            schema_id(&schemas.channel),
        ];
        assert_eq!(ids, IDS);
        // Forces the compilation; a defect in a vendored schema fails here.
        assert!(is_valid(Document::Spec, &minimal_spec()));
    }

    #[test]
    fn an_unknown_field_is_reported_where_it_sits() {
        let mut spec = minimal_spec();
        spec["warnings"] = json!("nope");
        let problems = validate(Document::Spec, &spec).expect_err("should be refused");
        assert_eq!(
            problems,
            vec![Problem {
                path: "warnings".to_owned(),
                keyword: "additionalProperties".to_owned()
            }]
        );
    }

    #[test]
    fn a_bad_value_key_is_reported_at_the_object() {
        let mut spec = minimal_spec();
        spec["values"] = json!({ "9lives": "x" });
        let problems = validate(Document::Spec, &spec).expect_err("should be refused");
        assert_eq!(
            problems,
            vec![Problem {
                path: "values".to_owned(),
                keyword: "propertyNames".to_owned()
            }]
        );
    }

    #[test]
    fn an_any_of_descends_into_the_branch_whose_type_matched() {
        let mut spec = minimal_spec();
        spec["values"] = json!({ "events": ["a".repeat(4097)] });
        let problems = validate(Document::Spec, &spec).expect_err("should be refused");
        assert_eq!(
            problems,
            vec![Problem {
                path: "values.events[0]".to_owned(),
                keyword: "maxLength".to_owned()
            }]
        );
    }

    #[test]
    fn an_any_of_with_no_matching_type_stays_at_the_any_of() {
        let mut spec = minimal_spec();
        spec["values"] = json!({ "retries": 3 });
        let problems = validate(Document::Spec, &spec).expect_err("should be refused");
        assert_eq!(
            problems,
            vec![Problem {
                path: "values.retries".to_owned(),
                keyword: "anyOf".to_owned()
            }]
        );
    }

    #[test]
    fn a_missing_field_is_reported_at_the_field() {
        let mut spec = minimal_spec();
        spec.as_object_mut().expect("an object").remove("why_human");
        let problems = validate(Document::Spec, &spec).expect_err("should be refused");
        assert_eq!(
            problems,
            vec![Problem {
                path: "why_human".to_owned(),
                keyword: "required".to_owned()
            }]
        );
    }

    #[test]
    fn a_date_time_format_is_asserted_not_annotated() {
        // The runbook schema is the one that carries `format: date-time`; a validator that
        // treated formats as annotations would accept this.
        let runbook = json!({ "last_verified_at": "not a date" });
        let problems = validate(Document::Runbook, &runbook).expect_err("should be refused");
        assert!(problems
            .iter()
            .any(|problem| problem.path == "last_verified_at" && problem.keyword == "format"));
    }
}
