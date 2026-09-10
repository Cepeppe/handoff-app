//! Settings → Network: every outbound connection since the app was installed (NET-01).
//!
//! The page is a **self-declaration**, and NET-02 is explicit that it is not the proof: the
//! proof is the firewall test the documentation describes. What makes the declaration worth
//! reading is where the rows come from — `network_events` has exactly one writer,
//! [`crate::net::egress`], and §7.13's four guards say that nothing else in this application
//! can reach the network at all. So a connection that is not on this page is a connection
//! that could not have happened.
//!
//! In this build the page is empty on every machine, because there is nothing to write: the
//! update check is deferred with the public release (`TASKS.md` §0.4 item 8, T-078). The page
//! says so in as many words rather than showing an empty list and letting the user guess
//! whether that means "nothing happened" or "nothing was recorded".
//!
//! It reads the window's own connection, like [`super::log`] and for the same reason: the
//! store owns handoffs, and this table is not one of them.

use serde::Serialize;
use tauri::{AppHandle, Manager as _};

use crate::log::network_events::{self, NetworkEventRow};
use crate::log::{Db, Timestamp};

use super::Ui;

/// One row of the Network page (NET-01: date, domain, bytes sent), with its purpose.
///
/// `purpose` travels as the stable key `net::egress` recorded (`update-check`, §7.13, F-14)
/// and not as a sentence: the row outlives the language the window was in when it was
/// written, so the page is what turns it into one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkEventView {
    /// When the connection was made.
    pub at: Timestamp,
    /// The host that was reached.
    pub domain: String,
    /// How many bytes of request left.
    pub bytes_sent: i64,
    /// Why, as the key the page renders.
    pub purpose: String,
}

impl From<&NetworkEventRow> for NetworkEventView {
    fn from(row: &NetworkEventRow) -> Self {
        Self {
            at: row.at.clone(),
            domain: row.domain.clone(),
            bytes_sent: row.bytes_sent,
            purpose: row.purpose.clone(),
        }
    }
}

/// Every recorded connection, most recent first (NET-01).
///
/// Empty when the window has no connection to the log, which is the answer every other read
/// of a settings page gives: an error the user cannot act on is worse than an empty page,
/// and the sentence beside the list already says this build connects to nothing.
#[tauri::command]
pub fn network_events(app: AppHandle) -> Vec<NetworkEventView> {
    app.state::<Ui>()
        .with_db(events_of)
        .unwrap_or_else(|| Ok(Vec::new()))
        .unwrap_or_else(|error| {
            tracing::warn!(error = %error, "the network events could not be listed");
            Vec::new()
        })
}

fn events_of(db: &Db) -> crate::log::error::Result<Vec<NetworkEventView>> {
    Ok(network_events::list(db)?
        .iter()
        .map(NetworkEventView::from)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_is_empty_on_a_machine_that_has_connected_to_nothing() {
        // Which is every machine in this build (§0.4 item 8): `net::egress` is the only
        // writer and it has no caller.
        let db = Db::open_in_memory().expect("a database");
        assert!(events_of(&db).expect("the events").is_empty());
    }

    #[test]
    fn a_recorded_connection_is_drawn_with_its_domain_bytes_and_purpose() {
        let db = Db::open_in_memory().expect("a database");
        network_events::append(
            &db,
            &NetworkEventRow {
                id: 0,
                at: Timestamp::parse("2026-09-10T09:00:00Z").expect("rfc 3339"),
                domain: "updates.example.test".to_owned(),
                bytes_sent: 121,
                purpose: crate::net::egress::PURPOSE_UPDATE_CHECK.to_owned(),
            },
        )
        .expect("an event");

        let drawn = events_of(&db).expect("the events");
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].domain, "updates.example.test");
        assert_eq!(drawn[0].bytes_sent, 121);
        assert_eq!(drawn[0].purpose, "update-check");
        assert_eq!(drawn[0].at.as_str(), "2026-09-10T09:00:00.000Z");
    }
}
