//! The `network_events` table: every outbound connection since the app was installed
//! (NET-01, §7.13, PRIN-05).
//!
//! The Network page in settings shows this table. It is fed by the single point in the code
//! through which the app can reach the network (`net::egress`), which is what makes the
//! page a record rather than a claim: a connection that did not pass through that module
//! could not have happened, because `deny.toml` and `clippy.toml` keep every HTTP client
//! out of the tree and out of every other module (§7.13).
//!
//! While the public release and the update check are deferred (`TASKS.md` §0.4 item 8) the
//! app makes **zero** network connections, so this table stays empty on every machine. It
//! exists now because NET-01 is a promise about what the app records, and a promise that
//! only starts being kept the day there is something to record is not one.

use rusqlite::{params, Row};

use super::db::Db;
use super::error::{Result, StoreError};
use super::time::Timestamp;

/// One row of `network_events`. `id` is assigned by the database on append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkEventRow {
    /// Assigned on append; zero on the value handed to [`append`].
    pub id: i64,
    /// When the connection was made.
    pub at: Timestamp,
    /// The host that was reached.
    pub domain: String,
    /// How many bytes went out.
    pub bytes_sent: i64,
    /// Why, in a phrase the Network page can show.
    pub purpose: String,
}

/// Appends an event and returns the id the database gave it.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the write fails.
pub fn append(db: &Db, row: &NetworkEventRow) -> Result<i64> {
    db.conn()
        .execute(
            "INSERT INTO network_events (at, domain, bytes_sent, purpose) \
             VALUES (?1, ?2, ?3, ?4)",
            params![row.at, row.domain, row.bytes_sent, row.purpose],
        )
        .map_err(|error| StoreError::of("appending a network event", error))?;
    Ok(db.conn().last_insert_rowid())
}

/// Every event, most recent first: the order the Network page reads them in.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the rows cannot be read.
pub fn list(db: &Db) -> Result<Vec<NetworkEventRow>> {
    let mut statement = db
        .conn()
        .prepare("SELECT id, at, domain, bytes_sent, purpose FROM network_events ORDER BY id DESC")
        .map_err(|error| StoreError::of("reading the network events", error))?;
    let rows = statement
        .query_map([], read_row)
        .map_err(|error| StoreError::of("reading the network events", error))?
        .collect::<rusqlite::Result<Vec<NetworkEventRow>>>()
        .map_err(|error| StoreError::of("reading the network events", error))?;
    Ok(rows)
}

fn read_row(row: &Row<'_>) -> rusqlite::Result<NetworkEventRow> {
    Ok(NetworkEventRow {
        id: row.get(0)?,
        at: row.get(1)?,
        domain: row.get(2)?,
        bytes_sent: row.get(3)?,
        purpose: row.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(domain: &str) -> NetworkEventRow {
        NetworkEventRow {
            id: 0,
            at: Timestamp::parse("2026-09-08T11:00:00Z").expect("rfc 3339"),
            domain: domain.to_owned(),
            bytes_sent: 128,
            purpose: "update check".to_owned(),
        }
    }

    #[test]
    fn the_page_starts_empty_and_reads_the_most_recent_first() {
        let db = Db::open_in_memory().expect("a database");
        assert!(list(&db).expect("the events").is_empty());
        append(&db, &event("updates.example.test")).expect("an event");
        append(&db, &event("later.example.test")).expect("another event");
        let listed = list(&db).expect("the events");
        assert_eq!(
            listed
                .iter()
                .map(|row| row.domain.as_str())
                .collect::<Vec<_>>(),
            ["later.example.test", "updates.example.test"]
        );
        assert_eq!(listed[0].bytes_sent, 128);
    }
}
