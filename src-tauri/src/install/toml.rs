//! Reading and writing an agent's TOML configuration without disturbing it (INST-04).
//!
//! Codex keeps its MCP servers in `config.toml`, a file people edit by hand and annotate with
//! comments. INST-04 is the promise that registering Baton there costs them nothing, and for
//! TOML the promise can be kept more strictly than for JSON: `toml_edit` holds every comment,
//! every blank line and the spelling of every value it is not asked to change, so what changes
//! in the file is our section and nothing else.
//!
//! - **Only our section is rewritten.** Replacing an existing `[mcp_servers.handoff]` keeps its
//!   place in the file and the comment above its header; a new one goes after the sections
//!   already under the same parent, and a parent we had to create has no header of its own.
//! - **A value is compared in a canonical form.** The plan shows `before` and `after` and calls
//!   a modification a no-op when the two are equal, so both are rendered from their values
//!   alone ([`render_at`]): no comment, no spelling of the user's, no position. A section we
//!   wrote reads back as exactly what we wrote, whatever else the file holds.
//! - **A file that is not TOML is never repaired.** It is an error and the caller stops, and the
//!   error names the line and the reason but never quotes the line: another server's section
//!   may hold a key, and the message reaches a log (R-19).
//! - **Line endings are the file's.** A file written with CRLF gets our lines with CRLF too, so
//!   the user's editor does not find a file of mixed endings after an install.

use std::fs;
use std::path::Path;

use toml_edit::{Decor, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use super::error::{InstallError, Result};

/// A TOML configuration file, as read from disk.
#[derive(Debug, Clone)]
pub struct Document {
    /// The parsed document, formatting included.
    doc: DocumentMut,
    /// Whether every line of the file ended with CRLF, so the lines we add should too.
    crlf: bool,
    /// Whether the file existed at all. An absent file is an empty document that has never
    /// been written, which is what tells [`super::apply`] there is nothing to back up.
    existed: bool,
}

impl Document {
    /// A document with nothing in it and no file behind it.
    fn empty() -> Self {
        Self {
            doc: DocumentMut::new(),
            crlf: false,
            existed: false,
        }
    }

    /// The document `path` holds, or an empty one when the file is not there.
    ///
    /// An empty file is an empty document: TOML has nothing to say about a file with no
    /// lines, and every tool that reads one treats it as "no configuration".
    ///
    /// # Errors
    ///
    /// [`InstallError::Unreadable`] when the file exists and cannot be read,
    /// [`InstallError::Malformed`] when it is not TOML.
    pub fn read(path: &Path) -> Result<Self> {
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::empty()),
            Err(error) => return Err(InstallError::unreadable(path, error)),
        };
        let doc = source
            .parse::<DocumentMut>()
            .map_err(|error| InstallError::malformed_toml(path, &error, &source))?;
        Ok(Self {
            doc,
            crlf: uses_crlf(&source),
            existed: true,
        })
    }

    /// Whether the file was on disk when it was read.
    #[must_use]
    pub fn existed(&self) -> bool {
        self.existed
    }

    /// The item at `path`, or nothing when any segment of it is missing.
    #[must_use]
    pub fn get_at(&self, path: &[&str]) -> Option<&Item> {
        let (first, rest) = path.split_first()?;
        let mut current = self.doc.as_table().get(first)?;
        for segment in rest {
            current = current.as_table_like()?.get(segment)?;
        }
        Some(current)
    }

    /// The item at `path` in the canonical rendering of [`render_at`], or nothing.
    #[must_use]
    pub fn rendered_at(&self, path: &[&str]) -> Option<String> {
        self.get_at(path).map(|item| render_at(path, item))
    }

    /// Whether `path` is absent or a `[section]` of its own.
    ///
    /// The one shape of a parent this module writes under. The others — an inline table
    /// (`mcp_servers = { … }`), dotted keys (`mcp_servers.x.command = …`) or a plain value —
    /// are the user's own spelling, and turning them into sections would be rewriting their
    /// configuration rather than adding to it.
    #[must_use]
    pub fn is_section_or_absent(&self, path: &[&str]) -> bool {
        match self.get_at(path) {
            None => true,
            Some(Item::Table(table)) => !table.is_dotted(),
            Some(_) => false,
        }
    }

    /// Puts `table` at `path` as a `[section]`, creating the parents on the way.
    ///
    /// A parent that does not exist is created **implicit**, so it gets no header of its own:
    /// `[mcp_servers.handoff]` alone, the way Codex writes a server itself. A section already
    /// at `path` is replaced where it stands, with the comment above its header, because the
    /// diff the user consented to was about its values and not about moving it.
    pub fn set_at(&mut self, path: &[&str], mut table: Table) {
        let Some((last, parents)) = path.split_last() else {
            return;
        };
        let mut current: &mut Table = self.doc.as_table_mut();
        for segment in parents {
            let entry = current.entry(segment).or_insert_with(implicit_table);
            // Cannot happen after a plan, which refuses a parent that is not a section; the
            // replacement keeps the function total without guessing at the user's intent.
            if !matches!(entry, Item::Table(_)) {
                *entry = implicit_table();
            }
            current = entry
                .as_table_mut()
                .expect("the entry was just made a table");
        }

        match current.get(last) {
            Some(Item::Table(old)) => {
                table.set_position(old.position());
                *table.decor_mut() = old.decor().clone();
            }
            // A table parsed from a plan carries the position it had in *that* text, which
            // means nothing in this one: without a position it is placed after its siblings.
            _ => {
                table.set_position(None);
                *table.decor_mut() = Decor::default();
            }
        }
        current.insert(last, Item::Table(table));
    }

    /// Removes the item at `path`, and every parent it leaves empty **that has no header**.
    ///
    /// The second half is what makes an uninstall byte-identical: a parent we created was
    /// implicit, so removing our section leaves nothing behind. A parent the user wrote as
    /// `[mcp_servers]` is theirs and stays, empty or not.
    pub fn remove_at(&mut self, path: &[&str]) {
        let Some((last, parents)) = path.split_last() else {
            return;
        };
        let mut current: &mut dyn TableLike = self.doc.as_table_mut();
        for segment in parents {
            match current.get_mut(segment).and_then(Item::as_table_like_mut) {
                Some(child) => current = child,
                None => return,
            }
        }
        current.remove(last);

        for depth in (1..=parents.len()).rev() {
            let prefix = &parents[..depth];
            let emptied = matches!(
                self.get_at(prefix),
                Some(Item::Table(table)) if table.is_implicit() && table.is_empty()
            );
            if emptied {
                self.remove_at(prefix);
            }
        }
    }

    /// The document as it would be written, in the file's own line endings.
    #[must_use]
    pub fn render(&self) -> String {
        let text = self.doc.to_string();
        if self.crlf {
            with_crlf(&text)
        } else {
            text
        }
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

/// `item`, placed at `path` of an otherwise empty document and rendered from its values alone.
///
/// This is the form every `before` and `after` of a TOML modification is written in, so two
/// sections holding the same values render alike however either was spelled: a basic or a
/// literal string, `1_800` or `1800`, an inline `env` or an `[…env]` sub-section, a comment in
/// between. Sub-sections become inline tables; nothing else changes shape.
#[must_use]
pub fn render_at(path: &[&str], item: &Item) -> String {
    let mut fresh = Document::empty();
    match canonical(item) {
        Item::Table(table) => fresh.set_at(path, table),
        other => {
            if let Some((last, parents)) = path.split_last() {
                let mut holder = Table::new();
                holder.insert(last, other);
                fresh.set_at(parents, holder);
            }
        }
    }
    fresh.doc.to_string()
}

/// The section a plan rendered as `after`, parsed back.
///
/// # Panics
///
/// When `after` holds no section at `path`, which cannot happen for text [`render_at`]
/// produced from a table.
#[must_use]
pub fn table_of(after: &str, path: &[&str]) -> Table {
    let document: DocumentMut = after
        .parse()
        .expect("the plan rendered this section itself");
    let mut current: &Item = document.as_item();
    for segment in path {
        current = current
            .get(segment)
            .expect("the plan rendered this section itself");
    }
    current
        .as_table()
        .cloned()
        .expect("the plan rendered a section, not a value")
}

/// A table with no header of its own, for a parent that has to exist and was never written.
fn implicit_table() -> Item {
    let mut table = Table::new();
    table.set_implicit(true);
    Item::Table(table)
}

/// `item` with its comments, spellings and positions taken out: its values alone.
fn canonical(item: &Item) -> Item {
    match item.clone().into_value() {
        Ok(Value::InlineTable(table)) => Item::Table(section_of(&table)),
        Ok(value) => Item::Value(fresh(&value)),
        Err(_) => Item::None,
    }
}

/// The key-values of `table` as a `[section]`.
fn section_of(table: &InlineTable) -> Table {
    let mut section = Table::new();
    for (key, value) in table.iter() {
        section.insert(key, Item::Value(fresh(value)));
    }
    section
}

/// `value` rebuilt from what it holds, so it takes the default spelling.
fn fresh(value: &Value) -> Value {
    match value {
        Value::String(string) => Value::from(string.value().as_str()),
        Value::Integer(integer) => Value::from(*integer.value()),
        Value::Float(float) => Value::from(*float.value()),
        Value::Boolean(boolean) => Value::from(*boolean.value()),
        Value::Datetime(datetime) => Value::from(*datetime.value()),
        Value::Array(array) => Value::Array(array.iter().map(fresh).collect()),
        Value::InlineTable(table) => Value::InlineTable(
            table
                .iter()
                .map(|(key, value)| (key, fresh(value)))
                .collect(),
        ),
    }
}

/// Whether every line break of `source` is CRLF.
fn uses_crlf(source: &str) -> bool {
    let breaks = source.matches('\n').count();
    breaks > 0 && source.matches("\r\n").count() == breaks
}

/// `text` with every lone LF made CRLF.
fn with_crlf(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 16);
    let mut previous = '\0';
    for character in text.chars() {
        if character == '\n' && previous != '\r' {
            out.push('\r');
        }
        out.push(character);
        previous = character;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: [&str; 2] = ["mcp_servers", "handoff"];

    fn document(source: &str) -> Document {
        Document {
            doc: source.parse().expect("the fixture is TOML"),
            crlf: uses_crlf(source),
            existed: true,
        }
    }

    fn ours() -> Table {
        let mut table = Table::new();
        table.insert("command", toml_edit::value("/apps/Baton/handoff-mcp"));
        table.insert("tool_timeout_sec", toml_edit::value(1800));
        table
    }

    /// A folder of this test's own, so two runs never share a file.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "handoff-toml-{name}-{}-{}",
            std::process::id(),
            crate::ids::new_session_ref()
        ));
        fs::create_dir_all(&dir).expect("the folder is created");
        dir
    }

    #[test]
    fn a_missing_file_reads_as_an_empty_document_that_was_never_written() {
        let document = Document::read(&scratch("missing").join("config.toml"))
            .expect("an absent file is empty");
        assert!(!document.existed());
        assert_eq!(document.render(), "");
    }

    #[test]
    fn a_file_that_is_not_toml_is_refused_and_the_refusal_quotes_nothing() {
        let dir = scratch("broken");
        let path = dir.join("config.toml");
        // A line another server could really hold: the message may name its place and never
        // repeat it, because it reaches a log.
        fs::write(
            &path,
            "[mcp_servers.other]\nenv = { API_KEY = \"sk-not-a-real-key\" }\nbroken = \n",
        )
        .expect("the file is written");

        match Document::read(&path) {
            Err(InstallError::Malformed { detail, .. }) => {
                assert!(detail.contains("line 3"), "{detail}");
                assert!(!detail.contains("sk-not-a-real-key"), "{detail}");
                assert!(!detail.contains("broken ="), "{detail}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_section_put_into_an_empty_document_reads_back_as_its_canonical_rendering() {
        let mut document = Document::empty();
        document.set_at(&PATH, ours());
        let rendered = render_at(&PATH, &Item::Table(ours()));
        assert_eq!(document.render(), rendered);
        assert_eq!(
            document.rendered_at(&PATH).as_deref(),
            Some(rendered.as_str())
        );
        assert!(
            rendered.starts_with("[mcp_servers.handoff]\n"),
            "an implicit parent has no header of its own:\n{rendered}"
        );
    }

    #[test]
    fn two_spellings_of_the_same_values_render_alike() {
        let spelled = document(concat!(
            "[mcp_servers.handoff]\n",
            "# a comment inside the section\n",
            "command = '/apps/Baton/handoff-mcp'\n",
            "tool_timeout_sec = 1_800\n",
        ));
        assert_eq!(
            spelled.rendered_at(&PATH),
            Some(render_at(&PATH, &Item::Table(ours())))
        );
    }

    #[test]
    fn an_inline_entry_and_a_sub_section_render_as_the_section_does() {
        let inline = document(
            "[mcp_servers]\nhandoff = { command = \"/apps/Baton/handoff-mcp\", tool_timeout_sec = 1800 }\n",
        );
        assert_eq!(
            inline.rendered_at(&PATH),
            Some(render_at(&PATH, &Item::Table(ours())))
        );

        let nested = document(concat!(
            "[mcp_servers.handoff]\ncommand = \"x\"\n",
            "[mcp_servers.handoff.env]\nHANDOFF_AGENT = \"codex\"\n",
        ));
        let rendered = nested.rendered_at(&PATH).expect("a section");
        assert!(
            rendered.contains("env = { HANDOFF_AGENT = \"codex\" }"),
            "{rendered}"
        );
    }

    #[test]
    fn adding_a_section_and_removing_it_gives_the_file_back_byte_for_byte() {
        let source = concat!(
            "# my Codex settings\n",
            "model = \"gpt-5.6-luna\"  # the one I like\n",
            "\n",
            "[mcp_servers.other]\n",
            "command = \"npx\"\n",
            "args = [\"-y\", \"other\"]   # spaced as I like it\n",
            "\n",
            "[profiles.fast]\n",
            "model_reasoning_effort = \"low\"\n",
        );
        let mut document = document(source);
        document.set_at(&PATH, ours());
        let installed = document.render();
        assert!(installed.contains("# my Codex settings\n"));
        assert!(installed.contains("args = [\"-y\", \"other\"]   # spaced as I like it\n"));
        assert!(installed.contains("[mcp_servers.handoff]"));

        document.remove_at(&PATH);
        assert_eq!(document.render(), source);
    }

    #[test]
    fn replacing_a_section_keeps_its_place_and_the_comment_above_it() {
        let mut document = document(concat!(
            "[mcp_servers.handoff]\n",
            "command = \"/old/Baton/handoff-mcp\"\n",
            "\n",
            "# the server I use every day\n",
            "[mcp_servers.other]\n",
            "command = \"npx\"\n",
        ));
        document.set_at(&PATH, ours());
        let rendered = document.render();
        let ours_at = rendered.find("[mcp_servers.handoff]").expect("ours");
        let theirs_at = rendered.find("[mcp_servers.other]").expect("theirs");
        assert!(ours_at < theirs_at, "the section moved:\n{rendered}");
        assert!(rendered.contains("# the server I use every day\n[mcp_servers.other]"));
        assert!(!rendered.contains("/old/Baton"), "{rendered}");
    }

    #[test]
    fn a_parent_we_created_goes_with_our_section_and_one_the_user_wrote_stays() {
        let mut ours_alone = Document::empty();
        ours_alone.set_at(&PATH, ours());
        ours_alone.remove_at(&PATH);
        assert_eq!(ours_alone.render(), "");

        let source = "[mcp_servers]\n";
        let mut theirs = document(source);
        theirs.set_at(&PATH, ours());
        theirs.remove_at(&PATH);
        assert_eq!(theirs.render(), source);
    }

    #[test]
    fn only_a_section_of_its_own_is_a_parent_we_write_under() {
        assert!(document("model = \"x\"\n").is_section_or_absent(&["mcp_servers"]));
        assert!(document("[mcp_servers.other]\ncommand = \"x\"\n")
            .is_section_or_absent(&["mcp_servers"]));
        assert!(document("[mcp_servers]\n").is_section_or_absent(&["mcp_servers"]));
        assert!(!document("mcp_servers = { other = { command = \"x\" } }\n")
            .is_section_or_absent(&["mcp_servers"]));
        assert!(
            !document("mcp_servers.other.command = \"x\"\n").is_section_or_absent(&["mcp_servers"])
        );
        assert!(!document("mcp_servers = 3\n").is_section_or_absent(&["mcp_servers"]));
    }

    #[test]
    fn a_crlf_file_gets_our_lines_in_crlf_and_comes_back_unchanged() {
        let source = "# mine\r\n[mcp_servers.other]\r\ncommand = \"npx\"\r\n";
        let mut document = document(source);
        document.set_at(&PATH, ours());
        let installed = document.render();
        assert_eq!(
            installed.matches('\n').count(),
            installed.matches("\r\n").count(),
            "a lone LF in a CRLF file:\n{installed:?}"
        );
        document.remove_at(&PATH);
        assert_eq!(document.render(), source);
    }

    #[test]
    fn a_plan_rendering_parses_back_into_the_section_it_came_from() {
        let rendered = render_at(&PATH, &Item::Table(ours()));
        let table = table_of(&rendered, &PATH);
        assert_eq!(render_at(&PATH, &Item::Table(table)), rendered);
    }
}
