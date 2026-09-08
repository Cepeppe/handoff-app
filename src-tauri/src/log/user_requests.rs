//! The `user_requests` table: what the user opened with the shortcut (§7.7, OPEN-03..08).
//!
//! A request is minted with a `hf_` id and keeps it when the spec arrives, so one tab lives
//! from "waiting for spec" to a final state (DD-13). Until then the two rows exist side by
//! side: the request records what the user asked for and how it was delivered, the handoff
//! records what the agent made of it.
//!
//! `session_ref` here has no foreign key on purpose. A request may be queued with no
//! session at all (OPEN-04a), and once delivered it is a note about *where it went* — the
//! session purge of §8.3 must not be able to rewrite that history.

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{params, OptionalExtension, Row, ToSql};
use serde::{Deserialize, Serialize};

use super::db::Db;
use super::error::{PersistenceCause, Result, StoreError};
use super::time::Timestamp;

/// Which of the two delivery paths reached the agent (OPEN-05, OPEN-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveredVia {
    /// The fast path: the request text was put on the clipboard (OPEN-05).
    Clipboard,
    /// The safety net: the Stop hook delivered it at the end of a turn (OPEN-06).
    StopHook,
}

impl DeliveredVia {
    /// The column name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clipboard => "clipboard",
            Self::StopHook => "stop_hook",
        }
    }

    /// The path named by `text`, if it is one.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        [Self::Clipboard, Self::StopHook]
            .into_iter()
            .find(|via| via.as_str() == text)
    }
}

impl ToSql for DeliveredVia {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for DeliveredVia {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        Self::from_name(text).ok_or_else(|| {
            FromSqlError::Other(Box::new(StoreError::of(
                "reading a user request",
                PersistenceCause::Schema(format!("{text} is not a delivery path of the design")),
            )))
        })
    }
}

/// One row of `user_requests`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRequestRow {
    /// The `hf_` id the handoff will adopt (DD-13).
    pub id: String,
    /// The session it was addressed to, or none while it waits for one (OPEN-04a).
    pub session_ref: Option<String>,
    /// What the user wrote in the request window.
    pub text: String,
    /// When they wrote it.
    pub created_at: Timestamp,
    /// How it reached the agent, once it did.
    pub delivered_via: Option<DeliveredVia>,
    /// The handoff that answered it (OPEN-08).
    pub linked_handoff_id: Option<String>,
}

/// Writes a request, creating it or replacing it.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the write fails.
pub fn upsert(db: &Db, row: &UserRequestRow) -> Result<()> {
    db.conn()
        .execute(
            "INSERT INTO user_requests (\
                 id, session_ref, text, created_at, delivered_via, linked_handoff_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT (id) DO UPDATE SET \
                 session_ref = excluded.session_ref, \
                 text = excluded.text, \
                 created_at = excluded.created_at, \
                 delivered_via = excluded.delivered_via, \
                 linked_handoff_id = excluded.linked_handoff_id",
            params![
                row.id,
                row.session_ref,
                row.text,
                row.created_at,
                row.delivered_via,
                row.linked_handoff_id,
            ],
        )
        .map_err(|error| StoreError::of("writing a user request", error))?;
    Ok(())
}

/// Records that a request reached an agent, and by which path.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the update fails.
pub fn mark_delivered(db: &Db, id: &str, via: DeliveredVia) -> Result<bool> {
    let changed = db
        .conn()
        .execute(
            "UPDATE user_requests SET delivered_via = ?2 WHERE id = ?1",
            params![id, via],
        )
        .map_err(|error| StoreError::of("marking a user request delivered", error))?;
    Ok(changed > 0)
}

/// Ties a request to the handoff that answered it (OPEN-08).
///
/// # Errors
///
/// [`StoreError::Persistence`] when the update fails.
pub fn link(db: &Db, id: &str, handoff_id: &str) -> Result<bool> {
    let changed = db
        .conn()
        .execute(
            "UPDATE user_requests SET linked_handoff_id = ?2 WHERE id = ?1",
            params![id, handoff_id],
        )
        .map_err(|error| StoreError::of("linking a user request", error))?;
    Ok(changed > 0)
}

/// One request, if it is there.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the row cannot be read.
pub fn get(db: &Db, id: &str) -> Result<Option<UserRequestRow>> {
    db.conn()
        .query_row(
            "SELECT id, session_ref, text, created_at, delivered_via, linked_handoff_id \
             FROM user_requests WHERE id = ?1",
            [id],
            read_row,
        )
        .optional()
        .map_err(|error| StoreError::of("reading a user request", error))
}

/// The requests still waiting for a handoff, oldest first: the queue of §7.5 and §7.7.
///
/// `session` selects a session's own requests **and** the unassigned ones, which is exactly
/// the set the hook decision of §7.5 offers; `None` returns the whole open queue.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the rows cannot be read.
pub fn list_open(db: &Db, session: Option<&str>) -> Result<Vec<UserRequestRow>> {
    let (sql, params): (&str, Vec<&dyn ToSql>) = match &session {
        Some(session_ref) => (
            "SELECT id, session_ref, text, created_at, delivered_via, linked_handoff_id \
             FROM user_requests \
             WHERE linked_handoff_id IS NULL AND (session_ref = ?1 OR session_ref IS NULL) \
             ORDER BY created_at, id",
            vec![session_ref],
        ),
        None => (
            "SELECT id, session_ref, text, created_at, delivered_via, linked_handoff_id \
             FROM user_requests WHERE linked_handoff_id IS NULL ORDER BY created_at, id",
            Vec::new(),
        ),
    };
    let mut statement = db
        .conn()
        .prepare(sql)
        .map_err(|error| StoreError::of("reading the user requests", error))?;
    let rows = statement
        .query_map(params.as_slice(), read_row)
        .map_err(|error| StoreError::of("reading the user requests", error))?
        .collect::<rusqlite::Result<Vec<UserRequestRow>>>()
        .map_err(|error| StoreError::of("reading the user requests", error))?;
    Ok(rows)
}

fn read_row(row: &Row<'_>) -> rusqlite::Result<UserRequestRow> {
    Ok(UserRequestRow {
        id: row.get(0)?,
        session_ref: row.get(1)?,
        text: row.get(2)?,
        created_at: row.get(3)?,
        delivered_via: row.get(4)?,
        linked_handoff_id: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::testing::{handoff, request, session};
    use crate::log::{handoffs, sessions};

    #[test]
    fn a_request_is_queued_delivered_and_then_linked() {
        let db = Db::open_in_memory().expect("a database");
        sessions::register(&db, &session("ses_00000001")).expect("a session");
        let mut row = request("hf_0123456789");
        row.session_ref = Some("ses_00000001".to_owned());
        upsert(&db, &row).expect("a request");

        assert!(mark_delivered(&db, "hf_0123456789", DeliveredVia::StopHook).expect("delivered"));
        handoffs::upsert(&db, &handoff("hf_0123456789")).expect("the handoff it became");
        assert!(link(&db, "hf_0123456789", "hf_0123456789").expect("linked"));

        let read = get(&db, "hf_0123456789").expect("a read").expect("a row");
        assert_eq!(read.delivered_via, Some(DeliveredVia::StopHook));
        assert_eq!(read.linked_handoff_id.as_deref(), Some("hf_0123456789"));
        assert!(list_open(&db, None).expect("the queue").is_empty());
    }

    #[test]
    fn the_open_queue_of_a_session_holds_its_own_and_the_unassigned() {
        let db = Db::open_in_memory().expect("a database");
        sessions::register(&db, &session("ses_00000001")).expect("a session");
        sessions::register(&db, &session("ses_00000002")).expect("another session");

        let mut mine = request("hf_0000000001");
        mine.session_ref = Some("ses_00000001".to_owned());
        upsert(&db, &mine).expect("a request");

        let unassigned = request("hf_0000000002");
        upsert(&db, &unassigned).expect("a queued request");

        let mut someone_elses = request("hf_0000000003");
        someone_elses.session_ref = Some("ses_00000002".to_owned());
        upsert(&db, &someone_elses).expect("another session's request");

        let open = list_open(&db, Some("ses_00000001")).expect("the queue");
        assert_eq!(
            open.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["hf_0000000001", "hf_0000000002"]
        );
        assert_eq!(list_open(&db, None).expect("the whole queue").len(), 3);
    }

    #[test]
    fn a_request_outlives_the_session_it_names() {
        // No foreign key on `session_ref`: the purge of §8.3 forgets the session, and the
        // record of where the request went stays.
        let db = Db::open_in_memory().expect("a database");
        let mut row = request("hf_0123456789");
        row.session_ref = Some("ses_purgedaw".to_owned());
        upsert(&db, &row).expect("a request naming a session that is not there");
        assert_eq!(
            get(&db, "hf_0123456789")
                .expect("a read")
                .expect("a row")
                .session_ref
                .as_deref(),
            Some("ses_purgedaw")
        );
    }

    #[test]
    fn a_delivery_path_the_design_does_not_name_is_refused_by_the_database() {
        let db = Db::open_in_memory().expect("a database");
        let error = db
            .conn()
            .execute(
                "INSERT INTO user_requests (id, text, created_at, delivered_via) \
                 VALUES ('hf_0123456789', 'x', '2026-09-08T11:00:00.000Z', 'carrier pigeon')",
                [],
            )
            .expect_err("the CHECK refuses it");
        assert!(error.to_string().contains("CHECK"), "{error}");
    }
}
