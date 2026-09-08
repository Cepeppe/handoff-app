//! The `events` table: the diary of a handoff (§7.11, LOG-02).
//!
//! Append-only. "Steps are the recipe; the log is the diary" (§4.5.1): the runbook writer
//! reads the rounds for what should happen again, and these rows for what actually did.
//!
//! The thirteen kinds are the ones §7.11 lists, and the schema's CHECK holds them: a kind
//! nobody in the design named cannot be written by a typo.

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{params, Row, ToSql};
use serde::{Deserialize, Serialize};

use super::db::Db;
use super::error::{PersistenceCause, Result, StoreError};
use super::time::Timestamp;

/// What happened, from the vocabulary of §7.11.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// The user marked a step done (RESP-01).
    Confirm,
    /// The user wrote a note (RESP-03).
    Note,
    /// The user skipped a step (RESP-02).
    Skip,
    /// The user asked the agent a question (RESP-04).
    Ask,
    /// The user sent a screenshot (CAP-01).
    Screenshot,
    /// The user deferred the handoff (RESP-05).
    Defer,
    /// The user abandoned it (RESP-07).
    Abandon,
    /// The agent answered a question (RESP-04).
    Reply,
    /// The agent sent replacement steps, opening a new round (VER-09).
    Replace,
    /// An agent resumed the handoff (TOOL-07).
    Resume,
    /// A blocking call attached to it (TOOL-03).
    Attach,
    /// The attached call detached (TOOL-08, SRV-22).
    Detach,
    /// The state machine moved (§8.1).
    State,
}

impl EventKind {
    /// The column name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirm => "confirm",
            Self::Note => "note",
            Self::Skip => "skip",
            Self::Ask => "ask",
            Self::Screenshot => "screenshot",
            Self::Defer => "defer",
            Self::Abandon => "abandon",
            Self::Reply => "reply",
            Self::Replace => "replace",
            Self::Resume => "resume",
            Self::Attach => "attach",
            Self::Detach => "detach",
            Self::State => "state",
        }
    }

    /// The kind named by `text`, if it is one.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        ALL_KINDS.iter().copied().find(|kind| kind.as_str() == text)
    }
}

/// The thirteen kinds, in the order §7.11 prints them.
pub const ALL_KINDS: [EventKind; 13] = [
    EventKind::Confirm,
    EventKind::Note,
    EventKind::Skip,
    EventKind::Ask,
    EventKind::Screenshot,
    EventKind::Defer,
    EventKind::Abandon,
    EventKind::Reply,
    EventKind::Replace,
    EventKind::Resume,
    EventKind::Attach,
    EventKind::Detach,
    EventKind::State,
];

impl ToSql for EventKind {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for EventKind {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        Self::from_name(text).ok_or_else(|| {
            FromSqlError::Other(Box::new(StoreError::of(
                "reading an event",
                PersistenceCause::Schema(format!("{text} is not an event kind of the design")),
            )))
        })
    }
}

/// One row of `events`. `id` is assigned by the database on append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    /// Assigned on append; zero on the value handed to [`append`].
    pub id: i64,
    /// The handoff this happened to.
    pub handoff_id: String,
    /// The round it happened in, when it belongs to one.
    pub round: Option<i64>,
    /// When.
    pub at: Timestamp,
    /// What.
    pub kind: EventKind,
    /// 1-based index into the round's steps, for the kinds that happen on a step.
    pub step_index: Option<i64>,
    /// Whatever the kind carries, as the store serialises it.
    pub payload_json: Option<String>,
}

/// Appends an event and returns the id the database gave it.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the write fails; an event of a handoff that does not
/// exist is refused by the foreign key.
pub fn append(db: &Db, row: &EventRow) -> Result<i64> {
    db.conn()
        .execute(
            "INSERT INTO events (handoff_id, round, at, kind, step_index, payload_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                row.handoff_id,
                row.round,
                row.at,
                row.kind,
                row.step_index,
                row.payload_json,
            ],
        )
        .map_err(|error| StoreError::of("appending an event", error))?;
    Ok(db.conn().last_insert_rowid())
}

/// Every event of a handoff, oldest first.
///
/// Ordered by `id` and not by `at`: two events of the same millisecond are ordered by the
/// order they were appended in, which is the order they happened in.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the rows cannot be read.
pub fn list_for_handoff(db: &Db, handoff_id: &str) -> Result<Vec<EventRow>> {
    let mut statement = db
        .conn()
        .prepare(
            "SELECT id, handoff_id, round, at, kind, step_index, payload_json \
             FROM events WHERE handoff_id = ?1 ORDER BY id",
        )
        .map_err(|error| StoreError::of("reading the events", error))?;
    let rows = statement
        .query_map([handoff_id], read_row)
        .map_err(|error| StoreError::of("reading the events", error))?
        .collect::<rusqlite::Result<Vec<EventRow>>>()
        .map_err(|error| StoreError::of("reading the events", error))?;
    Ok(rows)
}

fn read_row(row: &Row<'_>) -> rusqlite::Result<EventRow> {
    Ok(EventRow {
        id: row.get(0)?,
        handoff_id: row.get(1)?,
        round: row.get(2)?,
        at: row.get(3)?,
        kind: row.get(4)?,
        step_index: row.get(5)?,
        payload_json: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::handoffs;
    use crate::log::testing::{event, handoff};

    #[test]
    fn every_kind_of_the_design_round_trips_through_the_column() {
        let db = Db::open_in_memory().expect("a database");
        handoffs::upsert(&db, &handoff("hf_0123456789")).expect("a handoff");
        for kind in ALL_KINDS {
            let mut row = event("hf_0123456789", kind);
            row.step_index = Some(2);
            let id = append(&db, &row).expect("an event");
            assert!(id > 0);
        }
        let events = list_for_handoff(&db, "hf_0123456789").expect("the events");
        assert_eq!(
            events.iter().map(|row| row.kind).collect::<Vec<_>>(),
            ALL_KINDS.to_vec()
        );
        // Appended order is read order, whatever the instants say.
        assert!(events.windows(2).all(|pair| pair[0].id < pair[1].id));
    }

    #[test]
    fn a_kind_the_design_does_not_name_is_refused_by_the_database() {
        let db = Db::open_in_memory().expect("a database");
        handoffs::upsert(&db, &handoff("hf_0123456789")).expect("a handoff");
        let error = db
            .conn()
            .execute(
                "INSERT INTO events (handoff_id, at, kind) \
                 VALUES ('hf_0123456789', '2026-09-08T11:00:00.000Z', 'shrug')",
                [],
            )
            .expect_err("the CHECK refuses it");
        assert!(error.to_string().contains("CHECK"), "{error}");
    }

    #[test]
    fn an_event_of_a_handoff_that_does_not_exist_is_refused() {
        let db = Db::open_in_memory().expect("a database");
        let error =
            append(&db, &event("hf_nosuchthing", EventKind::Note)).expect_err("no such handoff");
        assert!(error.to_string().contains("appending an event"), "{error}");
    }
}
