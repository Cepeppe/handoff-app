//! The live session registry and the binding of a hook to it (§7.5, §8.3, SRV-17..21).
//!
//! One record per connected server: the completed ancestor chain, the working directory,
//! the project folder, the capability row the server resolved and, once a hook has proved
//! which session it belongs to, the agent's own `session_id`. The registry is the memory
//! half of §2.3; `log::sessions` is the SQLite half, written through on every change so the
//! Log page and the purge of §8.3 see the same sessions the overlay does.
//!
//! # A reconnection is a new session
//!
//! §8.3 is explicit: a server never restarts inside a live agent, so a `hello` carrying the
//! same agent pid as a disconnected session is a **new** session with a new `session_ref`,
//! and the old record is kept as history. Nothing here merges the two.
//!
//! # Binding, and the trap in "intersect the chains"
//!
//! §7.5 binds a hook to a session by intersecting its completed chain with the pids of the
//! registered sessions. Taken as a plain set intersection that rule does not survive
//! Windows: every session of the machine shares the desktop shell at the top of its chain,
//! so every session would match every hook and every hook would be ambiguous. [`bind_hook`]
//! therefore walks the hook's chain **generation by generation, nearest to the hook first**,
//! and the first generation that belongs to exactly one session is the answer — which is
//! the agent process itself, exactly the key SRV-17 names. Two sessions under the same
//! shell separate at the agent, one generation below the shell they share.
//!
//! The rest is the design's, in its order: a unique pid match binds `session_id` for the
//! rest of the session's life, the working directory is only a fallback key (SRV-18), and
//! anything still ambiguous is answered neutrally with the picker of FM-22 left to the UI.

use std::collections::HashMap;

use indexmap::IndexMap;

use crate::channel::{ConnId, Peer, PeerRole};
use crate::format::channel::{AncestorProcess, CapabilityRow, ClientInfo, HookInput, Identity};
use crate::format::outcome::ResumedFrom;
use crate::log::sessions::SessionRow;
use crate::log::{sessions as log_sessions, Db, Result, Timestamp};

use super::process_table::{complete_chain, ProcessTable};

/// One registered session (§7.5).
///
/// The fields §7.5 names, plus two the record cannot do its job without and that `hello`
/// already carried: the connection it arrived on, because `disconnect` is addressed by
/// connection and not by session, and the capability row, which is the only place the
/// agent's readable name exists ([`Session::display_name`], OPEN-02).
#[derive(Debug, Clone)]
pub struct Session {
    /// `ses_` + 8 characters, minted by the listener when the `hello` was accepted.
    pub session_ref: String,
    /// The connection it registered on.
    pub conn_id: ConnId,
    /// The capability-table key the server resolved (§5.6), e.g. `claude-code`.
    pub agent_id: Option<String>,
    /// `clientInfo` of the MCP handshake.
    pub client: Option<ClientInfo>,
    /// The row the server resolved for this session.
    pub capability_row: Option<CapabilityRow>,
    /// The completed ancestor chain (DD-22), nearest parent first.
    pub pid_chain: Vec<AncestorProcess>,
    /// The server's working directory.
    pub cwd: String,
    /// `CLAUDE_PROJECT_DIR` when the agent set one, else the working directory (§5.8).
    pub project_dir: Option<String>,
    /// Whether the connection is live (§8.3).
    pub connected: bool,
    /// The agent's own session identifier, once a hook proved this is the session (§7.5).
    pub claude_session_id: Option<String>,
    /// When it registered.
    pub first_seen: Timestamp,
    /// Its last sign of life: a message, a ping, or the disconnection itself.
    pub last_seen: Timestamp,
}

impl Session {
    /// The agent as a person reads it: the capability table's display name, else the key it
    /// resolved, else the MCP client's own name.
    ///
    /// One of the three is always there for a server — `agent_id` is required of every
    /// server `hello` — so this never has to invent a word.
    #[must_use]
    pub fn agent_label(&self) -> &str {
        let from_row = self
            .capability_row
            .as_ref()
            .and_then(|row| row.display_name.as_deref());
        first_non_empty(&[
            from_row,
            self.agent_id.as_deref(),
            self.client.as_ref().map(|client| client.name.as_str()),
        ])
        .unwrap_or(&self.session_ref)
    }

    /// The project folder as the tab shows it (OPEN-02): the last segment of the project
    /// folder, else of the working directory.
    ///
    /// The `session_ref` is the last resort, and it is there because it is never empty:
    /// `resumed_from.project` is a non-empty string in the published outcome schema, so a
    /// peer that reported a blank working directory must not produce an outcome the schema
    /// refuses.
    #[must_use]
    pub fn project_label(&self) -> &str {
        let project = self.project_dir.as_deref();
        first_non_empty(&[
            project.map(base_name),
            project,
            Some(base_name(&self.cwd)),
            Some(self.cwd.as_str()),
        ])
        .unwrap_or(&self.session_ref)
    }

    /// Agent and project folder in one line, for a tab, a picker or a log (OPEN-02).
    ///
    /// The two parts are what the design names; the separator is not a design text, and a
    /// view that needs them apart uses [`Session::agent_label`] and
    /// [`Session::project_label`] rather than splitting this.
    #[must_use]
    pub fn display_name(&self) -> String {
        format!(
            "{}{LABEL_SEPARATOR}{}",
            self.agent_label(),
            self.project_label()
        )
    }

    /// This session as the `resumed_from` of an outcome opened by it (TOOL-08, §4.3).
    #[must_use]
    pub fn resumed_from(&self) -> ResumedFrom {
        ResumedFrom {
            agent: self.agent_label().to_owned(),
            project: self.project_label().to_owned(),
        }
    }

    /// The pids this session can be recognised by: its completed ancestor chain (SRV-17).
    fn owns_pid(&self, pid: u32) -> bool {
        self.pid_chain.iter().any(|ancestor| ancestor.pid == pid)
    }

    /// Whether a hook that reported `cwd` is working in this session's folder (SRV-18).
    ///
    /// Both the working directory and the project folder count. §7.5 writes the fallback
    /// key as `cwd` equality and §5.8 writes it as the project folder; a hook sends only a
    /// working directory, and a server whose agent set `CLAUDE_PROJECT_DIR` reports a
    /// project folder that differs from it, so accepting either is the only reading under
    /// which both sections describe a key that can match.
    fn works_in(&self, cwd: &str) -> bool {
        same_folder(&self.cwd, cwd)
            || self
                .project_dir
                .as_deref()
                .is_some_and(|project_dir| same_folder(project_dir, cwd))
    }
}

/// What sits between the agent and the project wherever the two are printed together: a
/// tab (§7.6), the FM-22 picker, a log line.
///
/// One constant rather than one `format!` per view, because a user meets the same session
/// under both spellings otherwise — the tab strip and the `resumed_from` of an outcome name
/// the same thing.
pub const LABEL_SEPARATOR: &str = " · ";

/// What [`Registry::bind_hook`] concluded (§7.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookBinding {
    /// Exactly one session owns the hook.
    Bound(String),
    /// Several sessions match and nothing separates them (FM-22). The hook is answered
    /// neutrally; the candidates are what the picker of §7.5 asks the user about.
    Ambiguous(Vec<String>),
    /// No connected session matches. The hook is answered neutrally.
    None,
}

/// Told that the set of sessions, or something shown about one of them, has changed.
///
/// The core never touches Tauri (`lib.rs`), so the `sessions_changed` event of §7.6 leaves
/// through this trait: `ui_bridge` implements it over `AppHandle::emit` (T-036) and a test
/// implements it over a counter. It carries no payload because the view re-reads the
/// registry; there is one source of truth and it is not the event.
pub trait SessionsObserver: Send + Sync {
    /// Something a session view shows has changed.
    fn sessions_changed(&self);
}

/// The observer of a registry nobody is watching: the startup window before the UI exists,
/// and every test that is not about the event.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoObserver;

impl SessionsObserver for NoObserver {
    fn sessions_changed(&self) {}
}

/// The sessions of this run, in registration order (§7.5, §2.3).
pub struct Registry {
    sessions: IndexMap<String, Session>,
    by_conn: HashMap<ConnId, String>,
    observer: Box<dyn SessionsObserver>,
    /// The last set of sessions a hook could not be told apart between (FM-22, SRV-18),
    /// with the agent `session_id` that could not be placed.
    ///
    /// In memory and not in the log: it is a question about the run, and a run that has
    /// ended has no session to attribute anything to.
    ambiguous_hook: Option<AmbiguousHook>,
}

/// The unanswered question of FM-22: which of these sessions is the one that ran the hook.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AmbiguousHook {
    /// The `session_ref`s nothing separated.
    candidates: Vec<String>,
    /// The agent's own session identifier the hook carried, which is what the user's answer
    /// binds ([`Registry::answer_session_picker`]). Without it the picker would clear a
    /// question and change nothing, and the next hook of the same session would ask again.
    session_id: String,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Registry")
            .field("sessions", &self.sessions.len())
            .field("connections", &self.by_conn.len())
            .finish_non_exhaustive()
    }
}

impl Registry {
    /// An empty registry that persists nothing until it is told to.
    #[must_use]
    pub fn new(observer: Box<dyn SessionsObserver>) -> Self {
        Self {
            sessions: IndexMap::new(),
            by_conn: HashMap::new(),
            observer,
            ambiguous_hook: None,
        }
    }

    /// An empty registry, after repairing what a previous run left behind (§8.3).
    ///
    /// The live registry always starts empty — §2.3 keeps the sessions in memory and the
    /// table as history — but the table may still carry rows flagged connected by a process
    /// that was killed, and the purge of §8.3 only ever forgets a disconnected session.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the repair cannot be written.
    pub fn open(db: &Db, observer: Box<dyn SessionsObserver>) -> Result<Self> {
        let repaired = log_sessions::disconnect_all(db)?;
        if repaired > 0 {
            tracing::info!(
                count = repaired,
                "sessions left connected by a previous run were closed"
            );
        }
        Ok(Self::new(observer))
    }

    /// Registers the session a server just opened (SRV-20, §7.5).
    ///
    /// The ancestor chain is completed here, from `table`, while the peer is alive (DD-22).
    /// Returns the `session_ref` of the new session, or nothing when the peer is a hook:
    /// §6.2 assigns a hook no `session_ref` and a hook registers nothing.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the row cannot be written. The session
    /// is not kept in memory either in that case, so the two halves cannot disagree.
    pub fn register(
        &mut self,
        db: &Db,
        peer: &Peer,
        table: &dyn ProcessTable,
    ) -> Result<Option<String>> {
        let (PeerRole::Server, Some(session_ref)) = (peer.role, peer.session_ref.as_deref()) else {
            return Ok(None);
        };

        let session = Session {
            session_ref: session_ref.to_owned(),
            conn_id: peer.conn_id,
            agent_id: peer.agent_id.clone(),
            client: peer.client.clone(),
            capability_row: peer.capability_row.clone(),
            pid_chain: complete_chain(table, &peer.identity),
            cwd: peer.identity.cwd.clone(),
            project_dir: peer.identity.project_dir.clone(),
            connected: true,
            claude_session_id: None,
            first_seen: peer.authenticated_at.clone(),
            last_seen: peer.authenticated_at.clone(),
        };
        log_sessions::register(db, &row_of(&session))?;

        tracing::info!(
            session_ref = session.session_ref,
            agent_id = session.agent_id.as_deref().unwrap_or_default(),
            pid = peer.identity.pid,
            ppid = peer.identity.ppid,
            chain_count = session.pid_chain.len(),
            "a session registered"
        );

        self.by_conn.insert(peer.conn_id, session_ref.to_owned());
        self.sessions.insert(session_ref.to_owned(), session);
        self.observer.sessions_changed();
        Ok(Some(session_ref.to_owned()))
    }

    /// Records a sign of life on the connection: a message, or an answered ping (§8.3).
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the row cannot be updated.
    pub fn touch(&mut self, db: &Db, conn_id: ConnId, at: &Timestamp) -> Result<bool> {
        let Some(session) = self
            .by_conn
            .get(&conn_id)
            .and_then(|session_ref| self.sessions.get_mut(session_ref))
        else {
            return Ok(false);
        };
        log_sessions::touch(db, &session.session_ref, at)?;
        session.last_seen = at.clone();
        // No `sessions_changed`: nothing a view shows has moved.
        Ok(true)
    }

    /// The connection is gone: EOF, `session.bye`, or two unanswered pings (§8.3, FM-08).
    ///
    /// The session's handoffs stay in their own state and any session may resume them
    /// (TOOL-08); the record only stops being a live registration. Returns the
    /// `session_ref` that went down, or nothing when the connection carried no session (a
    /// hook, or a peer that never got past `hello`).
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the row cannot be updated.
    pub fn disconnect(
        &mut self,
        db: &Db,
        conn_id: ConnId,
        at: &Timestamp,
    ) -> Result<Option<String>> {
        let Some(session_ref) = self.by_conn.remove(&conn_id) else {
            return Ok(None);
        };
        let Some(session) = self.sessions.get_mut(&session_ref) else {
            return Ok(None);
        };
        log_sessions::disconnect(db, &session_ref, at)?;
        session.connected = false;
        session.last_seen = at.clone();
        tracing::info!(session_ref = session_ref, "a session disconnected");
        self.observer.sessions_changed();
        Ok(Some(session_ref))
    }

    /// Which session a hook belongs to (§7.5, SRV-17, SRV-18, FM-22).
    ///
    /// Three keys, strongest first, and every one of them looks only at **connected**
    /// sessions: a disconnected session's chain is a list of processes that are gone, and a
    /// pid the operating system has since handed to somebody else would bind a hook to the
    /// wrong session for good.
    ///
    /// 1. the agent's own `session_id`, once a pid intersection has bound it — §7.5 binds it
    ///    "for the rest of its life", so a later hook of the same agent needs nothing else;
    /// 2. the pid intersection of SRV-17, nearest generation to the hook first (see the
    ///    module documentation for why the order is what makes the rule work at all). Where
    ///    a generation is shared by several sessions, the working directory separates them;
    /// 3. the working directory alone (SRV-18), when no generation matched.
    ///
    /// A unique **pid** match binds `session_id` to the session and writes it through. A
    /// unique working-directory match does not: SRV-18 calls that key a fallback, and a
    /// binding is for the rest of the session's life.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the binding cannot be written.
    pub fn bind_hook(
        &mut self,
        db: &Db,
        identity: &Identity,
        hook: &HookInput,
        table: &dyn ProcessTable,
    ) -> Result<HookBinding> {
        if let Some(session_ref) = self.bound_to_session_id(&hook.session_id) {
            return Ok(HookBinding::Bound(session_ref));
        }

        let chain = complete_chain(table, identity);
        for ancestor in &chain {
            let candidates = self.connected_refs(|session| session.owns_pid(ancestor.pid));
            if candidates.is_empty() {
                continue;
            }
            if let [session_ref] = candidates.as_slice() {
                let session_ref = session_ref.clone();
                self.bind_session_id(db, &session_ref, &hook.session_id)?;
                return Ok(HookBinding::Bound(session_ref));
            }
            // Several sessions live under the same process. §7.5 lets the working
            // directory separate what the chain could not.
            let narrowed = self.connected_refs(|session| {
                session.owns_pid(ancestor.pid) && session.works_in(&identity.cwd)
            });
            if let [session_ref] = narrowed.as_slice() {
                let session_ref = session_ref.clone();
                self.bind_session_id(db, &session_ref, &hook.session_id)?;
                return Ok(HookBinding::Bound(session_ref));
            }
            return Ok(HookBinding::Ambiguous(candidates));
        }

        let candidates = self.connected_refs(|session| session.works_in(&identity.cwd));
        match candidates.as_slice() {
            [] => Ok(HookBinding::None),
            [session_ref] => Ok(HookBinding::Bound(session_ref.clone())),
            _ => Ok(HookBinding::Ambiguous(candidates)),
        }
    }

    /// Forgets disconnected sessions older than seven days that no handoff cites (§8.3).
    ///
    /// Returns how many were forgotten. The database decides — the query and the
    /// `ON DELETE RESTRICT` of `handoffs.session_ref` are both part of it — and memory
    /// follows: a record still held here whose row is gone is dropped.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the deletion fails.
    pub fn purge(&mut self, db: &Db, now: &Timestamp) -> Result<usize> {
        let forgotten = log_sessions::purge(db, now)?;
        if forgotten == 0 {
            return Ok(0);
        }
        let mut gone = Vec::new();
        for session in self.sessions.values() {
            if !session.connected && log_sessions::get(db, &session.session_ref)?.is_none() {
                gone.push(session.session_ref.clone());
            }
        }
        for session_ref in &gone {
            self.sessions.shift_remove(session_ref);
        }
        self.observer.sessions_changed();
        Ok(forgotten)
    }

    /// Records that a hook matched several sessions and nothing separated them (FM-22).
    ///
    /// The hook itself was answered neutrally; this is the question the overlay puts to the
    /// user the next time they interact ("which session is this?", SRV-18). It is kept
    /// rather than acted on because there is nobody to ask at the moment a hook arrives —
    /// the window may not even be open.
    pub fn needs_session_picker(&mut self, candidates: &[String], session_id: &str) {
        let question = AmbiguousHook {
            candidates: candidates.to_vec(),
            session_id: session_id.to_owned(),
        };
        if self.ambiguous_hook.as_ref() == Some(&question) {
            return;
        }
        tracing::info!(
            candidates = candidates.len(),
            "a hook matched several sessions and none of the keys separated them"
        );
        self.ambiguous_hook = Some(question);
        self.observer.sessions_changed();
    }

    /// The sessions the overlay has to ask the user to choose between, if any (FM-22).
    #[must_use]
    pub fn session_picker(&self) -> &[String] {
        match self.ambiguous_hook.as_ref() {
            Some(question) => &question.candidates,
            None => &[],
        }
    }

    /// The user picked one of the candidates: bind the hook's session id to it (FM-22).
    ///
    /// This is what makes the picker worth asking. The binding is the same one
    /// [`Registry::bind_hook`] writes when the chain is unambiguous, so from here on every
    /// hook of that agent session is recognised at once and the safety net of SRV-12 stops
    /// being delayed for it.
    ///
    /// A `session_ref` that is not one of the candidates is refused: the question was about
    /// those, and a window that answers something else is answering a question nobody asked.
    /// Returns whether the answer was taken.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the binding cannot be written. The
    /// question is then left standing, so the user can answer it again.
    pub fn answer_session_picker(&mut self, db: &Db, session_ref: &str) -> Result<bool> {
        let Some(question) = self.ambiguous_hook.clone() else {
            return Ok(false);
        };
        if !question
            .candidates
            .iter()
            .any(|candidate| candidate == session_ref)
        {
            return Ok(false);
        }
        self.bind_session_id(db, session_ref, &question.session_id)?;
        self.ambiguous_hook = None;
        self.observer.sessions_changed();
        Ok(true)
    }

    /// The user answered the picker, or the question stopped being one.
    pub fn session_picker_answered(&mut self) {
        if self.ambiguous_hook.is_none() {
            return;
        }
        self.ambiguous_hook = None;
        self.observer.sessions_changed();
    }

    /// One session, if this run registered it.
    #[must_use]
    pub fn get(&self, session_ref: &str) -> Option<&Session> {
        self.sessions.get(session_ref)
    }

    /// The session a connection belongs to.
    #[must_use]
    pub fn of_connection(&self, conn_id: ConnId) -> Option<&Session> {
        self.sessions.get(self.by_conn.get(&conn_id)?)
    }

    /// Every session of this run, in registration order — which is the order of the tabs.
    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values()
    }

    /// The connected ones, in registration order.
    pub fn connected(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values().filter(|session| session.connected)
    }

    /// Every session of this run as the rows the log holds.
    ///
    /// The one caller is "delete everything" (LOG-04): §7.11 truncates `sessions` with the
    /// rest, and a run that is still going needs the rows its live handoffs reference back
    /// (`store::Store::delete_log`). It answers with the whole set and not only the
    /// connected ones, because a handoff opened by a session that has since gone still
    /// names it, and the column is a foreign key.
    #[must_use]
    pub fn rows(&self) -> Vec<SessionRow> {
        self.sessions.values().map(row_of).collect()
    }

    /// The `{ agent, project }` an outcome carries when a call resumes a handoff another
    /// session opened (TOOL-08, §4.3).
    #[must_use]
    pub fn resumed_from(&self, session_ref: &str) -> Option<ResumedFrom> {
        Some(self.get(session_ref)?.resumed_from())
    }

    fn bound_to_session_id(&self, session_id: &str) -> Option<String> {
        self.sessions
            .values()
            .find(|session| {
                session.connected && session.claude_session_id.as_deref() == Some(session_id)
            })
            .map(|session| session.session_ref.clone())
    }

    fn connected_refs(&self, mut wanted: impl FnMut(&Session) -> bool) -> Vec<String> {
        self.sessions
            .values()
            .filter(|session| session.connected && wanted(session))
            .map(|session| session.session_ref.clone())
            .collect()
    }

    /// Writes the agent's `session_id` onto a session, once and for its whole life (§7.5).
    fn bind_session_id(&mut self, db: &Db, session_ref: &str, session_id: &str) -> Result<()> {
        let Some(session) = self.sessions.get_mut(session_ref) else {
            return Ok(());
        };
        if session.claude_session_id.as_deref() == Some(session_id) {
            return Ok(());
        }
        // A session already bound to another id keeps it: the first proof wins, and a
        // second agent claiming the same registration is FM-22's ambiguity, not a rebind.
        if session.claude_session_id.is_some() {
            return Ok(());
        }
        log_sessions::bind_claude_session_id(db, session_ref, session_id)?;
        session.claude_session_id = Some(session_id.to_owned());
        tracing::info!(session_ref = session_ref, "a hook bound a session id");
        self.observer.sessions_changed();
        Ok(())
    }
}

fn row_of(session: &Session) -> SessionRow {
    let pids: Vec<u32> = session
        .pid_chain
        .iter()
        .map(|ancestor| ancestor.pid)
        .collect();
    SessionRow {
        session_ref: session.session_ref.clone(),
        agent_id: session.agent_id.clone(),
        client_name: session.client.as_ref().map(|client| client.name.clone()),
        client_version: session.client.as_ref().map(|client| client.version.clone()),
        // The column holds the pids and not the names: the names are for a log line and a
        // picker, the pids are what the binding of §7.5 reads back.
        pid_chain_json: serde_json::to_string(&pids).unwrap_or_else(|_| "[]".to_owned()),
        cwd: Some(session.cwd.clone()),
        project_dir: session.project_dir.clone(),
        claude_session_id: session.claude_session_id.clone(),
        connected: session.connected,
        first_seen: session.first_seen.clone(),
        last_seen: session.last_seen.clone(),
    }
}

fn first_non_empty<'a>(candidates: &[Option<&'a str>]) -> Option<&'a str> {
    candidates
        .iter()
        .flatten()
        .copied()
        .find(|value| !value.is_empty())
}

/// The last segment of a path, with both separators accepted and trailing ones ignored.
///
/// Written out rather than taken from `Path::file_name`, which splits on `\` only where the
/// code is compiled for Windows: the app and its peers are always on the same machine, but
/// the tests of this rule run on the macOS leg of CI as well and must not change meaning
/// with the target.
/// The last segment of a path, which is how a project folder is shown (OPEN-02).
///
/// `pub` for the Log page of §7.11, which labels a row from `handoffs.project_dir` and has
/// no session to ask: two spellings of "the project folder's name" would be two answers to
/// a question the tab strip already answers.
#[must_use]
pub fn base_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(cut) => &trimmed[cut + 1..],
        None => trimmed,
    }
}

/// Whether two paths name the same folder, for the fallback key of SRV-18.
///
/// Trailing separators are ignored and the comparison is case-insensitive: both supported
/// platforms have case-insensitive file systems by default (§1.5), and the failure
/// direction of the looser rule is the safe one — two folders wrongly called equal make a
/// hook ambiguous, which is answered neutrally, while two spellings of one folder wrongly
/// called different would send the safety net to the wrong session or to none.
fn same_folder(left: &str, right: &str) -> bool {
    left.trim_end_matches(['/', '\\']).to_lowercase()
        == right.trim_end_matches(['/', '\\']).to_lowercase()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::format::channel::{HookEventName, SupportLevel};
    use crate::log::handoffs;
    use crate::sessions::process_table::SyntheticProcessTable;

    /// Counts what the UI would have been told (§7.6, `sessions_changed`).
    #[derive(Debug, Default)]
    struct CountingObserver(AtomicUsize);

    impl SessionsObserver for CountingObserver {
        fn sessions_changed(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    struct Fixture {
        db: Db,
        registry: Registry,
        changes: Arc<CountingObserver>,
    }

    impl Fixture {
        fn new() -> Self {
            let changes = Arc::new(CountingObserver::default());
            Self {
                db: Db::open_in_memory().expect("a database"),
                registry: Registry::new(Box::new(Arc::clone(&changes))),
                changes,
            }
        }

        fn changes(&self) -> usize {
            self.changes.0.load(Ordering::Relaxed)
        }
    }

    impl SessionsObserver for Arc<CountingObserver> {
        fn sessions_changed(&self) {
            self.as_ref().sessions_changed();
        }
    }

    fn at(text: &str) -> Timestamp {
        Timestamp::parse(text).expect("rfc 3339")
    }

    fn server_peer(conn_id: ConnId, session_ref: &str, pid: u32, ppid: u32, cwd: &str) -> Peer {
        Peer {
            conn_id,
            role: PeerRole::Server,
            session_ref: Some(session_ref.to_owned()),
            identity: Identity {
                pid,
                ppid,
                ancestors: Vec::new(),
                cwd: cwd.to_owned(),
                project_dir: Some(cwd.to_owned()),
            },
            capability_row: Some(CapabilityRow {
                agent_id: "claude-code".to_owned(),
                support: SupportLevel::Full,
                images_in_results: true,
                stop_hook: true,
                tool_timeout_ms: Some(1_800_000),
                display_name: Some("Claude Code".to_owned()),
                subagent_stop_hook: Some(true),
                session_identity: None,
                user_request_delivery: None,
                cancellation_notifications: Some(true),
            }),
            authenticated_at: at("2026-09-08T10:00:00Z"),
            agent_id: Some("claude-code".to_owned()),
            client: Some(ClientInfo {
                name: "claude-code".to_owned(),
                version: "2.1.263".to_owned(),
            }),
            server_version: Some("0.2.0".to_owned()),
            hook: None,
        }
    }

    fn hook_identity(pid: u32, ppid: u32, cwd: &str) -> Identity {
        Identity {
            pid,
            ppid,
            ancestors: Vec::new(),
            cwd: cwd.to_owned(),
            project_dir: None,
        }
    }

    fn hook_input(session_id: &str) -> HookInput {
        HookInput {
            session_id: session_id.to_owned(),
            hook_event_name: HookEventName::Stop,
            stop_hook_active: false,
            agent_id: None,
            agent_type: None,
        }
    }

    /// Two agents in one shell: `shell` → `agent` → `server`, and the hook twelve
    /// generations under its own agent, which is what A-11 measured on Windows.
    fn one_machine() -> SyntheticProcessTable {
        SyntheticProcessTable::new()
            .with(9000, None, "explorer.exe")
            .with(8000, Some(9000), "WindowsTerminal.exe")
            .with(7000, Some(8000), "pwsh.exe")
            // Session A.
            .with(1000, Some(7000), "claude.exe")
            .with(1001, Some(1000), "handoff-mcp.exe")
            .with(1002, Some(1000), "node.exe")
            .with(1003, Some(1002), "hook.exe")
            // Session B, in the same shell.
            .with(2000, Some(7000), "claude.exe")
            .with(2001, Some(2000), "handoff-mcp.exe")
            .with(2002, Some(2000), "node.exe")
            .with(2003, Some(2002), "hook.exe")
    }

    #[test]
    fn a_registration_completes_the_chain_and_writes_it_through() {
        let mut fixture = Fixture::new();
        let table = one_machine();
        let peer = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");

        let session_ref = fixture
            .registry
            .register(&fixture.db, &peer, &table)
            .expect("a registration")
            .expect("a server registers");

        assert_eq!(session_ref, "ses_00000001");
        let session = fixture.registry.get(&session_ref).expect("the session");
        assert_eq!(
            session
                .pid_chain
                .iter()
                .map(|ancestor| ancestor.pid)
                .collect::<Vec<u32>>(),
            [1000, 7000, 8000, 9000],
            "the agent, the shell, the terminal and the desktop"
        );
        assert!(session.connected);
        assert_eq!(session.display_name(), "Claude Code · baton");

        let row = crate::log::sessions::get(&fixture.db, &session_ref)
            .expect("a read")
            .expect("a row");
        assert_eq!(row.pid_chain_json, "[1000,7000,8000,9000]");
        assert_eq!(row.client_version.as_deref(), Some("2.1.263"));
        assert!(row.connected);
        assert_eq!(fixture.changes(), 1);
    }

    #[test]
    fn a_codex_session_is_named_by_the_row_the_server_resolved() {
        // OPEN-02 and ADPT-03: the tab says which agent a session is, and the name comes from
        // the server's capability table, never from a list this application keeps. The Codex
        // row is the second one it knows (T-066), and nothing here had to learn about it.
        let mut fixture = Fixture::new();
        let table = one_machine();
        let mut peer = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");
        peer.agent_id = Some("codex".to_owned());
        peer.client = Some(ClientInfo {
            name: "codex-mcp-client".to_owned(),
            version: "0.153.4".to_owned(),
        });
        peer.capability_row = Some(CapabilityRow {
            agent_id: "codex".to_owned(),
            support: SupportLevel::Base,
            images_in_results: true,
            stop_hook: false,
            tool_timeout_ms: Some(1_800_000),
            display_name: Some("Codex CLI".to_owned()),
            subagent_stop_hook: Some(false),
            session_identity: None,
            user_request_delivery: None,
            cancellation_notifications: Some(false),
        });

        let session_ref = fixture
            .registry
            .register(&fixture.db, &peer, &table)
            .expect("a registration")
            .expect("a server registers");
        let session = fixture.registry.get(&session_ref).expect("the session");
        assert_eq!(session.display_name(), "Codex CLI · baton");
        assert_eq!(session.resumed_from().agent, "Codex CLI");
    }

    #[test]
    fn an_opencode_session_is_named_by_the_row_the_server_resolved() {
        // The third row the table knows (T-074), and again nothing here had to learn about it:
        // the name is the server's, carried in `hello`.
        let mut fixture = Fixture::new();
        let table = one_machine();
        let mut peer = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");
        peer.agent_id = Some("opencode".to_owned());
        peer.client = Some(ClientInfo {
            name: "opencode".to_owned(),
            version: "1.18.29".to_owned(),
        });
        peer.capability_row = Some(CapabilityRow {
            agent_id: "opencode".to_owned(),
            support: SupportLevel::Base,
            images_in_results: true,
            stop_hook: false,
            tool_timeout_ms: Some(1_800_000),
            display_name: Some("OpenCode".to_owned()),
            subagent_stop_hook: Some(false),
            session_identity: None,
            user_request_delivery: None,
            cancellation_notifications: Some(true),
        });

        let session_ref = fixture
            .registry
            .register(&fixture.db, &peer, &table)
            .expect("a registration")
            .expect("a server registers");
        let session = fixture.registry.get(&session_ref).expect("the session");
        assert_eq!(session.display_name(), "OpenCode · baton");
        assert_eq!(session.resumed_from().agent, "OpenCode");
    }

    #[test]
    fn a_hook_registers_nothing() {
        let mut fixture = Fixture::new();
        let mut peer = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");
        peer.role = PeerRole::Hook;
        peer.session_ref = None;

        assert_eq!(
            fixture
                .registry
                .register(&fixture.db, &peer, &one_machine())
                .expect("a hook peer"),
            None
        );
        assert_eq!(fixture.registry.sessions().count(), 0);
        assert_eq!(fixture.changes(), 0);
    }

    #[test]
    fn the_hook_of_two_sessions_in_one_shell_binds_to_its_own_agent() {
        // The case a plain set intersection cannot answer: both sessions share the shell,
        // the terminal and the desktop, and only the agent tells them apart.
        let mut fixture = Fixture::new();
        let table = one_machine();
        for (conn_id, session_ref, pid, ppid) in [
            (1, "ses_00000001", 1001, 1000),
            (2, "ses_00000002", 2001, 2000),
        ] {
            let peer = server_peer(conn_id, session_ref, pid, ppid, "C:\\projects\\baton");
            fixture
                .registry
                .register(&fixture.db, &peer, &table)
                .expect("a registration");
        }

        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(1003, 1002, "C:\\projects\\baton"),
                &hook_input("session-a"),
                &table,
            )
            .expect("a binding");
        assert_eq!(bound, HookBinding::Bound("ses_00000001".to_owned()));

        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(2003, 2002, "C:\\projects\\baton"),
                &hook_input("session-b"),
                &table,
            )
            .expect("a binding");
        assert_eq!(bound, HookBinding::Bound("ses_00000002".to_owned()));
    }

    #[test]
    fn a_unique_pid_match_binds_the_agents_session_id_for_good() {
        let mut fixture = Fixture::new();
        let table = one_machine();
        let peer = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");
        fixture
            .registry
            .register(&fixture.db, &peer, &table)
            .expect("a registration");

        fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(1003, 1002, "C:\\projects\\baton"),
                &hook_input("session-a"),
                &table,
            )
            .expect("a binding");

        let session = fixture.registry.get("ses_00000001").expect("the session");
        assert_eq!(session.claude_session_id.as_deref(), Some("session-a"));
        assert_eq!(
            crate::log::sessions::get(&fixture.db, "ses_00000001")
                .expect("a read")
                .expect("a row")
                .claude_session_id
                .as_deref(),
            Some("session-a")
        );

        // A later hook of the same agent needs no process table at all: §7.5 binds the id
        // for the rest of the session's life.
        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(4242, 4241, "C:\\elsewhere"),
                &hook_input("session-a"),
                &SyntheticProcessTable::new(),
            )
            .expect("a binding");
        assert_eq!(bound, HookBinding::Bound("ses_00000001".to_owned()));
    }

    #[test]
    fn the_working_directory_is_the_fallback_key_when_no_generation_matches() {
        // The editor case of SRV-19 turned inside out: nothing of the hook's chain is in
        // the session's, and only SRV-18 is left.
        let mut fixture = Fixture::new();
        let peer = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");
        fixture
            .registry
            .register(&fixture.db, &peer, &one_machine())
            .expect("a registration");

        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(5003, 5002, "c:\\projects\\baton\\"),
                &hook_input("session-a"),
                &SyntheticProcessTable::new(),
            )
            .expect("a binding");

        assert_eq!(bound, HookBinding::Bound("ses_00000001".to_owned()));
        assert_eq!(
            fixture
                .registry
                .get("ses_00000001")
                .expect("the session")
                .claude_session_id,
            None,
            "a fallback key does not bind an id for life"
        );
    }

    #[test]
    fn two_sessions_in_one_folder_with_no_intersection_are_ambiguous() {
        // FM-22 exactly: the hook is answered neutrally and the UI asks which tab.
        let mut fixture = Fixture::new();
        for (conn_id, session_ref, pid, ppid) in [
            (1, "ses_00000001", 1001, 1000),
            (2, "ses_00000002", 2001, 2000),
        ] {
            let peer = server_peer(conn_id, session_ref, pid, ppid, "C:\\projects\\baton");
            fixture
                .registry
                .register(&fixture.db, &peer, &one_machine())
                .expect("a registration");
        }

        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(5003, 5002, "C:\\projects\\baton"),
                &hook_input("session-a"),
                &SyntheticProcessTable::new(),
            )
            .expect("a binding");

        assert_eq!(
            bound,
            HookBinding::Ambiguous(vec!["ses_00000001".to_owned(), "ses_00000002".to_owned()])
        );
    }

    #[test]
    fn the_picker_answer_binds_the_session_id_the_hook_carried() {
        // FM-22 is only worth asking about if the answer changes something: from here on
        // the same agent session is recognised at once and SRV-12 stops being delayed.
        let mut fixture = Fixture::new();
        for (conn_id, session_ref, pid, ppid) in [
            (1, "ses_00000001", 1001, 1000),
            (2, "ses_00000002", 2001, 2000),
        ] {
            let peer = server_peer(conn_id, session_ref, pid, ppid, "C:\\projects\\baton");
            fixture
                .registry
                .register(&fixture.db, &peer, &one_machine())
                .expect("a registration");
        }

        let candidates = vec!["ses_00000001".to_owned(), "ses_00000002".to_owned()];
        fixture
            .registry
            .needs_session_picker(&candidates, "session-a");
        assert_eq!(fixture.registry.session_picker(), candidates.as_slice());

        assert!(fixture
            .registry
            .answer_session_picker(&fixture.db, "ses_00000002")
            .expect("an answer"));
        assert!(fixture.registry.session_picker().is_empty());
        assert_eq!(
            fixture
                .registry
                .get("ses_00000002")
                .expect("the session")
                .claude_session_id
                .as_deref(),
            Some("session-a")
        );

        // And the next hook of that agent session needs no chain at all.
        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(9003, 9002, "C:\\elsewhere"),
                &hook_input("session-a"),
                &SyntheticProcessTable::new(),
            )
            .expect("a binding");
        assert_eq!(bound, HookBinding::Bound("ses_00000002".to_owned()));
    }

    #[test]
    fn the_picker_refuses_an_answer_that_is_not_one_of_its_candidates() {
        let mut fixture = Fixture::new();
        let peer = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");
        fixture
            .registry
            .register(&fixture.db, &peer, &one_machine())
            .expect("a registration");

        fixture
            .registry
            .needs_session_picker(&["ses_00000009".to_owned()], "session-a");
        assert!(!fixture
            .registry
            .answer_session_picker(&fixture.db, "ses_00000001")
            .expect("an answer"));
        assert_eq!(
            fixture.registry.session_picker(),
            ["ses_00000009".to_owned()],
            "the question the user did not answer is still there"
        );
    }

    #[test]
    fn dismissing_the_picker_binds_nothing() {
        let mut fixture = Fixture::new();
        let peer = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");
        fixture
            .registry
            .register(&fixture.db, &peer, &one_machine())
            .expect("a registration");

        fixture
            .registry
            .needs_session_picker(&["ses_00000001".to_owned()], "session-a");
        fixture.registry.session_picker_answered();

        assert!(fixture.registry.session_picker().is_empty());
        assert_eq!(
            fixture
                .registry
                .get("ses_00000001")
                .expect("the session")
                .claude_session_id,
            None
        );
    }

    #[test]
    fn a_shared_generation_is_separated_by_the_working_directory() {
        // Both sessions hang off the same shell and the hook's chain reaches no further
        // than it; §7.5 lets the folder decide what the chain could not.
        let mut fixture = Fixture::new();
        let table = one_machine();
        for (conn_id, session_ref, pid, ppid, cwd) in [
            (1, "ses_00000001", 1001, 1000, "C:\\projects\\baton"),
            (2, "ses_00000002", 2001, 2000, "C:\\projects\\other"),
        ] {
            let peer = server_peer(conn_id, session_ref, pid, ppid, cwd);
            fixture
                .registry
                .register(&fixture.db, &peer, &table)
                .expect("a registration");
        }

        // A hook whose only known ancestor is the shared shell.
        let orphaned_hook = SyntheticProcessTable::new().with(6003, Some(7000), "hook.exe");
        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(6003, 7000, "C:\\projects\\other"),
                &hook_input("session-b"),
                &orphaned_hook,
            )
            .expect("a binding");

        assert_eq!(bound, HookBinding::Bound("ses_00000002".to_owned()));
    }

    #[test]
    fn nothing_matches_at_all() {
        let mut fixture = Fixture::new();
        let peer = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");
        fixture
            .registry
            .register(&fixture.db, &peer, &one_machine())
            .expect("a registration");

        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(5003, 5002, "C:\\somewhere\\else"),
                &hook_input("session-z"),
                &SyntheticProcessTable::new(),
            )
            .expect("a binding");
        assert_eq!(bound, HookBinding::None);
    }

    #[test]
    fn a_reconnection_under_the_same_agent_pid_is_a_new_session() {
        // §8.3: the old record is kept as history, and the hook follows the live one.
        let mut fixture = Fixture::new();
        let table = one_machine();
        let first = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");
        fixture
            .registry
            .register(&fixture.db, &first, &table)
            .expect("a registration");
        assert_eq!(
            fixture
                .registry
                .disconnect(&fixture.db, 1, &at("2026-09-08T11:00:00Z"))
                .expect("a disconnection")
                .as_deref(),
            Some("ses_00000001")
        );

        let second = server_peer(2, "ses_00000002", 1099, 1000, "C:\\projects\\baton");
        fixture
            .registry
            .register(&fixture.db, &second, &table)
            .expect("a second registration");

        assert_eq!(fixture.registry.sessions().count(), 2);
        assert_eq!(fixture.registry.connected().count(), 1);
        assert!(
            !fixture
                .registry
                .get("ses_00000001")
                .expect("kept")
                .connected
        );
        assert_eq!(
            crate::log::sessions::list(&fixture.db)
                .expect("the rows")
                .len(),
            2,
            "§8.3 keeps the old registration as history"
        );

        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(1003, 1002, "C:\\projects\\baton"),
                &hook_input("session-a"),
                &table,
            )
            .expect("a binding");
        assert_eq!(
            bound,
            HookBinding::Bound("ses_00000002".to_owned()),
            "the disconnected registration is not a candidate"
        );
    }

    #[test]
    fn disconnecting_a_connection_that_carried_no_session_says_so() {
        let mut fixture = Fixture::new();
        assert_eq!(
            fixture
                .registry
                .disconnect(&fixture.db, 42, &at("2026-09-08T11:00:00Z"))
                .expect("a disconnection"),
            None
        );
        assert_eq!(fixture.changes(), 0);
    }

    #[test]
    fn a_touch_moves_the_last_sign_of_life_and_tells_the_ui_nothing() {
        let mut fixture = Fixture::new();
        let peer = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");
        fixture
            .registry
            .register(&fixture.db, &peer, &one_machine())
            .expect("a registration");
        let before = fixture.changes();

        assert!(fixture
            .registry
            .touch(&fixture.db, 1, &at("2026-09-08T12:34:56Z"))
            .expect("a touch"));
        assert!(!fixture
            .registry
            .touch(&fixture.db, 99, &at("2026-09-08T12:34:56Z"))
            .expect("no such connection"));

        assert_eq!(
            fixture
                .registry
                .get("ses_00000001")
                .expect("the session")
                .last_seen,
            at("2026-09-08T12:34:56Z")
        );
        assert_eq!(fixture.changes(), before);
    }

    #[test]
    fn the_purge_forgets_the_old_and_keeps_what_a_handoff_cites() {
        let mut fixture = Fixture::new();
        let table = one_machine();
        for (conn_id, session_ref, pid, ppid) in [
            (1, "ses_00000001", 1001, 1000),
            (2, "ses_00000002", 2001, 2000),
        ] {
            let peer = server_peer(conn_id, session_ref, pid, ppid, "C:\\projects\\baton");
            fixture
                .registry
                .register(&fixture.db, &peer, &table)
                .expect("a registration");
        }
        let long_ago = at("2026-08-01T10:00:00Z");
        for conn_id in [1, 2] {
            fixture
                .registry
                .disconnect(&fixture.db, conn_id, &long_ago)
                .expect("a disconnection");
        }
        let mut row = crate::log::testing::handoff("hf_0123456789");
        row.session_ref = Some("ses_00000002".to_owned());
        handoffs::upsert(&fixture.db, &row).expect("a handoff");

        let forgotten = fixture
            .registry
            .purge(&fixture.db, &at("2026-09-08T10:00:00Z"))
            .expect("a purge");

        assert_eq!(forgotten, 1);
        assert!(fixture.registry.get("ses_00000001").is_none());
        assert!(fixture.registry.get("ses_00000002").is_some());
    }

    #[test]
    fn a_restart_closes_what_the_previous_run_left_open() {
        let db = Db::open_in_memory().expect("a database");
        crate::log::sessions::register(&db, &crate::log::testing::session("ses_00000001"))
            .expect("a session of a previous run");

        let registry = Registry::open(&db, Box::new(NoObserver)).expect("a registry");

        assert_eq!(registry.sessions().count(), 0, "memory starts empty (§2.3)");
        assert!(
            !crate::log::sessions::get(&db, "ses_00000001")
                .expect("a read")
                .expect("a row")
                .connected
        );
    }

    #[test]
    fn resumed_from_is_the_agent_and_the_project_folder() {
        let mut fixture = Fixture::new();
        let mut peer = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");
        peer.identity.project_dir = Some("/Users/someone/work/checkout-service/".to_owned());
        fixture
            .registry
            .register(&fixture.db, &peer, &one_machine())
            .expect("a registration");

        assert_eq!(
            fixture.registry.resumed_from("ses_00000001"),
            Some(ResumedFrom {
                agent: "Claude Code".to_owned(),
                project: "checkout-service".to_owned(),
            })
        );
        assert_eq!(fixture.registry.resumed_from("ses_nosuchth"), None);
    }

    #[test]
    fn the_labels_fall_back_until_they_find_something_to_show() {
        let mut fixture = Fixture::new();
        let mut peer = server_peer(1, "ses_00000001", 1001, 1000, "");
        peer.capability_row = None;
        peer.agent_id = None;
        peer.identity.project_dir = None;
        fixture
            .registry
            .register(&fixture.db, &peer, &one_machine())
            .expect("a registration");

        let session = fixture.registry.get("ses_00000001").expect("the session");
        assert_eq!(session.agent_label(), "claude-code", "the MCP client name");
        assert_eq!(
            session.project_label(),
            "ses_00000001",
            "never empty: the outcome schema refuses a blank project"
        );
    }

    #[test]
    fn a_path_is_cut_at_either_separator_and_compared_without_case() {
        assert_eq!(base_name("C:\\projects\\baton"), "baton");
        assert_eq!(base_name("/Users/someone/baton/"), "baton");
        assert_eq!(base_name("baton"), "baton");
        assert_eq!(base_name("/"), "");
        assert!(same_folder("C:\\Projects\\Baton", "c:\\projects\\baton\\"));
        assert!(!same_folder("C:\\projects\\baton", "C:\\projects\\other"));
    }
}
