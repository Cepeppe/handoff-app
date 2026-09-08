//! The channel listener, driven by a peer that is not the real server (§6.1, §6.2, §6.3,
//! §7.3, FM-10, FM-11, FM-12).
//!
//! The client below speaks the wire and nothing else: it writes lines and reads lines, and
//! it knows about `channel.v1.schema.json` only to assert that **everything the listener
//! writes validates against it**. That check is in [`Client::receive`], so every test in
//! this file carries it without asking, and a reply the app invents a shape for fails the
//! first test that reads it rather than the first session a user starts.
//!
//! What cannot be tested from here is whether the DACL of A-16 really keeps a second user
//! out: that needs a second Windows account and is the manual test of T-057. What is tested
//! here is that the pipe is created with it, that a second listener cannot take the same
//! name, and that this process can still reach its own pipe — the three ways a wrong
//! descriptor would show up without one.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::mpsc::Receiver;

use handoff_app_lib::channel::listener::{
    listen, ChannelEvent, ChannelHandle, ConnId, DisconnectReason, ListenerConfig, PeerRole,
};
use handoff_app_lib::channel::token::Token;
use handoff_app_lib::channel::Endpoint;
use handoff_app_lib::format::channel::{
    ChannelMessage, HandoffOpenResult, HookStopResult, RequestBody, RequestId, ResultBody,
};
use handoff_app_lib::format::schema::{is_valid, Document};
use handoff_app_lib::ids;

/// Long enough that a loaded runner does not fail a test, short enough that a hang is a
/// failure and not a two-minute wait.
const PATIENCE: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------------------

/// One listener, its event stream, and the endpoint and token a client needs to reach it.
struct Harness {
    handle: ChannelHandle,
    events: Receiver<ChannelEvent>,
    endpoint: Endpoint,
    token: Token,
    /// Removed when the harness is dropped; `None` on Windows, where the endpoint is a name
    /// and not a file.
    dir: Option<PathBuf>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

impl Harness {
    /// The next event, or a failure naming what was being waited for.
    async fn event(&mut self, what: &str) -> ChannelEvent {
        tokio::time::timeout(PATIENCE, self.events.recv())
            .await
            .unwrap_or_else(|_| panic!("no {what} arrived within {PATIENCE:?}"))
            .unwrap_or_else(|| panic!("the event stream closed while waiting for {what}"))
    }

    /// Asserts that nothing else is on the stream right now.
    fn no_event(&mut self) {
        assert!(
            self.events.try_recv().is_err(),
            "an event arrived that no test asked for"
        );
    }
}

/// An endpoint of this test's own, so that tests run in parallel and never touch the real
/// one: a pipe name nobody else uses, or a socket in a fresh temporary folder.
fn private_endpoint() -> (Endpoint, Option<PathBuf>) {
    let unique = format!("{}-{}", std::process::id(), ids::new_session_ref());
    if cfg!(windows) {
        (
            Endpoint::Pipe {
                name: format!(r"\\.\pipe\handoff-test-{unique}"),
            },
            None,
        )
    } else {
        let dir = std::env::temp_dir().join(format!("handoff-listener-{unique}"));
        std::fs::create_dir_all(&dir).expect("the temporary directory is created");
        (
            Endpoint::Unix {
                path: dir.join("app.sock"),
                pointer: None,
            },
            Some(dir),
        )
    }
}

/// A listener with the timings of the design.
async fn harness() -> Harness {
    harness_with(|_| {}).await
}

/// A listener whose timings a test may shorten. Nothing else about the configuration
/// changes: the framing, the schema check and the handshake are the ones that ship.
async fn harness_with(tweak: impl FnOnce(&mut ListenerConfig)) -> Harness {
    let (endpoint, dir) = private_endpoint();
    let token = Token::generate();
    let mut config = ListenerConfig::new(endpoint.clone(), token.clone());
    tweak(&mut config);
    let (handle, events) = listen(config).await.expect("the listener binds");
    Harness {
        handle,
        events,
        endpoint,
        token,
        dir,
    }
}

// ---------------------------------------------------------------------------------------
// The test client
// ---------------------------------------------------------------------------------------

#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(not(windows))]
type Stream = tokio::net::UnixStream;

/// A peer that speaks the wire and knows nothing about the app.
struct Client {
    stream: Stream,
    buffer: Vec<u8>,
    next_id: u64,
}

impl Client {
    async fn connect(endpoint: &Endpoint) -> Self {
        Self {
            stream: open(endpoint).await,
            buffer: Vec::new(),
            next_id: 0,
        }
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    async fn send(&mut self, message: &Value) {
        let mut line = serde_json::to_vec(message).expect("the message serialises");
        line.push(b'\n');
        self.send_raw(&line).await;
    }

    /// Bytes, exactly as given: what a test needs to send half a line, an oversized one or
    /// something that is not JSON at all.
    async fn send_raw(&mut self, bytes: &[u8]) {
        self.stream
            .write_all(bytes)
            .await
            .expect("the client writes");
        self.stream.flush().await.expect("the client flushes");
    }

    /// The next line the listener wrote, checked against the channel schema.
    ///
    /// Returns `None` at end of stream, which is how every "the connection closes" test
    /// says what it means.
    async fn receive(&mut self) -> Option<Value> {
        let line = tokio::time::timeout(PATIENCE, self.next_line())
            .await
            .expect("the listener answered or closed in time")?;
        let value: Value = serde_json::from_str(&line).unwrap_or_else(|error| {
            panic!("the listener wrote something that is not JSON ({error}): {line}")
        });
        assert!(
            is_valid(Document::ChannelMessage, &value),
            "the listener wrote a line the channel schema refuses: {value}"
        );
        Some(value)
    }

    async fn expect(&mut self, what: &str) -> Value {
        self.receive()
            .await
            .unwrap_or_else(|| panic!("the connection closed while waiting for {what}"))
    }

    /// Asserts the listener closed the connection, and wrote nothing first.
    async fn expect_closed(&mut self) {
        assert_eq!(
            self.receive().await,
            None,
            "the listener answered where the protocol says it closes"
        );
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

    /// The `hello` of a server, and the `session_ref` the app answered with.
    async fn register(&mut self, token: &Token) -> String {
        let id = self.id();
        self.send(&server_hello(id, token.as_str(), 1)).await;
        let answer = self.expect("the hello result").await;
        assert_eq!(answer["id"], json!(id));
        answer["result"]["session_ref"]
            .as_str()
            .expect("a server is given a session_ref")
            .to_owned()
    }
}

#[cfg(windows)]
async fn open(endpoint: &Endpoint) -> Stream {
    use tokio::net::windows::named_pipe::ClientOptions;

    let Endpoint::Pipe { name } = endpoint else {
        panic!("this host listens on a pipe");
    };
    // ERROR_FILE_NOT_FOUND (2) and ERROR_PIPE_BUSY (231): the accept loop creates the next
    // instance after it has taken the previous one, so a client arriving in that window has
    // to wait rather than fail. This is the retry tokio's own documentation prescribes.
    for _ in 0..500 {
        match ClientOptions::new().open(name) {
            Ok(client) => return client,
            Err(error) if matches!(error.raw_os_error(), Some(2 | 231)) => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => panic!("the client could not open {name}: {error}"),
        }
    }
    panic!("{name} never became available")
}

#[cfg(not(windows))]
async fn open(endpoint: &Endpoint) -> Stream {
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

// ---------------------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------------------

fn server_hello(id: u64, token: &str, protocol_version: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "hello",
        "params": {
            "protocol_version": protocol_version,
            "token": token,
            "role": "server",
            "server_version": "1.0.3",
            "identity": {
                "pid": 48211, "ppid": 48190,
                "ancestors": [{ "pid": 48190, "name": "node" }],
                "cwd": "/Users/g/dev/shop", "project_dir": "/Users/g/dev/shop"
            },
            "agent_id": "claude-code",
            "client": { "name": "claude-code", "version": "2.1.211" },
            "capability_row": {
                "agent_id": "claude-code", "support": "full", "images_in_results": true,
                "stop_hook": true, "tool_timeout_ms": 1800000
            }
        }
    })
}

fn hook_hello(id: u64, token: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "hello",
        "params": {
            "protocol_version": 1,
            "token": token,
            "role": "hook",
            "identity": {
                "pid": 50310, "ppid": 48190, "ancestors": [], "cwd": "/Users/g/dev/shop"
            },
            "hook": {
                "session_id": "0b1e7c94-6f3a-4d21-9f0c-2ab5e8d17c43",
                "hook_event_name": "Stop",
                "stop_hook_active": false
            }
        }
    })
}

fn handoff_open(id: u64, call_id: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "handoff.open",
        "params": {
            "call_id": call_id,
            "spec": {
                "spec_version": 1,
                "goal": "Register the Stripe webhook for payment events",
                "where": "Stripe dashboard",
                "why_human": "the account is not accessible to an agent",
                "values": {},
                "steps": [{ "text": "Open the developers page" }]
            },
            "secret_treated": [],
            "request_id": null
        }
    })
}

// ---------------------------------------------------------------------------------------
// The handshake (§6.2)
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_server_is_registered_and_its_peer_reaches_the_store() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;

    let id = client.id();
    client
        .send(&server_hello(id, harness.token.as_str(), 1))
        .await;

    // The `Connected` event comes first, and it comes before the answer is written: a
    // server that sends its first request in the same breath as reading the answer would
    // otherwise arrive at a store that has never heard of it (T-020's mirror).
    let ChannelEvent::Connected(peer) = harness.event("the connected peer").await else {
        panic!("the first event of a connection is Connected");
    };
    assert_eq!(peer.role, PeerRole::Server);
    assert_eq!(peer.agent_id.as_deref(), Some("claude-code"));
    assert_eq!(peer.identity.pid, 48211);
    assert_eq!(peer.identity.ancestors.len(), 1);
    assert_eq!(
        peer.capability_row
            .as_ref()
            .expect("a server sends its row")
            .agent_id,
        "claude-code"
    );
    assert!(peer.hook.is_none());
    let session_ref = peer.session_ref.clone().expect("a server is registered");

    let answer = client.expect("the hello result").await;
    assert_eq!(answer["id"], json!(id));
    assert_eq!(answer["result"]["protocol_version"], json!(1));
    assert_eq!(
        answer["result"]["app_version"],
        json!(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(answer["result"]["session_ref"], json!(session_ref));
    assert!(
        ids::SESSION_REF_RE.is_match(&session_ref),
        "{session_ref} is not a session_ref of §4.1"
    );
    assert_eq!(harness.handle.connections().await, 1);
}

#[tokio::test]
async fn a_hook_is_served_without_a_session() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;

    let id = client.id();
    client.send(&hook_hello(id, harness.token.as_str())).await;

    let ChannelEvent::Connected(peer) = harness.event("the connected hook").await else {
        panic!("the first event of a connection is Connected");
    };
    assert_eq!(peer.role, PeerRole::Hook);
    assert!(peer.session_ref.is_none(), "a hook registers no session");
    assert!(peer.capability_row.is_none());
    let hook = peer.hook.as_ref().expect("a hook sends its input");
    assert_eq!(hook.session_id, "0b1e7c94-6f3a-4d21-9f0c-2ab5e8d17c43");

    let answer = client.expect("the hello result").await;
    // §6.2: null, and not an absent field. The schema requires the key.
    assert_eq!(answer["result"]["session_ref"], Value::Null);
    assert!(answer["result"]
        .as_object()
        .expect("an object")
        .contains_key("session_ref"));
}

#[tokio::test]
async fn a_wrong_token_is_refused_with_auth_failed_and_the_connection_closes() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;

    let id = client.id();
    let stranger = Token::generate();
    client.send(&server_hello(id, stranger.as_str(), 1)).await;

    let answer = client.expect("the refusal").await;
    assert_eq!(answer["id"], json!(id));
    assert_eq!(answer["error"]["code"], json!(-32001));
    assert_eq!(answer["error"]["message"], json!("auth_failed"));
    assert!(
        answer["error"].get("data").is_none(),
        "auth_failed carries no data"
    );
    client.expect_closed().await;

    // FM-10 is a connection that never was: no peer, so no `Connected` and no
    // `Disconnected`. A store that saw a session appear and vanish would show a tab for it.
    harness.no_event();
    assert_eq!(harness.handle.connections().await, 0);
}

#[tokio::test]
async fn another_protocol_version_is_refused_with_the_version_the_app_speaks() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;

    // The right token, the wrong version: §6.5 requires this peer to be told what to
    // update to, which is the whole reason the schema accepts any version in a `hello`.
    let id = client.id();
    client
        .send(&server_hello(id, harness.token.as_str(), 2))
        .await;

    let answer = client.expect("the refusal").await;
    assert_eq!(answer["error"]["code"], json!(-32002));
    assert_eq!(answer["error"]["message"], json!("protocol_unsupported"));
    assert_eq!(answer["error"]["data"]["protocol_version"], json!(1));
    client.expect_closed().await;
    harness.no_event();
}

#[tokio::test]
async fn a_peer_wrong_about_both_is_told_to_update_rather_than_to_repair_its_token() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;

    // The one case where the order of the two checks is visible. §6.5 exists so that a peer
    // speaking another version learns what to update to; a future version that also changed
    // the token would otherwise be sent looking for a broken installation instead.
    let id = client.id();
    client
        .send(&server_hello(id, Token::generate().as_str(), 2))
        .await;

    let answer = client.expect("the refusal").await;
    assert_eq!(answer["error"]["message"], json!("protocol_unsupported"));
    client.expect_closed().await;
    harness.no_event();
}

#[tokio::test]
async fn a_hello_that_never_comes_closes_the_connection() {
    let mut harness = harness_with(|config| {
        config.hello_timeout = Duration::from_millis(150);
    })
    .await;
    let mut client = Client::connect(&harness.endpoint).await;

    // Nothing is sent. §6.2 gives the peer two seconds and then closes, with no answer:
    // there is no request to answer.
    let started = Instant::now();
    client.expect_closed().await;
    assert!(started.elapsed() < PATIENCE);
    harness.no_event();
}

#[tokio::test]
async fn half_a_hello_is_no_hello() {
    let mut harness = harness_with(|config| {
        config.hello_timeout = Duration::from_millis(150);
    })
    .await;
    let mut client = Client::connect(&harness.endpoint).await;

    // A line with no newline is not a line. The timeout is what catches a peer that opens
    // the socket, writes most of a message and stops.
    let mut bytes = serde_json::to_vec(&server_hello(1, harness.token.as_str(), 1))
        .expect("the message serialises");
    bytes.truncate(bytes.len() - 4);
    client.send_raw(&bytes).await;

    client.expect_closed().await;
    harness.no_event();
}

#[tokio::test]
async fn a_first_message_that_is_not_a_hello_closes_the_connection() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;

    client
        .send(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping", "params": {} }))
        .await;

    client.expect_closed().await;
    harness.no_event();
}

// ---------------------------------------------------------------------------------------
// Framing and validation (§6.1, §6.3)
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_line_over_the_cap_closes_the_connection() {
    let mut harness = harness_with(|config| {
        config.max_message_bytes = 4096;
    })
    .await;
    let mut client = Client::connect(&harness.endpoint).await;
    client.register(&harness.token).await;
    let ChannelEvent::Connected(_) = harness.event("the connected peer").await else {
        panic!("Connected first");
    };

    client.send_raw(&vec![b'a'; 8192]).await;

    client.expect_closed().await;
    let ChannelEvent::Disconnected { reason, .. } = harness.event("the disconnection").await else {
        panic!("a closed connection is reported");
    };
    assert_eq!(reason, DisconnectReason::ProtocolViolation);
}

#[tokio::test]
async fn a_message_the_schema_refuses_closes_the_connection_without_an_answer() {
    for line in [
        // An unknown field on a known method: the schema is closed everywhere.
        json!({ "jsonrpc": "2.0", "id": 2, "method": "ping", "params": {}, "extra": 1 }),
        // A method that is not in the protocol.
        json!({ "jsonrpc": "2.0", "id": 2, "method": "handoff.explode", "params": {} }),
        // A `handoff.open` whose spec is not a spec.
        json!({ "jsonrpc": "2.0", "id": 2, "method": "handoff.open",
                "params": { "call_id": "call_2q7m8r1t", "spec": {}, "secret_treated": [] } }),
        // The wrong direction: `handoff.event` is the app's to send, never the server's.
        json!({ "jsonrpc": "2.0", "method": "app.shutdown", "params": { "reason": "no" } }),
    ] {
        let mut harness = harness().await;
        let mut client = Client::connect(&harness.endpoint).await;
        client.register(&harness.token).await;
        let ChannelEvent::Connected(_) = harness.event("the connected peer").await else {
            panic!("Connected first");
        };

        client.send(&line).await;

        client.expect_closed().await;
        let ChannelEvent::Disconnected { reason, .. } = harness.event("the disconnection").await
        else {
            panic!("a closed connection is reported");
        };
        assert_eq!(reason, DisconnectReason::ProtocolViolation, "{line}");
    }
}

#[tokio::test]
async fn a_second_hello_is_a_violation() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;
    client.register(&harness.token).await;
    let ChannelEvent::Connected(_) = harness.event("the connected peer").await else {
        panic!("Connected first");
    };

    let id = client.id();
    client
        .send(&server_hello(id, harness.token.as_str(), 1))
        .await;

    client.expect_closed().await;
    let ChannelEvent::Disconnected { reason, .. } = harness.event("the disconnection").await else {
        panic!("a closed connection is reported");
    };
    assert_eq!(reason, DisconnectReason::ProtocolViolation);
}

#[tokio::test]
async fn a_hook_may_not_open_a_handoff_and_a_server_may_not_ask_for_a_decision() {
    // §6.2 gives each role its own states: a hook is served one decision and closed, a
    // server carries handoff traffic. Neither may borrow the other's methods.
    let cases: [(bool, Value); 2] = [
        (true, handoff_open(2, "call_2q7m8r1t")),
        (
            false,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "hook.stop", "params": {} }),
        ),
    ];
    for (as_hook, line) in cases {
        let mut harness = harness().await;
        let mut client = Client::connect(&harness.endpoint).await;
        let id = client.id();
        if as_hook {
            client.send(&hook_hello(id, harness.token.as_str())).await;
        } else {
            client
                .send(&server_hello(id, harness.token.as_str(), 1))
                .await;
        }
        let ChannelEvent::Connected(_) = harness.event("the connected peer").await else {
            panic!("Connected first");
        };
        client.expect("the hello result").await;

        client.send(&line).await;

        client.expect_closed().await;
        let ChannelEvent::Disconnected { reason, .. } = harness.event("the disconnection").await
        else {
            panic!("a closed connection is reported");
        };
        assert_eq!(reason, DisconnectReason::ProtocolViolation, "{line}");
    }
}

// ---------------------------------------------------------------------------------------
// Traffic (§6.3, §7.3)
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_request_reaches_the_store_and_its_answer_reaches_the_peer() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;
    client.register(&harness.token).await;
    let ChannelEvent::Connected(peer) = harness.event("the connected peer").await else {
        panic!("Connected first");
    };
    let conn_id: ConnId = peer.conn_id;

    let id = client.id();
    client.send(&handoff_open(id, "call_2q7m8r1t")).await;

    let ChannelEvent::Request {
        conn_id: on,
        id: request_id,
        body,
    } = harness.event("the handoff.open").await
    else {
        panic!("a request that is not hello or ping goes to the store");
    };
    assert_eq!(on, conn_id);
    assert_eq!(request_id, RequestId::Number(id));
    let RequestBody::HandoffOpen(open) = *body else {
        panic!("the body is the one that was sent");
    };
    assert_eq!(open.call_id, "call_2q7m8r1t");
    assert_eq!(
        open.spec.goal,
        "Register the Stripe webhook for payment events"
    );
    // An explicit null and an absent key are two different messages, and this one is null.
    assert_eq!(open.request_id, Some(None));

    harness
        .handle
        .reply(
            conn_id,
            request_id,
            ResultBody::HandoffOpen(HandoffOpenResult {
                handoff_id: "hf_7k3m9p2q4r".to_owned(),
                resumed_from: None,
            }),
        )
        .await
        .expect("the connection is still there");

    let answer = client.expect("the handoff.open result").await;
    assert_eq!(answer["id"], json!(id));
    assert_eq!(answer["result"]["handoff_id"], json!("hf_7k3m9p2q4r"));
    assert_eq!(answer["result"]["resumed_from"], Value::Null);
}

#[tokio::test]
async fn a_hook_is_answered_once_and_then_the_connection_is_closed() {
    // §6.2's HookServed state: one `hook.stop`, one answer, close. The decision is the
    // store's (T-035) and so is the close, which is why `close` is on the handle: the
    // listener cannot know that an answer was the last thing this peer was owed.
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;
    let id = client.id();
    client.send(&hook_hello(id, harness.token.as_str())).await;
    let ChannelEvent::Connected(peer) = harness.event("the connected hook").await else {
        panic!("Connected first");
    };
    let conn_id = peer.conn_id;
    client.expect("the hello result").await;

    let ask = client.id();
    client
        .send(&json!({ "jsonrpc": "2.0", "id": ask, "method": "hook.stop", "params": {} }))
        .await;
    let ChannelEvent::Request {
        id: request_id,
        body,
        ..
    } = harness.event("the hook.stop").await
    else {
        panic!("hook.stop goes to the store");
    };
    assert!(matches!(*body, RequestBody::HookStop(_)));

    harness
        .handle
        .reply(
            conn_id,
            request_id,
            ResultBody::HookStop(HookStopResult {
                block: true,
                reason: Some(
                    "Handoff hf_7k3m9p2q4r is awaiting your verification report.".to_owned(),
                ),
            }),
        )
        .await
        .expect("the hook is still there");
    harness.handle.close(conn_id).await;

    // The answer is written first and the socket goes afterwards: a close that raced the
    // queue would leave the hook with nothing to say and the agent unblocked.
    let answer = client.expect("the decision").await;
    assert_eq!(answer["id"], json!(ask));
    assert_eq!(answer["result"]["block"], json!(true));
    client.expect_closed().await;

    let ChannelEvent::Disconnected { reason, .. } = harness.event("the disconnection").await else {
        panic!("a closed connection is reported");
    };
    assert_eq!(reason, DisconnectReason::Closed);
    assert_eq!(harness.handle.connections().await, 0);
}

#[tokio::test]
async fn a_ping_from_the_peer_is_answered_by_the_listener_itself() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;
    client.register(&harness.token).await;
    let ChannelEvent::Connected(_) = harness.event("the connected peer").await else {
        panic!("Connected first");
    };

    let id = client.id();
    client
        .send(&json!({ "jsonrpc": "2.0", "id": id, "method": "ping", "params": {} }))
        .await;

    let answer = client.expect("the pong").await;
    assert_eq!(answer["id"], json!(id));
    assert_eq!(answer["result"], json!({}));
    // A liveness check is a property of the connection, not of any handoff: the store
    // never hears about it.
    harness.no_event();
}

#[tokio::test]
async fn the_app_pings_after_silence_and_a_peer_that_answers_stays() {
    let mut harness = harness_with(|config| {
        config.ping_interval = Duration::from_millis(120);
    })
    .await;
    let mut client = Client::connect(&harness.endpoint).await;
    client.register(&harness.token).await;
    let ChannelEvent::Connected(_) = harness.event("the connected peer").await else {
        panic!("Connected first");
    };

    for _ in 0..3 {
        let ping = client.expect("a ping from the app").await;
        assert_eq!(ping["method"], json!("ping"));
        assert_eq!(ping["params"], json!({}));
        let id = ping["id"].clone();
        client
            .send(&json!({ "jsonrpc": "2.0", "id": id, "result": {} }))
            .await;
    }
    // Three pings answered and the connection is still the same one.
    assert_eq!(harness.handle.connections().await, 1);
    harness.no_event();
}

#[tokio::test]
async fn two_missed_pings_end_the_connection() {
    let mut harness = harness_with(|config| {
        config.ping_interval = Duration::from_millis(80);
    })
    .await;
    let mut client = Client::connect(&harness.endpoint).await;
    client.register(&harness.token).await;
    let ChannelEvent::Connected(_) = harness.event("the connected peer").await else {
        panic!("Connected first");
    };

    // Two pings arrive and are ignored; the third interval is what kills it (§6.3).
    for _ in 0..2 {
        let ping = client.expect("a ping from the app").await;
        assert_eq!(ping["method"], json!("ping"));
    }
    client.expect_closed().await;

    let ChannelEvent::Disconnected { reason, .. } = harness.event("the disconnection").await else {
        panic!("a dead connection is reported");
    };
    assert_eq!(reason, DisconnectReason::Unresponsive);
    assert_eq!(harness.handle.connections().await, 0);
}

#[tokio::test]
async fn an_answer_to_a_request_the_app_never_sent_is_a_violation() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;
    client.register(&harness.token).await;
    let ChannelEvent::Connected(_) = harness.event("the connected peer").await else {
        panic!("Connected first");
    };

    client
        .send(&json!({ "jsonrpc": "2.0", "id": 4242, "result": {} }))
        .await;

    client.expect_closed().await;
    let ChannelEvent::Disconnected { reason, .. } = harness.event("the disconnection").await else {
        panic!("a closed connection is reported");
    };
    assert_eq!(reason, DisconnectReason::ProtocolViolation);
}

#[tokio::test]
async fn session_bye_ends_the_connection_and_the_store_is_told() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;
    client.register(&harness.token).await;
    let ChannelEvent::Connected(_) = harness.event("the connected peer").await else {
        panic!("Connected first");
    };

    client
        .send(&json!({ "jsonrpc": "2.0", "method": "session.bye", "params": {} }))
        .await;

    let ChannelEvent::Notification { .. } = harness.event("the goodbye").await else {
        panic!("session.bye is a notification the store hears");
    };
    let ChannelEvent::Disconnected { reason, .. } = harness.event("the disconnection").await else {
        panic!("a closed connection is reported");
    };
    assert_eq!(reason, DisconnectReason::PeerLeft);
    client.expect_closed().await;
}

#[tokio::test]
async fn a_peer_that_drops_the_socket_is_reported_as_gone() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;
    client.register(&harness.token).await;
    let ChannelEvent::Connected(_) = harness.event("the connected peer").await else {
        panic!("Connected first");
    };

    drop(client);

    let ChannelEvent::Disconnected { reason, .. } = harness.event("the disconnection").await else {
        panic!("a closed connection is reported");
    };
    assert_eq!(reason, DisconnectReason::PeerLeft);
    assert_eq!(harness.handle.connections().await, 0);
}

// ---------------------------------------------------------------------------------------
// Refusals delay the next peer, and quitting tells everybody (§6.2, §6.3)
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_refused_hello_delays_the_next_connection() {
    let delay = Duration::from_millis(600);
    let mut harness = harness_with(move |config| {
        config.auth_failure_delay = delay;
    })
    .await;

    let mut refused = Client::connect(&harness.endpoint).await;
    let id = refused.id();
    refused
        .send(&server_hello(id, Token::generate().as_str(), 1))
        .await;
    assert_eq!(
        refused.expect("the refusal").await["error"]["message"],
        json!("auth_failed")
    );
    refused.expect_closed().await;

    let started = Instant::now();
    let mut honest = Client::connect(&harness.endpoint).await;
    honest.register(&harness.token).await;
    let waited = started.elapsed();

    // Not a measurement of the delay, a check that there was one: the point of §6.2 is that
    // a peer guessing tokens cannot try thousands of them a second.
    assert!(
        waited >= delay / 2,
        "the next peer was served after only {waited:?}"
    );
    let ChannelEvent::Connected(_) = harness.event("the connected peer").await else {
        panic!("the honest peer is served, only later");
    };
}

#[tokio::test]
async fn quitting_tells_every_peer_why_before_the_socket_goes() {
    let mut harness = harness().await;
    let mut first = Client::connect(&harness.endpoint).await;
    first.register(&harness.token).await;
    let ChannelEvent::Connected(_) = harness.event("the first peer").await else {
        panic!("Connected first");
    };
    let mut second = Client::connect(&harness.endpoint).await;
    second.register(&harness.token).await;
    let ChannelEvent::Connected(_) = harness.event("the second peer").await else {
        panic!("Connected first");
    };
    assert_eq!(harness.handle.connections().await, 2);

    harness
        .handle
        .shutdown("the user quit the app from the tray menu")
        .await;

    for client in [&mut first, &mut second] {
        let shutdown = client.expect("the shutdown notification").await;
        assert_eq!(shutdown["method"], json!("app.shutdown"));
        assert_eq!(
            shutdown["params"]["reason"],
            json!("the user quit the app from the tray menu")
        );
        // The notification is written and only then is the socket closed: a server told why
        // its channel went down degrades to text mode at once (§6.3).
        client.expect_closed().await;
    }
    assert_eq!(harness.handle.connections().await, 0);
}

#[tokio::test]
async fn sending_to_a_connection_that_has_gone_is_an_error_and_not_a_panic() {
    let mut harness = harness().await;
    let mut client = Client::connect(&harness.endpoint).await;
    client.register(&harness.token).await;
    let ChannelEvent::Connected(peer) = harness.event("the connected peer").await else {
        panic!("Connected first");
    };
    let conn_id = peer.conn_id;
    drop(client);
    let ChannelEvent::Disconnected { .. } = harness.event("the disconnection").await else {
        panic!("a closed connection is reported");
    };

    // SRV-21: a session that left is an ordinary state. The store finds out when it tries
    // to answer, and the handoff keeps running.
    let error = harness
        .handle
        .send(
            conn_id,
            ChannelMessage::Notification(Box::new(
                handoff_app_lib::format::channel::Notification {
                    jsonrpc: handoff_app_lib::format::channel::JsonRpcVersion::V2,
                    body: handoff_app_lib::format::channel::NotificationBody::AppShutdown(
                        handoff_app_lib::format::channel::AppShutdownParams {
                            reason: "nobody is listening".to_owned(),
                        },
                    ),
                },
            )),
        )
        .await;
    assert_eq!(
        error,
        Err(handoff_app_lib::channel::SendError::NotConnected)
    );
}

// ---------------------------------------------------------------------------------------
// The endpoint itself
// ---------------------------------------------------------------------------------------

#[cfg(windows)]
#[tokio::test]
async fn the_pipe_is_created_and_a_second_listener_cannot_take_its_name() {
    let harness = harness().await;
    let Endpoint::Pipe { name } = harness.endpoint.clone() else {
        panic!("this host listens on a pipe");
    };

    // It exists, with the DACL of A-16, and this process can reach it: the two ways a wrong
    // descriptor shows up without a second account.
    let _client = Client::connect(&harness.endpoint).await;

    // `first_pipe_instance` is what makes a name a claim. Without it a second process would
    // quietly create a second instance and take every other connection.
    let second = listen(ListenerConfig::new(
        Endpoint::Pipe { name: name.clone() },
        Token::generate(),
    ))
    .await;
    assert!(
        second.is_err(),
        "a second listener took the pipe name of the first"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_live_socket_is_kept_and_a_stale_one_is_cleared() {
    let dir = std::env::temp_dir().join(format!(
        "handoff-stale-{}-{}",
        std::process::id(),
        ids::new_session_ref()
    ));
    std::fs::create_dir_all(&dir).expect("the temporary directory is created");
    let path = dir.join("app.sock");
    let endpoint = || Endpoint::Unix {
        path: path.clone(),
        pointer: None,
    };

    let (first, _first_events) = listen(ListenerConfig::new(endpoint(), Token::generate()))
        .await
        .expect("the listener binds");
    assert!(path.exists(), "the socket file is there while it is bound");

    // Live: FM-12 removes a stale file only **after a failed liveness connect**, so a socket
    // somebody is serving has to survive an attempt to take it. Deleting first and asking
    // afterwards would take the endpoint away from a running instance.
    assert!(
        listen(ListenerConfig::new(endpoint(), Token::generate()))
            .await
            .is_err(),
        "a second listener took a live socket"
    );
    assert!(path.exists(), "the live socket file was removed");

    // Stale: the owner is gone and the file is not, which is what a crash leaves behind.
    // Nothing unlinks it, here or in production; the next start is what clears it.
    first.shutdown("the app quit").await;
    for _ in 0..200 {
        if std::os::unix::net::UnixStream::connect(&path).is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(path.exists(), "the test needs a file left behind to clear");

    let (third, _third_events) = listen(ListenerConfig::new(endpoint(), Token::generate()))
        .await
        .expect("a stale socket file is removed and the endpoint is bound");
    assert!(path.exists());
    third.shutdown("the app quit").await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn a_socket_that_does_not_fit_is_moved_and_the_pointer_file_says_where() {
    use handoff_app_lib::channel::endpoint::SUN_PATH_MAX_BYTES;

    let deep = std::env::temp_dir().join(format!(
        "handoff-deep-{}-{}",
        std::process::id(),
        "d".repeat(SUN_PATH_MAX_BYTES)
    ));
    std::fs::create_dir_all(&deep).expect("the deep directory is created");
    let path = std::env::temp_dir().join(format!("handoff-short-{}.sock", std::process::id()));
    let pointer = deep.join("app.sock.path");

    let (handle, _events) = listen(ListenerConfig::new(
        Endpoint::Unix {
            path: path.clone(),
            pointer: Some(pointer.clone()),
        },
        Token::generate(),
    ))
    .await
    .expect("the listener binds the short path");

    // FM-12: the server reads this file and finds the socket the app really bound.
    let written = std::fs::read_to_string(&pointer).expect("the pointer file is written");
    assert_eq!(written.trim(), path.display().to_string());

    handle.shutdown("done").await;
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&deep);
}

#[cfg(unix)]
#[tokio::test]
async fn the_socket_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let harness = harness().await;
    let Endpoint::Unix { path, .. } = harness.endpoint.clone() else {
        panic!("this host listens on a socket");
    };
    let mode = std::fs::metadata(&path)
        .expect("the socket file is there")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "another user of this machine can reach the socket"
    );
}
