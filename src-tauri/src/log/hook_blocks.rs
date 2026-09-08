//! The `hook_blocks` table: the once-per-item-per-session counter of SRV-12.
//!
//! The Stop hook blocks the agent at most once per handoff per session (SRV-12, §7.5). The
//! rule is the primary key `(session_ref, item_key)`: a second block for the same item in
//! the same session cannot be recorded, so it cannot be issued. The alternative — a counter
//! column and a comparison — would put the rule in a query instead of in the schema, and it
//! is the rule that keeps a badly behaved hook from holding the agent for ever (NFR-11).
//!
//! `item_key` is composed by the hook decision (T-035), which is the only place that knows
//! what an item is; the log stores it and enforces uniqueness over it.
//!
//! Rows go with the session: a purged session (§8.3) has no agent left to block, so the
//! schema cascades them away.

use rusqlite::{params, OptionalExtension};

use super::db::Db;
use super::error::{Result, StoreError};
use super::time::Timestamp;

/// Whether this session was already blocked for this item.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the row cannot be read.
pub fn blocked_before(db: &Db, session_ref: &str, item_key: &str) -> Result<bool> {
    db.conn()
        .query_row(
            "SELECT 1 FROM hook_blocks WHERE session_ref = ?1 AND item_key = ?2",
            params![session_ref, item_key],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|found| found.is_some())
        .map_err(|error| StoreError::of("reading a hook block", error))
}

/// Records a block. Returns `false` when this session had already been blocked for this
/// item, in which case nothing is written and the caller must not block again (SRV-12).
///
/// # Errors
///
/// [`StoreError::Persistence`] when the write fails.
pub fn record(db: &Db, session_ref: &str, item_key: &str, at: &Timestamp) -> Result<bool> {
    let written = db
        .conn()
        .execute(
            "INSERT INTO hook_blocks (session_ref, item_key, at) VALUES (?1, ?2, ?3) \
             ON CONFLICT (session_ref, item_key) DO NOTHING",
            params![session_ref, item_key, at],
        )
        .map_err(|error| StoreError::of("recording a hook block", error))?;
    Ok(written > 0)
}

/// Every item this session has already been blocked for, in the order they were recorded.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the rows cannot be read.
pub fn list_for_session(db: &Db, session_ref: &str) -> Result<Vec<String>> {
    let mut statement = db
        .conn()
        .prepare("SELECT item_key FROM hook_blocks WHERE session_ref = ?1 ORDER BY at, item_key")
        .map_err(|error| StoreError::of("reading the hook blocks", error))?;
    let rows = statement
        .query_map([session_ref], |row| row.get::<_, String>(0))
        .map_err(|error| StoreError::of("reading the hook blocks", error))?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|error| StoreError::of("reading the hook blocks", error))?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::sessions;
    use crate::log::testing::session;

    fn at(text: &str) -> Timestamp {
        Timestamp::parse(text).expect("rfc 3339")
    }

    #[test]
    fn a_session_is_blocked_once_per_item_and_never_twice() {
        let db = Db::open_in_memory().expect("a database");
        sessions::register(&db, &session("ses_00000001")).expect("a session");

        assert!(!blocked_before(&db, "ses_00000001", "deferred:hf_0123456789").expect("a read"));
        assert!(record(
            &db,
            "ses_00000001",
            "deferred:hf_0123456789",
            &at("2026-09-08T11:00:00Z")
        )
        .expect("a block"));
        assert!(blocked_before(&db, "ses_00000001", "deferred:hf_0123456789").expect("a read"));
        // The second attempt writes nothing and says so: SRV-12 is the primary key.
        assert!(!record(
            &db,
            "ses_00000001",
            "deferred:hf_0123456789",
            &at("2026-09-08T12:00:00Z")
        )
        .expect("no second block"));

        // A different item of the same session is a different block.
        assert!(record(
            &db,
            "ses_00000001",
            "unreported:hf_0123456789",
            &at("2026-09-08T12:00:00Z")
        )
        .expect("a block"));
        assert_eq!(
            list_for_session(&db, "ses_00000001").expect("the blocks"),
            ["deferred:hf_0123456789", "unreported:hf_0123456789"]
        );
    }

    #[test]
    fn the_counter_of_a_session_goes_when_the_session_goes() {
        let db = Db::open_in_memory().expect("a database");
        sessions::register(&db, &session("ses_00000001")).expect("a session");
        record(
            &db,
            "ses_00000001",
            "deferred:hf_0123456789",
            &at("2026-09-01T11:00:00Z"),
        )
        .expect("a block");
        sessions::disconnect(&db, "ses_00000001", &at("2026-09-01T12:00:00Z")).expect("a bye");
        assert_eq!(
            sessions::purge(&db, &at("2026-09-20T12:00:00Z")).expect("a purge"),
            1
        );
        assert!(list_for_session(&db, "ses_00000001")
            .expect("the blocks")
            .is_empty());
    }
}
