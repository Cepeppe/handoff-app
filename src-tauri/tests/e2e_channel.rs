//! The transport of the automation channel, end to end over a real endpoint (DD-33, §11.5).
//!
//! What this suite is for: the accept loop, the framing, the token and the shape of an
//! answer are code that can be wrong on its own, and every one of those failures looks the
//! same from a scenario — "the harness could not reach the app". So the loop is driven here
//! with a counting double in place of the application, over a real named pipe (or a real
//! Unix socket), by a client that speaks the same NDJSON the TypeScript harness speaks.
//!
//! What it deliberately does **not** cover: what the six methods *do*. That is
//! `ui_bridge::commands` — already covered by the suites of T-036 and T-038 — and the
//! adapter over it is exercised for real by `pnpm e2e`, against a running app and a running
//! agent, which is the only place it can be exercised honestly.
#![cfg(feature = "e2e")]

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use handoff_app_lib::channel::token::Token;
use handoff_app_lib::e2e::api::{codes, Request};
use handoff_app_lib::e2e::server::{self, MethodFuture, Methods};
use handoff_app_lib::e2e::E2eEndpoint;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

/// The application, counted rather than performed.
#[derive(Default)]
struct Counting {
    served: AtomicU32,
    quit: AtomicBool,
}

impl Methods for Counting {
    fn serve<'a>(&'a self, request: &'a Request) -> MethodFuture<'a> {
        let method = request.method.clone();
        let params = request.params.clone();
        self.served.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move {
            match method.as_str() {
                "state" => Ok(json!({ "handoffs": [], "sessions": [], "requests": [] })),
                "act" => Ok(json!({ "echo": params })),
                "quit" => Ok(json!({})),
                other => Err(handoff_app_lib::e2e::api::Failure::new(
                    codes::METHOD_NOT_FOUND,
                    format!("no method named {other}"),
                )),
            }
        })
    }

    fn quit(&self) {
        self.quit.store(true, Ordering::Relaxed);
    }
}

/// An endpoint no other test and no running app can be using.
fn unique_endpoint(what: &str) -> E2eEndpoint {
    let unique = format!(
        "{}-{}-{}",
        what,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock after 1970")
            .as_nanos()
    );
    if cfg!(windows) {
        E2eEndpoint::Pipe {
            name: format!(r"\\.\pipe\handoff-e2e-test-{unique}"),
        }
    } else {
        E2eEndpoint::Unix {
            path: std::env::temp_dir().join(format!("he2e-{unique}.sock")),
            pointer: None,
        }
    }
}

/// A connected client, as two halves.
struct Client {
    lines: Box<dyn tokio::io::AsyncBufRead + Unpin + Send>,
    writer: Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
}

impl Client {
    /// Connects, retrying while the accept loop is between two pipe instances.
    ///
    /// The retry is not optional on Windows and the T-031 handoff entry says why: the loop
    /// creates the next instance only after taking the previous one, so a client arriving in
    /// that window is refused with `ERROR_FILE_NOT_FOUND` or `ERROR_PIPE_BUSY` rather than
    /// waiting in a backlog.
    async fn connect(endpoint: &E2eEndpoint) -> Self {
        for attempt in 0..100_u32 {
            match Self::connect_once(endpoint).await {
                Ok(client) => return client,
                Err(error) if attempt == 99 => panic!("the endpoint never answered: {error}"),
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
        unreachable!("the loop above either returns or panics")
    }

    #[cfg(windows)]
    async fn connect_once(endpoint: &E2eEndpoint) -> std::io::Result<Self> {
        let E2eEndpoint::Pipe { name } = endpoint else {
            panic!("a pipe on Windows");
        };
        let stream = tokio::net::windows::named_pipe::ClientOptions::new().open(name)?;
        let (reader, writer) = tokio::io::split(stream);
        Ok(Self {
            lines: Box::new(BufReader::new(reader)),
            writer: Box::new(writer),
        })
    }

    #[cfg(not(windows))]
    async fn connect_once(endpoint: &E2eEndpoint) -> std::io::Result<Self> {
        let E2eEndpoint::Unix { path, .. } = endpoint else {
            panic!("a socket off Windows");
        };
        let stream = tokio::net::UnixStream::connect(path).await?;
        let (reader, writer) = tokio::io::split(stream);
        Ok(Self {
            lines: Box::new(BufReader::new(reader)),
            writer: Box::new(writer),
        })
    }

    async fn send(&mut self, message: &Value) {
        let line = format!("{message}\n");
        self.writer
            .write_all(line.as_bytes())
            .await
            .expect("the request goes out");
        self.writer.flush().await.expect("the request is flushed");
    }

    /// The next answer, or `None` when the peer closed.
    async fn answer(&mut self) -> Option<Value> {
        let mut line = String::new();
        let read = self
            .lines
            .read_line(&mut line)
            .await
            .expect("the answer arrives");
        if read == 0 {
            return None;
        }
        Some(serde_json::from_str(&line).expect("an answer in JSON"))
    }

    async fn call(&mut self, id: u32, method: &str, params: Value) -> Value {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        self.answer().await.expect("an answer")
    }
}

/// A listening channel and the double behind it.
fn listening(what: &str) -> (E2eEndpoint, Token, Arc<Counting>) {
    let endpoint = unique_endpoint(what);
    let token = Token::generate();
    let methods = Arc::new(Counting::default());
    server::start(
        Arc::clone(&methods) as Arc<dyn Methods>,
        &endpoint,
        token.clone(),
    )
    .expect("the automation channel binds");
    (endpoint, token, methods)
}

#[tokio::test]
async fn a_method_before_auth_is_refused_and_the_connection_stays_open() {
    let (endpoint, token, methods) = listening("before-auth");
    let mut client = Client::connect(&endpoint).await;

    let refused = client.call(1, "state", json!({})).await;
    assert_eq!(refused["error"]["code"], codes::UNAUTHENTICATED);
    assert_eq!(
        methods.served.load(Ordering::Relaxed),
        0,
        "nothing reached the application"
    );

    // The connection is still usable: a harness that forgot the token is told so and may
    // authenticate on the same socket rather than reconnecting.
    let hello = client
        .call(2, "auth", json!({ "token": token.as_str() }))
        .await;
    assert!(hello["result"]["app_version"].is_string());

    let state = client.call(3, "state", json!({})).await;
    assert_eq!(state["result"]["handoffs"], json!([]));
    assert_eq!(state["id"], 3, "the answer carries the request's id");
}

#[tokio::test]
async fn a_wrong_token_is_refused_and_the_connection_is_closed() {
    let (endpoint, _token, methods) = listening("wrong-token");
    let mut client = Client::connect(&endpoint).await;

    let refused = client
        .call(1, "auth", json!({ "token": "0".repeat(64) }))
        .await;
    assert_eq!(refused["error"]["code"], codes::UNAUTHENTICATED);
    assert_eq!(
        client.answer().await,
        None,
        "the peer closed the connection"
    );
    assert_eq!(methods.served.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn params_travel_and_an_unknown_method_is_named() {
    let (endpoint, token, _methods) = listening("params");
    let mut client = Client::connect(&endpoint).await;
    client
        .call(1, "auth", json!({ "token": token.as_str() }))
        .await;

    let echoed = client
        .call(
            2,
            "act",
            json!({ "handoff_id": "hf_0123456789", "action": "confirm" }),
        )
        .await;
    assert_eq!(echoed["result"]["echo"]["action"], "confirm");

    let unknown = client.call(3, "nonesuch", json!({})).await;
    assert_eq!(unknown["error"]["code"], codes::METHOD_NOT_FOUND);
    assert!(
        unknown["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("nonesuch"),
        "the message names the method"
    );
}

#[tokio::test]
async fn a_line_that_is_not_json_is_answered_rather_than_dropped() {
    let (endpoint, token, _methods) = listening("bad-line");
    let mut client = Client::connect(&endpoint).await;
    client
        .call(1, "auth", json!({ "token": token.as_str() }))
        .await;

    client
        .writer
        .write_all(b"not json at all\n")
        .await
        .expect("written");
    client.writer.flush().await.expect("flushed");
    let answer = client.answer().await.expect("an answer");
    assert_eq!(answer["error"]["code"], codes::INVALID_REQUEST);
    assert_eq!(answer["id"], Value::Null);

    // And the connection survives it, so a harness bug costs one line and not a run.
    let state = client.call(2, "state", json!({})).await;
    assert_eq!(state["result"]["handoffs"], json!([]));
}

#[tokio::test]
async fn quit_is_answered_before_the_application_is_asked_to_leave() {
    let (endpoint, token, methods) = listening("quit");
    let mut client = Client::connect(&endpoint).await;
    client
        .call(1, "auth", json!({ "token": token.as_str() }))
        .await;

    let answer = client.call(2, "quit", json!({})).await;
    assert_eq!(answer["result"], json!({}));

    // The harness has its answer; the exit follows. Asserting the order is the point: an
    // implementation that exited first would leave every scenario reading an EOF and
    // reporting a crash instead of a clean shutdown.
    for _ in 0..100 {
        if methods.quit.load(Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        methods.quit.load(Ordering::Relaxed),
        "the application was asked to leave"
    );
}

#[tokio::test]
async fn two_harnesses_can_be_connected_at_once() {
    let (endpoint, token, _methods) = listening("two-clients");
    let mut first = Client::connect(&endpoint).await;
    let mut second = Client::connect(&endpoint).await;

    first
        .call(1, "auth", json!({ "token": token.as_str() }))
        .await;
    second
        .call(1, "auth", json!({ "token": token.as_str() }))
        .await;

    // Authentication is per connection: the second one is not let in by the first.
    assert_eq!(first.call(2, "state", json!({})).await["id"], 2);
    assert_eq!(second.call(2, "state", json!({})).await["id"], 2);
}
