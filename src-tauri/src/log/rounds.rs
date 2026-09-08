//! The `rounds` table: one pass through a step list (§7.11, LOG-02, VER-10).
//!
//! A round is opened when the handoff starts and again whenever replacement steps arrive
//! (§7.4); the verification of §8.1 is recorded on the round it belongs to, including a
//! report that arrived after `not_verified` and was accepted as late (VER-10, DD-16).

use rusqlite::{params, Row};

use super::db::Db;
use super::error::{Result, StoreError};
use super::time::Timestamp;

/// One row of `rounds`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundRow {
    /// The handoff this round belongs to.
    pub handoff_id: String,
    /// 1-based; a correction opens round n+1 (VER-09).
    pub no: i64,
    /// The steps of this round, as the store serialises them.
    pub steps_json: String,
    /// When the round opened.
    pub started_at: Timestamp,
    /// When it closed; absent while it is the current one.
    pub ended_at: Option<Timestamp>,
    /// What the agent reported: `true`, `false`, or absent for no report at all.
    pub verify_ok: Option<bool>,
    /// What the agent said about it.
    pub verify_detail: Option<String>,
    /// When the report arrived.
    pub verify_reported_at: Option<Timestamp>,
    /// The report arrived after the handoff had already gone to `not_verified` (DD-16).
    pub verify_late: bool,
}

/// Writes a round, creating it or replacing it.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the write fails; a round of a handoff that does not
/// exist is refused by the foreign key.
pub fn upsert(db: &Db, row: &RoundRow) -> Result<()> {
    db.conn()
        .execute(
            "INSERT INTO rounds (\
                 handoff_id, no, steps_json, started_at, ended_at, verify_ok, verify_detail, \
                 verify_reported_at, verify_late) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT (handoff_id, no) DO UPDATE SET \
                 steps_json = excluded.steps_json, \
                 started_at = excluded.started_at, \
                 ended_at = excluded.ended_at, \
                 verify_ok = excluded.verify_ok, \
                 verify_detail = excluded.verify_detail, \
                 verify_reported_at = excluded.verify_reported_at, \
                 verify_late = excluded.verify_late",
            params![
                row.handoff_id,
                row.no,
                row.steps_json,
                row.started_at,
                row.ended_at,
                row.verify_ok,
                row.verify_detail,
                row.verify_reported_at,
                row.verify_late,
            ],
        )
        .map_err(|error| StoreError::of("writing a round", error))?;
    Ok(())
}

/// Every round of a handoff, in order.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the rows cannot be read.
pub fn list_for_handoff(db: &Db, handoff_id: &str) -> Result<Vec<RoundRow>> {
    let mut statement = db
        .conn()
        .prepare(
            "SELECT handoff_id, no, steps_json, started_at, ended_at, verify_ok, \
                    verify_detail, verify_reported_at, verify_late \
             FROM rounds WHERE handoff_id = ?1 ORDER BY no",
        )
        .map_err(|error| StoreError::of("reading the rounds", error))?;
    let rows = statement
        .query_map([handoff_id], read_row)
        .map_err(|error| StoreError::of("reading the rounds", error))?
        .collect::<rusqlite::Result<Vec<RoundRow>>>()
        .map_err(|error| StoreError::of("reading the rounds", error))?;
    Ok(rows)
}

fn read_row(row: &Row<'_>) -> rusqlite::Result<RoundRow> {
    Ok(RoundRow {
        handoff_id: row.get(0)?,
        no: row.get(1)?,
        steps_json: row.get(2)?,
        started_at: row.get(3)?,
        ended_at: row.get(4)?,
        verify_ok: row.get(5)?,
        verify_detail: row.get(6)?,
        verify_reported_at: row.get(7)?,
        verify_late: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::handoffs;
    use crate::log::testing::{handoff, round};

    #[test]
    fn a_round_is_written_updated_and_read_back_in_order() {
        let db = Db::open_in_memory().expect("a database");
        handoffs::upsert(&db, &handoff("hf_0123456789")).expect("a handoff");
        upsert(&db, &round("hf_0123456789", 1)).expect("round 1");
        upsert(&db, &round("hf_0123456789", 2)).expect("round 2");

        let mut corrected = round("hf_0123456789", 1);
        corrected.ended_at = Some(Timestamp::parse("2026-09-08T12:00:00Z").expect("rfc 3339"));
        corrected.verify_ok = Some(false);
        corrected.verify_detail = Some("the webhook never fired".to_owned());
        corrected.verify_reported_at = corrected.ended_at.clone();
        corrected.verify_late = true;
        upsert(&db, &corrected).expect("the same round");

        let rounds = list_for_handoff(&db, "hf_0123456789").expect("the rounds");
        assert_eq!(rounds.len(), 2);
        assert_eq!(rounds[0], corrected);
        assert_eq!(rounds[1].no, 2);
        assert_eq!(rounds[1].verify_ok, None);
    }

    #[test]
    fn a_round_of_a_handoff_that_does_not_exist_is_refused() {
        let db = Db::open_in_memory().expect("a database");
        let error = upsert(&db, &round("hf_nosuchthing", 1)).expect_err("no such handoff");
        assert!(error.to_string().contains("writing a round"), "{error}");
    }
}
