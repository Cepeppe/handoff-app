//! The `sends` table: everything that left the machine towards an agent (LOG-03).
//!
//! Append-only, and the one table with a rule stronger than "record it": **never pixels**.
//! An image is recorded as its hash, its dimensions and the boxes that were redacted over
//! it; the bytes stay where the user captured them. The schema says so with a length check
//! on `image_sha256` — 64 hexadecimal characters cannot be an encoded image — and this
//! module says so by having no column to put them in.
//!
//! `text_as_sent` is the text **after** redaction, exactly as it left (LOG-03, PREV-01): it
//! is the answer to "what did the agent actually see", which is the only question this
//! table exists to answer.

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{params, Row, ToSql};
use serde::{Deserialize, Serialize};

use super::db::Db;
use super::error::{PersistenceCause, Result, StoreError};
use super::time::Timestamp;

/// What kind of send it was (§7.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendKind {
    /// A question from the user to the agent (RESP-04).
    Question,
    /// A screenshot sent as the text extracted from it (PREV-04, PREV-05).
    ScreenshotText,
    /// A screenshot sent as an image (PREV-04).
    ScreenshotImage,
    /// A deferral (RESP-05).
    Defer,
    /// An abandonment (RESP-07).
    Abandon,
}

impl SendKind {
    /// The column name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::ScreenshotText => "screenshot_text",
            Self::ScreenshotImage => "screenshot_image",
            Self::Defer => "defer",
            Self::Abandon => "abandon",
        }
    }

    /// The kind named by `text`, if it is one.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        ALL_KINDS.iter().copied().find(|kind| kind.as_str() == text)
    }
}

/// The five kinds, in the order §7.11 prints them.
pub const ALL_KINDS: [SendKind; 5] = [
    SendKind::Question,
    SendKind::ScreenshotText,
    SendKind::ScreenshotImage,
    SendKind::Defer,
    SendKind::Abandon,
];

impl ToSql for SendKind {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for SendKind {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        Self::from_name(text).ok_or_else(|| {
            FromSqlError::Other(Box::new(StoreError::of(
                "reading a send",
                PersistenceCause::Schema(format!("{text} is not a send kind of the design")),
            )))
        })
    }
}

/// One row of `sends`. `id` is assigned by the database on append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendRow {
    /// Assigned on append; zero on the value handed to [`append`].
    pub id: i64,
    /// The handoff it belonged to.
    pub handoff_id: String,
    /// When it left.
    pub at: Timestamp,
    /// What kind of send it was.
    pub kind: SendKind,
    /// The text exactly as it left, after redaction (LOG-03).
    pub text_as_sent: Option<String>,
    /// 64 lowercase hexadecimal characters, or nothing. **Never the image.**
    pub image_sha256: Option<String>,
    /// Pixel width of what was captured.
    pub image_w: Option<i64>,
    /// Pixel height of what was captured.
    pub image_h: Option<i64>,
    /// The boxes that were redacted over it, as the preview recorded them.
    pub redaction_boxes_json: Option<String>,
    /// Which OCR engine produced the text (§7.9).
    pub ocr_engine: Option<String>,
    /// The version of the certain-secret pattern file that was applied (§4.6).
    pub patterns_version: Option<String>,
}

/// Appends a send and returns the id the database gave it.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the write fails; an `image_sha256` that is not 64
/// characters is refused by the schema, and so is a send of a handoff that does not exist.
pub fn append(db: &Db, row: &SendRow) -> Result<i64> {
    db.conn()
        .execute(
            "INSERT INTO sends (\
                 handoff_id, at, kind, text_as_sent, image_sha256, image_w, image_h, \
                 redaction_boxes_json, ocr_engine, patterns_version) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                row.handoff_id,
                row.at,
                row.kind,
                row.text_as_sent,
                row.image_sha256,
                row.image_w,
                row.image_h,
                row.redaction_boxes_json,
                row.ocr_engine,
                row.patterns_version,
            ],
        )
        .map_err(|error| StoreError::of("appending a send", error))?;
    Ok(db.conn().last_insert_rowid())
}

/// Every send of a handoff, oldest first.
///
/// # Errors
///
/// [`StoreError::Persistence`] when the rows cannot be read.
pub fn list_for_handoff(db: &Db, handoff_id: &str) -> Result<Vec<SendRow>> {
    let mut statement = db
        .conn()
        .prepare(
            "SELECT id, handoff_id, at, kind, text_as_sent, image_sha256, image_w, image_h, \
                    redaction_boxes_json, ocr_engine, patterns_version \
             FROM sends WHERE handoff_id = ?1 ORDER BY id",
        )
        .map_err(|error| StoreError::of("reading the sends", error))?;
    let rows = statement
        .query_map([handoff_id], read_row)
        .map_err(|error| StoreError::of("reading the sends", error))?
        .collect::<rusqlite::Result<Vec<SendRow>>>()
        .map_err(|error| StoreError::of("reading the sends", error))?;
    Ok(rows)
}

fn read_row(row: &Row<'_>) -> rusqlite::Result<SendRow> {
    Ok(SendRow {
        id: row.get(0)?,
        handoff_id: row.get(1)?,
        at: row.get(2)?,
        kind: row.get(3)?,
        text_as_sent: row.get(4)?,
        image_sha256: row.get(5)?,
        image_w: row.get(6)?,
        image_h: row.get(7)?,
        redaction_boxes_json: row.get(8)?,
        ocr_engine: row.get(9)?,
        patterns_version: row.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::handoffs;
    use crate::log::testing::{handoff, send};

    #[test]
    fn every_kind_of_the_design_round_trips_and_an_image_is_a_hash() {
        let db = Db::open_in_memory().expect("a database");
        handoffs::upsert(&db, &handoff("hf_0123456789")).expect("a handoff");
        for kind in ALL_KINDS {
            append(&db, &send("hf_0123456789", kind)).expect("a send");
        }
        let sends = list_for_handoff(&db, "hf_0123456789").expect("the sends");
        assert_eq!(
            sends.iter().map(|row| row.kind).collect::<Vec<_>>(),
            ALL_KINDS.to_vec()
        );
        let image = sends
            .iter()
            .find(|row| row.kind == SendKind::ScreenshotImage)
            .expect("the image send");
        assert_eq!(image.image_sha256.as_ref().expect("a hash").len(), 64);
        assert_eq!(image.image_w, Some(1600));
    }

    #[test]
    fn anything_longer_than_a_hash_is_refused_in_the_image_column() {
        // The cheap half of "never pixels" (LOG-03): a base64 image cannot pass this.
        let db = Db::open_in_memory().expect("a database");
        handoffs::upsert(&db, &handoff("hf_0123456789")).expect("a handoff");
        let mut row = send("hf_0123456789", SendKind::ScreenshotImage);
        row.image_sha256 = Some("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQ".to_owned());
        let error = append(&db, &row).expect_err("the CHECK refuses it");
        assert!(error.to_string().contains("appending a send"), "{error}");
    }
}
