//! Opening `handoff.sqlite`, and bringing it to the current schema (§7.11, DD-31).
//!
//! # Why one database
//!
//! DD-31: the handoff state and the log live together so that a transition and the row that
//! records it commit in the same transaction. A second database would make "the state says
//! verified but the log has no verification" a reachable state after a crash, and NFR-12
//! promises it is not.
//!
//! # The three pragmas, and why each one
//!
//! - **WAL.** A reader (the Log page, an export) never blocks the writer (the store actor),
//!   which is the whole traffic pattern of this application: one writer, occasional
//!   readers. It is a property of the file, so it survives being set once.
//! - **Foreign keys on.** SQLite has them off by default, per connection. Every cascade in
//!   the schema — a deleted handoff taking its rounds, events and sends with it (LOG-04) —
//!   is a no-op without this line, and the deletion would silently leave orphaned rows.
//! - **Busy timeout.** Two connections do exist in practice (the actor, and whatever reads
//!   for an export), and WAL still serialises writers. Five seconds is long enough that a
//!   normal overlap waits instead of failing, and short enough that a real deadlock is
//!   reported to the user as FM-28 rather than hanging the UI.
//!
//! # Migrations
//!
//! Forward-only, versioned, and applied inside one transaction each: either the file is at
//! version *n* with the statements of *n* applied, or it is still at *n-1*. The SQL lives
//! in `src-tauri/migrations/` and is compiled in, because the app ships as one executable
//! and a migration that could go missing is a migration that will.
//!
//! A file already at a **higher** version than this build knows is refused rather than
//! touched: it was written by a newer release, and a downgrade that ran the old code
//! against the new schema is how a log gets corrupted.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
#[cfg(test)]
use rusqlite::OptionalExtension;

use super::error::{PersistenceCause, Result, StoreError};
use super::time::Timestamp;
use crate::paths;

/// The database file name inside the app data directory (§7.2).
pub const DATABASE_FILE_NAME: &str = "handoff.sqlite";

/// How long a statement waits for another connection's write lock before giving up.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// One versioned step towards the current schema.
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

/// Every step, in order. Append; never edit an entry that has shipped.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "resume_requests",
        sql: include_str!("../../migrations/0002_resume_requests.sql"),
    },
];

/// The schema version this build knows how to read and write.
#[must_use]
pub fn current_schema_version() -> i64 {
    match MIGRATIONS.last() {
        Some(last) => last.version,
        None => 0,
    }
}

/// An open database, already migrated.
///
/// It owns one `rusqlite::Connection`, which is neither `Sync` nor cheap to clone by
/// design. That suits §7.3: all state mutations are serialised on the store's single actor
/// task, so the connection has one owner and no lock of ours is needed around it.
#[derive(Debug)]
pub struct Db {
    conn: Connection,
    path: Option<PathBuf>,
}

impl Db {
    /// Opens (creating it if needed) `handoff.sqlite` in the app data directory.
    ///
    /// The directory is created as well: on a first launch nothing has made it yet, and
    /// `HANDOFF_APP_DATA_DIR` may point anywhere in a test.
    ///
    /// # Errors
    ///
    /// [`StoreError::Persistence`] when the directory cannot be created, the file cannot be
    /// opened, or a migration fails.
    pub fn open_app_data() -> Result<Self> {
        let dir = paths::app_data_dir();
        fs::create_dir_all(&dir)
            .map_err(|error| StoreError::of("creating the app data directory", error))?;
        Self::open_at(dir.join(DATABASE_FILE_NAME))
    }

    /// Opens (creating it if needed) a database at an explicit path.
    ///
    /// # Errors
    ///
    /// [`StoreError::Persistence`] when the file cannot be opened or a migration fails.
    pub fn open_at(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open(&path)
            .map_err(|error| StoreError::of("opening the database", error))?;
        let mut db = Self {
            conn,
            path: Some(path),
        };
        db.prepare()?;
        Ok(db)
    }

    /// A database that lives only as long as this value. Tests, and nothing else.
    ///
    /// # Errors
    ///
    /// [`StoreError::Persistence`] when a migration fails.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|error| StoreError::of("opening the database", error))?;
        let mut db = Self { conn, path: None };
        db.prepare()?;
        Ok(db)
    }

    /// Where the file is, when it is a file.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The version the file is at.
    ///
    /// # Errors
    ///
    /// [`StoreError::Persistence`] when the version table cannot be read.
    pub fn schema_version(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT max(version) FROM schema_migrations", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
            .map(Option::unwrap_or_default)
            .map_err(|error| StoreError::of("reading the schema version", error))
    }

    /// The connection, for the repositories of this module and for nothing else.
    ///
    /// `pub(super)`: "no SQL outside `log/`" is a rule of the task, and this is what
    /// enforces it — a module elsewhere in the crate cannot reach a statement handle.
    pub(super) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// The connection, mutably, for the operations that need a transaction.
    pub(super) fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Pragmas, then migrations.
    fn prepare(&mut self) -> Result<()> {
        self.apply_pragmas()?;
        self.migrate()
    }

    fn apply_pragmas(&self) -> Result<()> {
        // An in-memory database answers `memory` and keeps working; a file answers `wal`.
        // Neither is an error, so the returned mode is read and dropped rather than
        // asserted — a build that refused `memory` would refuse every unit test.
        self.conn
            .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
            .map_err(|error| StoreError::of("enabling the write-ahead log", error))?;
        self.conn
            .pragma_update(None, "foreign_keys", true)
            .map_err(|error| StoreError::of("enabling foreign keys", error))?;
        self.conn
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| StoreError::of("setting the busy timeout", error))?;
        Ok(())
    }

    fn migrate(&mut self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_migrations (\n\
                     version    INTEGER PRIMARY KEY,\n\
                     name       TEXT NOT NULL,\n\
                     applied_at TEXT NOT NULL\n\
                 )",
            )
            .map_err(|error| StoreError::of("creating the migrations table", error))?;

        let applied = self.schema_version()?;
        if applied > current_schema_version() {
            return Err(StoreError::of(
                "opening the database",
                PersistenceCause::Schema(format!(
                    "the log is at schema version {applied}, which this version of the \
                     application does not know (it knows {}). Install the newer version \
                     again rather than downgrading.",
                    current_schema_version()
                )),
            ));
        }

        for migration in MIGRATIONS.iter().filter(|step| step.version > applied) {
            let transaction = self
                .conn
                .transaction()
                .map_err(|error| StoreError::of("starting a migration", error))?;
            transaction
                .execute_batch(migration.sql)
                .map_err(|error| StoreError::of("applying a migration", error))?;
            transaction
                .execute(
                    "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
                    rusqlite::params![migration.version, migration.name, Timestamp::now()],
                )
                .map_err(|error| StoreError::of("recording a migration", error))?;
            transaction
                .commit()
                .map_err(|error| StoreError::of("committing a migration", error))?;
        }
        Ok(())
    }

    /// Whether a table exists. Only the schema tests ask.
    #[cfg(test)]
    pub(super) fn has_table(&self, name: &str) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map(|found| found.is_some())
            .map_err(|error| StoreError::of("reading the table list", error))
    }

    /// Every table of ours, sorted, with the bookkeeping ones left out.
    ///
    /// Sorted so that an export and a dump name their tables in the same order on every
    /// machine; `sqlite_master` has an order of its own that nothing promises.
    pub(super) fn user_tables(&self) -> Result<Vec<String>> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> 'schema_migrations' \
                 ORDER BY name",
            )
            .map_err(|error| StoreError::of("reading the table list", error))?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| StoreError::of("reading the table list", error))?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|error| StoreError::of("reading the table list", error))?;
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::testing::{clean, tempdir};

    /// The nine tables of §7.11.
    const TABLES: [&str; 9] = [
        "events",
        "handoffs",
        "hook_blocks",
        "network_events",
        "rounds",
        "sends",
        "sessions",
        "settings",
        "user_requests",
    ];

    #[test]
    fn a_fresh_database_carries_every_table_of_the_design() {
        let db = Db::open_in_memory().expect("a fresh database");
        assert_eq!(
            db.schema_version().expect("version"),
            current_schema_version()
        );
        assert_eq!(db.user_tables().expect("tables"), TABLES);
        for table in TABLES {
            assert!(db.has_table(table).expect("lookup"), "{table} is missing");
        }
    }

    #[test]
    fn foreign_keys_are_on_which_is_what_makes_the_cascades_real() {
        let db = Db::open_in_memory().expect("a fresh database");
        let enabled: i64 = db
            .conn()
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("pragma");
        assert_eq!(enabled, 1);
    }

    #[test]
    fn re_opening_a_file_applies_nothing_and_keeps_the_rows() {
        let dir = tempdir();
        let path = dir.join(DATABASE_FILE_NAME);
        {
            let db = Db::open_at(&path).expect("a fresh database");
            db.conn()
                .execute(
                    "INSERT INTO settings (key, value) VALUES ('language', '\"it\"')",
                    [],
                )
                .expect("a settings row");
            assert_eq!(
                db.schema_version().expect("version"),
                current_schema_version()
            );
        }
        let reopened = Db::open_at(&path).expect("the same database");
        assert_eq!(
            reopened.schema_version().expect("version"),
            current_schema_version()
        );
        let applied: i64 = reopened
            .conn()
            .query_row("SELECT count(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(
            applied,
            current_schema_version(),
            "a re-open must not replay a migration"
        );
        let value: String = reopened
            .conn()
            .query_row(
                "SELECT value FROM settings WHERE key = 'language'",
                [],
                |row| row.get(0),
            )
            .expect("the row survived");
        assert_eq!(value, "\"it\"");
        clean(&dir);
    }

    #[test]
    fn a_file_from_a_newer_release_is_refused_rather_than_migrated_backwards() {
        let dir = tempdir();
        let path = dir.join(DATABASE_FILE_NAME);
        {
            let db = Db::open_at(&path).expect("a fresh database");
            db.conn()
                .execute(
                    "INSERT INTO schema_migrations (version, name, applied_at) \
                     VALUES (99, 'from the future', '2030-01-01T00:00:00.000Z')",
                    [],
                )
                .expect("a future migration row");
        }
        let error = Db::open_at(&path).expect_err("a newer schema is refused");
        assert!(error.to_string().contains("schema version 99"), "{error}");
        clean(&dir);
    }

    #[test]
    fn the_file_lands_where_the_app_data_directory_says() {
        let db = Db::open_in_memory().expect("a fresh database");
        assert!(db.path().is_none());
        let dir = tempdir();
        let db = Db::open_at(dir.join(DATABASE_FILE_NAME)).expect("a fresh database");
        assert_eq!(
            db.path().expect("a path").file_name().expect("a name"),
            DATABASE_FILE_NAME
        );
        drop(db);
        clean(&dir);
    }
}
