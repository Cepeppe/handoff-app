//! Contract tests: the vendored fixtures against the Rust implementations (§11.2).
//!
//! "One fixture set, two implementations." Everything this suite reads comes from
//! `vendor/handoff-mcp/format/`, the unpacked format tarball of the pinned `handoff-mcp`
//! release — the same files the server's own `test/contract/` suite runs. Neither side is
//! checked against the other's source; both are checked against the artifact they ship
//! with, which is what makes a divergence a failing test instead of a bug report from a
//! user (§3.4, §3.5).
//!
//! Run it with `cargo test`, after `node scripts/fetch-server.mjs` has filled `vendor/`.

mod channel;
mod ids;
mod matching;
mod outcomes;
mod patterns;
mod runbooks;
mod secrets;
mod specs;
mod support;
