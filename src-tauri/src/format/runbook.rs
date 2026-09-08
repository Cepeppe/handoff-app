//! The runbook, version 1 (§4.5, `schemas/handoff-runbook.v1.schema.json`).
//!
//! One JSON file per runbook in `~/.handoff/runbooks/`. The app writes them (T-044), the
//! server reads them and converts them to a draft spec (RUN-10); this module is the shape
//! both sides agree on and the matching rule of [`crate::runbooks::matching`] reads.
//!
//! Every field is required by the schema, so nothing here is skipped when it serialises:
//! `url`, `lang`, `verify`, `last_run_failed_at` and a step's `url` and `warning` are
//! nullable, and `null` is what they carry when they have no value.
//!
//! A runbook holds value **names**, never values (RUN-04): a placeholder `{{name}}` stands
//! in the step texts where the value was, and the description of a value is the text of the
//! first step it appeared in, with the placeholder in its place.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::outcome::{Annotation, RunbookTrust};

/// What a value of the runbook is remembered as: a description, never a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunbookValue {
    /// The text of the first step the value appeared in, with `{{name}}` in its place, or
    /// `null` when it appeared in no step.
    pub description: Option<String>,
}

/// Where the runbook came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunbookOrigin {
    /// The application that wrote the file.
    pub app: String,
    /// Its version.
    pub app_version: String,
}

/// One step of the sequence actually executed (RUN-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunbookStep {
    /// The step's text, with placeholders where values were.
    pub text: String,
    /// The step's url, or `null`.
    pub url: Option<String>,
    /// The names of the placeholders used in this step. Empty is allowed here, unlike in a
    /// spec step.
    pub values: Vec<String>,
    /// The step's warning, or `null`.
    pub warning: Option<String>,
    /// What happened on this step across the rounds (§4.5.1).
    pub annotations: Vec<Annotation>,
}

/// A runbook, version 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runbook {
    /// `1` in this version.
    pub runbook_version: i64,
    /// `rb_` plus ten characters; also part of the file name (DD-17).
    pub id: String,
    /// Where the work happens; the matching rule normalises it (§4.5.3).
    #[serde(rename = "where")]
    pub r#where: String,
    /// What the run achieves; the matching rule tokenises it.
    pub goal: String,
    /// Why a person does it.
    pub why_human: String,
    /// The starting url, or `null`.
    pub url: Option<String>,
    /// The BCP-47 tag of the texts, or `null`.
    pub lang: Option<String>,
    /// Value names to their descriptions. Names only, never values.
    pub values: IndexMap<String, RunbookValue>,
    /// Variable name to destination file, as in the spec. Names only.
    pub secrets: IndexMap<String, String>,
    /// The sequence actually executed.
    pub steps: Vec<RunbookStep>,
    /// The verification text with placeholders, or `null`.
    pub verify: Option<String>,
    /// `verified` when an agent reported ok, `confirmed_by_user` when the spec had no
    /// `verify` (RUN-01).
    pub trust: RunbookTrust,
    /// When it was last verified, RFC 3339 (RUN-08).
    pub last_verified_at: String,
    /// When a run from it last failed, or `null` (RUN-09).
    pub last_run_failed_at: Option<String>,
    /// Verified or confirmed executions folded into this file; at least one.
    pub runs: u32,
    /// When the file was created, RFC 3339.
    pub created_at: String,
    /// When it was last written, RFC 3339.
    pub updated_at: String,
    /// Which application wrote it.
    pub origin: RunbookOrigin,
}
