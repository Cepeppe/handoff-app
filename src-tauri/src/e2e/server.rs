//! The transport of the automation channel: accept, authenticate, serve, answer (DD-33).
//!
//! NDJSON in both directions, one JSON-RPC object per line, exactly as the product channel
//! frames its traffic (§6.1) — the harness is a small Node client and a second framing to
//! learn would be a second thing to get wrong. What is *not* copied from §6 is the
//! handshake: there is no `hello`, no protocol version and no capability row, because the
//! peer is a test harness of this same commit and not an independently versioned program.
//! One `auth` carrying the token of `<HANDOFF_HOME>/e2e.token` opens the connection, every
//! method before it is refused, and a wrong token closes it after the same one-second delay
//! the product listener imposes (§6.2).
//!
//! A failure to start is logged and swallowed. The app under test must still come up when
//! the endpoint is taken — the harness reports "no automation channel" from the outside far
//! more legibly than a window that never appears.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::channel::token::Token;

use super::api::{self, codes, Failure, Request};
use super::endpoint::E2eEndpoint;

/// The largest line the channel accepts, in bytes. The product channel's limit (§6.6): a
/// `state` answer carrying a long spec is the biggest thing that crosses here.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// How long a refused connection stops the accept loop for (§6.2).
pub const AUTH_FAILURE_DELAY: Duration = Duration::from_secs(1);

/// The answer of one method, once it has been produced.
pub type MethodFuture<'a> = Pin<Box<dyn Future<Output = api::MethodResult> + Send + 'a>>;

/// What this transport serves.
///
/// A trait and not the application itself, for one reason: everything below — the accept
/// loop, the framing, the token, the one-second penalty, the shape of an answer — is code
/// that can be wrong on its own, and a test that had to stand a whole Tauri application up
/// to reach it would not be written. [`super::TauriMethods`] is the implementation the app
/// runs; `tests/e2e_channel.rs` drives the same loop with a counting double.
pub trait Methods: Send + Sync + 'static {
    /// Serves one authenticated request.
    fn serve<'a>(&'a self, request: &'a api::Request) -> MethodFuture<'a>;

    /// Ends the process, once the answer to `quit` is on the wire.
    fn quit(&self);
}

/// What the accept loop shares with every connection it spawns.
struct Shared {
    methods: Arc<dyn Methods>,
    token: Token,
}

/// Binds the endpoint and starts serving. Called from `setup()`, once there is an app.
///
/// # Errors
///
/// When the endpoint cannot be bound, which on Windows includes another instance of this
/// build already listening on the same pipe.
pub fn start(methods: Arc<dyn Methods>, endpoint: &E2eEndpoint, token: Token) -> io::Result<()> {
    let shared = Arc::new(Shared { methods, token });
    match endpoint {
        E2eEndpoint::Pipe { name } => spawn_pipe_accept_loop(shared, name),
        E2eEndpoint::Unix { path, pointer } => {
            spawn_unix_accept_loop(shared, path, pointer.as_deref())
        }
    }
}

#[cfg(windows)]
fn spawn_pipe_accept_loop(shared: Arc<Shared>, name: &str) -> io::Result<()> {
    use crate::channel::endpoint::security::SecurityDescriptor;

    let mut descriptor = SecurityDescriptor::user_only()?;
    let mut server = create_pipe(name, &mut descriptor, true)?;
    let name = name.to_owned();

    tokio::spawn(async move {
        loop {
            if let Err(error) = server.connect().await {
                tracing::warn!(error = %error, "the automation channel could not accept");
                continue;
            }
            let next = match create_pipe(&name, &mut descriptor, false) {
                Ok(next) => next,
                Err(error) => {
                    tracing::error!(error = %error, "the automation channel could not open a pipe instance");
                    break;
                }
            };
            let connected = std::mem::replace(&mut server, next);
            let shared = Arc::clone(&shared);
            tokio::spawn(serve(shared, connected));
        }
    });
    Ok(())
}

#[cfg(windows)]
fn create_pipe(
    name: &str,
    descriptor: &mut crate::channel::endpoint::security::SecurityDescriptor,
    first: bool,
) -> io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    let mut attributes = descriptor.attributes();
    let mut options = tokio::net::windows::named_pipe::ServerOptions::new();
    options.first_pipe_instance(first);
    // SAFETY: `attributes` points at `descriptor`, which outlives this call, and Windows
    // only reads it. The same call the product listener makes, for the same reason (A-16).
    unsafe { options.create_with_security_attributes_raw(name, (&raw mut attributes).cast()) }
}

#[cfg(not(windows))]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn spawn_pipe_accept_loop(_shared: Arc<Shared>, name: &str) -> io::Result<()> {
    Err(io::Error::other(format!(
        "{name} is a Windows named pipe and this is not Windows"
    )))
}

#[cfg(unix)]
fn spawn_unix_accept_loop(
    shared: Arc<Shared>,
    path: &std::path::Path,
    pointer: Option<&std::path::Path>,
) -> io::Result<()> {
    use crate::channel::endpoint::{clear_stale_socket, narrow_socket, StaleCheck};
    use tokio::net::UnixListener;

    // The rule of T-031, not the shortcut it corrects: only a refused connect means the
    // file is stale, everything else means somebody is there and this instance stops.
    match clear_stale_socket(path)? {
        StaleCheck::Live => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!(
                    "{} is already served by another e2e instance",
                    path.display()
                ),
            ))
        }
        StaleCheck::Absent | StaleCheck::Removed => {}
    }

    let listener = UnixListener::bind(path)?;
    narrow_socket(path)?;
    if let Some(pointer) = pointer {
        std::fs::write(pointer, format!("{}\n", path.display()))?;
    }

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let shared = Arc::clone(&shared);
                    tokio::spawn(serve(shared, stream));
                }
                Err(error) => {
                    tracing::warn!(error = %error, "the automation channel could not accept");
                }
            }
        }
    });
    Ok(())
}

#[cfg(not(unix))]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn spawn_unix_accept_loop(
    _shared: Arc<Shared>,
    path: &std::path::Path,
    _pointer: Option<&std::path::Path>,
) -> io::Result<()> {
    Err(io::Error::other(format!(
        "{} is a Unix socket and this is not a Unix host",
        path.display()
    )))
}

/// One connection, from the first line to the end of the stream.
async fn serve<S>(shared: Arc<Shared>, stream: S)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = Lines::new(reader, MAX_MESSAGE_BYTES);
    let mut authenticated = false;

    loop {
        let line = match lines.next().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                tracing::debug!(error = %error, "the automation channel dropped a connection");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let request: Request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                let failure = Failure::new(codes::INVALID_REQUEST, error.to_string());
                if answer(&mut writer, &serde_json::Value::Null, Err(failure))
                    .await
                    .is_err()
                {
                    break;
                }
                continue;
            }
        };
        let id = request.id.clone().unwrap_or(serde_json::Value::Null);

        let outcome = if authenticated {
            shared.methods.serve(&request).await
        } else if request.method == "auth" {
            match authenticate(&shared.token, &request) {
                Ok(result) => {
                    authenticated = true;
                    Ok(result)
                }
                Err(failure) => {
                    // §6.2: a wrong token costs a second before anything else is served, so
                    // a peer guessing at sixty-four hex characters gets one attempt per
                    // second and the log line that says so.
                    tracing::warn!("the automation channel refused a token");
                    let _ = answer(&mut writer, &id, Err(failure)).await;
                    tokio::time::sleep(AUTH_FAILURE_DELAY).await;
                    break;
                }
            }
        } else {
            Err(Failure::unauthenticated())
        };

        let quit = authenticated && api::is_quit(&request.method) && outcome.is_ok();
        if answer(&mut writer, &id, outcome).await.is_err() {
            break;
        }
        if quit {
            // The answer is on the wire; the app leaves through the same path the tray's
            // **Quit** takes, so the goodbye of §6.3 still reaches the peers (`lib.rs`).
            tracing::info!("the automation channel was asked to quit");
            let _ = writer.flush().await;
            shared.methods.quit();
            break;
        }
    }
}

/// Compares the presented token in constant time (§6.2, SRV-07).
fn authenticate(token: &Token, request: &Request) -> Result<serde_json::Value, Failure> {
    let presented = request
        .params
        .get("token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Failure::new(codes::INVALID_PARAMS, "auth takes a token"))?;
    if token.matches(presented) {
        Ok(serde_json::json!({ "app_version": env!("CARGO_PKG_VERSION") }))
    } else {
        Err(Failure::unauthenticated())
    }
}

/// Writes one JSON-RPC response.
async fn answer<W>(
    writer: &mut W,
    id: &serde_json::Value,
    outcome: api::MethodResult,
) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let message = match outcome {
        Ok(result) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(failure) => {
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": failure.to_json() })
        }
    };
    let mut line = serde_json::to_string(&message).unwrap_or_else(|error| {
        format!(
            r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":{},"message":"the answer could not be serialised: {}"}}}}"#,
            codes::INVALID_REQUEST,
            error.to_string().replace('"', "'")
        )
    });
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await
}

/// A newline-delimited reader with a cap, so one endless line cannot eat the process.
struct Lines<R> {
    reader: R,
    buffer: Vec<u8>,
    max: usize,
}

impl<R: AsyncRead + Unpin> Lines<R> {
    fn new(reader: R, max: usize) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
            max,
        }
    }

    /// The next line without its terminator, or `None` at the end of the stream.
    async fn next(&mut self) -> io::Result<Option<String>> {
        loop {
            if let Some(at) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let mut line: Vec<u8> = self.buffer.drain(..=at).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return String::from_utf8(line)
                    .map(Some)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not UTF-8"));
            }
            if self.buffer.len() > self.max {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a line longer than the limit",
                ));
            }
            let mut chunk = [0_u8; 8192];
            let read = self.reader.read(&mut chunk).await?;
            if read == 0 {
                return if self.buffer.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "a line with no terminator",
                    ))
                };
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lines_are_split_on_either_terminator() {
        let input: &[u8] = b"one\ntwo\r\nthree\n";
        let mut lines = Lines::new(input, 64);
        assert_eq!(lines.next().await.expect("a line"), Some("one".to_owned()));
        assert_eq!(lines.next().await.expect("a line"), Some("two".to_owned()));
        assert_eq!(
            lines.next().await.expect("a line"),
            Some("three".to_owned())
        );
        assert_eq!(lines.next().await.expect("the end"), None);
    }

    #[tokio::test]
    async fn a_line_over_the_limit_is_an_error_and_not_an_allocation() {
        let input = vec![b'x'; 200];
        let mut lines = Lines::new(input.as_slice(), 64);
        assert!(lines.next().await.is_err());
    }

    #[test]
    fn only_the_right_token_authenticates() {
        let token = Token::generate();
        let good: Request = serde_json::from_str(&format!(
            r#"{{"id":1,"method":"auth","params":{{"token":"{}"}}}}"#,
            token.as_str()
        ))
        .expect("a request");
        assert!(authenticate(&token, &good).is_ok());

        let bad: Request =
            serde_json::from_str(r#"{"id":1,"method":"auth","params":{"token":"nope"}}"#)
                .expect("a request");
        assert_eq!(
            authenticate(&token, &bad).expect_err("refused").code,
            codes::UNAUTHENTICATED
        );

        let none: Request = serde_json::from_str(r#"{"id":1,"method":"auth"}"#).expect("a request");
        assert_eq!(
            authenticate(&token, &none).expect_err("refused").code,
            codes::INVALID_PARAMS
        );
    }
}
