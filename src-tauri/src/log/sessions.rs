//! The `sessions` table: one row per registration (§7.11, §7.5, SRV-17..20, §8.3).
//!
//! The live registry is in memory (§2.3); this is its history and the place a hook's
//! ancestor chain is matched against after a restart. A reconnection is a **new** session
//! with a new `session_ref`: §8.3 is explicit that the old row is kept.
//!
//! # The purge, and why it is guarded twice
//!
//! §8.3 ends a disconnected session "after 7 days without handoffs". [`purge`] deletes only
//! rows that are disconnected, older than that, and referenced by no handoff — and the
//! schema's `ON DELETE RESTRICT` from `handoffs.session_ref` would refuse the deletion even
//! if the query forgot to. The belt is the query, the braces are the database, and the
//! reason for both is that this is the one routine here that deletes something the user
//! never asked to delete.

use rusqlite::{params, OptionalExtension, Row};

use super::db::Db;
use super::error::{Result, StoreError};
use super::time::Timestamp;

/// A disconnected session with no handoffs is forgotten after this long (§8.3).
pub const SESSION_HISTORY_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// One row of `sessions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    /// `ses_` + 8 characters (§4.1).
    pub session_ref: String,
    /// The agent identity resolved for it (§5.6).
    pub agent_id: Option<String>,
    /// `clientInfo.name` from the MCP handshake.
    pub client_name: Option<String>,
    /// `clientInfo.version`.
    pub client_version: Option<String>,
    /// The completed ancestor chain (SRV-17, DD-22), as a JSON array of PIDs.
    pub pid_chain_json: String,
    /// The working directory the server reported.
    pub cwd: Option<String>,
    /// The project folder it was tied to.
    pub project_dir: Option<String>,
    /// Bound when a hook's chain intersects this session's PIDs (§7.5).
    pub claude_session_id: Option<String>,
    /// Whether the connection is live (§7.5, §8.3).
    pub connected: bool,
    /// When it registered.
    pub first_seen: Timestamp,
    /// The last sign of life: a message, a ping, or the disconnection itself.
    pub last_seen: Timestamp,
}

/// Registers a session, or refreshes one that reconnected under the same `session_ref`.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the write fails.
pub fn register(db: &Db, row: &SessionRow) -> Result<()> {
    db.conn()
        .execute(
            "INSERT INTO sessions (\
                 session_ref, agent_id, client_name, client_version, pid_chain_json, cwd, \
                 project_dir, claude_session_id, connected, first_seen, last_seen) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             ON CONFLICT (session_ref) DO UPDATE SET \
                 agent_id = excluded.agent_id, \
                 client_name = excluded.client_name, \
                 client_version = excluded.client_version, \
                 pid_chain_json = excluded.pid_chain_json, \
                 cwd = excluded.cwd, \
                 project_dir = excluded.project_dir, \
                 claude_session_id = excluded.claude_session_id, \
                 connected = excluded.connected, \
                 last_seen = excluded.last_seen",
            params![
                row.session_ref,
                row.agent_id,
                row.client_name,
                row.client_version,
                row.pid_chain_json,
                row.cwd,
                row.project_dir,
                row.claude_session_id,
                row.connected,
                row.first_seen,
                row.last_seen,
            ],
        )
        .map_err(|error| StoreError::of("registering a session", error))?;
    Ok(())
}

/// Records a sign of life. Returns whether the session was there.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the update fails.
pub fn touch(db: &Db, session_ref: &str, at: &Timestamp) -> Result<bool> {
    let changed = db
        .conn()
        .execute(
            "UPDATE sessions SET last_seen = ?2 WHERE session_ref = ?1",
            params![session_ref, at],
        )
        .map_err(|error| StoreError::of("touching a session", error))?;
    Ok(changed > 0)
}

/// Marks the connection gone: EOF, `session.bye`, or a ping that went unanswered (§8.3).
///
/// The handoffs of a disconnected session stay in their own state and any session may
/// resume them (§8.3, TOOL-08); this row only stops being a live registration.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the update fails.
pub fn disconnect(db: &Db, session_ref: &str, at: &Timestamp) -> Result<bool> {
    let changed = db
        .conn()
        .execute(
            "UPDATE sessions SET connected = 0, last_seen = ?2 WHERE session_ref = ?1",
            params![session_ref, at],
        )
        .map_err(|error| StoreError::of("disconnecting a session", error))?;
    Ok(changed > 0)
}

/// Marks every row still flagged connected as disconnected, without moving `last_seen`.
///
/// Called once, when the registry is opened (§8.3): no channel connection survives an app
/// restart, so a row left `connected = 1` by a process that was killed is a lie, and a lie
/// the purge believes — it only ever forgets a **disconnected** session. `last_seen` is
/// deliberately not touched: the row's last sign of life was whenever it last spoke, and
/// stamping it with the current start-up instant would restart the seven-day clock of §8.3
/// on every launch, so a history row would never be forgotten at all.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the update fails.
pub fn disconnect_all(db: &Db) -> Result<usize> {
    let changed = db
        .conn()
        .execute("UPDATE sessions SET connected = 0 WHERE connected = 1", [])
        .map_err(|error| StoreError::of("disconnecting the sessions of a previous run", error))?;
    Ok(changed)
}

/// Binds the agent's own session identifier to this registration (SRV-19, §7.5).
///
/// # Errors
///
/// [`StoreError::Persistence`] when the update fails.
pub fn bind_claude_session_id(db: &Db, session_ref: &str, session_id: &str) -> Result<bool> {
    let changed = db
        .conn()
        .execute(
            "UPDATE sessions SET claude_session_id = ?2 WHERE session_ref = ?1",
            params![session_ref, session_id],
        )
        .map_err(|error| StoreError::of("binding a session id", error))?;
    Ok(changed > 0)
}

/// One session, if it is there.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the row cannot be read.
pub fn get(db: &Db, session_ref: &str) -> Result<Option<SessionRow>> {
    db.conn()
        .query_row(
            "SELECT session_ref, agent_id, client_name, client_version, pid_chain_json, cwd, \
                    project_dir, claude_session_id, connected, first_seen, last_seen \
             FROM sessions WHERE session_ref = ?1",
            [session_ref],
            read_row,
        )
        .optional()
        .map_err(|error| StoreError::of("reading a session", error))
}

/// Every session, oldest registration first.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the rows cannot be read.
pub fn list(db: &Db) -> Result<Vec<SessionRow>> {
    let mut statement = db
        .conn()
        .prepare(
            "SELECT session_ref, agent_id, client_name, client_version, pid_chain_json, cwd, \
                    project_dir, claude_session_id, connected, first_seen, last_seen \
             FROM sessions ORDER BY first_seen, session_ref",
        )
        .map_err(|error| StoreError::of("reading the sessions", error))?;
    let rows = statement
        .query_map([], read_row)
        .map_err(|error| StoreError::of("reading the sessions", error))?
        .collect::<rusqlite::Result<Vec<SessionRow>>>()
        .map_err(|error| StoreError::of("reading the sessions", error))?;
    Ok(rows)
}

/// Forgets disconnected sessions older than [`SESSION_HISTORY_MS`] that no handoff cites
/// (§8.3). Returns how many were forgotten.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the deletion fails.
pub fn purge(db: &Db, now: &Timestamp) -> Result<usize> {
    let cutoff = now.minus_millis(SESSION_HISTORY_MS);
    let deleted = db
        .conn()
        .execute(
            "DELETE FROM sessions \
             WHERE connected = 0 AND last_seen <= ?1 \
               AND session_ref NOT IN (\
                   SELECT session_ref FROM handoffs WHERE session_ref IS NOT NULL)",
            [&cutoff],
        )
        .map_err(|error| StoreError::of("purging the sessions", error))?;
    Ok(deleted)
}

fn read_row(row: &Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        session_ref: row.get(0)?,
        agent_id: row.get(1)?,
        client_name: row.get(2)?,
        client_version: row.get(3)?,
        pid_chain_json: row.get(4)?,
        cwd: row.get(5)?,
        project_dir: row.get(6)?,
        claude_session_id: row.get(7)?,
        connected: row.get(8)?,
        first_seen: row.get(9)?,
        last_seen: row.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::handoffs;
    use crate::log::testing::{handoff, session};

    fn at(text: &str) -> Timestamp {
        Timestamp::parse(text).expect("rfc 3339")
    }

    #[test]
    fn a_session_registers_is_touched_bound_and_disconnected() {
        let db = Db::open_in_memory().expect("a database");
        register(&db, &session("ses_00000001")).expect("a session");
        assert!(touch(&db, "ses_00000001", &at("2026-09-08T12:00:00Z")).expect("a touch"));
        assert!(bind_claude_session_id(&db, "ses_00000001", "9f0f8e1c").expect("a binding"));
        assert!(disconnect(&db, "ses_00000001", &at("2026-09-08T13:00:00Z")).expect("a bye"));

        let row = get(&db, "ses_00000001").expect("a read").expect("a row");
        assert!(!row.connected);
        assert_eq!(row.last_seen, at("2026-09-08T13:00:00Z"));
        assert_eq!(row.claude_session_id.as_deref(), Some("9f0f8e1c"));
        assert_eq!(row.first_seen, session("ses_00000001").first_seen);

        assert!(!touch(&db, "ses_nosuch", &at("2026-09-08T13:00:00Z")).expect("no such row"));
        assert!(get(&db, "ses_nosuch").expect("a read").is_none());
        assert_eq!(list(&db).expect("the sessions").len(), 1);
    }

    #[test]
    fn a_restart_disconnects_what_the_previous_run_left_connected_without_moving_last_seen() {
        let db = Db::open_in_memory().expect("a database");
        register(&db, &session("ses_00000001")).expect("a live session");
        let mut already_gone = session("ses_00000002");
        already_gone.connected = false;
        already_gone.last_seen = at("2026-08-01T09:00:00Z");
        register(&db, &already_gone).expect("a history row");

        assert_eq!(disconnect_all(&db).expect("a restart"), 1);
        assert_eq!(disconnect_all(&db).expect("a second restart"), 0);

        for session_ref in ["ses_00000001", "ses_00000002"] {
            let row = get(&db, session_ref).expect("a read").expect("a row");
            assert!(!row.connected);
        }
        // The seven-day clock of §8.3 must not restart on every launch.
        assert_eq!(
            get(&db, "ses_00000001")
                .expect("a read")
                .expect("a row")
                .last_seen,
            session("ses_00000001").last_seen
        );
        assert_eq!(
            get(&db, "ses_00000002")
                .expect("a read")
                .expect("a row")
                .last_seen,
            at("2026-08-01T09:00:00Z")
        );
    }

    #[test]
    fn the_purge_forgets_only_the_old_the_disconnected_and_the_unreferenced() {
        let db = Db::open_in_memory().expect("a database");
        let now = at("2026-09-20T12:00:00Z");
        let old = now.minus_millis(SESSION_HISTORY_MS + 1);
        let recent = now.minus_millis(SESSION_HISTORY_MS - 1);

        let mut forgotten = session("ses_00000001");
        forgotten.connected = false;
        forgotten.last_seen = old.clone();
        register(&db, &forgotten).expect("an old session");

        let mut still_connected = session("ses_00000002");
        still_connected.last_seen = old.clone();
        register(&db, &still_connected).expect("a live session");

        let mut too_recent = session("ses_00000003");
        too_recent.connected = false;
        too_recent.last_seen = recent;
        register(&db, &too_recent).expect("a recent session");

        let mut with_a_handoff = session("ses_00000004");
        with_a_handoff.connected = false;
        with_a_handoff.last_seen = old;
        register(&db, &with_a_handoff).expect("a session with history");
        let mut row = handoff("hf_0123456789");
        row.session_ref = Some("ses_00000004".to_owned());
        handoffs::upsert(&db, &row).expect("a handoff");

        assert_eq!(purge(&db, &now).expect("a purge"), 1);
        let left: Vec<String> = list(&db)
            .expect("the sessions")
            .into_iter()
            .map(|row| row.session_ref)
            .collect();
        assert_eq!(left, ["ses_00000002", "ses_00000003", "ses_00000004"]);
    }

    #[test]
    fn the_database_refuses_to_forget_a_session_a_handoff_still_cites() {
        // The braces to the purge query's belt: RESTRICT is what makes "without handoffs"
        // a property of the file rather than of one WHERE clause.
        let db = Db::open_in_memory().expect("a database");
        register(&db, &session("ses_00000001")).expect("a session");
        let mut row = handoff("hf_0123456789");
        row.session_ref = Some("ses_00000001".to_owned());
        handoffs::upsert(&db, &row).expect("a handoff");
        let error = db
            .conn()
            .execute(
                "DELETE FROM sessions WHERE session_ref = 'ses_00000001'",
                [],
            )
            .expect_err("the foreign key refuses it");
        assert!(error.to_string().contains("FOREIGN KEY"), "{error}");
    }

    #[test]
    fn a_handoff_cannot_cite_a_session_that_never_registered() {
        let db = Db::open_in_memory().expect("a database");
        let mut row = handoff("hf_0123456789");
        row.session_ref = Some("ses_nosuchth".to_owned());
        let error = handoffs::upsert(&db, &row).expect_err("no such session");
        assert!(error.to_string().contains("writing a handoff"), "{error}");
    }
}
