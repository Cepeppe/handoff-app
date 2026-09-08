//! One transition, committed as one transaction (§7.4, DD-31, NFR-12).
//!
//! Every move of the state machine writes `handoffs.state_json` **and** the rows that
//! record what happened: the round it touched, the diary entry, the text that left the
//! machine. DD-31 puts the state and the log in one database precisely so that those
//! writes commit together; a caller that made them one at a time would leave "the state
//! says verified but the log has no verification" reachable after a crash, which NFR-12
//! promises it is not.
//!
//! This is not a table module. It owns no columns and no SQL of its own: it opens the
//! transaction, calls the typed repositories in the order the foreign keys need (the
//! handoff first, everything that references it after) and commits. An error anywhere
//! drops the [`rusqlite::Transaction`], which rolls back — so the caller keeps its
//! in-memory state and reports the failure (FM-28).
//!
//! `unchecked_transaction` rather than `Connection::transaction`: the repositories take
//! `&Db` and reach the connection through a shared reference, so a transaction that
//! borrowed it mutably could not have them called inside it. The "unchecked" part is that
//! rusqlite cannot then prove at compile time that no second transaction is open on the
//! same connection; the store is a single actor with a single connection (§7.3), so there
//! is no second one to open.

use super::db::Db;
use super::error::{Result, StoreError};
use super::events::EventRow;
use super::handoffs::HandoffRow;
use super::rounds::RoundRow;
use super::sends::SendRow;
use super::{events, handoffs, rounds, sends};

/// Everything one move of the state machine writes.
///
/// The handoff row is always there — a transition that changed nothing about the state
/// would not be a transition — and the three lists are what that particular move produced.
#[derive(Debug)]
pub struct Transition<'a> {
    /// The handoff, with its new `state_json`.
    pub handoff: &'a HandoffRow,
    /// The rounds it opened or closed.
    pub rounds: &'a [RoundRow],
    /// The diary entries it appended.
    pub events: &'a [EventRow],
    /// The texts that left the machine because of it (LOG-03).
    pub sends: &'a [SendRow],
}

impl<'a> Transition<'a> {
    /// A transition that only writes the handoff row.
    #[must_use]
    pub fn of(handoff: &'a HandoffRow) -> Self {
        Self {
            handoff,
            rounds: &[],
            events: &[],
            sends: &[],
        }
    }

    /// The same transition with its rounds.
    #[must_use]
    pub fn with_rounds(mut self, rounds: &'a [RoundRow]) -> Self {
        self.rounds = rounds;
        self
    }

    /// The same transition with its diary entries.
    #[must_use]
    pub fn with_events(mut self, events: &'a [EventRow]) -> Self {
        self.events = events;
        self
    }

    /// The same transition with what it sent.
    #[must_use]
    pub fn with_sends(mut self, sends: &'a [SendRow]) -> Self {
        self.sends = sends;
        self
    }
}

/// Writes a whole transition, or nothing at all.
///
/// # Errors
///
/// [`StoreError::Persistence`] when any of the writes fails. Nothing is left behind: the
/// transaction is rolled back before the error is returned.
pub fn commit(db: &Db, transition: &Transition<'_>) -> Result<()> {
    let tx = db
        .conn()
        .unchecked_transaction()
        .map_err(|error| StoreError::of("opening a transaction", error))?;

    handoffs::upsert(db, transition.handoff)?;
    for round in transition.rounds {
        rounds::upsert(db, round)?;
    }
    for event in transition.events {
        events::append(db, event)?;
    }
    for send in transition.sends {
        sends::append(db, send)?;
    }

    tx.commit()
        .map_err(|error| StoreError::of("committing a transition", error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::events::EventKind;
    use crate::log::sends::SendKind;
    use crate::log::testing::{event, handoff, round, send, session};
    use crate::log::{handoffs as handoff_rows, sessions};

    fn database() -> Db {
        let db = Db::open_in_memory().expect("a database");
        sessions::register(&db, &session("ses_00000001")).expect("a session");
        db
    }

    #[test]
    fn a_transition_writes_its_four_kinds_of_row_together() {
        let db = database();
        let row = handoff("hf_0000000001");
        let rounds = [round("hf_0000000001", 1)];
        let events = [event("hf_0000000001", EventKind::State)];
        let sends = [send("hf_0000000001", SendKind::Question)];

        commit(
            &db,
            &Transition::of(&row)
                .with_rounds(&rounds)
                .with_events(&events)
                .with_sends(&sends),
        )
        .expect("the transition");

        assert!(handoff_rows::get(&db, "hf_0000000001")
            .expect("a read")
            .is_some());
        assert_eq!(
            crate::log::rounds::list_for_handoff(&db, "hf_0000000001")
                .expect("the rounds")
                .len(),
            1
        );
        assert_eq!(
            crate::log::events::list_for_handoff(&db, "hf_0000000001")
                .expect("the events")
                .len(),
            1
        );
        assert_eq!(
            crate::log::sends::list_for_handoff(&db, "hf_0000000001")
                .expect("the sends")
                .len(),
            1
        );
    }

    #[test]
    fn a_failure_anywhere_leaves_the_database_as_it_was() {
        let db = database();
        let row = handoff("hf_0000000001");
        let events = [event("hf_0000000001", EventKind::State)];
        // A send of 63 characters where the schema demands 64: the last write of the
        // transaction is the one that fails, so everything before it has already run.
        let mut bad = send("hf_0000000001", SendKind::ScreenshotImage);
        bad.image_sha256 = Some("0".repeat(63));
        let sends = [bad];

        let error = commit(
            &db,
            &Transition::of(&row).with_events(&events).with_sends(&sends),
        )
        .expect_err("the send is refused");
        assert!(error.to_string().contains("appending a send"), "{error}");

        assert!(
            handoff_rows::get(&db, "hf_0000000001")
                .expect("a read")
                .is_none(),
            "the handoff row must have been rolled back with the send"
        );
        assert!(crate::log::events::list_for_handoff(&db, "hf_0000000001")
            .expect("the events")
            .is_empty());
    }
}
