//! The shared formats, in Rust (§4, §6, §11.2).
//!
//! "One fixture set, two implementations." The spec, the outcome, the runbook, the
//! certain-secret patterns and the internal channel protocol are defined once, in the open
//! `handoff-mcp` repository, and reach the app through the pinned release artifact (§3.4,
//! §3.5). This module is the app's implementation of them: the types the rest of the crate
//! passes around, and the validators that decide whether a document is one of them.
//!
//! Nothing here is a copy of a definition. The schemas and the pattern file are embedded
//! verbatim from `vendor/handoff-mcp/format/` at build time, and `tests/contract/` runs the
//! vendored fixtures — the same files the server's own suite runs — through these types, so
//! the two implementations are held to the same documents rather than to each other's
//! source.
//!
//! # What lives where
//!
//! - [`spec`], [`outcome`], [`runbook`] — the three public formats.
//! - [`channel`] — the internal protocol: the JSON-RPC envelope and every message.
//! - [`patterns`] — the vendored certain-secret pattern file, read by
//!   [`crate::redaction::certain`] and [`crate::runbooks::matching`].
//! - [`schema`] — the four compiled validators and the collapsing of their errors.
//! - [`paths`] — the display notation a problem cites (`steps[0].text`).

pub mod channel;
pub mod outcome;
pub mod paths;
pub mod patterns;
pub mod runbook;
pub mod schema;
pub mod spec;

pub use outcome::{Outcome, OutcomeStatus, SecretTreated};
pub use runbook::Runbook;
pub use schema::{validate, Document, Problem};
pub use spec::{HandoffSpec, HandoffStep, SpecValue};
