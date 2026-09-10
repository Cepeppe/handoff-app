//! The security suite of §11.7 (T-053): what Baton promises never to do, asked as tests.
//!
//! `cargo test --test security -- --nocapture` runs it, prints the numbers §11.7 asks for,
//! and writes `target/security-report.json` (`report.rs`). `docs/dev/testing.md` says what
//! each check proves, and which parts of §11.7 are checked elsewhere or by hand.
//!
//! - `token` — a refused token is answered, logged, and leaves no token material in the log.
//! - `endpoint` — the pipe and the token file carry a DACL for this user alone, read back
//!   from the objects (Windows); the socket, its folder and the token are owner-only (POSIX).
//! - `redaction` — the corpus gates (certain precision and recall, suspected recall), the
//!   burn geometry, and the glyph-leak test.
//! - `log_invariants` — no value the certain detector matched reaches the database.
//! - `crash_files` — a crash file written after real traffic carries identifiers only, and
//!   the tracing output carries no value either (R-19).
//!
//! Every check that could pass by measuring nothing carries a control that must be seen
//! working first: the refusal line has to be in the output that is searched, a pipe with the
//! default DACL has to fail the DACL check, a planted value has to reach the tracing output
//! and not the crash file, and an engine has to read a planted key before its redacted band
//! is asked about.

#[path = "../corpus/mod.rs"]
mod corpus;
// The double `tests/integration_flows.rs` drives the app with. This suite only registers a
// session, sends a `hello` and opens a handoff with it; the replay of the goldens is that
// file's, hence the allowance.
#[allow(dead_code)]
#[path = "../fake-server/mod.rs"]
mod fake_server;

mod child;
mod crash_files;
mod endpoint;
mod forbidden;
mod log_invariants;
mod redaction;
mod report;
mod support;
mod token;
