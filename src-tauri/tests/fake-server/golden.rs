//! The golden sequences, and what "message for message, modulo ids and timestamps" means.
//!
//! `vendor/handoff-mcp/format/fixtures/channel/*.jsonl` holds one file per flow, each line
//! `{"dir": "→"|"←", "msg": {…}}`, written from the **server's** point of view: `→` is what
//! the server sends and `←` is what the app answers. [`FakeServer`](super::FakeServer) is
//! the server here, so a `→` line is one it sends and a `←` line is one it must receive.
//!
//! Nothing is transcribed. A [`Replay`] takes the messages out of the fixture, substitutes
//! the identifiers the live app minted for the ones the fixture invented, and compares what
//! actually crossed the socket with what the fixture says should have. The server's own
//! double does the mirror of this in `handoff-mcp/test/fake-app`; deriving on both sides is
//! what keeps the two from drifting the first time a fixture is touched.
//!
//! # Modulo ids and timestamps
//!
//! [`normalise`] rewrites every `hf_`, `call_`, `ses_` and `rb_` value as `<hf#1>`,
//! `<call#1>` and so on, numbered by first appearance in the sequence, and every RFC 3339
//! instant as `<at>`. Two different ids therefore stay different and an id reused where the
//! fixture reuses one stays equal, which is the property the flows are about. JSON-RPC ids
//! are numbered the same way, in two namespaces — one per originating peer — because both
//! peers count their requests from 1 and a response carries the id of the request it
//! answers, not one of its own.
//!
//! # The ignore list, and why every entry is written down
//!
//! Everything else is compared exactly, key order apart. A test may name field **keys**
//! whose value belongs to whichever peer is driving rather than to the flow: the app's
//! `app_version` is the obvious one. Each entry is written in the test beside the reason
//! it is there, because an ignore list nobody has to justify is a comparison that slowly
//! stops comparing anything.

use std::collections::HashMap;

use serde_json::{Map, Value};

use handoff_app_lib::ids::{CALL_ID_RE, HANDOFF_ID_RE, RUNBOOK_ID_RE, SESSION_REF_RE};

use super::{Direction, FakeServer};

/// One line of a fixture.
#[derive(Debug, Clone)]
pub struct GoldenLine {
    /// Which way it travelled.
    pub direction: Direction,
    /// The message itself.
    pub message: Value,
}

/// One fixture of `fixtures/channel/`.
#[derive(Debug, Clone)]
pub struct Golden {
    /// The file name without its extension, e.g. `f02-happy-path`.
    pub name: String,
    /// Its lines, in file order.
    pub lines: Vec<GoldenLine>,
}

impl Golden {
    /// Reads `vendor/handoff-mcp/format/fixtures/channel/<name>.jsonl`.
    ///
    /// # Panics
    ///
    /// When the file is missing — which means `vendor/` is not filled — or when a line is
    /// not the `{dir, msg}` shape every fixture of that folder has.
    #[must_use]
    pub fn load(name: &str) -> Self {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("vendor")
            .join("handoff-mcp")
            .join("format")
            .join("fixtures")
            .join("channel")
            .join(format!("{name}.jsonl"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "{} could not be read ({error}). Fill vendor/ with `node scripts/fetch-server.mjs`.",
                path.display()
            )
        });
        let lines = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .enumerate()
            .map(|(index, raw)| {
                let entry: Value = serde_json::from_str(raw)
                    .unwrap_or_else(|error| panic!("{name}:{} is not JSON: {error}", index + 1));
                let direction = match entry["dir"].as_str() {
                    Some("\u{2192}") => Direction::ToApp,
                    Some("\u{2190}") => Direction::ToServer,
                    other => panic!("{name}:{} has the direction {other:?}", index + 1),
                };
                GoldenLine {
                    direction,
                    message: entry["msg"].clone(),
                }
            })
            .collect();
        Self {
            name: name.to_owned(),
            lines,
        }
    }

    /// The lines at these indices, in the order given.
    ///
    /// A flow whose fixture cannot be replayed whole — because part of it needs a module
    /// that does not exist yet, or because the fixture illustrates a shape the store does
    /// not produce — is replayed as the subset that *can* be, and the test says which
    /// indices it took and why.
    #[must_use]
    pub fn subset(&self, indices: &[usize]) -> Self {
        Self {
            name: format!("{} (lines {indices:?})", self.name),
            lines: indices
                .iter()
                .map(|index| self.lines[*index].clone())
                .collect(),
        }
    }
}

/// Plays a fixture against a live app, one line at a time.
///
/// The test drives it, because the `←` lines of most fixtures are outcomes only a user
/// action produces: `send_next` writes the next `→` line, `expect_next` reads the next `←`
/// one, and between them the test does whatever the person at the machine did.
#[derive(Debug)]
pub struct Replay {
    golden: Golden,
    cursor: usize,
    /// The fixture's identifiers, mapped to the ones the live app minted.
    alias: HashMap<String, String>,
    /// The same for JSON-RPC ids: the app counts its own requests, so the fixture's `100`
    /// is whatever number this app's first ping happened to carry.
    ids: HashMap<String, Value>,
}

impl Replay {
    /// A replay of `golden`, at its first line.
    #[must_use]
    pub fn new(golden: Golden) -> Self {
        Self {
            golden,
            cursor: 0,
            alias: HashMap::new(),
            ids: HashMap::new(),
        }
    }

    /// Teaches the replay that the fixture's `from` is this run's `to`.
    ///
    /// Needed only where a fixture's identifier never comes back in an answer the replay
    /// reads — the `session_ref` of a registration, which is minted before the first
    /// scripted line.
    pub fn alias(&mut self, from: &str, to: &str) {
        self.alias.insert(from.to_owned(), to.to_owned());
    }

    /// Writes the next line of the fixture, with this run's identifiers in it.
    ///
    /// # Panics
    ///
    /// When the next line is one the app should send, or when the fixture is exhausted.
    pub async fn send_next(&mut self, fake: &mut FakeServer) {
        let line = self.next_line(Direction::ToApp);
        let mut message = substitute(&line.message, &self.alias);
        substitute_id(&mut message, &self.ids);
        fake.send(message).await;
    }

    /// Reads the next line the app writes and checks it is the one the fixture expects.
    ///
    /// Every identifier the fixture invented is paired here with the one the app really
    /// used, so the next `→` line goes out with the live values.
    ///
    /// # Panics
    ///
    /// When the next line is one the server should send, when nothing arrives, or when the
    /// message is of a different shape from the fixture's.
    pub async fn expect_next(&mut self, fake: &mut FakeServer) -> Value {
        let line = self.next_line(Direction::ToServer);
        let actual = fake
            .receive()
            .await
            .unwrap_or_else(|| panic!("{}: the connection closed early", self.golden.name));
        learn(&line.message, &actual, &mut self.alias);
        learn_id(&line.message, &actual, &mut self.ids);
        actual
    }

    /// The lines played so far, as the fixture writes them.
    #[must_use]
    pub fn played(&self) -> Vec<GoldenLine> {
        self.golden.lines[..self.cursor].to_vec()
    }

    /// Asserts that everything the fixture has scripted was played.
    ///
    /// # Panics
    ///
    /// When lines are left over, which means the test stopped in the middle of a flow.
    pub fn assert_finished(&self) {
        assert_eq!(
            self.cursor,
            self.golden.lines.len(),
            "{}: {} lines of the fixture were never played",
            self.golden.name,
            self.golden.lines.len() - self.cursor
        );
    }

    /// The transcript the fake recorded, against the lines this replay played.
    ///
    /// # Panics
    ///
    /// When they differ once identifiers and instants are normalised and the ignored keys
    /// are blanked.
    pub fn assert_transcript(&self, fake: &FakeServer, ignore: &[&str]) {
        assert_transcript(&self.golden.name, fake.transcript(), &self.played(), ignore);
    }

    /// The same, over one stretch of the flow.
    ///
    /// A whole-transcript comparison takes one ignore list, and an ignore that belongs to a
    /// single message would then apply to every message. Splitting the transcript is what
    /// keeps each exception attached to the message it is about; the two sides are cut at
    /// the same index, so the placeholders still number identically on both.
    ///
    /// # Panics
    ///
    /// When the two stretches differ, or when the range is not inside the transcript.
    pub fn assert_transcript_range(
        &self,
        fake: &FakeServer,
        range: std::ops::Range<usize>,
        ignore: &[&str],
        why: &str,
    ) {
        let played = self.played();
        let actual = fake.transcript();
        assert!(
            range.end <= played.len() && range.end <= actual.len(),
            "{}: {why} asks for messages {range:?} of a transcript of {} and a fixture of {}",
            self.golden.name,
            actual.len(),
            played.len()
        );
        assert_transcript(
            &format!("{} \u{b7} {why}", self.golden.name),
            &actual[range.clone()],
            &played[range],
            ignore,
        );
    }

    fn next_line(&mut self, expected: Direction) -> GoldenLine {
        let line = self
            .golden
            .lines
            .get(self.cursor)
            .unwrap_or_else(|| {
                panic!(
                    "{}: the fixture has only {} lines",
                    self.golden.name,
                    self.golden.lines.len()
                )
            })
            .clone();
        assert_eq!(
            line.direction,
            expected,
            "{}: line {} travels the other way",
            self.golden.name,
            self.cursor + 1
        );
        self.cursor += 1;
        line
    }
}

/// Compares what crossed the socket with what a fixture says should have.
///
/// # Panics
///
/// On the first line that differs, naming the flow, the line number and both messages.
pub fn assert_transcript(
    what: &str,
    actual: &[(Direction, Value)],
    expected: &[GoldenLine],
    ignore: &[&str],
) {
    let expected_lines: Vec<(Direction, Value)> = expected
        .iter()
        .map(|line| (line.direction, line.message.clone()))
        .collect();
    let actual_directions: Vec<Direction> =
        actual.iter().map(|(direction, _)| *direction).collect();
    let expected_directions: Vec<Direction> = expected.iter().map(|line| line.direction).collect();

    let normalised_actual = normalise(actual, ignore);
    let normalised_expected = normalise(&expected_lines, ignore);

    for (index, (got, want)) in normalised_actual
        .iter()
        .zip(normalised_expected.iter())
        .enumerate()
    {
        assert_eq!(
            actual_directions[index],
            expected_directions[index],
            "{what}, message {}: it travelled the other way",
            index + 1
        );
        assert_eq!(
            got,
            want,
            "{what}, message {}: the app did not say what the fixture says it says\n\
             sent:     {got}\n\
             expected: {want}",
            index + 1
        );
    }
    assert_eq!(
        normalised_actual.len(),
        normalised_expected.len(),
        "{what}: {} messages crossed the socket, the fixture has {}",
        normalised_actual.len(),
        normalised_expected.len()
    );
}

/// Every identifier and instant of a sequence replaced by a stable placeholder.
///
/// The numbering is by first appearance **in the sequence**, so the same value in two
/// messages becomes the same placeholder and two different values stay different.
#[must_use]
pub fn normalise(messages: &[(Direction, Value)], ignore: &[&str]) -> Vec<Value> {
    let mut names = Names::default();
    messages
        .iter()
        .map(|(direction, message)| {
            let mut copy = blank(message, ignore);
            names.rewrite_message(*direction, &mut copy);
            copy
        })
        .collect()
}

/// The placeholder table of one sequence.
#[derive(Default)]
struct Names {
    values: HashMap<String, String>,
    counts: HashMap<&'static str, usize>,
    /// JSON-RPC ids, keyed by the peer that minted them and the number itself.
    ids: HashMap<(bool, String), String>,
}

impl Names {
    fn rewrite_message(&mut self, direction: Direction, message: &mut Value) {
        // A request mints its id; a response carries the id of the request it answers, so
        // its originator is the peer on the other side. Both peers count their requests
        // from 1, and without the two namespaces the app's first ping would normalise to
        // the same name as the server's first request.
        let is_request = message.get("method").is_some() && message.get("id").is_some();
        let minted_by = if is_request {
            direction
        } else {
            direction.other()
        };
        if let Some(id) = message.get("id").cloned() {
            let key = (minted_by == Direction::ToApp, id.to_string());
            let next = self.ids.len() + 1;
            let name = self
                .ids
                .entry(key)
                .or_insert_with(|| format!("<id#{next}>"))
                .clone();
            message["id"] = Value::String(name);
        }
        self.rewrite(message);
    }

    fn rewrite(&mut self, value: &mut Value) {
        match value {
            Value::String(text) => {
                *text = self.placeholders_in(text);
            }
            Value::Array(items) => {
                for item in items {
                    self.rewrite(item);
                }
            }
            Value::Object(fields) => {
                for (_, field) in fields.iter_mut() {
                    self.rewrite(field);
                }
            }
            _ => {}
        }
    }

    /// Every identifier inside a string replaced by its placeholder.
    ///
    /// Inside, not only instead of: the `instruction` of half the outcomes carries the
    /// handoff id in the middle of a sentence ("call handoff_to_user with resume=hf_..."),
    /// and a comparison that only recognised an id occupying a whole field would compare
    /// those sentences literally and fail on every run.
    fn placeholders_in(&mut self, text: &str) -> String {
        if is_instant(text) {
            return "<at>".to_owned();
        }
        let mut result = String::with_capacity(text.len());
        let mut last = 0;
        for found in ANY_ID.find_iter(text) {
            result.push_str(&text[last..found.start()]);
            result.push_str(&self.name_of(found.as_str()));
            last = found.end();
        }
        result.push_str(&text[last..]);
        result
    }

    fn name_of(&mut self, id: &str) -> String {
        if let Some(known) = self.values.get(id) {
            return known.clone();
        }
        let kind = if HANDOFF_ID_RE.is_match(id) {
            "hf"
        } else if CALL_ID_RE.is_match(id) {
            "call"
        } else if SESSION_REF_RE.is_match(id) {
            "ses"
        } else {
            "rb"
        };
        let count = self.counts.entry(kind).or_insert(0);
        *count += 1;
        let name = format!("<{kind}#{count}>");
        self.values.insert(id.to_owned(), name.clone());
        name
    }
}

/// Any identifier of §4.1, anywhere in a string.
///
/// No boundary is asserted at either end — the `regex` crate has no look-around, and the
/// shapes are fixed-length, so an id can only be found where one is. Both sides of a
/// comparison are scanned by this same expression, so even an over-match would be
/// symmetrical.
static ANY_ID: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"hf_[0-9a-hjkmnp-tv-z]{10}|rb_[0-9a-hjkmnp-tv-z]{10}|ses_[0-9a-hjkmnp-tv-z]{8}|call_[0-9a-hjkmnp-tv-z]{8}",
    )
    .expect("the identifier shapes compile")
});

/// Whether a string is an RFC 3339 instant, which is the one shape of timestamp the formats
/// use (`log::time::Timestamp`, §7.11).
fn is_instant(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return false;
    }
    text.ends_with('Z') || text.len() >= 25 && (text.contains('+') || text[10..].contains('-'))
}

/// Replaces the value of every field named in `ignore`, anywhere in the document.
fn blank(value: &Value, ignore: &[&str]) -> Value {
    match value {
        Value::Object(fields) => {
            let mut copy = Map::new();
            for (name, field) in fields {
                if ignore.contains(&name.as_str()) {
                    copy.insert(name.clone(), Value::String("<ignored>".to_owned()));
                } else {
                    copy.insert(name.clone(), blank(field, ignore));
                }
            }
            Value::Object(copy)
        }
        Value::Array(items) => Value::Array(items.iter().map(|item| blank(item, ignore)).collect()),
        other => other.clone(),
    }
}

/// Replaces the fixture's identifiers with this run's, anywhere in an outgoing message.
fn substitute(message: &Value, alias: &HashMap<String, String>) -> Value {
    match message {
        Value::String(text) => {
            Value::String(alias.get(text).cloned().unwrap_or_else(|| text.clone()))
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| substitute(item, alias)).collect())
        }
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), substitute(field, alias)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Pairs the fixture's JSON-RPC id with the one the app really minted.
///
/// Only the app's own requests need it: the fixture's `hello` and `ping` ids are the ones
/// the fake sends, so they come back unchanged, while the app counts its pings from 1 and
/// the fixture wrote 100.
fn learn_id(expected: &Value, actual: &Value, ids: &mut HashMap<String, Value>) {
    if let (Some(want), Some(got)) = (expected.get("id"), actual.get("id")) {
        if want.is_number() && got.is_number() && want != got {
            ids.insert(want.to_string(), got.clone());
        }
    }
}

/// The live JSON-RPC id in place of the fixture's, on an outgoing message.
fn substitute_id(message: &mut Value, ids: &HashMap<String, Value>) {
    let Some(id) = message.get("id") else { return };
    if let Some(live) = ids.get(&id.to_string()).cloned() {
        message["id"] = live;
    }
}

/// Pairs the fixture's identifiers with the ones the app really used.
///
/// The two documents are walked together; wherever the fixture has an identifier and the
/// message that arrived has one in the same place, the pair is remembered. A structural
/// difference is not an error here — the comparison at the end of the flow is what judges
/// that, with a message a reader can act on.
fn learn(expected: &Value, actual: &Value, alias: &mut HashMap<String, String>) {
    match (expected, actual) {
        (Value::String(want), Value::String(got)) => {
            let is_id = HANDOFF_ID_RE.is_match(want)
                || CALL_ID_RE.is_match(want)
                || SESSION_REF_RE.is_match(want)
                || RUNBOOK_ID_RE.is_match(want);
            if is_id && want != got {
                alias.insert(want.clone(), got.clone());
            }
        }
        (Value::Array(wants), Value::Array(gots)) => {
            for (want, got) in wants.iter().zip(gots.iter()) {
                learn(want, got, alias);
            }
        }
        (Value::Object(wants), Value::Object(gots)) => {
            for (name, want) in wants {
                if let Some(got) = gots.get(name) {
                    learn(want, got, alias);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_eleven_fixtures_are_all_readable_and_none_is_empty() {
        for name in [
            "auth-failed",
            "f01-register",
            "f02-happy-path",
            "f04-ask-reply",
            "f05-defer-park",
            "f06-heartbeat-resume",
            "f07-user-request",
            "f08-failed-correction",
            "f10-hook-block",
            "f11-transfer",
            "protocol-mismatch",
        ] {
            let golden = Golden::load(name);
            assert!(!golden.lines.is_empty(), "{name} is empty");
        }
    }

    #[test]
    fn two_different_ids_stay_different_and_a_reused_one_stays_equal() {
        let messages = vec![
            (
                Direction::ToApp,
                json!({ "a": "hf_7k3m9p2q4r", "b": "call_2q7m8r1t" }),
            ),
            (
                Direction::ToApp,
                json!({ "a": "hf_7k3m9p2q4r", "b": "call_5w3n9k2v" }),
            ),
        ];
        let normalised = normalise(&messages, &[]);
        assert_eq!(normalised[0]["a"], normalised[1]["a"]);
        assert_ne!(normalised[0]["b"], normalised[1]["b"]);
        assert_eq!(normalised[0]["a"], json!("<hf#1>"));
        assert_eq!(normalised[1]["b"], json!("<call#2>"));
    }

    #[test]
    fn each_peers_request_ids_are_counted_apart() {
        // Both peers count from 1. Without the two namespaces the app's own first ping
        // would normalise to the same name as the server's first request, and a fixture
        // that pairs them differently would still compare equal.
        let messages = vec![
            (
                Direction::ToApp,
                json!({ "jsonrpc": "2.0", "id": 1, "method": "ping", "params": {} }),
            ),
            (
                Direction::ToServer,
                json!({ "jsonrpc": "2.0", "id": 1, "result": {} }),
            ),
            (
                Direction::ToServer,
                json!({ "jsonrpc": "2.0", "id": 1, "method": "ping", "params": {} }),
            ),
            (
                Direction::ToApp,
                json!({ "jsonrpc": "2.0", "id": 1, "result": {} }),
            ),
        ];
        let normalised = normalise(&messages, &[]);
        // The response answers the request, so it carries that request's name.
        assert_eq!(normalised[0]["id"], normalised[1]["id"]);
        // The same number minted by the other peer is a different name.
        assert_ne!(normalised[0]["id"], normalised[2]["id"]);
        assert_eq!(normalised[2]["id"], normalised[3]["id"]);
    }

    #[test]
    fn an_instant_becomes_one_placeholder_whatever_its_offset() {
        let messages = vec![(
            Direction::ToApp,
            json!({
                "utc": "2026-09-07T10:12:03Z",
                "millis": "2026-09-07T10:12:03.123Z",
                "offset": "2026-09-07T12:12:03+02:00",
                "not_a_date": "2026-09-07"
            }),
        )];
        let normalised = normalise(&messages, &[]);
        assert_eq!(normalised[0]["utc"], json!("<at>"));
        assert_eq!(normalised[0]["millis"], json!("<at>"));
        assert_eq!(normalised[0]["offset"], json!("<at>"));
        assert_eq!(normalised[0]["not_a_date"], json!("2026-09-07"));
    }

    #[test]
    fn an_ignored_key_is_blanked_wherever_it_appears() {
        let messages = vec![(
            Direction::ToServer,
            json!({ "result": { "app_version": "0.1.0" } }),
        )];
        let normalised = normalise(&messages, &["app_version"]);
        assert_eq!(normalised[0]["result"]["app_version"], json!("<ignored>"));
    }

    #[test]
    fn a_transcript_that_differs_is_refused() {
        // The comparison has to have failed once to be worth anything.
        let expected = vec![GoldenLine {
            direction: Direction::ToServer,
            message: json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } }),
        }];
        let actual = vec![(
            Direction::ToServer,
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": false } }),
        )];
        let failed = std::panic::catch_unwind(|| {
            assert_transcript("a deliberate mutation", &actual, &expected, &[]);
        });
        assert!(failed.is_err(), "a changed field went unnoticed");
    }

    #[test]
    fn the_alias_learned_from_an_answer_is_used_by_the_next_message() {
        let mut alias = HashMap::new();
        learn(
            &json!({ "handoff_id": "hf_7k3m9p2q4r" }),
            &json!({ "handoff_id": "hf_0000000001" }),
            &mut alias,
        );
        let outgoing = json!({ "params": { "handoff_id": "hf_7k3m9p2q4r", "reply": "yes" } });
        assert_eq!(
            substitute(&outgoing, &alias),
            json!({ "params": { "handoff_id": "hf_0000000001", "reply": "yes" } })
        );
    }
}
