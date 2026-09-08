//! The `settings` table: the app's own preferences (§2.3, §7.11).
//!
//! Typed, not stringly: a value is stored as the JSON of whatever type it is and read back
//! into that type, so a caller cannot write `"true"` and read `true` on the next release.
//! Reading a key into the wrong type is a [`super::error::StoreError::Persistence`], which
//! is what a settings table corrupted by hand should look like.
//!
//! This is the one table `delete_all` keeps (LOG-04): "delete everything" is about the
//! user's handoffs and what left the machine, not about their window position and their
//! chosen language.

use rusqlite::{params, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::db::Db;
use super::error::{Result, StoreError};

/// The value of `key`, if it has one.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the row cannot be read, or when the stored value is not
/// a `T`.
pub fn get<T: DeserializeOwned>(db: &Db, key: &str) -> Result<Option<T>> {
    let raw: Option<String> = db
        .conn()
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|error| StoreError::of("reading a setting", error))?;
    match raw {
        Some(json) => serde_json::from_str(&json)
            .map(Some)
            .map_err(|error| StoreError::of("reading a setting", error)),
        None => Ok(None),
    }
}

/// Writes `key`, replacing whatever was there.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the value cannot be encoded or written.
pub fn set<T: Serialize>(db: &Db, key: &str, value: &T) -> Result<()> {
    let json =
        serde_json::to_string(value).map_err(|error| StoreError::of("writing a setting", error))?;
    db.conn()
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, json],
        )
        .map_err(|error| StoreError::of("writing a setting", error))?;
    Ok(())
}

/// Removes `key`. Returns whether there was one.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the deletion fails.
pub fn remove(db: &Db, key: &str) -> Result<bool> {
    let deleted = db
        .conn()
        .execute("DELETE FROM settings WHERE key = ?1", [key])
        .map_err(|error| StoreError::of("removing a setting", error))?;
    Ok(deleted > 0)
}

/// Every key, sorted.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the rows cannot be read.
pub fn keys(db: &Db) -> Result<Vec<String>> {
    let mut statement = db
        .conn()
        .prepare("SELECT key FROM settings ORDER BY key")
        .map_err(|error| StoreError::of("reading the settings", error))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| StoreError::of("reading the settings", error))?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|error| StoreError::of("reading the settings", error))?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct WindowPosition {
        x: i64,
        y: i64,
    }

    #[test]
    fn a_value_comes_back_as_the_type_it_went_in_as() {
        let db = Db::open_in_memory().expect("a database");
        assert_eq!(get::<String>(&db, "language").expect("a read"), None);

        set(&db, "language", &"it".to_owned()).expect("a language");
        set(&db, "autostart", &true).expect("a flag");
        set(&db, "window", &WindowPosition { x: 12, y: 34 }).expect("a position");

        assert_eq!(
            get::<String>(&db, "language").expect("a read").as_deref(),
            Some("it")
        );
        assert_eq!(get::<bool>(&db, "autostart").expect("a read"), Some(true));
        assert_eq!(
            get::<WindowPosition>(&db, "window").expect("a read"),
            Some(WindowPosition { x: 12, y: 34 })
        );
        assert_eq!(
            keys(&db).expect("the keys"),
            ["autostart", "language", "window"]
        );
    }

    #[test]
    fn writing_a_key_twice_replaces_it() {
        let db = Db::open_in_memory().expect("a database");
        set(&db, "language", &"en".to_owned()).expect("a language");
        set(&db, "language", &"it".to_owned()).expect("another language");
        assert_eq!(keys(&db).expect("the keys").len(), 1);
        assert_eq!(
            get::<String>(&db, "language").expect("a read").as_deref(),
            Some("it")
        );
        assert!(remove(&db, "language").expect("a removal"));
        assert!(!remove(&db, "language").expect("nothing left to remove"));
    }

    #[test]
    fn reading_a_key_as_the_wrong_type_is_a_failure_and_not_a_default() {
        let db = Db::open_in_memory().expect("a database");
        set(&db, "autostart", &true).expect("a flag");
        let error = get::<WindowPosition>(&db, "autostart").expect_err("not a position");
        assert!(error.to_string().contains("reading a setting"), "{error}");
    }
}
