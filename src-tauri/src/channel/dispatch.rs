//! What the channel says to the store, and what the store says back (§7.3, §6.3, §5.7).
//!
//! The listener knows nothing about handoffs and the store knows nothing about sockets;
//! this module is the only place that knows both. It owns the session registry of §7.5,
//! holds the [`StoreHandle`] of §7.4, and runs one task that reads two streams:
//!
//! - the [`ChannelEvent`]s of the listener — a peer connected, a request, a notification, a
//!   connection ended — which it turns into store commands and answers on the same
//!   connection;
//! - the [`Delivery`]s of the store — every outcome that has a call waiting for it — which
//!   it sends as `handoff.event` on the `conn_id` recorded when that call attached.
//!
//! One task for both is deliberate: every answer is written after the transition that
//! produced it has been applied, and a delivery that a request produced (the
//! `transferred_to_other_session` of a takeover, §5.7) is queued behind that request's own
//! reply, which is the order the golden fixtures carry.
//!
//! # The three rules that are easy to get wrong
//!
//! - **An event is addressed by connection, never by session.** The store records the
//!   `conn_id` a call arrived on and this module sends there. A session with two
//!   connections, or a call resumed from another session, would otherwise be answered on
//!   the wrong socket — which is the "the agent never got the outcome" defect this task
//!   exists to avoid.
//! - **A connection loss detaches, it does not conclude.** The registry marks the session
//!   disconnected (banner, SRV-21/SRV-22) and the store detaches that session's calls; the
//!   handoffs keep their state and any session may resume them (TOOL-08). The one state
//!   change a disconnect does cause is VER-06's, and it is the store's.
//! - **A hook connection is closed from here.** §6.2 serves a hook one `hook.stop` and
//!   closes; the answer is this module's to send, so the close is this module's to ask for.
//!
//! # What a peer is told when the disk refuses (FM-28)
//!
//! [`Refusal::code`] maps five of the seven refusals onto the application errors of §6.3.
//! The other two carry no code and cannot be given one: `channel.v1.schema.json` closes the
//! error object to exactly the seven names of the protocol, so an invented `-32603` would
//! be a line the peer's own validator refuses. `NotActive` cannot reach here at all (it
//! answers a *user* action, which never arrives on the channel). [`Refusal::Persistence`]
//! is therefore **not answered**: the failure is logged, the in-memory state is untouched
//! (T-033 applies a transition to a copy and puts it back only after the write), and the
//! server's own 10 s request budget (§6.6) turns the silence into `APP_DISCONNECTED`,
//! whose fix text already says "retry the same call in a few seconds". Answering
//! `not_found` instead would tell an agent to give up on a handoff that exists, and closing
//! the connection would end a session over a full disk.

use std::collections::HashMap;

use tokio::sync::mpsc::Receiver;

use crate::format::channel::{
    ChannelError, ChannelErrorCode, ChannelErrorData, ChannelMessage, ErrorResponse,
    HandoffContinueParams, HandoffContinueResult, HandoffEventParams, HandoffOpenResult,
    HandoffResumeResult, HandoffState as WireState, HandoffVerifyResult, HookStopResult,
    JsonRpcVersion, Notification, NotificationBody, RequestBody, RequestId, ResultBody,
    UnknownValueKeyData,
};
use crate::log::{Db, Timestamp};
use crate::sessions::{HookBinding, Registry, SystemProcessTable};
use crate::store::{Call, Delivery, OpenParams, Opener, Refusal, StoreHandle};

use super::listener::{ChannelEvent, ChannelHandle, ConnId, DisconnectReason, Peer, PeerRole};

/// The channel's half of §7.3: one owner for the registry, one line to the store.
pub struct Dispatch {
    /// The registry's own connection. The store has its own; §7.11's note on the pragmas
    /// says two connections are expected, and WAL serialises the writers.
    db: Db,
    registry: Registry,
    store: StoreHandle,
    channel: ChannelHandle,
    /// What [`Registry::bind_hook`] concluded for each live hook connection, kept until its
    /// `hook.stop` is answered.
    hooks: HashMap<ConnId, HookBinding>,
}

impl std::fmt::Debug for Dispatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Dispatch")
            .field("hooks", &self.hooks.len())
            .finish_non_exhaustive()
    }
}

impl Dispatch {
    /// The dispatch, before it starts reading.
    #[must_use]
    pub fn new(db: Db, registry: Registry, store: StoreHandle, channel: ChannelHandle) -> Self {
        Self {
            db,
            registry,
            store,
            channel,
            hooks: HashMap::new(),
        }
    }

    /// Reads both streams until the listener stops.
    ///
    /// It ends when the event stream closes, which happens when the listener is dropped or
    /// shut down. A delivery stream that closes first only means the store's task has
    /// ended; the sessions are still worth registering, so the loop goes on without it.
    pub async fn run(
        mut self,
        mut events: Receiver<ChannelEvent>,
        mut deliveries: Receiver<Delivery>,
    ) {
        let mut store_alive = true;
        loop {
            tokio::select! {
                event = events.recv() => match event {
                    Some(event) => self.on_event(event).await,
                    None => break,
                },
                delivery = deliveries.recv(), if store_alive => match delivery {
                    Some(delivery) => self.on_delivery(delivery).await,
                    None => {
                        tracing::warn!("the store stopped producing outcomes");
                        store_alive = false;
                    }
                },
            }
        }
    }

    async fn on_event(&mut self, event: ChannelEvent) {
        match event {
            ChannelEvent::Connected(peer) => self.on_connected(*peer),
            ChannelEvent::Request { conn_id, id, body } => {
                self.touch(conn_id);
                self.on_request(conn_id, id, *body).await;
            }
            ChannelEvent::Notification { conn_id, body } => {
                self.touch(conn_id);
                self.on_notification(conn_id, *body).await;
            }
            ChannelEvent::Disconnected { conn_id, reason } => {
                self.on_disconnected(conn_id, reason).await;
            }
        }
    }

    /// A peer authenticated: a server registers a session (SRV-20), a hook is bound to one
    /// (SRV-17, SRV-18).
    fn on_connected(&mut self, peer: Peer) {
        // The chain is completed from a snapshot taken now, while the peer is certainly
        // alive (DD-22).
        let table = SystemProcessTable::snapshot();
        match peer.role {
            PeerRole::Server => {
                if let Err(error) = self.registry.register(&self.db, &peer, &table) {
                    // FM-28: a write that fails must not take the channel down with it. The
                    // connection stays up and every request on it is refused for want of a
                    // session, which is the same silence a full disk produces everywhere
                    // else.
                    tracing::error!(error = %error, "a session could not be registered");
                }
            }
            PeerRole::Hook => {
                let Some(hook) = peer.hook.as_ref() else {
                    // The schema requires `hook` of a hook's `hello`, so this is
                    // unreachable from a peer that got past the listener.
                    tracing::warn!(conn_id = peer.conn_id, "a hook connected without its input");
                    return;
                };
                match self
                    .registry
                    .bind_hook(&self.db, &peer.identity, hook, &table)
                {
                    Ok(binding) => {
                        self.hooks.insert(peer.conn_id, binding);
                    }
                    Err(error) => {
                        tracing::error!(error = %error, "a hook could not be bound to a session");
                        self.hooks.insert(peer.conn_id, HookBinding::None);
                    }
                }
            }
        }
    }

    async fn on_request(&mut self, conn_id: ConnId, id: RequestId, body: RequestBody) {
        let now = Timestamp::now();
        match body {
            RequestBody::HandoffOpen(params) => {
                let Some(opener) = self.opener(conn_id) else {
                    tracing::error!(conn_id, "handoff.open on a connection with no session");
                    return;
                };
                let call = Call {
                    conn_id,
                    call_id: params.call_id.clone(),
                    session_ref: Some(opener.session_ref.clone()),
                };
                let open = OpenParams {
                    spec: params.spec,
                    secret_treated: params.secret_treated,
                    // Optional *and* nullable on the wire: absent and null both mean "this
                    // handoff answers no user request" (DD-13).
                    request_id: params.request_id.flatten(),
                    opener,
                    call,
                };
                match self.store.open(open, now).await {
                    Ok(accepted) => {
                        reply(
                            &self.channel,
                            conn_id,
                            id,
                            ResultBody::HandoffOpen(HandoffOpenResult {
                                handoff_id: accepted.handoff_id,
                                resumed_from: accepted.resumed_from,
                            }),
                        )
                        .await;
                    }
                    Err(refusal) => {
                        refuse(&self.channel, conn_id, id, &refusal, "handoff.open").await
                    }
                }
            }
            RequestBody::HandoffContinue(params) => {
                let HandoffContinueParams {
                    call_id,
                    handoff_id,
                    reply: answer,
                    replacement_steps,
                } = *params;
                let call = self.call(conn_id, call_id);
                let result = self
                    .store
                    .continue_handoff(handoff_id, call, answer, replacement_steps, now)
                    .await;
                match result {
                    Ok(()) => {
                        reply(
                            &self.channel,
                            conn_id,
                            id,
                            ResultBody::HandoffContinue(HandoffContinueResult { ok: true }),
                        )
                        .await;
                    }
                    Err(refusal) => {
                        refuse(&self.channel, conn_id, id, &refusal, "handoff.continue").await
                    }
                }
            }
            RequestBody::HandoffResume(params) => {
                let call = self.call(conn_id, params.call_id.clone());
                match self
                    .store
                    .resume(params.handoff_id.clone(), call, now)
                    .await
                {
                    Ok(snapshot) => {
                        // `outcome` and `resumed_from` travel as explicit nulls: §5.7's
                        // snapshot always reports all three, and the goldens carry them.
                        reply(
                            &self.channel,
                            conn_id,
                            id,
                            ResultBody::HandoffResume(Box::new(HandoffResumeResult {
                                state: wire_state(snapshot.state),
                                outcome: Some(snapshot.outcome),
                                image: snapshot.image,
                                resumed_from: Some(snapshot.resumed_from),
                            })),
                        )
                        .await;
                    }
                    Err(refusal) => {
                        refuse(&self.channel, conn_id, id, &refusal, "handoff.resume").await
                    }
                }
            }
            RequestBody::HandoffVerify(params) => {
                let result = self
                    .store
                    .verify(
                        params.handoff_id,
                        params.verify.ok,
                        params.verify.detail,
                        now,
                    )
                    .await;
                match result {
                    Ok(accepted) => {
                        reply(
                            &self.channel,
                            conn_id,
                            id,
                            ResultBody::HandoffVerify(Box::new(HandoffVerifyResult {
                                outcome: accepted.outcome,
                            })),
                        )
                        .await;
                    }
                    Err(refusal) => {
                        refuse(&self.channel, conn_id, id, &refusal, "handoff.verify").await
                    }
                }
            }
            RequestBody::HookStop(_) => {
                let binding = self.hooks.remove(&conn_id).unwrap_or(HookBinding::None);
                let decision = hook_decision(conn_id, &binding);
                reply(&self.channel, conn_id, id, ResultBody::HookStop(decision)).await;
                // §6.2: one question, one answer, then the socket goes. The listener does
                // not close it, because the answer is not its to send.
                self.channel.close(conn_id).await;
            }
            // The listener answers both itself and never forwards them (§7.3).
            RequestBody::Hello(_) | RequestBody::Ping(_) => {
                tracing::warn!(
                    conn_id,
                    "the listener forwarded a message it answers itself"
                );
            }
        }
    }

    async fn on_notification(&mut self, conn_id: ConnId, body: NotificationBody) {
        match body {
            NotificationBody::HandoffDetachCall(params) => {
                // DD-24: the call stopped waiting and the tab may say so. The handoff is
                // untouched, so a refusal here is only worth a log line.
                if let Err(error) = self
                    .store
                    .detach_call(
                        params.handoff_id,
                        params.call_id,
                        params.reason,
                        Timestamp::now(),
                    )
                    .await
                {
                    tracing::debug!(error = %error, "a detach named a handoff the store does not hold");
                }
            }
            // §6.2: the listener closes the connection on a `session.bye`, so the
            // disconnection that follows is what ends the session. Nothing to do here but
            // say why it is about to happen.
            NotificationBody::SessionBye(_) => {
                tracing::debug!(conn_id, "a server said goodbye");
            }
            // Both are app to server. The listener refuses them before they reach here.
            NotificationBody::HandoffEvent(_) | NotificationBody::AppShutdown(_) => {
                tracing::warn!(
                    conn_id,
                    "the listener forwarded a notification of the wrong direction"
                );
            }
        }
    }

    /// The connection is gone (§8.3, SRV-21, SRV-22, FM-08).
    async fn on_disconnected(&mut self, conn_id: ConnId, reason: DisconnectReason) {
        self.hooks.remove(&conn_id);
        let now = Timestamp::now();
        let session_ref = match self.registry.disconnect(&self.db, conn_id, &now) {
            Ok(session_ref) => session_ref,
            Err(error) => {
                tracing::error!(error = %error, "a session could not be marked disconnected");
                None
            }
        };
        let Some(session_ref) = session_ref else {
            // A hook, or a peer that never registered: there is nothing to detach.
            tracing::debug!(conn_id, reason = ?reason, "a connection ended");
            return;
        };
        tracing::info!(conn_id, session_ref, reason = ?reason, "a session ended");
        if let Err(error) = self.store.session_disconnected(session_ref, now).await {
            tracing::error!(error = %error, "the disconnection could not be recorded on a handoff");
        }
    }

    /// One outcome to the call that is waiting for it (§6.3 `handoff.event`).
    async fn on_delivery(&mut self, delivery: Delivery) {
        let Delivery {
            conn_id,
            call_id,
            handoff_id,
            outcome,
            image,
        } = delivery;
        let message = ChannelMessage::Notification(Box::new(Notification {
            jsonrpc: JsonRpcVersion::V2,
            body: NotificationBody::HandoffEvent(Box::new(HandoffEventParams {
                call_id: call_id.clone(),
                handoff_id: handoff_id.clone(),
                outcome,
                image,
            })),
        }));
        if let Err(error) = self.channel.send(conn_id, message).await {
            // The connection went while the outcome was being built. The handoff keeps its
            // state and the disconnection that follows detaches the call; what this loses is
            // the report of the transition, and there is nothing on this side to re-queue it
            // with — the store handed it over when it applied the transition.
            tracing::warn!(
                error = %error,
                conn_id,
                call_id,
                handoff_id,
                "an outcome could not be delivered to the call that was waiting for it"
            );
        }
    }

    /// A sign of life on the connection (§8.3).
    ///
    /// §8.3 counts "a message, a ping, or the disconnection itself". A `ping` is answered
    /// inside the listener and never reaches this task, so `last_seen` moves on messages and
    /// at the disconnection. Nothing depends on the difference: the purge of §8.3 only ever
    /// considers sessions that are already disconnected, and a disconnection writes the
    /// instant itself.
    fn touch(&mut self, conn_id: ConnId) {
        if let Err(error) = self.registry.touch(&self.db, conn_id, &Timestamp::now()) {
            tracing::error!(error = %error, "a session's last sign of life could not be recorded");
        }
    }

    /// The session that opened a handoff on this connection (§7.4).
    fn opener(&self, conn_id: ConnId) -> Option<Opener> {
        self.registry.of_connection(conn_id).map(Opener::from)
    }

    /// The call a request carries, with the session it belongs to when there is one.
    fn call(&self, conn_id: ConnId, call_id: String) -> Call {
        Call {
            conn_id,
            call_id,
            session_ref: self
                .registry
                .of_connection(conn_id)
                .map(|session| session.session_ref.clone()),
        }
    }
}

/// Answers a request.
///
/// A free function rather than a method for a reason the borrow checker enforces: a future
/// that holds `&Dispatch` across an await would need the whole dispatch to be `Sync`, and
/// it never can be — it owns a `rusqlite::Connection`, which is `Send` and deliberately not
/// `Sync` (§7.11). Taking the handle alone keeps every await point holding only what is
/// shareable.
async fn reply(channel: &ChannelHandle, conn_id: ConnId, id: RequestId, result: ResultBody) {
    if let Err(error) = channel.reply(conn_id, id, result).await {
        tracing::debug!(error = %error, conn_id, "the peer left before its answer was written");
    }
}

/// Refuses a request with the application error of §6.3, when there is one for it.
async fn refuse(
    channel: &ChannelHandle,
    conn_id: ConnId,
    id: RequestId,
    refusal: &Refusal,
    method: &str,
) {
    let Some(code) = refusal.code() else {
        // FM-28 and the impossible case, both explained in the module documentation: the
        // protocol has no code, so the request is left to the peer's own budget.
        tracing::error!(
            error = %refusal,
            conn_id,
            method,
            "a request could not be served and the protocol has no error for it"
        );
        return;
    };
    let message = ChannelMessage::ErrorResponse(Box::new(ErrorResponse {
        jsonrpc: JsonRpcVersion::V2,
        id,
        error: ChannelError {
            code: code.code(),
            message: code.name().to_owned(),
            data: error_data(code, refusal),
        },
    }));
    tracing::info!(
        conn_id,
        method,
        error = code.name(),
        "a request was refused"
    );
    if let Err(error) = channel.send(conn_id, message).await {
        tracing::debug!(error = %error, conn_id, "the peer left before its refusal was written");
    }
}

/// The state of §8.1 as the resume snapshot reports it (§5.7).
///
/// Two enumerations spell the same ten states: the store's, which is also the column of
/// `handoffs.state`, and the protocol's, which is what `channel.v1.schema.json` accepts.
/// They are one vocabulary written twice — the unit test below pins that — and this is the
/// only place that has to know both.
fn wire_state(state: crate::log::HandoffState) -> WireState {
    use crate::log::HandoffState as Stored;
    match state {
        Stored::AwaitingSpec => WireState::AwaitingSpec,
        Stored::Active => WireState::Active,
        Stored::Deferred => WireState::Deferred,
        Stored::Parked => WireState::Parked,
        Stored::AwaitingVerification => WireState::AwaitingVerification,
        Stored::Verified => WireState::Verified,
        Stored::Failed => WireState::Failed,
        Stored::NotVerified => WireState::NotVerified,
        Stored::ConfirmedByUser => WireState::ConfirmedByUser,
        Stored::Abandoned => WireState::Abandoned,
    }
}

/// The payload of the one application error that carries one (§6.3).
fn error_data(code: ChannelErrorCode, refusal: &Refusal) -> Option<ChannelErrorData> {
    match (code, refusal) {
        (ChannelErrorCode::UnknownValueKey, Refusal::UnknownValueKey { keys }) => {
            Some(ChannelErrorData::UnknownValueKey(UnknownValueKeyData {
                keys: keys.clone(),
            }))
        }
        _ => None,
    }
}

/// The Stop-hook decision of §7.5 and F-10.
///
/// Neutral for now, whatever the binding says: which handoffs of the bound session are
/// worth blocking for, and the once-per-item counter that keeps a session from being
/// blocked twice for the same thing, are the whole of T-035. Neutral is the answer every
/// row of the F-10 decision table gives when there is nothing to say, and it is what the
/// hook already does when the app is unreachable, so a hook meeting this build behaves
/// exactly as FM-03 describes.
// TASK: T-035 — replace this with `hook::decide`, which reads the binding made at `hello`.
fn hook_decision(conn_id: ConnId, binding: &HookBinding) -> HookStopResult {
    tracing::debug!(conn_id, binding = ?binding, "a hook was answered neutrally");
    HookStopResult {
        block: false,
        reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_refusals_that_have_a_code_carry_the_name_of_63() {
        let cases: Vec<(Refusal, &str)> = vec![
            (Refusal::NotFound, "not_found"),
            (Refusal::NotWaiting, "not_waiting"),
            (Refusal::Final, "final"),
            (Refusal::NoVerifyInSpec, "no_verify_in_spec"),
            (
                Refusal::UnknownValueKey {
                    keys: vec!["endpoint_url".to_owned()],
                },
                "unknown_value_key",
            ),
        ];
        for (refusal, name) in cases {
            let code = refusal.code().expect("an application error");
            assert_eq!(code.name(), name);
        }
    }

    #[test]
    fn only_the_unknown_value_key_refusal_carries_data() {
        let keys = Refusal::UnknownValueKey {
            keys: vec!["events".to_owned()],
        };
        let code = keys.code().expect("an application error");
        assert_eq!(
            error_data(code, &keys),
            Some(ChannelErrorData::UnknownValueKey(UnknownValueKeyData {
                keys: vec!["events".to_owned()]
            }))
        );
        assert_eq!(
            error_data(ChannelErrorCode::NotFound, &Refusal::NotFound),
            None
        );
    }

    #[test]
    fn the_stored_states_and_the_protocol_states_are_one_vocabulary() {
        // Written twice, in `log::handoffs` and in `format::channel`, and a resume snapshot
        // is where they meet. Comparing the serialised names is what makes a variant added
        // to one and not to the other a failure here rather than a message the peer refuses.
        for stored in [
            crate::log::HandoffState::AwaitingSpec,
            crate::log::HandoffState::Active,
            crate::log::HandoffState::Deferred,
            crate::log::HandoffState::Parked,
            crate::log::HandoffState::AwaitingVerification,
            crate::log::HandoffState::Verified,
            crate::log::HandoffState::Failed,
            crate::log::HandoffState::NotVerified,
            crate::log::HandoffState::ConfirmedByUser,
            crate::log::HandoffState::Abandoned,
        ] {
            let on_the_wire = serde_json::to_value(wire_state(stored)).expect("a state serialises");
            assert_eq!(on_the_wire, serde_json::json!(stored.as_str()));
        }
    }

    #[test]
    fn a_hook_is_answered_neutrally_whatever_it_was_bound_to() {
        // Until T-035: every binding gives the same answer, and a neutral answer never
        // carries a reason (§7.5, the schema requires one only when `block` is true).
        for binding in [
            HookBinding::None,
            HookBinding::Bound("ses_00000001".to_owned()),
            HookBinding::Ambiguous(vec!["ses_00000001".to_owned(), "ses_00000002".to_owned()]),
        ] {
            let decision = hook_decision(1, &binding);
            assert!(!decision.block);
            assert_eq!(decision.reason, None);
        }
    }
}
