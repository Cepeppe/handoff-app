//! The accept loop and one connection's life (§7.3, §6.1, §6.2, §6.3, §6.5, §6.6).
//!
//! The app listens and servers connect (SRV-05). One task accepts; every connection gets a
//! reader task, a writer task and a [`Peer`] record. What the reader does with a line is
//! always the same four steps — frame it, validate it against `channel.v1.schema.json`,
//! decode it into the types of [`crate::format::channel`], then either answer it here or
//! hand it to the store — and a failure at any of the first three closes the connection
//! without an answer, which is what the protocol says a framing violation deserves.
//!
//! # What this module answers itself, and what it hands on
//!
//! Exactly two messages are answered here, because both are properties of the connection
//! rather than of any handoff: `hello` (the token, the version, the `session_ref`) and
//! `ping`. Everything else becomes a [`ChannelEvent`] for the store actor of T-034, which
//! replies through [`ChannelHandle::send`]. That split is what keeps the state machine on
//! one task and out of the socket code (§7.3), and it is why this module has no idea what a
//! handoff is.
//!
//! # Three rules that look like details
//!
//! - **The peer is registered before its `hello` is answered.** A server that reads the
//!   answer may send its first request in the same breath, and on a Unix socket both lines
//!   can arrive in one read; a peer that is not in the connection table yet cannot be
//!   answered. The server met the mirror of this bug in T-020 and it only ever failed on
//!   one platform.
//! - **A framing violation is never answered.** `protocol/channel/README.md` is explicit:
//!   a malformed or unknown message closes the connection, and only `auth_failed` and
//!   `protocol_unsupported` travel as JSON-RPC errors.
//! - **The version is checked before the token.** Both are checked and both close, so the
//!   order matters in one case only: a peer that speaks another protocol version *and*
//!   presents a token this version would refuse. §6.5 requires that peer to be told to
//!   update — that is the whole reason the schema accepts any `protocol_version` in a
//!   `hello` — and `auth_failed` would send it looking for a broken installation instead.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::{mpsc, watch, Mutex};
use tokio::time::Instant;

use crate::format::channel::{
    AppShutdownParams, CapabilityRow, ChannelError, ChannelErrorCode, ChannelErrorData,
    ChannelMessage, ClientInfo, ErrorResponse, HelloParams, HelloResult, HookInput, Identity,
    JsonRpcVersion, Notification, NotificationBody, ProtocolUnsupportedData, Request, RequestBody,
    RequestId, Response, ResultBody, PROTOCOL_VERSION,
};
use crate::format::schema::{is_valid, Document};
use crate::ids;
use crate::log::Timestamp;

use super::endpoint::Endpoint;
use super::token::Token;

/// §6.1: a longer line closes the connection.
pub const CHANNEL_MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// §6.2: the first message must arrive within this.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(2);

/// §6.3: a `ping` after this much silence.
pub const PING_INTERVAL: Duration = Duration::from_secs(30);

/// §6.3: this many pings without an answer mean the connection is dead.
pub const MISSED_PINGS_BEFORE_DEAD: usize = 2;

/// §6.2: how long the next accept waits after a refused connection.
pub const AUTH_FAILURE_DELAY: Duration = Duration::from_secs(1);

/// How many messages may be queued for one connection before a sender waits. A blocking
/// call's outcome is one message and a screenshot is one more; a peer that cannot drain
/// this many is a peer that has stopped reading.
const OUTBOUND_QUEUE: usize = 64;

/// How many events may be queued for the store actor before the reader that produced one
/// waits. Back-pressure on purpose: a store that has fallen behind slows the socket down
/// instead of growing a queue nobody bounds.
const EVENT_QUEUE: usize = 256;

/// The read buffer of one connection.
const READ_CHUNK: usize = 64 * 1024;

/// One connection, for as long as it lasts.
pub type ConnId = u64;

/// Which side of the contract a peer is on (§6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerRole {
    /// An MCP server: registers a session and carries handoff traffic.
    Server,
    /// A Stop hook: answers one `hook.stop` and goes away.
    Hook,
}

/// Who is on the other end of an authenticated connection (§7.3).
///
/// §7.3 lists six fields; the four after them are what the consumers of the event stream
/// need and what `hello` already carries. `sessions::register` (T-032) builds its record
/// from `agent_id`, `client` and `identity`, and `hook::decide` (T-035) from `hook`; making
/// them look the message up again would mean keeping the message, which is the same record
/// under another name.
#[derive(Debug, Clone)]
pub struct Peer {
    /// The connection it arrived on.
    pub conn_id: ConnId,
    /// Server or hook.
    pub role: PeerRole,
    /// Assigned to a server at registration; `None` for a hook (§6.2).
    pub session_ref: Option<String>,
    /// The process that connected, as it described itself (§5.8, DD-22).
    pub identity: Identity,
    /// The row the server resolved for this session; `None` for a hook.
    pub capability_row: Option<CapabilityRow>,
    /// When the `hello` was accepted.
    pub authenticated_at: Timestamp,
    /// The agent the capability row was resolved for; `None` for a hook.
    pub agent_id: Option<String>,
    /// `clientInfo` of the MCP handshake; `None` for a hook.
    pub client: Option<ClientInfo>,
    /// The server's own version; a hook may omit it.
    pub server_version: Option<String>,
    /// The hook payload, for a hook and only for a hook.
    pub hook: Option<HookInput>,
}

/// Why a connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    /// The peer closed the socket, or sent `session.bye`.
    PeerLeft,
    /// A framing, schema or direction violation (§6.3).
    ProtocolViolation,
    /// Two pings went unanswered (§6.3).
    Unresponsive,
    /// The app is quitting, or the store asked for the connection to be closed.
    Closed,
}

/// What the listener tells the store actor (T-034).
#[derive(Debug)]
pub enum ChannelEvent {
    /// A peer authenticated. Always the first event of a connection.
    Connected(Box<Peer>),
    /// A request that is not `hello` or `ping`: it needs an answer through
    /// [`ChannelHandle::send`], addressed to the same `conn_id` and carrying the same `id`.
    Request {
        /// Which connection asked.
        conn_id: ConnId,
        /// The id the answer must carry.
        id: RequestId,
        /// The method and its parameters.
        body: Box<RequestBody>,
    },
    /// A notification, which expects nothing back.
    Notification {
        /// Which connection sent it.
        conn_id: ConnId,
        /// The method and its parameters.
        body: Box<NotificationBody>,
    },
    /// The connection is gone. Only ever follows a [`ChannelEvent::Connected`].
    Disconnected {
        /// Which connection.
        conn_id: ConnId,
        /// Why.
        reason: DisconnectReason,
    },
}

/// What the listener needs to know before it binds anything.
///
/// The timings are fields rather than constants so a test can drive the same code paths in
/// milliseconds; [`ListenerConfig::new`] fills them from the constants of §6, and a unit
/// test asserts that it does.
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    /// Where to listen (§5.8, DD-26).
    pub endpoint: Endpoint,
    /// The token every peer has to present (SRV-07).
    pub token: Token,
    /// What the `hello` result reports as `app_version`.
    pub app_version: String,
    /// §6.2: how long a peer has to send its `hello`.
    pub hello_timeout: Duration,
    /// §6.3: how much silence triggers a `ping`.
    pub ping_interval: Duration,
    /// §6.2: how long the next accept waits after a refused connection.
    pub auth_failure_delay: Duration,
    /// §6.1: the size cap of one line.
    pub max_message_bytes: usize,
}

impl ListenerConfig {
    /// The configuration of §6: the constants of this module, and the version of this build.
    #[must_use]
    pub fn new(endpoint: Endpoint, token: Token) -> Self {
        Self {
            endpoint,
            token,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            hello_timeout: HELLO_TIMEOUT,
            ping_interval: PING_INTERVAL,
            auth_failure_delay: AUTH_FAILURE_DELAY,
            max_message_bytes: CHANNEL_MAX_MESSAGE_BYTES,
        }
    }
}

/// What [`ChannelHandle::send`] can fail with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// The connection has gone since the event that named it was emitted. Every caller
    /// treats this as "the session left": the handoff keeps running and the outcome is
    /// queued for the next resume (SRV-21, SRV-22).
    #[error("the channel connection is gone")]
    NotConnected,
}

/// One live connection, as the rest of the app addresses it.
struct Conn {
    /// What the writer task drains.
    outbound: mpsc::Sender<ChannelMessage>,
    /// Asked for by [`ChannelHandle::close`]. A permit is stored, so a reader that is not
    /// waiting yet still sees it on its next turn round the loop.
    close: Arc<tokio::sync::Notify>,
}

/// The state one listener shares between its accept loop and its connections.
struct Shared {
    config: ListenerConfig,
    events: mpsc::Sender<ChannelEvent>,
    connections: Mutex<HashMap<ConnId, Conn>>,
    next_conn_id: AtomicU64,
    stop: watch::Sender<bool>,
    /// The instant before which the accept loop must not serve anybody (§6.2).
    accept_not_before: Mutex<Option<Instant>>,
}

impl Shared {
    async fn emit(&self, event: ChannelEvent) -> bool {
        self.events.send(event).await.is_ok()
    }

    async fn register(
        &self,
        conn_id: ConnId,
        outbound: mpsc::Sender<ChannelMessage>,
        close: Arc<tokio::sync::Notify>,
    ) {
        self.connections
            .lock()
            .await
            .insert(conn_id, Conn { outbound, close });
    }

    async fn deregister(&self, conn_id: ConnId) {
        self.connections.lock().await.remove(&conn_id);
    }

    /// Delays the next accept, after a refused `hello` (§6.2).
    async fn penalise(&self) {
        let until = Instant::now() + self.config.auth_failure_delay;
        let mut slot = self.accept_not_before.lock().await;
        if slot.is_none_or(|current| current < until) {
            *slot = Some(until);
        }
    }

    /// Waits out a penalty left by a refused connection, if there is one.
    async fn await_accept_permission(&self) {
        let until = *self.accept_not_before.lock().await;
        if let Some(instant) = until {
            tokio::time::sleep_until(instant).await;
            let mut slot = self.accept_not_before.lock().await;
            if *slot == Some(instant) {
                *slot = None;
            }
        }
    }
}

/// The listener, once it is running.
///
/// Cloning it is cheap and every clone addresses the same listener. Dropping every clone
/// does **not** stop it: quitting is an act, [`ChannelHandle::shutdown`], because the peers
/// are owed the `app.shutdown` notification of §6.3 before the socket goes away.
#[derive(Clone)]
pub struct ChannelHandle {
    shared: Arc<Shared>,
}

impl ChannelHandle {
    /// Where this listener is listening.
    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        &self.shared.config.endpoint
    }

    /// Sends one message to one connection.
    ///
    /// # Errors
    ///
    /// [`SendError::NotConnected`] when that connection has gone.
    pub async fn send(&self, conn_id: ConnId, message: ChannelMessage) -> Result<(), SendError> {
        let outbound = {
            let connections = self.shared.connections.lock().await;
            connections.get(&conn_id).map(|conn| conn.outbound.clone())
        };
        let outbound = outbound.ok_or(SendError::NotConnected)?;
        outbound
            .send(message)
            .await
            .map_err(|_| SendError::NotConnected)
    }

    /// Answers a request. The convenience the store actor uses for every reply, so that no
    /// caller has to remember the envelope.
    ///
    /// # Errors
    ///
    /// [`SendError::NotConnected`] when that connection has gone.
    pub async fn reply(
        &self,
        conn_id: ConnId,
        id: RequestId,
        result: ResultBody,
    ) -> Result<(), SendError> {
        self.send(
            conn_id,
            ChannelMessage::Response(Box::new(Response {
                jsonrpc: JsonRpcVersion::V2,
                id,
                result,
            })),
        )
        .await
    }

    /// Closes one connection, after everything already queued for it has been written.
    ///
    /// This is what ends a hook connection: §6.2 serves a hook one `hook.stop` and closes,
    /// and the answer is the store's to send, so the close has to be the store's to ask for.
    pub async fn close(&self, conn_id: ConnId) {
        let conn = self.shared.connections.lock().await.remove(&conn_id);
        if let Some(conn) = conn {
            // The reader stops; the writer then drains what is already queued — the answer
            // this close follows — before the socket goes.
            conn.close.notify_one();
        }
    }

    /// How many connections are authenticated right now. For the UI banner and the tests.
    pub async fn connections(&self) -> usize {
        self.shared.connections.lock().await.len()
    }

    /// Tells every peer the app is going away, then stops listening (§6.3).
    ///
    /// The notification is queued before the accept loop is stopped, and every connection
    /// drains what is queued for it before its socket closes, so a server that is still
    /// alive learns why its channel went down instead of inferring it from an EOF.
    pub async fn shutdown(&self, reason: &str) {
        let message = ChannelMessage::Notification(Box::new(Notification {
            jsonrpc: JsonRpcVersion::V2,
            body: NotificationBody::AppShutdown(AppShutdownParams {
                reason: reason.to_owned(),
            }),
        }));
        let outbounds: Vec<mpsc::Sender<ChannelMessage>> = {
            let connections = self.shared.connections.lock().await;
            connections
                .values()
                .map(|conn| conn.outbound.clone())
                .collect()
        };
        for outbound in outbounds {
            let _ = outbound.send(message.clone()).await;
        }
        self.shared.connections.lock().await.clear();
        let _ = self.shared.stop.send(true);
    }
}

/// Binds the endpoint and starts accepting (§7.2 startup, §6.2).
///
/// Returns the handle and the event stream the store actor consumes. The accept loop runs
/// on its own task until [`ChannelHandle::shutdown`].
///
/// # Errors
///
/// When the endpoint cannot be bound: another instance is already listening on it, the
/// pointer file cannot be written, or — on Windows — the security descriptor of A-16
/// cannot be built, in which case the pipe is deliberately not created rather than created
/// with the default DACL.
pub async fn listen(
    config: ListenerConfig,
) -> io::Result<(ChannelHandle, mpsc::Receiver<ChannelEvent>)> {
    let (events, receiver) = mpsc::channel(EVENT_QUEUE);
    let (stop, _) = watch::channel(false);
    let shared = Arc::new(Shared {
        config,
        events,
        connections: Mutex::new(HashMap::new()),
        next_conn_id: AtomicU64::new(1),
        stop,
        accept_not_before: Mutex::new(None),
    });

    match shared.config.endpoint.clone() {
        Endpoint::Pipe { name } => spawn_pipe_accept_loop(Arc::clone(&shared), &name)?,
        Endpoint::Unix { path, pointer } => {
            spawn_unix_accept_loop(Arc::clone(&shared), &path, pointer.as_deref())?;
        }
    }

    Ok((ChannelHandle { shared }, receiver))
}

#[cfg(windows)]
fn spawn_pipe_accept_loop(shared: Arc<Shared>, name: &str) -> io::Result<()> {
    use super::endpoint::security::SecurityDescriptor;

    let mut descriptor = SecurityDescriptor::user_only()?;
    // The first instance is created here rather than in the task, so that a name already
    // taken by another Baton is an error from `listen` and not a message in a log nobody
    // reads. `first_pipe_instance` is what makes that check real: without it, a second
    // process would quietly create a second instance of the same pipe and take every
    // second connection.
    let mut server = create_pipe(name, &mut descriptor, true)?;
    let name = name.to_owned();

    tokio::spawn(async move {
        let mut stop = shared.stop.subscribe();
        loop {
            tokio::select! {
                _ = stop.changed() => break,
                result = server.connect() => {
                    if let Err(error) = result {
                        tracing::warn!(error = %error, "the channel could not accept a connection");
                        continue;
                    }
                }
            }
            // The pipe instance created before the loop was entered is connected even when
            // the shutdown won the select, so this is the second half of that check.
            if *shared.stop.borrow() {
                break;
            }
            // §6.2: after a refused hello, nothing is served for a second. The wait is here
            // rather than before the accept because a loop already blocked in `connect()`
            // when the refusal happened would otherwise let the very next peer straight in.
            shared.await_accept_permission().await;
            let next = match create_pipe(&name, &mut descriptor, false) {
                Ok(next) => next,
                Err(error) => {
                    tracing::error!(error = %error, "the channel could not open a pipe instance");
                    break;
                }
            };
            let connected = std::mem::replace(&mut server, next);
            spawn_connection(Arc::clone(&shared), connected);
        }
    });
    Ok(())
}

#[cfg(windows)]
fn create_pipe(
    name: &str,
    descriptor: &mut super::endpoint::security::SecurityDescriptor,
    first: bool,
) -> io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    let mut attributes = descriptor.attributes();
    let mut options = tokio::net::windows::named_pipe::ServerOptions::new();
    options.first_pipe_instance(first);
    // SAFETY: `attributes` points at `descriptor`, which outlives this call, and Windows
    // only reads it. This is the one way tokio offers to give a pipe a DACL of our own
    // (A-16); the default would be whatever the process token hands down.
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
    path: &Path,
    pointer: Option<&Path>,
) -> io::Result<()> {
    use tokio::net::UnixListener;

    // FM-12, and in this order: a socket file whose owner is gone refuses a connection at
    // once, one whose owner is alive accepts it. Deleting first and asking afterwards would
    // take the endpoint away from a running instance.
    use super::endpoint::StaleCheck;
    match super::endpoint::clear_stale_socket(path)? {
        StaleCheck::Live => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("{} is already served by another instance", path.display()),
            ))
        }
        StaleCheck::Absent | StaleCheck::Removed => {}
    }

    let listener = UnixListener::bind(path)?;
    super::endpoint::narrow_socket(path)?;
    if let Some(pointer) = pointer {
        std::fs::write(pointer, format!("{}\n", path.display()))?;
    }

    tokio::spawn(async move {
        let mut stop = shared.stop.subscribe();
        loop {
            let accepted = tokio::select! {
                _ = stop.changed() => break,
                accepted = listener.accept() => accepted,
            };
            // §6.2: after a refused hello, nothing is served for a second. See the Windows
            // loop above for why the wait is here and not before the accept.
            shared.await_accept_permission().await;
            match accepted {
                Ok((stream, _)) => spawn_connection(Arc::clone(&shared), stream),
                Err(error) => {
                    tracing::warn!(error = %error, "the channel could not accept a connection");
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
    path: &Path,
    _pointer: Option<&Path>,
) -> io::Result<()> {
    Err(io::Error::other(format!(
        "{} is a Unix socket and this is not a Unix host",
        path.display()
    )))
}

/// Gives one accepted stream its reader and writer tasks.
fn spawn_connection<S>(shared: Arc<Shared>, stream: S)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let conn_id = shared.next_conn_id.fetch_add(1, Ordering::Relaxed);
    tokio::spawn(async move {
        serve(shared, conn_id, stream).await;
    });
}

async fn serve<S>(shared: Arc<Shared>, conn_id: ConnId, stream: S)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (reader, writer) = tokio::io::split(stream);
    let (outbound, queue) = mpsc::channel::<ChannelMessage>(OUTBOUND_QUEUE);
    let writer_task = tokio::spawn(write_loop(writer, queue));

    let outcome = read_loop(&shared, conn_id, reader, &outbound).await;

    shared.deregister(conn_id).await;
    // Dropping the last sender is what ends the writer task, and it ends it only after
    // everything already queued has been written: that is how an `auth_failed` and an
    // `app.shutdown` reach a peer whose connection is being closed in the same breath.
    drop(outbound);
    let _ = writer_task.await;

    if outcome.authenticated {
        shared
            .emit(ChannelEvent::Disconnected {
                conn_id,
                reason: outcome.reason,
            })
            .await;
    }
}

/// What one connection came to.
struct Outcome {
    authenticated: bool,
    reason: DisconnectReason,
}

async fn read_loop<R>(
    shared: &Shared,
    conn_id: ConnId,
    reader: R,
    outbound: &mpsc::Sender<ChannelMessage>,
) -> Outcome
where
    R: AsyncRead + Unpin,
{
    let mut frames = Frames::new(reader, shared.config.max_message_bytes);
    let mut stop = shared.stop.subscribe();

    let (peer, hello_id) = match handshake(shared, conn_id, &mut frames, outbound, &mut stop).await
    {
        Handshake::Authenticated { peer, hello_id } => (*peer, hello_id),
        Handshake::Refused => {
            return Outcome {
                authenticated: false,
                reason: DisconnectReason::ProtocolViolation,
            }
        }
        Handshake::Closed(reason) => {
            return Outcome {
                authenticated: false,
                reason,
            }
        }
    };
    let role = peer.role;
    let session_ref = peer.session_ref.clone();

    // Registered before the answer is written: see the module documentation.
    let close = Arc::new(tokio::sync::Notify::new());
    shared
        .register(conn_id, outbound.clone(), Arc::clone(&close))
        .await;
    if !shared.emit(ChannelEvent::Connected(Box::new(peer))).await {
        return Outcome {
            authenticated: false,
            reason: DisconnectReason::Closed,
        };
    }
    let hello_result = ResultBody::Hello(HelloResult {
        app_version: shared.config.app_version.clone(),
        protocol_version: PROTOCOL_VERSION,
        session_ref,
    });
    if send_response(outbound, hello_id, hello_result)
        .await
        .is_err()
    {
        return Outcome {
            authenticated: true,
            reason: DisconnectReason::Closed,
        };
    }

    let reason = pump(
        shared,
        conn_id,
        role,
        &mut frames,
        outbound,
        &mut stop,
        &close,
    )
    .await;
    Outcome {
        authenticated: true,
        reason,
    }
}

/// The lines a connection carries after its `hello`.
async fn pump<R>(
    shared: &Shared,
    conn_id: ConnId,
    role: PeerRole,
    frames: &mut Frames<R>,
    outbound: &mpsc::Sender<ChannelMessage>,
    stop: &mut watch::Receiver<bool>,
    close: &tokio::sync::Notify,
) -> DisconnectReason
where
    R: AsyncRead + Unpin,
{
    let mut pings = Pings::default();
    let mut silence = Box::pin(tokio::time::sleep(shared.config.ping_interval));

    loop {
        let line = tokio::select! {
            _ = stop.changed() => return DisconnectReason::Closed,
            () = close.notified() => return DisconnectReason::Closed,
            () = &mut silence => {
                if pings.outstanding() >= MISSED_PINGS_BEFORE_DEAD {
                    tracing::info!(conn_id, "the channel peer stopped answering pings");
                    return DisconnectReason::Unresponsive;
                }
                let id = pings.next_id();
                if send_request(outbound, RequestId::Number(id), RequestBody::Ping(
                    crate::format::channel::NoParams {},
                ))
                .await
                .is_err()
                {
                    return DisconnectReason::Closed;
                }
                silence.as_mut().reset(Instant::now() + shared.config.ping_interval);
                continue;
            }
            line = frames.next() => line,
        };

        let line = match line {
            Ok(Some(line)) => line,
            Ok(None) => return DisconnectReason::PeerLeft,
            Err(FrameError::TooLarge) => {
                tracing::warn!(conn_id, "the channel peer sent an oversized line");
                return DisconnectReason::ProtocolViolation;
            }
            Err(FrameError::Io) => return DisconnectReason::PeerLeft,
        };

        silence
            .as_mut()
            .reset(Instant::now() + shared.config.ping_interval);

        let Some(message) = decode(&line) else {
            tracing::warn!(
                conn_id,
                "the channel peer sent a message the schema refuses"
            );
            return DisconnectReason::ProtocolViolation;
        };

        match dispatch(shared, conn_id, role, message, outbound, &mut pings).await {
            Flow::Continue => {}
            Flow::Close(reason) => return reason,
        }
    }
}

/// What one dispatched message decided about the connection.
enum Flow {
    Continue,
    Close(DisconnectReason),
}

async fn dispatch(
    shared: &Shared,
    conn_id: ConnId,
    role: PeerRole,
    message: ChannelMessage,
    outbound: &mpsc::Sender<ChannelMessage>,
    pings: &mut Pings,
) -> Flow {
    match message {
        ChannelMessage::Request(request) => {
            let Request { id, body, .. } = *request;
            match body {
                // §6.2: the first message is the `hello`, and there is only one.
                RequestBody::Hello(_) => Flow::Close(DisconnectReason::ProtocolViolation),
                RequestBody::Ping(_) => {
                    if send_response(outbound, id, ResultBody::Empty(Default::default()))
                        .await
                        .is_err()
                    {
                        return Flow::Close(DisconnectReason::Closed);
                    }
                    Flow::Continue
                }
                other => {
                    if !role_may_send(role, &other) {
                        tracing::warn!(
                            conn_id,
                            "the channel peer sent a method its role has no business sending"
                        );
                        return Flow::Close(DisconnectReason::ProtocolViolation);
                    }
                    if shared
                        .emit(ChannelEvent::Request {
                            conn_id,
                            id,
                            body: Box::new(other),
                        })
                        .await
                    {
                        Flow::Continue
                    } else {
                        Flow::Close(DisconnectReason::Closed)
                    }
                }
            }
        }
        ChannelMessage::Notification(notification) => {
            let body = notification.body;
            match body {
                // Both are app to server (§6.3). A peer sending one is not a peer of ours.
                NotificationBody::HandoffEvent(_) | NotificationBody::AppShutdown(_) => {
                    tracing::warn!(
                        conn_id,
                        "the channel peer sent a notification of the wrong direction"
                    );
                    Flow::Close(DisconnectReason::ProtocolViolation)
                }
                NotificationBody::SessionBye(_) => {
                    let delivered = shared
                        .emit(ChannelEvent::Notification {
                            conn_id,
                            body: Box::new(NotificationBody::SessionBye(Default::default())),
                        })
                        .await;
                    let _ = delivered;
                    // §6.2: `session.bye` ends the connection. The server sends it on stdin
                    // EOF and then exits, so waiting for its EOF would only add a race.
                    Flow::Close(DisconnectReason::PeerLeft)
                }
                other => {
                    if shared
                        .emit(ChannelEvent::Notification {
                            conn_id,
                            body: Box::new(other),
                        })
                        .await
                    {
                        Flow::Continue
                    } else {
                        Flow::Close(DisconnectReason::Closed)
                    }
                }
            }
        }
        ChannelMessage::Response(response) => {
            // The app sends exactly one kind of request, `ping`, so the only response it
            // can be owed is the answer to one it sent.
            if matches!(response.result, ResultBody::Empty(_)) && pings.answered(&response.id) {
                Flow::Continue
            } else {
                tracing::warn!(
                    conn_id,
                    "the channel peer answered a request the app never sent"
                );
                Flow::Close(DisconnectReason::ProtocolViolation)
            }
        }
        ChannelMessage::ErrorResponse(_) => {
            tracing::warn!(
                conn_id,
                "the channel peer refused a request the app never sent"
            );
            Flow::Close(DisconnectReason::ProtocolViolation)
        }
    }
}

/// Which methods a role may send after its `hello` (§6.2).
fn role_may_send(role: PeerRole, body: &RequestBody) -> bool {
    match role {
        // A hook is served one `hook.stop` and closed; it has no session and no handoffs.
        PeerRole::Hook => matches!(body, RequestBody::HookStop(_)),
        // A server never speaks for a hook: the decision is per hook process (§5.11).
        PeerRole::Server => !matches!(body, RequestBody::HookStop(_)),
    }
}

/// How the `hello` ended.
enum Handshake {
    /// The peer is who it says it is, and the id its answer has to carry.
    Authenticated {
        peer: Box<Peer>,
        hello_id: RequestId,
    },
    /// Answered with `auth_failed` or `protocol_unsupported` (§6.2): the next accept waits.
    Refused,
    Closed(DisconnectReason),
}

async fn handshake<R>(
    shared: &Shared,
    conn_id: ConnId,
    frames: &mut Frames<R>,
    outbound: &mpsc::Sender<ChannelMessage>,
    stop: &mut watch::Receiver<bool>,
) -> Handshake
where
    R: AsyncRead + Unpin,
{
    let first = tokio::select! {
        _ = stop.changed() => return Handshake::Closed(DisconnectReason::Closed),
        line = tokio::time::timeout(shared.config.hello_timeout, frames.next()) => line,
    };

    let line = match first {
        // §6.2: no `hello` within two seconds and the connection closes, with no answer —
        // there is no request to answer.
        Err(_elapsed) => {
            tracing::info!(conn_id, "the channel peer sent no hello in time");
            return Handshake::Closed(DisconnectReason::ProtocolViolation);
        }
        Ok(Ok(Some(line))) => line,
        Ok(Ok(None)) => return Handshake::Closed(DisconnectReason::PeerLeft),
        Ok(Err(FrameError::TooLarge)) => {
            tracing::warn!(conn_id, "the channel peer opened with an oversized line");
            return Handshake::Closed(DisconnectReason::ProtocolViolation);
        }
        Ok(Err(FrameError::Io)) => return Handshake::Closed(DisconnectReason::PeerLeft),
    };

    let Some(ChannelMessage::Request(request)) = decode(&line) else {
        tracing::warn!(
            conn_id,
            "the channel peer opened with something that is not a request"
        );
        return Handshake::Closed(DisconnectReason::ProtocolViolation);
    };
    let Request { id, body, .. } = *request;
    let RequestBody::Hello(params) = body else {
        tracing::warn!(
            conn_id,
            "the channel peer opened with a method that is not hello"
        );
        return Handshake::Closed(DisconnectReason::ProtocolViolation);
    };

    let (protocol_version, token) = match &params {
        HelloParams::Server(server) => (server.protocol_version, server.token.as_str()),
        HelloParams::Hook(hook) => (hook.protocol_version, hook.token.as_str()),
    };

    if protocol_version != PROTOCOL_VERSION {
        tracing::warn!(
            conn_id,
            peer_protocol_version = protocol_version,
            "refused a channel connection speaking another protocol version"
        );
        shared.penalise().await;
        let _ = send_error(
            outbound,
            id,
            ChannelErrorCode::ProtocolUnsupported,
            Some(ChannelErrorData::ProtocolUnsupported(
                ProtocolUnsupportedData {
                    protocol_version: PROTOCOL_VERSION,
                },
            )),
        )
        .await;
        return Handshake::Refused;
    }

    if !shared.config.token.matches(token) {
        // No token material, not even a length or a prefix: FM-10 is a state the user
        // repairs from the settings screen, and a log is a file anyone can read.
        tracing::warn!(
            conn_id,
            "refused a channel connection presenting the wrong token"
        );
        shared.penalise().await;
        let _ = send_error(outbound, id, ChannelErrorCode::AuthFailed, None).await;
        return Handshake::Refused;
    }

    let authenticated_at = Timestamp::now();
    let peer = match params {
        HelloParams::Server(server) => Peer {
            conn_id,
            role: PeerRole::Server,
            session_ref: Some(ids::new_session_ref()),
            identity: server.identity,
            capability_row: Some(server.capability_row),
            authenticated_at,
            agent_id: Some(server.agent_id),
            client: Some(server.client),
            server_version: Some(server.server_version),
            hook: None,
        },
        HelloParams::Hook(hook) => Peer {
            conn_id,
            role: PeerRole::Hook,
            session_ref: None,
            identity: hook.identity,
            capability_row: None,
            authenticated_at,
            agent_id: None,
            client: None,
            server_version: hook.server_version,
            hook: Some(hook.hook),
        },
    };
    Handshake::Authenticated {
        peer: Box::new(peer),
        hello_id: id,
    }
}

/// One line, framed, schema-checked and decoded, or nothing.
fn decode(line: &str) -> Option<ChannelMessage> {
    let value: Value = serde_json::from_str(line).ok()?;
    // The schema is the contract, and it is closed: it refuses an unknown field, a wrong
    // direction of a known one and every shape the codec would otherwise have to guess at
    // (`protocol/channel/README.md`). The typed decode after it is what the rest of the
    // app sees, and a disagreement between the two is a defect of one of them, not a line
    // to be tolerated.
    if !is_valid(Document::ChannelMessage, &value) {
        return None;
    }
    serde_json::from_value(value).ok()
}

async fn send_response(
    outbound: &mpsc::Sender<ChannelMessage>,
    id: RequestId,
    result: ResultBody,
) -> Result<(), ()> {
    outbound
        .send(ChannelMessage::Response(Box::new(Response {
            jsonrpc: JsonRpcVersion::V2,
            id,
            result,
        })))
        .await
        .map_err(|_| ())
}

async fn send_request(
    outbound: &mpsc::Sender<ChannelMessage>,
    id: RequestId,
    body: RequestBody,
) -> Result<(), ()> {
    outbound
        .send(ChannelMessage::Request(Box::new(Request {
            jsonrpc: JsonRpcVersion::V2,
            id,
            body,
        })))
        .await
        .map_err(|_| ())
}

async fn send_error(
    outbound: &mpsc::Sender<ChannelMessage>,
    id: RequestId,
    code: ChannelErrorCode,
    data: Option<ChannelErrorData>,
) -> Result<(), ()> {
    outbound
        .send(ChannelMessage::ErrorResponse(Box::new(ErrorResponse {
            jsonrpc: JsonRpcVersion::V2,
            id,
            error: ChannelError {
                code: code.code(),
                message: code.name().to_owned(),
                data,
            },
        })))
        .await
        .map_err(|_| ())
}

/// The pings this side has sent and not yet had answered (§6.3).
#[derive(Debug, Default)]
struct Pings {
    next: u64,
    outstanding: HashSet<u64>,
}

impl Pings {
    fn next_id(&mut self) -> u64 {
        self.next += 1;
        self.outstanding.insert(self.next);
        self.next
    }

    fn outstanding(&self) -> usize {
        self.outstanding.len()
    }

    /// True when `id` is one of ours. A late answer to a ping we already counted as missed
    /// still counts: the peer is talking, which is the whole question.
    fn answered(&mut self, id: &RequestId) -> bool {
        match id {
            RequestId::Number(number) => self.outstanding.remove(number),
            RequestId::Text(_) => false,
        }
    }
}

/// Why a line could not be read.
enum FrameError {
    /// Longer than `CHANNEL_MAX_MESSAGE_BYTES` (§6.1).
    TooLarge,
    /// The socket failed. Indistinguishable from a peer that went away, and treated as one.
    Io,
}

/// NDJSON framing (§6.1).
///
/// Three properties, each of them a bug the design already anticipated:
///
/// - **Bytes, not characters.** A read can end inside a multi-byte character, so the buffer
///   is bytes and the split is on the newline **byte**; decoding each chunk to a string
///   would corrupt any line carrying an accented letter at the wrong offset.
/// - **The cap is checked before the line ends.** A peer that never sends a newline must
///   not be able to grow the buffer for ever, so the overflow is reported as soon as what
///   is held exceeds the cap.
/// - **The scan does not start over.** A 16 MiB line arriving in 64 KiB reads would be
///   rescanned two hundred and fifty times from the front; `scanned` is how far the search
///   for a newline has already got.
struct Frames<R> {
    reader: R,
    buffer: Vec<u8>,
    scanned: usize,
    chunk: Vec<u8>,
    max: usize,
}

impl<R: AsyncRead + Unpin> Frames<R> {
    fn new(reader: R, max: usize) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
            scanned: 0,
            chunk: vec![0; READ_CHUNK],
            max,
        }
    }

    /// The next line, or `Ok(None)` at end of stream.
    async fn next(&mut self) -> Result<Option<String>, FrameError> {
        loop {
            if let Some(offset) = self.buffer[self.scanned..]
                .iter()
                .position(|byte| *byte == b'\n')
            {
                let end = self.scanned + offset;
                if end > self.max {
                    return Err(FrameError::TooLarge);
                }
                let line: Vec<u8> = self.buffer.drain(..=end).collect();
                self.scanned = 0;
                let text = String::from_utf8_lossy(&line[..end]);
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    continue;
                }
                return Ok(Some(trimmed.to_owned()));
            }
            self.scanned = self.buffer.len();
            if self.scanned > self.max {
                return Err(FrameError::TooLarge);
            }

            let read = self
                .reader
                .read(&mut self.chunk)
                .await
                .map_err(|_| FrameError::Io)?;
            if read == 0 {
                return Ok(None);
            }
            self.buffer.extend_from_slice(&self.chunk[..read]);
        }
    }
}

/// Writes what is queued for a connection, one compact JSON object per line, until the last
/// sender is dropped.
async fn write_loop<W>(mut writer: W, mut queue: mpsc::Receiver<ChannelMessage>)
where
    W: AsyncWrite + Unpin,
{
    while let Some(message) = queue.recv().await {
        let Ok(mut line) = serde_json::to_vec(&message) else {
            // Nothing the app builds can fail to serialise; if one ever did, dropping the
            // line would desynchronise the peer, so the connection goes instead.
            tracing::error!("the channel could not serialise an outgoing message");
            break;
        };
        line.push(b'\n');
        if writer.write_all(&line).await.is_err() || writer.flush().await.is_err() {
            break;
        }
    }
    let _ = writer.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_carries_the_constants_of_the_design() {
        // The timings are fields so the tests can shorten them; this is what stops a
        // shortened value from reaching a build.
        let config = ListenerConfig::new(
            Endpoint::Pipe {
                name: r"\\.\pipe\handoff-test".to_owned(),
            },
            Token::generate(),
        );
        assert_eq!(config.hello_timeout, Duration::from_secs(2));
        assert_eq!(config.ping_interval, Duration::from_secs(30));
        assert_eq!(config.auth_failure_delay, Duration::from_secs(1));
        assert_eq!(config.max_message_bytes, 16 * 1024 * 1024);
        assert_eq!(config.app_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn a_hook_may_only_ask_for_a_decision_and_a_server_may_not_ask_at_all() {
        let hook_stop = RequestBody::HookStop(Default::default());
        let resume = RequestBody::HandoffResume(crate::format::channel::HandoffResumeParams {
            call_id: "call_2q7m8r1t".to_owned(),
            handoff_id: "hf_7k3m9p2q4r".to_owned(),
            session_ref: None,
        });
        assert!(role_may_send(PeerRole::Hook, &hook_stop));
        assert!(!role_may_send(PeerRole::Hook, &resume));
        assert!(!role_may_send(PeerRole::Server, &hook_stop));
        assert!(role_may_send(PeerRole::Server, &resume));
    }

    #[test]
    fn a_ping_is_answered_once_and_a_stranger_never() {
        let mut pings = Pings::default();
        assert_eq!(pings.outstanding(), 0);
        let first = RequestId::Number(pings.next_id());
        let second = RequestId::Number(pings.next_id());
        assert_eq!(pings.outstanding(), MISSED_PINGS_BEFORE_DEAD);
        assert!(pings.answered(&second));
        assert!(!pings.answered(&second), "the same answer counted twice");
        assert!(pings.answered(&first));
        assert_eq!(pings.outstanding(), 0);
        assert!(!pings.answered(&RequestId::Number(99)));
        assert!(!pings.answered(&RequestId::Text("1".to_owned())));
    }

    #[tokio::test]
    async fn the_frames_split_on_the_newline_byte_and_skip_empty_lines() {
        let input = "{\"a\":1}\n\n  \n{\"b\":\"é\"}\n".as_bytes().to_vec();
        let mut frames = Frames::new(&input[..], 1024);
        assert_eq!(
            frames.next().await.ok().flatten().as_deref(),
            Some("{\"a\":1}")
        );
        assert_eq!(
            frames.next().await.ok().flatten().as_deref(),
            Some("{\"b\":\"é\"}")
        );
        assert!(matches!(frames.next().await, Ok(None)));
    }

    #[tokio::test]
    async fn a_multi_byte_character_split_across_reads_survives() {
        // The trap this framing exists for: a chunk boundary inside a character. A decoder
        // that turned each read into a string would produce two replacement characters and
        // a line that no longer parses.
        struct Halves {
            parts: Vec<Vec<u8>>,
        }
        impl AsyncRead for Halves {
            fn poll_read(
                mut self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
                buf: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<io::Result<()>> {
                if self.parts.is_empty() {
                    return std::task::Poll::Ready(Ok(()));
                }
                let part = self.parts.remove(0);
                buf.put_slice(&part);
                std::task::Poll::Ready(Ok(()))
            }
        }

        let whole = "{\"b\":\"é\"}\n".as_bytes().to_vec();
        let split = 6; // inside the two bytes of é
        let reader = Halves {
            parts: vec![whole[..split].to_vec(), whole[split..].to_vec()],
        };
        let mut frames = Frames::new(reader, 1024);
        assert_eq!(
            frames.next().await.ok().flatten().as_deref(),
            Some("{\"b\":\"é\"}")
        );
    }

    #[tokio::test]
    async fn a_line_over_the_cap_is_refused_before_it_ends() {
        // No newline at all: the cap has to be checked on what is held, not on what a
        // completed line measures, or a peer could grow the buffer for ever.
        let input = [b'a'; 64];
        let mut frames = Frames::new(&input[..], 16);
        assert!(matches!(frames.next().await, Err(FrameError::TooLarge)));
    }

    #[test]
    fn a_line_the_schema_refuses_does_not_decode() {
        // An unknown field at the envelope level: serde alone would accept it, because the
        // JSON-RPC envelopes cannot carry `deny_unknown_fields` together with `flatten`.
        // The schema is what closes them, which is why `decode` validates first.
        assert!(decode(r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#).is_some());
        assert!(decode(r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{},"x":1}"#).is_none());
        assert!(decode(r#"{"jsonrpc":"2.0","id":1,"method":"nope","params":{}}"#).is_none());
        assert!(decode("not json").is_none());
        assert!(decode("[]").is_none());
    }
}
