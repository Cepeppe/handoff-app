//! Display paths (§4.7.5, `vendor/handoff-mcp/format/schemas/README.md`).
//!
//! A problem cites the offending location in the notation the design uses in its error
//! texts — `goal`, `steps[0].text`, `values.events[0]` — which is the JSON pointer of the
//! location written with dots and brackets, indexed from 0 like the document itself. The
//! 1-based step numbers a person sees in the overlay are the step counter, not this.
//!
//! The server writes the same paths from ajv's `instancePath` (`src/format/paths.ts`), and
//! `<name>.expected.json` next to every invalid spec fixture is the contract both sides
//! answer. Keep the two in step: a path that differs is two implementations disagreeing
//! about which field is wrong.

use jsonschema::paths::{Location, LocationSegment};

/// One step down a display path: a field name or an array index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment<'a> {
    /// A field of an object, appended after a dot (or bare at the root).
    Field(&'a str),
    /// An index into an array, appended in brackets.
    Index(usize),
}

/// Appends one field name or one array index to a display path.
#[must_use]
pub fn child_path(parent: &str, segment: Segment<'_>) -> String {
    match segment {
        Segment::Index(index) => format!("{parent}[{index}]"),
        Segment::Field(name) if parent.is_empty() => name.to_owned(),
        Segment::Field(name) => format!("{parent}.{name}"),
    }
}

/// Turns a validator's instance location into a display path.
///
/// `root` is prepended, so the same rules can report on a spec (root `""`, giving `goal`)
/// and on a spec nested in a channel message or an outcome (root `spec`, giving
/// `spec.goal`).
#[must_use]
pub fn display_path(location: &Location, root: &str) -> String {
    let mut path = root.to_owned();
    for segment in location.iter() {
        path = match segment {
            LocationSegment::Property(name) => child_path(&path, Segment::Field(name.as_ref())),
            LocationSegment::Index(index) => child_path(&path, Segment::Index(index)),
        };
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_fields_with_dots_and_indices_with_brackets() {
        let path = child_path("", Segment::Field("steps"));
        let path = child_path(&path, Segment::Index(0));
        let path = child_path(&path, Segment::Field("text"));
        assert_eq!(path, "steps[0].text");
    }

    #[test]
    fn a_field_at_the_root_carries_no_leading_dot() {
        assert_eq!(child_path("", Segment::Field("goal")), "goal");
    }

    #[test]
    fn an_index_follows_its_field_without_a_dot() {
        let path = child_path("values", Segment::Field("events"));
        assert_eq!(child_path(&path, Segment::Index(0)), "values.events[0]");
    }
}
