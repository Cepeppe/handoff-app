//! The `handoffs` table: one row per unit of human work (§7.11, LOG-02, §8.1).
//!
//! This is where DD-31 pays: the row carries both the log fields (who opened it, when, in
//! which project, with which spec) and `state_json`, the store's own serialised state, so a
//! transition and its record commit together.
//!
//! Two things this module owns and no caller can take back:
//!
//! - **`spec_json` is masked** ([`super::redact`]). The spec the app holds in memory is the
//!   true one — the copy button needs it (DET-04) — and the one on disk never is.
//! - **The vocabulary of §8.1.** [`HandoffState`] knows which of the ten states are final,
//!   which is what separates `list_active` from `list_final` and what makes the orphan rule
//!   of SRV-23 expressible. The transitions between them belong to the store (T-033); this
//!   is only what a state is called once it is written down.

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{params, OptionalExtension, Row, ToSql};
use serde::{Deserialize, Serialize};

use super::db::Db;
use super::error::{PersistenceCause, Result, StoreError};
use super::redact;
use super::time::Timestamp;
use crate::format::spec::HandoffSpec;

/// A final outcome nobody collected for this long is an orphan (§4.1 `ORPHAN_AGE_MS`,
/// SRV-23).
pub const ORPHAN_AGE_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// The states of §8.1, as they are written down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffState {
    /// A user request with no spec yet (OPEN-04).
    AwaitingSpec,
    /// Being guided, step by step.
    Active,
    /// Deferred once; the agent will come back.
    Deferred,
    /// Deferred twice; it waits for the user in the overlay.
    Parked,
    /// Done on the last step of a spec that carries a `verify` (RESP-09, VER-04).
    AwaitingVerification,
    /// The agent reported `ok = true`.
    Verified,
    /// The agent reported `ok = false` (VER-08).
    Failed,
    /// No report, or 30 minutes, or a session disconnect (VER-06).
    NotVerified,
    /// Done on the last step of a spec with no `verify` (PRIN-08).
    ConfirmedByUser,
    /// Abandoned by the user.
    Abandoned,
}

impl HandoffState {
    /// The wire and column name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingSpec => "awaiting_spec",
            Self::Active => "active",
            Self::Deferred => "deferred",
            Self::Parked => "parked",
            Self::AwaitingVerification => "awaiting_verification",
            Self::Verified => "verified",
            Self::Failed => "failed",
            Self::NotVerified => "not_verified",
            Self::ConfirmedByUser => "confirmed_by_user",
            Self::Abandoned => "abandoned",
        }
    }

    /// The five final states of VER-01 (§8.1).
    ///
    /// `failed` and `not_verified` are final although an agent action on the same id can
    /// still leave them (VER-08, DD-16): "final" here means the handoff has an outcome to
    /// deliver, which is what the orphan rule and the active-tab list both ask about.
    #[must_use]
    pub fn is_final(self) -> bool {
        matches!(
            self,
            Self::Verified
                | Self::Failed
                | Self::NotVerified
                | Self::ConfirmedByUser
                | Self::Abandoned
        )
    }

    /// The state named by `text`, if it is one.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        [
            Self::AwaitingSpec,
            Self::Active,
            Self::Deferred,
            Self::Parked,
            Self::AwaitingVerification,
            Self::Verified,
            Self::Failed,
            Self::NotVerified,
            Self::ConfirmedByUser,
            Self::Abandoned,
        ]
        .into_iter()
        .find(|state| state.as_str() == text)
    }
}

impl ToSql for HandoffState {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for HandoffState {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        Self::from_name(text).ok_or_else(|| {
            FromSqlError::Other(Box::new(StoreError::of(
                "reading a handoff state",
                PersistenceCause::Schema(format!("{text} is not a state of the design")),
            )))
        })
    }
}

/// One row of `handoffs`, with the spec as a document rather than as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffRow {
    /// `hf_` + 10 characters (§4.1).
    pub id: String,
    /// When the handoff (or the user request that became it) was opened.
    pub created_at: Timestamp,
    /// When it reached a final state.
    pub closed_at: Option<Timestamp>,
    /// The session that opened it (§7.4); absent for a request opened with no session.
    pub session_ref: Option<String>,
    /// The agent identity resolved for that session (§5.6).
    pub agent_id: Option<String>,
    /// `clientInfo.name` from the MCP handshake.
    pub client_name: Option<String>,
    /// The project folder the session was tied to.
    pub project_dir: Option<String>,
    /// The user's own words, when the handoff grew from a request (OPEN-04).
    pub request_text: Option<String>,
    /// Where it is in §8.1.
    pub state: HandoffState,
    /// The final state, once there is one.
    pub final_state: Option<HandoffState>,
    /// The spec. Stored masked; what is read back is the masked one.
    pub spec: Option<HandoffSpec>,
    /// The store's serialised state (§7.4). Opaque to the log, and swept for the literals
    /// the spec masking found.
    pub state_json: String,
    /// When the final outcome reached an agent.
    pub delivered_at: Option<Timestamp>,
    /// What a resume was answered with, when this handoff was resumed (§5.7).
    pub resumed_from_json: Option<String>,
    /// BCP-47 language tag of the spec's texts.
    pub lang: Option<String>,
}

/// Writes a handoff, creating it or replacing every field of an existing one.
///
/// The spec is masked here and the other free-text columns are swept with the literals the
/// masking found, so no caller can put a secret-treated value into this row by accident
/// (LOG-02). That is also why the value written back differs from the value passed in:
/// reading the row returns the masked spec, never the true one.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the row cannot be encoded or written.
pub fn upsert(db: &Db, row: &HandoffRow) -> Result<()> {
    let (spec_json, literals) = match &row.spec {
        Some(spec) => {
            let (masked, literals) = redact::mask_spec(spec);
            let json = serde_json::to_string(&masked)
                .map_err(|error| StoreError::of("writing a handoff", error))?;
            (Some(json), literals)
        }
        None => (None, Vec::new()),
    };
    let state_json = redact::sweep(&row.state_json, &literals);
    let request_text = row
        .request_text
        .as_ref()
        .map(|text| redact::sweep(text, &literals));

    db.conn()
        .execute(
            "INSERT INTO handoffs (\
                 id, created_at, closed_at, session_ref, agent_id, client_name, project_dir, \
                 request_text, state, final_state, spec_json, state_json, delivered_at, \
                 resumed_from_json, lang) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
             ON CONFLICT (id) DO UPDATE SET \
                 created_at = excluded.created_at, \
                 closed_at = excluded.closed_at, \
                 session_ref = excluded.session_ref, \
                 agent_id = excluded.agent_id, \
                 client_name = excluded.client_name, \
                 project_dir = excluded.project_dir, \
                 request_text = excluded.request_text, \
                 state = excluded.state, \
                 final_state = excluded.final_state, \
                 spec_json = excluded.spec_json, \
                 state_json = excluded.state_json, \
                 delivered_at = excluded.delivered_at, \
                 resumed_from_json = excluded.resumed_from_json, \
                 lang = excluded.lang",
            params![
                row.id,
                row.created_at,
                row.closed_at,
                row.session_ref,
                row.agent_id,
                row.client_name,
                row.project_dir,
                request_text,
                row.state,
                row.final_state,
                spec_json,
                state_json,
                row.delivered_at,
                row.resumed_from_json,
                row.lang,
            ],
        )
        .map_err(|error| StoreError::of("writing a handoff", error))?;
    Ok(())
}

/// Records that the final outcome reached an agent (SRV-23).
///
/// # Errors
///
/// [`StoreError::Persistence`] when the update fails.
pub fn mark_delivered(db: &Db, id: &str, at: &Timestamp) -> Result<bool> {
    let changed = db
        .conn()
        .execute(
            "UPDATE handoffs SET delivered_at = ?2 WHERE id = ?1",
            params![id, at],
        )
        .map_err(|error| StoreError::of("marking a handoff delivered", error))?;
    Ok(changed > 0)
}

/// One handoff, if it is there.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the row cannot be read.
pub fn get(db: &Db, id: &str) -> Result<Option<HandoffRow>> {
    db.conn()
        .query_row(
            "SELECT id, created_at, closed_at, session_ref, agent_id, client_name, \
                    project_dir, request_text, state, final_state, spec_json, state_json, \
                    delivered_at, resumed_from_json, lang \
             FROM handoffs WHERE id = ?1",
            [id],
            read_row,
        )
        .optional()
        .map_err(|error| StoreError::of("reading a handoff", error))?
        .transpose()
}

/// Every handoff that is not in a final state, oldest first: the tabs to restore at
/// startup (§7.2, FM-13).
///
/// # Errors
///
/// [`StoreError::Persistence`] when the rows cannot be read.
pub fn list_active(db: &Db) -> Result<Vec<HandoffRow>> {
    list(
        db,
        "WHERE final_state IS NULL ORDER BY created_at, id",
        &[],
        "listing the active handoffs",
    )
}

/// Every handoff that reached a final state, most recently closed first: the Log page.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the rows cannot be read.
pub fn list_final(db: &Db) -> Result<Vec<HandoffRow>> {
    list(
        db,
        "WHERE final_state IS NOT NULL ORDER BY closed_at DESC, id",
        &[],
        "listing the closed handoffs",
    )
}

/// Final outcomes nobody has collected for `ORPHAN_AGE_MS` (SRV-23).
///
/// The flag itself is computed, never stored (§7.4), so this is a query and not a column:
/// a handoff becomes an orphan by the passage of time and stops being one the moment an
/// agent resumes it.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the rows cannot be read.
pub fn list_orphan_candidates(db: &Db, now: &Timestamp) -> Result<Vec<HandoffRow>> {
    let cutoff = now.minus_millis(ORPHAN_AGE_MS);
    list(
        db,
        "WHERE final_state IS NOT NULL AND delivered_at IS NULL \
           AND closed_at IS NOT NULL AND closed_at <= ?1 \
         ORDER BY closed_at, id",
        &[&cutoff],
        "listing the orphan candidates",
    )
}

/// Deletes one handoff and, through the cascade, its rounds, events and sends (LOG-04).
///
/// Returns whether there was one to delete.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the deletion fails.
pub fn delete_handoff(db: &Db, id: &str) -> Result<bool> {
    let deleted = db
        .conn()
        .execute("DELETE FROM handoffs WHERE id = ?1", [id])
        .map_err(|error| StoreError::of("deleting a handoff", error))?;
    Ok(deleted > 0)
}

fn list(
    db: &Db,
    tail: &str,
    params: &[&dyn ToSql],
    operation: &'static str,
) -> Result<Vec<HandoffRow>> {
    let sql = format!(
        "SELECT id, created_at, closed_at, session_ref, agent_id, client_name, project_dir, \
                request_text, state, final_state, spec_json, state_json, delivered_at, \
                resumed_from_json, lang \
         FROM handoffs {tail}"
    );
    let mut statement = db
        .conn()
        .prepare(&sql)
        .map_err(|error| StoreError::of(operation, error))?;
    let rows = statement
        .query_map(params, read_row)
        .map_err(|error| StoreError::of(operation, error))?
        .collect::<rusqlite::Result<Vec<Result<HandoffRow>>>>()
        .map_err(|error| StoreError::of(operation, error))?;
    rows.into_iter().collect()
}

/// The inner `Result` is ours: a `spec_json` that no longer parses is a persistence
/// failure, and `rusqlite`'s row mapper has no room for one.
fn read_row(row: &Row<'_>) -> rusqlite::Result<Result<HandoffRow>> {
    let spec_json: Option<String> = row.get(10)?;
    let spec = match spec_json {
        Some(json) => match serde_json::from_str::<HandoffSpec>(&json) {
            Ok(spec) => Some(spec),
            Err(error) => return Ok(Err(StoreError::of("reading a handoff", error))),
        },
        None => None,
    };
    Ok(Ok(HandoffRow {
        id: row.get(0)?,
        created_at: row.get(1)?,
        closed_at: row.get(2)?,
        session_ref: row.get(3)?,
        agent_id: row.get(4)?,
        client_name: row.get(5)?,
        project_dir: row.get(6)?,
        request_text: row.get(7)?,
        state: row.get(8)?,
        final_state: row.get(9)?,
        spec,
        state_json: row.get(11)?,
        delivered_at: row.get(12)?,
        resumed_from_json: row.get(13)?,
        lang: row.get(14)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::testing::{handoff, spec_with_secret, STRIPE_KEY};

    #[test]
    fn every_state_of_the_design_survives_a_round_trip_and_knows_whether_it_is_final() {
        let db = Db::open_in_memory().expect("a database");
        let finals = [
            HandoffState::Verified,
            HandoffState::Failed,
            HandoffState::NotVerified,
            HandoffState::ConfirmedByUser,
            HandoffState::Abandoned,
        ];
        for state in [
            HandoffState::AwaitingSpec,
            HandoffState::Active,
            HandoffState::Deferred,
            HandoffState::Parked,
            HandoffState::AwaitingVerification,
            HandoffState::Verified,
            HandoffState::Failed,
            HandoffState::NotVerified,
            HandoffState::ConfirmedByUser,
            HandoffState::Abandoned,
        ] {
            assert_eq!(state.is_final(), finals.contains(&state), "{state:?}");
            assert_eq!(HandoffState::from_name(state.as_str()), Some(state));
            let mut row = handoff("hf_0123456789");
            row.state = state;
            row.final_state = state.is_final().then_some(state);
            upsert(&db, &row).expect("a handoff");
            let read = get(&db, "hf_0123456789").expect("a read").expect("a row");
            assert_eq!(read.state, state);
            assert_eq!(read.final_state, row.final_state);
        }
    }

    #[test]
    fn a_state_the_design_does_not_name_is_refused_by_the_database() {
        let db = Db::open_in_memory().expect("a database");
        let error = db
            .conn()
            .execute(
                "INSERT INTO handoffs (id, created_at, state, state_json) \
                 VALUES ('hf_0123456789', '2026-09-08T11:00:00.000Z', 'nearly_done', '{}')",
                [],
            )
            .expect_err("the CHECK refuses it");
        assert!(error.to_string().contains("CHECK"), "{error}");
    }

    #[test]
    fn an_upsert_replaces_rather_than_duplicates() {
        let db = Db::open_in_memory().expect("a database");
        let mut row = handoff("hf_0123456789");
        upsert(&db, &row).expect("a handoff");
        row.state = HandoffState::Deferred;
        row.lang = Some("it".to_owned());
        upsert(&db, &row).expect("the same handoff");
        let count: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM handoffs", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 1);
        let read = get(&db, "hf_0123456789").expect("a read").expect("a row");
        assert_eq!(read.state, HandoffState::Deferred);
        assert_eq!(read.lang.as_deref(), Some("it"));
    }

    #[test]
    fn the_stored_spec_is_the_masked_one_and_so_is_the_state_it_travels_with() {
        let db = Db::open_in_memory().expect("a database");
        let mut row = handoff("hf_0123456789");
        row.spec = Some(spec_with_secret());
        row.state_json = format!("{{\"copy_of\":\"{STRIPE_KEY}\"}}");
        row.request_text = Some(format!("rotate {STRIPE_KEY} please"));
        upsert(&db, &row).expect("a handoff");

        let stored: String = db
            .conn()
            .query_row(
                "SELECT spec_json || state_json || request_text FROM handoffs WHERE id = ?1",
                ["hf_0123456789"],
                |r| r.get(0),
            )
            .expect("the stored text");
        assert!(!stored.contains(STRIPE_KEY), "{stored}");
        assert!(stored.contains("[treated as secret: api_key]"));

        let read = get(&db, "hf_0123456789").expect("a read").expect("a row");
        let spec = read.spec.expect("a spec");
        assert!(!serde_json::to_string(&spec)
            .expect("json")
            .contains(STRIPE_KEY));
    }

    #[test]
    fn active_and_final_are_the_two_halves_of_the_table() {
        let db = Db::open_in_memory().expect("a database");
        let mut open = handoff("hf_0000000001");
        open.created_at = Timestamp::parse("2026-09-01T10:00:00Z").expect("rfc 3339");
        upsert(&db, &open).expect("an active handoff");

        let mut closed = handoff("hf_0000000002");
        closed.state = HandoffState::Verified;
        closed.final_state = Some(HandoffState::Verified);
        closed.closed_at = Some(Timestamp::parse("2026-09-02T10:00:00Z").expect("rfc 3339"));
        upsert(&db, &closed).expect("a closed handoff");

        let active = list_active(&db).expect("the active list");
        assert_eq!(
            active.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["hf_0000000001"]
        );
        let final_rows = list_final(&db).expect("the final list");
        assert_eq!(
            final_rows
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["hf_0000000002"]
        );
    }

    #[test]
    fn an_orphan_is_final_undelivered_and_seven_days_old() {
        let db = Db::open_in_memory().expect("a database");
        let now = Timestamp::parse("2026-09-08T12:00:00Z").expect("rfc 3339");

        let mut old = handoff("hf_0000000001");
        old.state = HandoffState::Verified;
        old.final_state = Some(HandoffState::Verified);
        old.closed_at = Some(now.minus_millis(ORPHAN_AGE_MS + 1));
        upsert(&db, &old).expect("an old handoff");

        let mut just_young_enough = handoff("hf_0000000002");
        just_young_enough.state = HandoffState::Verified;
        just_young_enough.final_state = Some(HandoffState::Verified);
        just_young_enough.closed_at = Some(now.minus_millis(ORPHAN_AGE_MS - 1));
        upsert(&db, &just_young_enough).expect("a recent handoff");

        let mut collected = handoff("hf_0000000003");
        collected.state = HandoffState::Verified;
        collected.final_state = Some(HandoffState::Verified);
        collected.closed_at = Some(now.minus_millis(ORPHAN_AGE_MS * 2));
        collected.delivered_at = Some(now.clone());
        upsert(&db, &collected).expect("a delivered handoff");

        let orphans = list_orphan_candidates(&db, &now).expect("the orphan list");
        assert_eq!(
            orphans
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["hf_0000000001"]
        );

        // Delivering it is what stops it being one; nothing is stored on the row.
        assert!(mark_delivered(&db, "hf_0000000001", &now).expect("an update"));
        assert!(list_orphan_candidates(&db, &now)
            .expect("the orphan list")
            .is_empty());
        assert!(!mark_delivered(&db, "hf_nosuchthing", &now).expect("no such row"));
    }
}
