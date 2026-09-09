//! Reading and writing an agent's configuration file without disturbing it (INST-04).
//!
//! Every adapter of §7.15 edits a JSON file somebody else owns: `~/.claude.json` is Claude
//! Code's own state file, `settings.json` is the user's. INST-04 is the promise that this
//! never costs them anything — "existing user configuration is never replaced" — so the
//! rules here are narrower than "parse and re-serialise":
//!
//! - **Key order is preserved.** `serde_json` is compiled with `preserve_order`, so a
//!   document round-trips in the order it was written and setting an existing key leaves it
//!   where it was. A new key is appended, which is the only place a new key can go.
//! - **The file's own indentation is preserved.** A file indented with four spaces or with
//!   tabs is written back the same way ([`Indent::sniff`]), so the diff a user sees in
//!   `git diff` or in their editor is our three keys and nothing else.
//! - **A file that is not JSON is never repaired.** It is an error, and the caller stops.
//!   Overwriting a configuration we could not read is the one failure INST-04 cannot
//!   survive.
//!
//! What is *not* preserved is the rest of the whitespace: blank lines between keys and
//! comments (which JSON has none of, but JSONC files in the wild do) do not survive a
//! parse. The adapters' golden files pin the round trip that matters — a canonically
//! formatted file comes back byte-identical after an install and an uninstall.

use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use super::error::{InstallError, Result};

/// How a file indents, so it can be written back the way it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indent(String);

impl Default for Indent {
    /// Two spaces: what `serde_json`, `npm`, `prettier` and Claude Code itself write.
    fn default() -> Self {
        Self("  ".to_owned())
    }
}

impl Indent {
    /// The indentation of the first indented line of `source`, or the default.
    ///
    /// One line is enough and more would be worse: a file mixing tabs and spaces has no
    /// single answer, and picking the first is at least the answer its author would have
    /// got from their editor.
    #[must_use]
    pub fn sniff(source: &str) -> Self {
        for line in source.lines() {
            let whitespace: String = line
                .chars()
                .take_while(|character| *character == ' ' || *character == '\t')
                .collect();
            // A line that is *only* whitespace says nothing about indentation.
            if !whitespace.is_empty() && whitespace.len() < line.len() {
                return Self(whitespace);
            }
        }
        Self::default()
    }

    /// The characters one level of indentation is made of.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A JSON configuration file, as read from disk.
#[derive(Debug, Clone)]
pub struct Document {
    /// The parsed object. A file holding anything but an object is refused on the way in.
    root: Map<String, Value>,
    /// How the file indented itself, so it can be written back the same way.
    indent: Indent,
    /// Whether the file existed at all. An absent file is an empty object that has never
    /// been written, which is what tells [`super::apply`] there is nothing to back up.
    existed: bool,
}

impl Document {
    /// The document `path` holds, or an empty one when the file is not there.
    ///
    /// # Errors
    ///
    /// [`InstallError::Unreadable`] when the file exists and cannot be read,
    /// [`InstallError::Malformed`] when it is not a JSON object.
    pub fn read(path: &Path) -> Result<Self> {
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    root: Map::new(),
                    indent: Indent::default(),
                    existed: false,
                })
            }
            Err(error) => return Err(InstallError::unreadable(path, error)),
        };

        // An empty file is a file somebody truncated, and every one of these tools treats
        // it as "no configuration". Refusing it would leave the user with an error they can
        // only fix by deleting the file we refused to touch.
        let indent = Indent::sniff(&source);
        if source.trim().is_empty() {
            return Ok(Self {
                root: Map::new(),
                indent,
                existed: true,
            });
        }

        let value: Value =
            serde_json::from_str(&source).map_err(|error| InstallError::malformed(path, error))?;
        let Value::Object(root) = value else {
            return Err(InstallError::not_an_object(path));
        };
        Ok(Self {
            root,
            indent,
            existed: true,
        })
    }

    /// Whether the file was on disk when it was read.
    #[must_use]
    pub fn existed(&self) -> bool {
        self.existed
    }

    /// The value at `path`, or nothing when any segment of it is missing.
    #[must_use]
    pub fn get_at(&self, path: &[&str]) -> Option<&Value> {
        let (first, rest) = path.split_first()?;
        let mut current = self.root.get(*first)?;
        for segment in rest {
            current = current.as_object()?.get(*segment)?;
        }
        Some(current)
    }

    /// Puts `value` at `path`, creating the objects on the way.
    ///
    /// A segment that exists but is not an object is replaced, which cannot silently lose a
    /// user's configuration: the caller has already compared the current value against the
    /// `before` of the modification it is applying, so a mismatch stopped it earlier.
    pub fn set_at(&mut self, path: &[&str], value: Value) {
        let Some((last, parents)) = path.split_last() else {
            return;
        };
        let mut current = &mut self.root;
        for segment in parents {
            let entry = current
                .entry((*segment).to_owned())
                .or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            current = entry
                .as_object_mut()
                .expect("the entry was just made an object");
        }
        current.insert((*last).to_owned(), value);
    }

    /// Removes the value at `path`, and every object it leaves empty **that we created**.
    ///
    /// The second half is what makes an uninstall byte-identical: `hooks` did not exist
    /// before the install, so leaving `"hooks": {}` behind would be a line the user has to
    /// delete by hand. A parent that still holds anything of the user's is left alone.
    pub fn remove_at(&mut self, path: &[&str]) {
        let Some((last, parents)) = path.split_last() else {
            return;
        };
        let mut current = &mut self.root;
        // Walk down, remembering nothing: the pruning below re-walks, which is cheap on a
        // path of two segments and much easier to read than a stack of raw pointers.
        for segment in parents {
            match current.get_mut(*segment).and_then(Value::as_object_mut) {
                Some(child) => current = child,
                None => return,
            }
        }
        current.remove(*last);

        for depth in (1..=parents.len()).rev() {
            let prefix = &parents[..depth];
            if self
                .get_at(prefix)
                .and_then(Value::as_object)
                .is_some_and(Map::is_empty)
            {
                self.remove_at(prefix);
            }
        }
    }

    /// The document as it would be written: pretty-printed with the file's own indentation
    /// and a trailing newline.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = render_with_indent(&Value::Object(self.root.clone()), self.indent.as_str());
        out.push('\n');
        out
    }

    /// Writes the document to `path`, creating the folders above it.
    ///
    /// # Errors
    ///
    /// [`InstallError::Unwritable`] when the folder or the file cannot be written.
    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| InstallError::unwritable(parent, error))?;
        }
        fs::write(path, self.render()).map_err(|error| InstallError::unwritable(path, error))
    }
}

/// Pretty-prints `value` with `indent` as one level of indentation.
///
/// `serde_json::to_string_pretty` is two spaces and nothing else, so the formatter is built
/// by hand; the byte string it takes must outlive the serializer, which is why `indent` is
/// borrowed rather than owned.
#[must_use]
pub fn render_with_indent(value: &Value, indent: &str) -> String {
    use serde::Serialize as _;

    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut buffer = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, formatter);
    value
        .serialize(&mut serializer)
        .expect("a JSON value serialises into a vector");
    String::from_utf8(buffer).expect("serde_json writes UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document(source: &str) -> Document {
        Document {
            root: serde_json::from_str(source).expect("the fixture is an object"),
            indent: Indent::sniff(source),
            existed: true,
        }
    }

    #[test]
    fn the_indentation_of_the_file_is_the_indentation_it_is_written_back_with() {
        assert_eq!(Indent::sniff("{\n    \"a\": 1\n}").as_str(), "    ");
        assert_eq!(Indent::sniff("{\n\t\"a\": 1\n}").as_str(), "\t");
        assert_eq!(Indent::sniff("{\n  \"a\": 1\n}").as_str(), "  ");
        // Nothing to learn from: a one-line file, and a file whose only indented line is
        // blank.
        assert_eq!(Indent::sniff("{\"a\":1}").as_str(), "  ");
        assert_eq!(Indent::sniff("{\n   \n}").as_str(), "  ");
    }

    #[test]
    fn a_missing_file_reads_as_an_empty_document_that_was_never_written() {
        let dir = std::env::temp_dir().join(format!("handoff-json-{}", std::process::id()));
        let document = Document::read(&dir.join("nowhere.json")).expect("an absent file is empty");
        assert!(!document.existed());
        assert_eq!(document.render(), "{}\n");
    }

    #[test]
    fn a_file_that_is_not_json_is_refused_rather_than_repaired() {
        let dir = std::env::temp_dir().join(format!(
            "handoff-json-bad-{}-{}",
            std::process::id(),
            crate::ids::new_session_ref()
        ));
        fs::create_dir_all(&dir).expect("the folder is created");
        let path = dir.join("broken.json");

        fs::write(&path, "{ not json").expect("the file is written");
        assert!(matches!(
            Document::read(&path),
            Err(InstallError::Malformed { .. })
        ));

        fs::write(&path, "[1, 2, 3]").expect("the file is written");
        assert!(matches!(
            Document::read(&path),
            Err(InstallError::Malformed { .. })
        ));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_file_reads_as_no_configuration() {
        let dir = std::env::temp_dir().join(format!(
            "handoff-json-empty-{}-{}",
            std::process::id(),
            crate::ids::new_session_ref()
        ));
        fs::create_dir_all(&dir).expect("the folder is created");
        let path = dir.join("empty.json");
        fs::write(&path, "   \n").expect("the file is written");

        let document = Document::read(&path).expect("an empty file is an empty object");
        assert!(document.existed());
        assert_eq!(document.render(), "{}\n");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn setting_a_key_keeps_the_order_of_the_file() {
        let mut document = document("{\n  \"zebra\": 1,\n  \"alpha\": 2\n}");
        document.set_at(&["alpha"], json!(3));
        document.set_at(&["middle"], json!(4));
        assert_eq!(
            document.render(),
            "{\n  \"zebra\": 1,\n  \"alpha\": 3,\n  \"middle\": 4\n}\n"
        );
    }

    #[test]
    fn setting_a_nested_key_creates_the_objects_above_it() {
        let mut document = document("{\n  \"kept\": true\n}");
        document.set_at(&["hooks", "Stop"], json!([1]));
        assert_eq!(document.get_at(&["hooks", "Stop"]), Some(&json!([1])));
        assert_eq!(document.get_at(&["hooks", "Nothing"]), None);
        assert_eq!(document.get_at(&["nothing", "at", "all"]), None);
    }

    #[test]
    fn removing_the_last_key_of_an_object_removes_the_object_too() {
        let mut document = document("{\n  \"kept\": true\n}");
        document.set_at(&["hooks", "Stop"], json!([1]));
        document.remove_at(&["hooks", "Stop"]);
        assert_eq!(document.render(), "{\n  \"kept\": true\n}\n");
    }

    #[test]
    fn removing_a_key_leaves_a_parent_that_still_holds_something_of_the_users() {
        let mut document = document("{\n  \"hooks\": {\n    \"PreToolUse\": []\n  }\n}");
        document.set_at(&["hooks", "Stop"], json!([1]));
        document.remove_at(&["hooks", "Stop"]);
        assert_eq!(
            document.render(),
            "{\n  \"hooks\": {\n    \"PreToolUse\": []\n  }\n}\n"
        );
    }

    #[test]
    fn removing_a_key_that_is_not_there_changes_nothing() {
        let mut document = document("{\n  \"kept\": true\n}");
        document.remove_at(&["hooks", "Stop"]);
        document.remove_at(&["kept", "deeper"]);
        assert_eq!(document.render(), "{\n  \"kept\": true\n}\n");
    }

    #[test]
    fn a_four_space_file_is_written_back_with_four_spaces() {
        let mut document = document("{\n    \"kept\": true\n}");
        document.set_at(&["added"], json!("value"));
        assert_eq!(
            document.render(),
            "{\n    \"kept\": true,\n    \"added\": \"value\"\n}\n"
        );
    }
}
