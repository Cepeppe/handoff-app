//! The one shape an instant has on disk.
//!
//! Every instant column of the schema holds canonical RFC 3339 in UTC with milliseconds:
//! `2026-09-08T12:34:56.789Z`. One form, always, because two of the rules the log has to
//! apply are age comparisons — the orphan flag of SRV-23 (`ORPHAN_AGE_MS`, 7 days) and the
//! session purge of §8.3 — and they are cheapest and most obviously correct as a `<` on
//! text. That only holds while every value has the same offset, the same precision and the
//! same width, which is what [`Timestamp`] guarantees: a value reaches a column through
//! this type or not at all.
//!
//! An instant arriving from outside — a `verify_reported_at` an agent wrote, an outcome's
//! `at` — may carry any offset the formats allow, so [`Timestamp::parse`] converts rather
//! than refuses. `13:00+02:00` and `11:00Z` are the same instant and become the same text.
//!
//! `chrono` is compiled here without its `clock` feature (it would pull a time-zone
//! database this crate has no use for), so "now" comes from `SystemTime` and is converted.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::ToSql;
use serde::{Deserialize, Serialize};

use super::error::{PersistenceCause, StoreError};

/// An instant, in the only form the database stores.
///
/// It serialises as its string, so an exported row reads as the instant it is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(String);

impl Timestamp {
    /// Now, from the system clock.
    #[must_use]
    pub fn now() -> Self {
        // Before 1970 the clock is broken beyond anything this function could repair; the
        // epoch is the answer that keeps the ordering of everything written afterwards.
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(0));
        Self::from_millis(millis)
    }

    /// An RFC 3339 instant from anywhere, converted to the canonical form.
    ///
    /// # Errors
    ///
    /// [`StoreError::Persistence`] with a [`PersistenceCause::Timestamp`] when the text is
    /// not RFC 3339.
    pub fn parse(text: &str) -> Result<Self, StoreError> {
        DateTime::parse_from_rfc3339(text)
            .map(|parsed| Self::from(parsed.with_timezone(&Utc)))
            .map_err(|_| {
                StoreError::of(
                    "reading an instant",
                    PersistenceCause::Timestamp(text.to_owned()),
                )
            })
    }

    /// Milliseconds since the epoch, as the canonical form.
    #[must_use]
    pub fn from_millis(millis: i64) -> Self {
        // `from_timestamp_millis` only refuses values far outside the representable range;
        // the epoch is again the answer that keeps ordering sane.
        let moment = DateTime::from_timestamp_millis(millis).unwrap_or_else(|| {
            DateTime::from_timestamp_millis(0).expect("the epoch is representable")
        });
        Self::from(moment)
    }

    /// This instant moved back by `millis`, for the two age rules of this module.
    #[must_use]
    pub fn minus_millis(&self, millis: i64) -> Self {
        Self::from_millis(self.millis().saturating_sub(millis))
    }

    /// Milliseconds since the epoch.
    #[must_use]
    pub fn millis(&self) -> i64 {
        DateTime::parse_from_rfc3339(&self.0)
            .map(|parsed| parsed.timestamp_millis())
            .unwrap_or_default()
    }

    /// The stored text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<DateTime<Utc>> for Timestamp {
    fn from(moment: DateTime<Utc>) -> Self {
        Self(moment.to_rfc3339_opts(SecondsFormat::Millis, true))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl ToSql for Timestamp {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for Timestamp {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        Self::parse(text).map_err(|error| FromSqlError::Other(Box::new(error)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_canonical_form_is_utc_with_milliseconds_and_a_z() {
        assert_eq!(
            Timestamp::from_millis(0).as_str(),
            "1970-01-01T00:00:00.000Z"
        );
        assert_eq!(
            Timestamp::from_millis(1_000).as_str(),
            "1970-01-01T00:00:01.000Z"
        );
        // Milliseconds are kept, and they are exactly three digits.
        assert_eq!(
            Timestamp::from_millis(86_400_000 + 7).as_str(),
            "1970-01-02T00:00:00.007Z"
        );
    }

    #[test]
    fn millis_and_the_text_are_the_same_instant_in_both_directions() {
        let moment = Timestamp::parse("2026-09-08T11:00:00.250Z").expect("rfc 3339");
        assert_eq!(Timestamp::from_millis(moment.millis()), moment);
    }

    #[test]
    fn an_offset_is_converted_rather_than_refused() {
        // The same instant written three ways reaches the same column value, which is what
        // makes a `<` comparison between two rows mean what it says.
        let with_offset = Timestamp::parse("2026-09-08T13:00:00+02:00").expect("rfc 3339");
        let in_utc = Timestamp::parse("2026-09-08T11:00:00Z").expect("rfc 3339");
        let with_more_precision =
            Timestamp::parse("2026-09-08T11:00:00.000000Z").expect("rfc 3339");
        assert_eq!(with_offset, in_utc);
        assert_eq!(with_more_precision, in_utc);
        assert_eq!(in_utc.as_str(), "2026-09-08T11:00:00.000Z");
    }

    #[test]
    fn text_order_is_chronological_order() {
        // The whole reason for one canonical form: the age rules compare text.
        let earlier = Timestamp::parse("2026-09-01T23:59:59.999Z").expect("rfc 3339");
        let later = Timestamp::parse("2026-09-02T00:00:00.000Z").expect("rfc 3339");
        assert!(earlier.as_str() < later.as_str());
        assert!(earlier < later);
    }

    #[test]
    fn something_that_is_not_an_instant_is_refused() {
        let error = Timestamp::parse("last tuesday").expect_err("not an instant");
        assert!(error.to_string().contains("reading an instant"));
    }

    #[test]
    fn seven_days_back_is_seven_days_back() {
        let now = Timestamp::parse("2026-09-08T11:00:00Z").expect("rfc 3339");
        assert_eq!(
            now.minus_millis(7 * 24 * 60 * 60 * 1000).as_str(),
            "2026-09-01T11:00:00.000Z"
        );
    }

    #[test]
    fn now_is_after_the_epoch_and_round_trips() {
        let now = Timestamp::now();
        assert!(now.as_str() > "2020-01-01T00:00:00.000Z");
        assert_eq!(Timestamp::parse(now.as_str()).expect("canonical"), now);
    }
}
