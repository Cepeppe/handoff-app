//! The local log (§7.11, LOG-01..05, NET-01).
//!
//! SQLite schema, migrations, queries, export and deletion, in the app data directory. It
//! records what was sent, when and to which session, with hashes rather than pixels, and
//! it is the only place a past handoff can be read from. Nothing here ever leaves the
//! machine.
// TASK: T-030 (schema, migrations, repositories), T-045 (Log page and its queries)
