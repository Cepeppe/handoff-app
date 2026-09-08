//! Reading the vendored fixtures, and nothing else.
//!
//! Every path this suite touches is under `vendor/handoff-mcp/format/`, the unpacked format
//! tarball of the pinned `handoff-mcp` release (§3.5). That is the point of these tests: the
//! app is checked against the artifact it will actually ship with, not against a copy of the
//! fixtures kept here, which is the drift `handoff-app` has no way to notice (§3.4).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// `handoff-app/vendor/handoff-mcp/format/`.
pub fn format_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("vendor")
        .join("handoff-mcp")
        .join("format")
}

/// A folder under `vendor/handoff-mcp/format/`, e.g. `fixtures/specs/valid`.
pub fn fixture_dir(relative: &str) -> PathBuf {
    let mut path = format_dir();
    for segment in relative.split('/') {
        path.push(segment);
    }
    path
}

/// Reads a file, naming it when it cannot be read: an absent fixture means `vendor/` is not
/// filled, and the message has to say so rather than print an io error alone.
pub fn read_to_string(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "{} could not be read ({error}). Fill vendor/ with `node scripts/fetch-server.mjs`.",
            path.display()
        )
    })
}

/// Reads a JSON file into a `Value`.
pub fn read_json(path: &Path) -> Value {
    serde_json::from_str(&read_to_string(path))
        .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()))
}

/// The names of the files of a fixture folder, sorted, filtered by suffix.
///
/// Sorted so a failure names the same file on every machine and on both runners; a
/// directory listing has no order of its own.
pub fn fixture_files(relative: &str, suffix: &str) -> Vec<PathBuf> {
    let dir = fixture_dir(relative);
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|error| {
            panic!(
                "{} could not be listed ({error}). Fill vendor/ with \
                 `node scripts/fetch-server.mjs`.",
                dir.display()
            )
        })
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(suffix))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "{} holds no {suffix}", dir.display());
    files
}

/// The file name, for a message that has to name which fixture failed.
pub fn name_of(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned()
}

/// Serialises a value back and compares it with what was read.
///
/// `serde_json` is built with `preserve_order`, so its object is an `IndexMap` and its
/// equality is order-insensitive: a round trip is judged on the fields and their values,
/// never on the order the file happened to write them in.
pub fn assert_round_trips<T>(original: &Value, what: &str)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let parsed: T = serde_json::from_value(original.clone())
        .unwrap_or_else(|error| panic!("{what} does not deserialise: {error}"));
    let written = serde_json::to_value(&parsed)
        .unwrap_or_else(|error| panic!("{what} does not serialise: {error}"));
    assert_eq!(&written, original, "{what} does not round-trip");
}
