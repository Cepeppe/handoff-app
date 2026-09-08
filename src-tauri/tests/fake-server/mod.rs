//! `fake-server` — a channel client that plays `handoff-mcp` against the real app (§11.3).
//!
//! The counterpart of `handoff-mcp/test/fake-app`, which plays the app against the real
//! server. This one speaks the wire and knows nothing about handoffs: it connects to the
//! endpoint the app is listening on, sends `hello` with the identity and capability row of
//! the golden fixtures, and then writes and reads lines. Everything it receives **and**
//! everything it sends is validated against `channel.v1.schema.json`, so a double that
//! taught the app a protocol that does not exist would fail in its own `send`.
//!
//! What it deliberately does not have is a scenario language. The server's double needs
//! one because it has to *answer* like an app; this one only has to *ask* like a server,
//! and what it asks comes straight out of the fixtures through [`golden::Replay`]. No
//! payload is copied out of a fixture into Rust.
//!
//! # Connecting on Windows
//!
//! The accept loop creates the next pipe instance only after taking the previous one, so a
//! client arriving in that window is refused with `ERROR_FILE_NOT_FOUND` (2) or
//! `ERROR_PIPE_BUSY` (231) rather than queued, unlike a Unix socket backlog. [`connect`]
//! retries on exactly those two, which is what tokio's own documentation prescribes.

pub mod golden;

use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use handoff_app_lib::channel::token::Token;
use handoff_app_lib::channel::Endpoint;
use handoff_app_lib::format::schema::{is_valid, Document};

/// Long enough that a loaded runner does not fail a test, short enough that a hang is a
/// failure and not a two-minute wait. The same budget `tests/channel_listener.rs` uses.
pub const PATIENCE: Duration = Duration::from_secs(5);

/// Which way a message travelled, in the fixtures' own notation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// `→`: the server writes, the app reads.
    ToApp,
    /// `←`: the app writes, the server reads.
    ToServer,
}

impl Direction {
    /// The other one.
    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::ToApp => Self::ToServer,
            Self::ToServer => Self::ToApp,
        }
    }
}

#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(not(windows))]
type Stream = tokio::net::UnixStream;

/// One connection to the app, as `handoff-mcp` would make it.
pub struct FakeServer {
    stream: Stream,
    buffer: Vec<u8>,
    transcript: Vec<(Direction, Value)>,
    /// What the app assigned at `hello`, for a server connection.
    pub session_ref: Option<String>,
}

impl FakeServer {
    /// Opens a connection. Nothing has been sent yet.
    pub async fn connect(endpoint: &Endpoint) -> Self {
        Self {
            stream: connect(endpoint).await,
            buffer: Vec::new(),
            transcript: Vec::new(),
            session_ref: None,
        }
    }

    /// Connects and registers a session, and returns the `session_ref` the app assigned.
    ///
    /// The `hello` is the one `fixtures/channel/f01-register.jsonl` carries, with this
    /// installation's token in it: the identity, the agent id and the capability row are
    /// the fixture's, so a session registered here is the session every other fixture was
    /// written against.
    ///
    /// # Panics
    ///
    /// When the app refuses the handshake.
    pub async fn register(endpoint: &Endpoint, token: &Token) -> Self {
        let mut fake = Self::connect(endpoint).await;
        let hello = with_token(&hello_of("f01-register"), token);
        fake.send(hello).await;
        let answer = fake.expect("the hello result").await;
        let session_ref = answer["result"]["session_ref"]
            .as_str()
            .expect("a server is given a session_ref")
            .to_owned();
        fake.session_ref = Some(session_ref);
        fake
    }

    /// Writes one line.
    ///
    /// # Panics
    ///
    /// When the message is one the channel schema refuses: a double that sends what the
    /// real server could not send teaches the app a protocol that does not exist.
    pub async fn send(&mut self, message: Value) {
        assert!(
            is_valid(Document::ChannelMessage, &message),
            "fake-server tried to send a line the channel schema refuses: {message}"
        );
        let mut line = serde_json::to_vec(&message).expect("the message serialises");
        line.push(b'\n');
        self.stream
            .write_all(&line)
            .await
            .expect("the fake server writes");
        self.stream.flush().await.expect("the fake server flushes");
        self.transcript.push((Direction::ToApp, message));
    }

    /// The next line the app wrote, checked against the channel schema and recorded.
    ///
    /// Returns `None` at end of stream, which is how a test says "the app closed it".
    ///
    /// # Panics
    ///
    /// When nothing arrives within [`PATIENCE`], or when the line is not a valid channel
    /// message.
    pub async fn receive(&mut self) -> Option<Value> {
        let line = tokio::time::timeout(PATIENCE, self.next_line())
            .await
            .expect("the app answered or closed in time")?;
        let value: Value = serde_json::from_str(&line).unwrap_or_else(|error| {
            panic!("the app wrote something that is not JSON ({error}): {line}")
        });
        assert!(
            is_valid(Document::ChannelMessage, &value),
            "the app wrote a line the channel schema refuses: {value}"
        );
        self.transcript.push((Direction::ToServer, value.clone()));
        Some(value)
    }

    /// The next line, or a failure naming what was being waited for.
    ///
    /// # Panics
    ///
    /// When the connection closes instead.
    pub async fn expect(&mut self, what: &str) -> Value {
        self.receive()
            .await
            .unwrap_or_else(|| panic!("the connection closed while waiting for {what}"))
    }

    /// Asserts the app closed the connection, and wrote nothing first.
    ///
    /// # Panics
    ///
    /// When it wrote something.
    pub async fn expect_closed(&mut self) {
        assert_eq!(
            self.receive().await,
            None,
            "the app answered where the protocol says it closes"
        );
    }

    /// Everything that crossed this connection, in order.
    #[must_use]
    pub fn transcript(&self) -> &[(Direction, Value)] {
        &self.transcript
    }

    /// Forgets the handshake, so a comparison starts where a fixture starts.
    ///
    /// Every fixture except `f01`, `f10` and the two refusals begins after a successful
    /// registration; this is what the server's own double calls `mark()`.
    pub fn mark(&mut self) {
        self.transcript.clear();
    }

    async fn next_line(&mut self) -> Option<String> {
        loop {
            if let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=end).collect();
                return Some(String::from_utf8_lossy(&line[..end]).trim().to_owned());
            }
            let mut chunk = [0_u8; 8192];
            match self.stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return None,
                Ok(read) => self.buffer.extend_from_slice(&chunk[..read]),
            }
        }
    }
}

/// The `hello` message of a fixture, whichever role it carries.
///
/// # Panics
///
/// When that fixture does not begin with one.
#[must_use]
pub fn hello_of(fixture: &str) -> Value {
    let golden = golden::Golden::load(fixture);
    let first = golden
        .lines
        .first()
        .unwrap_or_else(|| panic!("{fixture} is empty"));
    assert_eq!(
        first.message["method"],
        json!("hello"),
        "{fixture} does not begin with a hello"
    );
    first.message.clone()
}

/// The same `hello`, carrying this installation's token instead of the fixture's.
#[must_use]
pub fn with_token(hello: &Value, token: &Token) -> Value {
    let mut message = hello.clone();
    message["params"]["token"] = json!(token.as_str());
    message
}

#[cfg(windows)]
async fn connect(endpoint: &Endpoint) -> Stream {
    use tokio::net::windows::named_pipe::ClientOptions;

    let Endpoint::Pipe { name } = endpoint else {
        panic!("this host listens on a pipe");
    };
    for _ in 0..500 {
        match ClientOptions::new().open(name) {
            Ok(client) => return client,
            Err(error) if matches!(error.raw_os_error(), Some(2 | 231)) => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => panic!("the fake server could not open {name}: {error}"),
        }
    }
    panic!("{name} never became available")
}

#[cfg(not(windows))]
async fn connect(endpoint: &Endpoint) -> Stream {
    let Endpoint::Unix { path, .. } = endpoint else {
        panic!("this host listens on a socket");
    };
    for _ in 0..500 {
        if let Ok(client) = tokio::net::UnixStream::connect(path).await {
            return client;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{} never became available", path.display())
}
