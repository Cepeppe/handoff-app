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
//!
//! The table holds the two things `requests::queue` puts in front of an agent: a request
//! for a spec, and a request to come back to a handoff the user resumed from the overlay
//! (FM-31). `about_handoff_id` is what tells them apart; `migrations/0002` says why it has
//! to be a column and not a convention.

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
    /// The handoff that answered it: the one that adopted its id or was linked to it
    /// (OPEN-08), or — for a resume — the one whose call finally attached (FM-31). A
    /// request with one set is closed, whichever kind it is, and that is what every "still
    /// open" query below reads.
    pub linked_handoff_id: Option<String>,
    /// The handoff this asks an agent to come back to, when that is what it asks (FM-31,
    /// RESP-07). `None` is an ordinary request for a spec, and only those are adopted or
    /// linked by a new handoff (OPEN-08).
    pub about_handoff_id: Option<String>,
}

/// The columns of a request, in the order [`read_row`] reads them.
///
/// One place rather than six: a column added to the table and forgotten in one of the
/// queries would be read as another column's value, which `read_row` cannot detect.
const READ: &str = "SELECT id, session_ref, text, created_at, delivered_via, \
                    linked_handoff_id, about_handoff_id FROM user_requests";

/// Writes a request, creating it or replacing it.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the write fails.
pub fn upsert(db: &Db, row: &UserRequestRow) -> Result<()> {
    db.conn()
        .execute(
            "INSERT INTO user_requests (\
                 id, session_ref, text, created_at, delivered_via, linked_handoff_id, \
                 about_handoff_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT (id) DO UPDATE SET \
                 session_ref = excluded.session_ref, \
                 text = excluded.text, \
                 created_at = excluded.created_at, \
                 delivered_via = excluded.delivered_via, \
                 linked_handoff_id = excluded.linked_handoff_id, \
                 about_handoff_id = excluded.about_handoff_id",
            params![
                row.id,
                row.session_ref,
                row.text,
                row.created_at,
                row.delivered_via,
                row.linked_handoff_id,
                row.about_handoff_id,
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
        .query_row(&format!("{READ} WHERE id = ?1"), [id], read_row)
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
    let (sql, params): (String, Vec<&dyn ToSql>) = match &session {
        Some(session_ref) => (
            format!(
                "{READ} WHERE linked_handoff_id IS NULL \
                 AND (session_ref = ?1 OR session_ref IS NULL) ORDER BY created_at, id"
            ),
            vec![session_ref],
        ),
        None => (
            format!("{READ} WHERE linked_handoff_id IS NULL ORDER BY created_at, id"),
            Vec::new(),
        ),
    };
    query(db, &sql, params.as_slice())
}

/// The oldest request of `session` that is still waiting for a spec (OPEN-08).
///
/// Only ordinary requests: a resume asks an agent to come back to a handoff that already
/// exists, so linking a brand-new one to it would answer it with the wrong work.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the row cannot be read.
pub fn oldest_open_for_session(db: &Db, session_ref: &str) -> Result<Option<UserRequestRow>> {
    let sql = format!(
        "{READ} WHERE linked_handoff_id IS NULL AND about_handoff_id IS NULL \
         AND session_ref = ?1 ORDER BY created_at, id LIMIT 1"
    );
    db.conn()
        .query_row(&sql, [session_ref], read_row)
        .optional()
        .map_err(|error| StoreError::of("reading the user requests", error))
}

/// The open requests no session has been given yet, oldest first (OPEN-04a).
///
/// # Errors
///
/// [`StoreError::Persistence`] when the rows cannot be read.
pub fn list_unassigned_open(db: &Db) -> Result<Vec<UserRequestRow>> {
    let sql = format!(
        "{READ} WHERE linked_handoff_id IS NULL AND session_ref IS NULL \
         ORDER BY created_at, id"
    );
    query(db, &sql, &[])
}

/// Gives an open request to a session (OPEN-04a, OPEN-06).
///
/// # Errors
///
/// [`StoreError::Persistence`] when the update fails.
pub fn assign(db: &Db, id: &str, session_ref: &str) -> Result<bool> {
    let changed = db
        .conn()
        .execute(
            "UPDATE user_requests SET session_ref = ?2 \
             WHERE id = ?1 AND linked_handoff_id IS NULL",
            params![id, session_ref],
        )
        .map_err(|error| StoreError::of("assigning a user request", error))?;
    Ok(changed > 0)
}

/// Puts a detached session's still-open requests back on the unassigned queue (FM-34).
///
/// Returns how many were re-queued. What has already been answered is history and stays
/// where it is: only a request nobody produced a spec for can still reach another session.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the update fails.
pub fn unassign_open_of(db: &Db, session_ref: &str) -> Result<usize> {
    db.conn()
        .execute(
            "UPDATE user_requests SET session_ref = NULL \
             WHERE session_ref = ?1 AND linked_handoff_id IS NULL",
            [session_ref],
        )
        .map_err(|error| StoreError::of("re-queueing the user requests of a session", error))
}

/// Puts every still-open request back on the unassigned queue, whichever session it named.
///
/// For the one moment when that is true of all of them at once: the app has just started
/// and no session is connected, so every `session_ref` in the table belongs to a previous
/// run (FM-34, `requests::queue`).
///
/// # Errors
///
/// [`StoreError::Persistence`] when the update fails.
pub fn unassign_all_open(db: &Db) -> Result<usize> {
    db.conn()
        .execute(
            "UPDATE user_requests SET session_ref = NULL \
             WHERE session_ref IS NOT NULL AND linked_handoff_id IS NULL",
            [],
        )
        .map_err(|error| StoreError::of("re-queueing the user requests", error))
}

/// Opens again every request this handoff was answering (FM-20).
///
/// The one-click correction of §12.4 moves a handoff from one request to another, and the
/// request it leaves has to become collectable again — otherwise a mis-link would consume
/// it for good and the user would have no way to get it back.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the update fails.
pub fn unlink_handoff(db: &Db, handoff_id: &str) -> Result<usize> {
    db.conn()
        .execute(
            "UPDATE user_requests SET linked_handoff_id = NULL WHERE linked_handoff_id = ?1",
            [handoff_id],
        )
        .map_err(|error| StoreError::of("unlinking a user request", error))
}

/// Closes the resume requests about `handoff_id`: an agent has come back to it (FM-31).
///
/// Returns how many were closed. `linked_handoff_id` takes the same handoff the request was
/// about, so a resume ends its life exactly as an ordinary request does — answered by a
/// handoff — and no query has to know which kind it was.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the update fails.
pub fn close_resumes_about(db: &Db, handoff_id: &str) -> Result<usize> {
    db.conn()
        .execute(
            "UPDATE user_requests SET linked_handoff_id = about_handoff_id \
             WHERE about_handoff_id = ?1 AND linked_handoff_id IS NULL",
            [handoff_id],
        )
        .map_err(|error| StoreError::of("closing a resume request", error))
}

fn query(db: &Db, sql: &str, params: &[&dyn ToSql]) -> Result<Vec<UserRequestRow>> {
    let mut statement = db
        .conn()
        .prepare(sql)
        .map_err(|error| StoreError::of("reading the user requests", error))?;
    let rows = statement
        .query_map(params, read_row)
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
        about_handoff_id: row.get(6)?,
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
