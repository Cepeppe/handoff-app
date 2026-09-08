//! "Delete everything" and "export everything" (LOG-04).
//!
//! Both exist for the same reason and it is written in the requirement: *it is the user's
//! data*. Deleting a single handoff is [`super::handoffs::delete_handoff`]; the two
//! whole-database operations are here because they are the two that must never treat one
//! table differently by accident.
//!
//! - [`delete_all`] empties every table **except `settings`**, in one transaction. Settings
//!   are the app's own preferences, not a record of the user's work, and losing the chosen
//!   language and the window position on "delete my handoffs" would be a surprise, not a
//!   guarantee.
//! - [`export_json`] writes every table, `settings` included, as one JSON document. It is
//!   generated from the table list the file actually has rather than from a list written
//!   here, so a table added by a later migration is exported the day it exists instead of
//!   the day someone remembers this file.
//!
//! There is no retention job and there never will be one (LOG-05): nothing in this module
//! runs on a timer.

use std::fs;
use std::path::Path;

use rusqlite::types::ValueRef;
use serde_json::{Map, Value};

use super::db::{current_schema_version, Db};
use super::error::{Result, StoreError};
use super::time::Timestamp;

/// The `format` field of an export, so a reader knows what it is holding.
pub const EXPORT_FORMAT: &str = "baton-log-export-v1";

/// The one table "delete everything" keeps.
const KEPT_BY_DELETE_ALL: &str = "settings";

/// Empties every table except `settings` (LOG-04).
///
/// One transaction: a half-deleted log is worse than either outcome, and the cascades mean
/// a partial failure could otherwise leave rounds without their handoff.
///
/// The tables are emptied in the order [`Db::user_tables`] gives them, with foreign keys
/// deferred to the commit. The alternative — deleting children before parents, from a list
/// of tables kept in this file — would break the day a migration adds one, and only a user
/// would find out.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the transaction cannot be completed.
pub fn delete_all(db: &mut Db) -> Result<()> {
    let tables: Vec<String> = db
        .user_tables()?
        .into_iter()
        .filter(|name| name != KEPT_BY_DELETE_ALL)
        .collect();

    let transaction = db
        .conn_mut()
        .transaction()
        .map_err(|error| StoreError::of("deleting everything", error))?;
    // Foreign keys stay enforced and are checked at the commit rather than at each
    // statement, so the tables can be emptied in any order: at the commit every table but
    // `settings` is empty, and nothing can be left dangling. **Measured**, because the
    // reference reads the other way — "the RESTRICT action processing happens as soon as
    // the field is updated" — and `handoffs.session_ref` is a RESTRICT: with this pragma
    // the bundled SQLite defers it like any other constraint, and
    // `delete_all_holds_when_a_parent_table_sorts_before_its_children` is what says so.
    transaction
        .execute_batch("PRAGMA defer_foreign_keys = ON")
        .map_err(|error| StoreError::of("deleting everything", error))?;
    for table in &tables {
        transaction
            .execute(&format!("DELETE FROM {table}"), [])
            .map_err(|error| StoreError::of("deleting everything", error))?;
    }
    transaction
        .commit()
        .map_err(|error| StoreError::of("deleting everything", error))?;
    Ok(())
}

/// Writes every table as one JSON document at `path` (LOG-04).
///
/// The shape is `{ format, schema_version, exported_at, tables: { <name>: [ { <column>:
/// <value> } ] } }`, with the rows of each table in their primary-key order and the tables
/// in alphabetical order, so two exports of the same database are byte-identical apart from
/// `exported_at`.
///
/// # Errors
///
/// [`StoreError::Persistence`] when a table cannot be read or the file cannot be written.
pub fn export_json(db: &Db, path: impl AsRef<Path>) -> Result<()> {
    let document = export_value(db)?;
    let text = serde_json::to_string_pretty(&document)
        .map_err(|error| StoreError::of("exporting the log", error))?;
    fs::write(path.as_ref(), text).map_err(|error| StoreError::of("exporting the log", error))
}

/// The document [`export_json`] writes, for the tests and for anything that wants it in
/// memory rather than on disk.
///
/// # Errors
///
/// [`StoreError::Persistence`] when a table cannot be read.
pub fn export_value(db: &Db) -> Result<Value> {
    let mut tables = Map::new();
    for table in db.user_tables()? {
        let rows = rows_of(db, &table)?;
        tables.insert(table, Value::Array(rows));
    }
    let mut document = Map::new();
    document.insert("format".to_owned(), Value::String(EXPORT_FORMAT.to_owned()));
    document.insert(
        "schema_version".to_owned(),
        Value::Number(current_schema_version().into()),
    );
    document.insert(
        "exported_at".to_owned(),
        Value::String(Timestamp::now().to_string()),
    );
    document.insert("tables".to_owned(), Value::Object(tables));
    Ok(Value::Object(document))
}

/// Every row of one table, as objects keyed by column name.
fn rows_of(db: &Db, table: &str) -> Result<Vec<Value>> {
    // The table name comes from `sqlite_master`, never from a caller, so there is nothing
    // here for a parameter to protect: SQLite has no placeholder for an identifier.
    let sql = format!("SELECT * FROM {table}");
    let mut statement = db
        .conn()
        .prepare(&sql)
        .map_err(|error| StoreError::of("exporting the log", error))?;
    let columns: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    let mut rows = statement
        .query([])
        .map_err(|error| StoreError::of("exporting the log", error))?;
    let mut exported = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| StoreError::of("exporting the log", error))?
    {
        let mut object = Map::new();
        for (index, name) in columns.iter().enumerate() {
            let value = row
                .get_ref(index)
                .map_err(|error| StoreError::of("exporting the log", error))?;
            object.insert(name.clone(), json_value(value));
        }
        exported.push(Value::Object(object));
    }
    Ok(exported)
}

/// A SQLite value as JSON. No column of this schema holds a blob; one would be exported as
/// its bytes rather than dropped, so a future column cannot disappear from an export
/// silently.
fn json_value(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(number) => Value::Number(number.into()),
        ValueRef::Real(number) => {
            serde_json::Number::from_f64(number).map_or(Value::Null, Value::Number)
        }
        ValueRef::Text(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => Value::String(text.to_owned()),
            Err(_) => Value::Array(
                bytes
                    .iter()
                    .map(|byte| Value::Number((*byte).into()))
                    .collect(),
            ),
        },
        ValueRef::Blob(bytes) => Value::Array(
            bytes
                .iter()
                .map(|byte| Value::Number((*byte).into()))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::events::EventKind;
    use crate::log::sends::SendKind;
    use crate::log::testing::{clean, event, handoff, populated, send, session, tempdir};
    use crate::log::{events, handoffs, sends, sessions, settings};

    #[test]
    fn delete_all_empties_the_log_and_keeps_the_settings() {
        let mut db = populated();
        settings::set(&db, "language", &"it".to_owned()).expect("a setting");

        delete_all(&mut db).expect("everything deleted");

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
            let count: i64 = db
                .conn()
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("count");
            assert_eq!(count, 0, "{table} still has rows");
        }
        assert_eq!(
            settings::get::<String>(&db, "language")
                .expect("a read")
                .as_deref(),
            Some("it")
        );
    }

    #[test]
    fn delete_all_holds_when_a_parent_table_sorts_before_its_children() {
        // The measurement `delete_all` leans on: with `defer_foreign_keys` even a RESTRICT
        // action is checked at the commit, so the tables may be emptied in any order. The
        // SQLite reference says RESTRICT is enforced "as soon as the field is updated"; if
        // a future bundled version behaves that way, this test is what says so, instead of
        // a user finding that "delete everything" refuses.
        let mut db = Db::open_in_memory().expect("a database");
        db.conn()
            .execute_batch(
                "CREATE TABLE aaa_parent (id INTEGER PRIMARY KEY);\n\
                 CREATE TABLE zzz_child (\
                     id INTEGER PRIMARY KEY, \
                     parent INTEGER NOT NULL REFERENCES aaa_parent (id) ON DELETE RESTRICT);\n\
                 INSERT INTO aaa_parent (id) VALUES (1);\n\
                 INSERT INTO zzz_child (id, parent) VALUES (1, 1);",
            )
            .expect("two tables");
        delete_all(&mut db).expect("everything deleted");
        let left: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM aaa_parent", [], |row| row.get(0))
            .expect("count");
        assert_eq!(left, 0);
    }

    #[test]
    fn deleting_one_handoff_takes_its_rounds_events_and_sends_with_it() {
        let db = populated();
        // The other handoff and its rows are the control: a cascade that took everything
        // would pass a test that only looked at the deleted one.
        assert!(handoffs::delete_handoff(&db, "hf_0000000001").expect("a deletion"));
        assert!(!handoffs::delete_handoff(&db, "hf_0000000001").expect("nothing left"));

        assert!(events::list_for_handoff(&db, "hf_0000000001")
            .expect("the events")
            .is_empty());
        assert!(sends::list_for_handoff(&db, "hf_0000000001")
            .expect("the sends")
            .is_empty());
        let rounds: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM rounds WHERE handoff_id = 'hf_0000000001'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(rounds, 0);

        assert_eq!(
            events::list_for_handoff(&db, "hf_0000000002")
                .expect("the events")
                .len(),
            1
        );
        // The session it belonged to is history and stays.
        assert!(sessions::get(&db, "ses_00000001")
            .expect("a read")
            .is_some());
    }

    #[test]
    fn an_export_carries_every_table_and_the_schema_it_came_from() {
        let db = populated();
        settings::set(&db, "language", &"it".to_owned()).expect("a setting");
        let document = export_value(&db).expect("an export");

        assert_eq!(document["format"], Value::String(EXPORT_FORMAT.to_owned()));
        assert_eq!(
            document["schema_version"],
            Value::from(current_schema_version())
        );
        assert!(document["exported_at"]
            .as_str()
            .expect("an instant")
            .ends_with('Z'));

        let tables = document["tables"].as_object().expect("the tables");
        assert_eq!(
            tables.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "events",
                "handoffs",
                "hook_blocks",
                "network_events",
                "rounds",
                "sends",
                "sessions",
                "settings",
                "user_requests"
            ]
        );
        let handoff_rows = tables["handoffs"].as_array().expect("the handoffs");
        assert_eq!(handoff_rows.len(), 2);
        assert_eq!(
            handoff_rows[0]["id"],
            Value::String("hf_0000000001".to_owned())
        );
        // A null column is exported as null, not dropped: an export is the row as it is.
        assert_eq!(handoff_rows[0]["resumed_from_json"], Value::Null);
        assert_eq!(
            tables["settings"].as_array().expect("the settings").len(),
            1
        );
    }

    #[test]
    fn an_export_lands_on_disk_as_the_document_it_built() {
        let dir = tempdir();
        let path = dir.join("baton-log.json");
        let db = populated();
        export_json(&db, &path).expect("a file");
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("the file")).expect("json");
        assert_eq!(written["format"], Value::String(EXPORT_FORMAT.to_owned()));
        assert_eq!(
            written["tables"]["handoffs"]
                .as_array()
                .expect("the handoffs")
                .len(),
            2
        );
        clean(&dir);
    }

    #[test]
    fn an_export_of_an_empty_log_is_still_a_document_with_every_table() {
        let db = Db::open_in_memory().expect("a database");
        let document = export_value(&db).expect("an export");
        let tables = document["tables"].as_object().expect("the tables");
        assert_eq!(tables.len(), 9);
        assert!(tables
            .values()
            .all(|rows| rows.as_array().expect("rows").is_empty()));
    }

    #[test]
    fn a_table_added_after_the_delete_and_the_export_were_written_is_covered_by_both() {
        // Neither routine holds a list of tables. A migration that adds one must not have
        // to remember to come back here, and this is what says so.
        let mut db = Db::open_in_memory().expect("a database");
        db.conn()
            .execute_batch("CREATE TABLE later_arrival (id INTEGER PRIMARY KEY, note TEXT)")
            .expect("a new table");
        db.conn()
            .execute("INSERT INTO later_arrival (note) VALUES ('hello')", [])
            .expect("a row");

        let document = export_value(&db).expect("an export");
        assert_eq!(
            document["tables"]["later_arrival"][0]["note"],
            Value::String("hello".to_owned())
        );

        delete_all(&mut db).expect("everything deleted");
        let count: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM later_arrival", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 0);
    }

    #[test]
    fn the_export_of_a_send_carries_the_hash_and_no_pixels() {
        let db = Db::open_in_memory().expect("a database");
        sessions::register(&db, &session("ses_00000001")).expect("a session");
        handoffs::upsert(&db, &handoff("hf_0000000001")).expect("a handoff");
        sends::append(&db, &send("hf_0000000001", SendKind::ScreenshotImage)).expect("a send");
        events::append(&db, &event("hf_0000000001", EventKind::Screenshot)).expect("an event");

        let document = export_value(&db).expect("an export");
        let row = &document["tables"]["sends"][0];
        assert_eq!(row["image_w"], Value::from(1600));
        assert_eq!(row["image_sha256"].as_str().expect("a hash").len(), 64);
        assert!(row.get("image").is_none());
    }
}
