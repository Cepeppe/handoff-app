//! The invariants helper: everything the database holds, as one string.
//!
//! [`dump_all_text`] concatenates every column of every row of every table. It exists for
//! one kind of test — "this value must not be anywhere in the log" — and it is written this
//! way on purpose:
//!
//! - it reads the table list and the column list **from the file**, so a table or a column
//!   added by a later migration is covered without anyone remembering this module;
//! - it looks at columns, not at types, so a value hidden inside a JSON blob is found as
//!   surely as one in its own column.
//!
//! A test asking "does the log contain this secret" can only be trusted if the answer
//! covers places nobody thought of; a helper that listed the columns it knew about would
//! answer for the places somebody did think of, which is the wrong question.
//!
//! It is compiled into the application rather than gated behind `#[cfg(test)]` because the
//! test that matters most runs from `tests/`, against the crate as a consumer sees it. It
//! reads and returns; it changes nothing.

use rusqlite::types::ValueRef;

use super::db::Db;
use super::error::{Result, StoreError};

/// Every column of every row of every table, one value per line, prefixed by
/// `<table>.<column>: `.
///
/// The order is the table order of [`Db::user_tables`] and then the rows as SQLite returns
/// them, which is enough for a containment test — the only thing this is for. `NULL` is
/// written as an empty value, so a row with a null column still produces its line.
///
/// # Errors
///
/// [`StoreError::Persistence`] when a table cannot be read.
pub fn dump_all_text(db: &Db) -> Result<String> {
    let mut dump = String::new();
    for table in db.user_tables()? {
        // The name comes from `sqlite_master`, never from a caller.
        let sql = format!("SELECT * FROM {table}");
        let mut statement = db
            .conn()
            .prepare(&sql)
            .map_err(|error| StoreError::of("dumping the log", error))?;
        let columns: Vec<String> = statement
            .column_names()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        let mut rows = statement
            .query([])
            .map_err(|error| StoreError::of("dumping the log", error))?;
        while let Some(row) = rows
            .next()
            .map_err(|error| StoreError::of("dumping the log", error))?
        {
            for (index, column) in columns.iter().enumerate() {
                let value = row
                    .get_ref(index)
                    .map_err(|error| StoreError::of("dumping the log", error))?;
                dump.push_str(table.as_str());
                dump.push('.');
                dump.push_str(column);
                dump.push_str(": ");
                dump.push_str(&as_text(value));
                dump.push('\n');
            }
        }
    }
    Ok(dump)
}

/// Any stored value as the text a containment test searches.
///
/// Every type is rendered, none is skipped: a value that reached the wrong column with the
/// wrong type is exactly the leak this helper exists to catch, so a `NULL` becomes an empty
/// line and a blob becomes its bytes rather than either of them disappearing.
fn as_text(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => String::new(),
        ValueRef::Integer(number) => number.to_string(),
        ValueRef::Real(number) => number.to_string(),
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
            String::from_utf8_lossy(bytes).into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::testing::populated;

    #[test]
    fn the_dump_names_every_table_and_carries_the_values() {
        let db = populated();
        let dump = dump_all_text(&db).expect("a dump");
        for table in [
            "events",
            "handoffs",
            "hook_blocks",
            "network_events",
            "rounds",
            "sends",
            "sessions",
            "user_requests",
        ] {
            assert!(dump.contains(&format!("{table}.")), "{table} is missing");
        }
        assert!(dump.contains("handoffs.id: hf_0000000001"));
        assert!(dump.contains("sessions.session_ref: ses_00000001"));
    }

    #[test]
    fn a_value_inside_a_json_column_is_found_as_well_as_one_in_its_own_column() {
        // The whole point: the dump does not know which columns are documents.
        let db = populated();
        let dump = dump_all_text(&db).expect("a dump");
        assert!(dump.contains("\"cursor\""), "state_json was not dumped");
    }

    #[test]
    fn a_column_added_later_is_dumped_without_anyone_editing_this_module() {
        let db = Db::open_in_memory().expect("a database");
        db.conn()
            .execute_batch(
                "CREATE TABLE later_arrival (id INTEGER PRIMARY KEY, note TEXT);\n\
                 INSERT INTO later_arrival (note) VALUES ('a value nobody listed');",
            )
            .expect("a new table");
        let dump = dump_all_text(&db).expect("a dump");
        assert!(
            dump.contains("later_arrival.note: a value nobody listed"),
            "{dump}"
        );
    }

    #[test]
    fn a_null_column_still_produces_its_line() {
        let db = populated();
        let dump = dump_all_text(&db).expect("a dump");
        assert!(dump.contains("handoffs.resumed_from_json: \n"), "{dump}");
    }
}
