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
//!
//! # A hook belongs to an agent that runs one
//!
//! Only a session whose capability row says its agent runs an end-of-turn hook of ours can be
//! the one a hook came from ([`Session::can_own_a_hook`]). Codex, OpenCode and Cursor run none,
//! and each of them can share generations with a Claude Code session: a shell, a terminal, an
//! editor's extension host. Were they candidates, the hook of a Claude Code chat whose server
//! had gone would walk up past its own agent and could be bound, for the rest of that chat's
//! life, to a Cursor window above it.
//!
//! # Sessions an editor started (T-070, §5.6, R-12)
//!
//! An editor of the VS Code family starts one server per window from that window's extension
//! host, and `hello` says so (`session_identity: ancestor_chain:editor`, T-069). Such a session
//! is keyed on the editor in its chain and on its workspace folder, not on the server's parent.
//! Every window has the editor's main process in its chain, so a hook whose nearest owned
//! generation is the editor reaches every window at once: the workspace folder is what separates
//! them, and the picker is what is left. The working directory is no key for such a session:
//! the editor starts every server in the user's home folder, which two windows share, and so
//! does every agent a person starts from there ([`Session::project_folder`]). Nothing here names
//! an agent — one agent can arrive with or without an editor above it (Cursor's editor and its
//! CLI share a row) — so GitHub Copilot's and Kilo Code's editor surfaces get the same rule
//! (T-072, T-081).

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

    /// Whether an editor started this session's server, which keys it at the editor
    /// (`ancestor_chain:editor`, §5.6, R-12).
    ///
    /// Read from the capability row `hello` carried, never from the agent id: the server
    /// resolves it per session (T-069), because one agent can arrive with or without an editor
    /// above it.
    #[must_use]
    pub fn is_editor_hosted(&self) -> bool {
        self.capability_row
            .as_ref()
            .is_some_and(CapabilityRow::is_editor_hosted)
    }

    /// The folder a person would say this session works on (OPEN-02, SRV-18).
    ///
    /// The project folder of §5.8, except for a session an editor started: there it is the
    /// workspace folder the editor named, and there is none when the project folder is still
    /// the working directory the editor started the server in — a window with no folder open,
    /// where the server fell back to that directory, which is the user's home folder (T-069).
    #[must_use]
    pub fn project_folder(&self) -> Option<&str> {
        let project = self.project_dir.as_deref();
        if self.is_editor_hosted() {
            project.filter(|project| !same_folder(project, &self.cwd))
        } else {
            project
        }
    }

    /// Whether a hook can be this session's at all (§7.5).
    ///
    /// Only an agent that runs an end-of-turn hook of ours can be where a hook came from, and
    /// the capability row says whether it does. A session with no row is a candidate, as every
    /// session was before this rule.
    #[must_use]
    pub fn can_own_a_hook(&self) -> bool {
        self.capability_row
            .as_ref()
            .is_none_or(CapabilityRow::runs_a_hook)
    }

    /// Whether a hook that reported `cwd` is working in this session's folder (SRV-18).
    ///
    /// Both the working directory and the project folder count. §7.5 writes the fallback
    /// key as `cwd` equality and §5.8 writes it as the project folder; a hook sends only a
    /// working directory, and a server whose agent set `CLAUDE_PROJECT_DIR` reports a
    /// project folder that differs from it, so accepting either is the only reading under
    /// which both sections describe a key that can match.
    ///
    /// A session an editor started counts its workspace folder alone (T-070): the editor
    /// starts every server in the user's home folder, whichever window it serves, so that
    /// working directory is shared by every window and by every agent started from there, and
    /// says nothing about the session.
    fn works_in(&self, cwd: &str) -> bool {
        if self.is_editor_hosted() {
            return self
                .project_folder()
                .is_some_and(|folder| same_folder(folder, cwd));
        }
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
    /// sessions whose agent runs a hook: a disconnected session's chain is a list of processes
    /// that are gone, and a pid the operating system has since handed to somebody else would
    /// bind a hook to the wrong session for good; and a session whose agent runs no hook of
    /// ours cannot be where one came from ([`Session::can_own_a_hook`], T-070).
    ///
    /// 1. the agent's own `session_id`, once a pid intersection has bound it — §7.5 binds it
    ///    "for the rest of its life", so a later hook of the same agent needs nothing else;
    /// 2. the pid intersection of SRV-17, nearest generation to the hook first (see the
    ///    module documentation for why the order is what makes the rule work at all). Where
    ///    a generation is shared by several sessions, the working directory separates them —
    ///    for a session an editor started, its workspace folder, which is what tells two
    ///    windows of one editor apart;
    /// 3. the working directory alone (SRV-18), when no generation matched.
    ///
    /// The walk stops at the first generation that any connected session owns. When every
    /// owner of it is an agent that runs no hook of ours, the hook is that agent's and the
    /// answer is neutral: the GitHub Copilot CLI runs a project's Claude Code `Stop` hook as
    /// its own (T-072, measured), and a walk that went on past its session would bind that
    /// hook — by a terminal both share, or by the folder — to a Claude Code session, and hand
    /// one agent the reminders owed to the other.
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
            if self
                .connected_refs(|session| session.owns_pid(ancestor.pid))
                .is_empty()
            {
                continue;
            }
            let candidates = self.hook_candidates(|session| session.owns_pid(ancestor.pid));
            if candidates.is_empty() {
                // The nearest session above the hook runs no hook of ours: the hook is its
                // agent's own, not a session's further up (T-072).
                return Ok(HookBinding::None);
            }
            if let [session_ref] = candidates.as_slice() {
                let session_ref = session_ref.clone();
                self.bind_session_id(db, &session_ref, &hook.session_id)?;
                return Ok(HookBinding::Bound(session_ref));
            }
            // Several sessions live under the same process — every window of an editor lives
            // under the editor. §7.5 lets the folder separate what the chain could not: the
            // working directory, or an editor window's workspace folder.
            let narrowed = self.hook_candidates(|session| {
                session.owns_pid(ancestor.pid) && session.works_in(&identity.cwd)
            });
            if let [session_ref] = narrowed.as_slice() {
                let session_ref = session_ref.clone();
                self.bind_session_id(db, &session_ref, &hook.session_id)?;
                return Ok(HookBinding::Bound(session_ref));
            }
            return Ok(HookBinding::Ambiguous(candidates));
        }

        let candidates = self.hook_candidates(|session| session.works_in(&identity.cwd));
        match candidates.as_slice() {
            [] => Ok(HookBinding::None),
            [session_ref] => Ok(HookBinding::Bound(session_ref.clone())),
            _ => Ok(HookBinding::Ambiguous(candidates)),
        }
    }

    /// The connected sessions a hook can belong to that `wanted` accepts (§7.5).
    ///
    /// Every key of [`Registry::bind_hook`] goes through here, so no key can reach a session
    /// whose agent runs no hook of ours ([`Session::can_own_a_hook`]).
    fn hook_candidates(&self, mut wanted: impl FnMut(&Session) -> bool) -> Vec<String> {
        self.connected_refs(|session| session.can_own_a_hook() && wanted(session))
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

    // ------------------------------------------------------------ editor sessions (T-070)

    /// The home folder an editor starts every one of its servers in (T-069).
    const HOME: &str = "C:\\Users\\someone";

    /// One Cursor editor with two windows, and around it the agents a person runs beside it.
    ///
    /// `explorer` → the editor's main process → one extension host per window, each hosting
    /// our server; a pty host whose shell runs Claude Code in the editor's terminal; a Claude
    /// Code chat of its VS Code extension, started by window A's extension host; and a process
    /// the editor itself starts beside its windows.
    fn editor_machine() -> SyntheticProcessTable {
        SyntheticProcessTable::new()
            .with(9000, None, "explorer.exe")
            .with(3000, Some(9000), "Cursor.exe")
            // Window A: its extension host, and the server it started.
            .with(3100, Some(3000), "Cursor.exe")
            .with(3101, Some(3100), "handoff-mcp.exe")
            // Window B.
            .with(3200, Some(3000), "Cursor.exe")
            .with(3201, Some(3200), "handoff-mcp.exe")
            // Claude Code in the editor's terminal: its server, and its hook under a bash.
            .with(3300, Some(3000), "Cursor.exe")
            .with(3310, Some(3300), "pwsh.exe")
            .with(3320, Some(3310), "claude.exe")
            .with(3321, Some(3320), "handoff-mcp.exe")
            .with(3322, Some(3320), "bash.exe")
            .with(3323, Some(3322), "handoff-mcp.exe")
            // A Claude Code chat of the extension, under window A's extension host.
            .with(3130, Some(3100), "claude.exe")
            .with(3131, Some(3130), "handoff-mcp.exe")
            .with(3132, Some(3130), "bash.exe")
            .with(3133, Some(3132), "handoff-mcp.exe")
            // Something the editor itself starts, beside its windows.
            .with(3900, Some(3000), "handoff-mcp.exe")
    }

    /// The row a server sends for a session an editor started: the editor key, and whether the
    /// agent runs a hook — Cursor's does not (T-069); a later editor agent may (T-072).
    fn editor_row(runs_a_hook: bool) -> CapabilityRow {
        CapabilityRow {
            agent_id: "cursor".to_owned(),
            support: SupportLevel::Base,
            images_in_results: true,
            stop_hook: runs_a_hook,
            tool_timeout_ms: Some(60_000),
            display_name: Some("Cursor".to_owned()),
            subagent_stop_hook: Some(false),
            session_identity: Some(crate::format::channel::EDITOR_SESSION_IDENTITY.to_owned()),
            user_request_delivery: None,
            cancellation_notifications: Some(true),
        }
    }

    /// The server an editor started for one window: in the home folder, on the window's folder.
    fn editor_peer(
        conn_id: ConnId,
        session_ref: &str,
        (pid, ppid): (u32, u32),
        workspace: &str,
        runs_a_hook: bool,
    ) -> Peer {
        let mut peer = server_peer(conn_id, session_ref, pid, ppid, HOME);
        peer.identity.project_dir = Some(workspace.to_owned());
        peer.agent_id = Some("cursor".to_owned());
        peer.client = Some(ClientInfo {
            name: "cursor-vscode".to_owned(),
            version: "1.0.0".to_owned(),
        });
        peer.capability_row = Some(editor_row(runs_a_hook));
        peer
    }

    fn register_all(fixture: &mut Fixture, table: &SyntheticProcessTable, peers: &[Peer]) {
        for peer in peers {
            fixture
                .registry
                .register(&fixture.db, peer, table)
                .expect("a registration");
        }
    }

    #[test]
    fn a_session_an_editor_started_is_keyed_at_the_editor_and_named_by_its_folder() {
        let mut fixture = Fixture::new();
        let window = editor_peer(1, "ses_0000000a", (3101, 3100), "C:\\work\\shop", false);
        register_all(&mut fixture, &editor_machine(), &[window]);

        let session = fixture.registry.get("ses_0000000a").expect("the session");
        assert!(session.is_editor_hosted());
        assert_eq!(
            session.display_name(),
            "Cursor · shop",
            "OPEN-02: the window's folder, not the home folder the server runs in"
        );
        assert_eq!(session.project_folder(), Some("C:\\work\\shop"));
        assert_eq!(
            session
                .pid_chain
                .iter()
                .map(|ancestor| ancestor.pid)
                .collect::<Vec<u32>>(),
            [3100, 3000, 9000],
            "the window's extension host, the editor, the desktop"
        );
    }

    #[test]
    fn a_window_with_no_folder_open_has_no_folder_to_be_matched_by() {
        // The server fell back to its working directory, the home folder (§5.8, T-069).
        let mut fixture = Fixture::new();
        let window = editor_peer(1, "ses_0000000a", (3101, 3100), HOME, false);
        register_all(&mut fixture, &editor_machine(), &[window]);
        let session = fixture.registry.get("ses_0000000a").expect("the session");
        assert!(session.is_editor_hosted());
        assert_eq!(session.project_folder(), None);

        // A session of any other kind keeps its project folder, whatever it is.
        let mut other = Fixture::new();
        let agent = server_peer(1, "ses_00000001", 1001, 1000, HOME);
        register_all(&mut other, &one_machine(), &[agent]);
        let session = other.registry.get("ses_00000001").expect("the session");
        assert!(!session.is_editor_hosted());
        assert_eq!(session.project_folder(), Some(HOME));
    }

    #[test]
    fn two_windows_of_one_editor_are_told_apart_by_their_folder_then_by_the_picker() {
        // A hook the editor itself started reaches both windows at the editor; the workspace
        // folder decides, and the home folder every window runs in decides nothing. The row is
        // a hook-capable editor agent's: Cursor runs none (T-069), a later editor agent may.
        let mut fixture = Fixture::new();
        let table = editor_machine();
        let windows = [
            editor_peer(1, "ses_0000000a", (3101, 3100), "C:\\work\\shop", true),
            editor_peer(2, "ses_0000000b", (3201, 3200), "C:\\work\\blog", true),
        ];
        register_all(&mut fixture, &table, &windows);

        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(3900, 3000, "C:\\work\\blog"),
                &hook_input("chat-b"),
                &table,
            )
            .expect("a binding");
        assert_eq!(bound, HookBinding::Bound("ses_0000000b".to_owned()));

        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(3900, 3000, HOME),
                &hook_input("chat-x"),
                &table,
            )
            .expect("a binding");
        assert_eq!(
            bound,
            HookBinding::Ambiguous(vec!["ses_0000000a".to_owned(), "ses_0000000b".to_owned()]),
            "FM-22: the picker asks which window"
        );
    }

    #[test]
    fn a_hook_through_one_windows_extension_host_is_that_windows() {
        let mut fixture = Fixture::new();
        let table = editor_machine().with(3190, Some(3100), "handoff-mcp.exe");
        let windows = [
            editor_peer(1, "ses_0000000a", (3101, 3100), "C:\\work\\shop", true),
            editor_peer(2, "ses_0000000b", (3201, 3200), "C:\\work\\blog", true),
        ];
        register_all(&mut fixture, &table, &windows);

        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(3190, 3100, HOME),
                &hook_input("chat-a"),
                &table,
            )
            .expect("a binding");
        assert_eq!(bound, HookBinding::Bound("ses_0000000a".to_owned()));
    }

    #[test]
    fn a_claude_code_session_inside_the_editor_keeps_its_own_hooks() {
        // The regression the task's risk is about (T-070): the editor's windows share the
        // editor, and Claude Code runs inside it, in its terminal and in its extension.
        let mut fixture = Fixture::new();
        let table = editor_machine();
        let sessions = [
            editor_peer(1, "ses_0000000a", (3101, 3100), "C:\\work\\shop", false),
            editor_peer(2, "ses_0000000b", (3201, 3200), "C:\\work\\blog", false),
            server_peer(3, "ses_0000000c", 3321, 3320, "C:\\work\\shop"),
            server_peer(4, "ses_0000000d", 3131, 3130, "C:\\work\\shop"),
        ];
        register_all(&mut fixture, &table, &sessions);

        let terminal = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(3323, 3322, "C:\\work\\shop"),
                &hook_input("terminal"),
                &table,
            )
            .expect("a binding");
        assert_eq!(terminal, HookBinding::Bound("ses_0000000c".to_owned()));

        let extension = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(3133, 3132, "C:\\work\\shop"),
                &hook_input("extension"),
                &table,
            )
            .expect("a binding");
        assert_eq!(extension, HookBinding::Bound("ses_0000000d".to_owned()));
    }

    #[test]
    fn a_session_whose_agent_runs_no_hook_is_never_given_one() {
        // Cursor runs none (T-069). Without the rule, the hook of a Claude Code chat whose
        // server had gone walked up past its own agent to the extension host it shares with a
        // Cursor window, found that window alone there, and bound itself to it for good.
        let mut fixture = Fixture::new();
        let table = editor_machine();
        let sessions = [
            editor_peer(1, "ses_0000000a", (3101, 3100), "C:\\work\\shop", false),
            server_peer(2, "ses_0000000d", 3131, 3130, "C:\\work\\shop"),
        ];
        register_all(&mut fixture, &table, &sessions);
        fixture
            .registry
            .disconnect(&fixture.db, 2, &at("2026-09-11T12:00:00Z"))
            .expect("a disconnection");

        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(3133, 3132, "C:\\work\\shop"),
                &hook_input("extension"),
                &table,
            )
            .expect("a binding");
        assert_eq!(bound, HookBinding::None);
        assert_eq!(
            fixture
                .registry
                .get("ses_0000000a")
                .expect("the window")
                .claude_session_id,
            None
        );
    }

    #[test]
    fn an_agent_started_in_the_home_folder_is_not_taken_for_an_editor_window() {
        // The editor starts its servers in the home folder, where a person starts agents too:
        // the fallback of SRV-18 finds the agent and not the window. The editor row runs a hook
        // here, so that it is the folder rule that keeps the window out, not the hook rule.
        let mut fixture = Fixture::new();
        let agent = server_peer(1, "ses_00000001", 1001, 1000, HOME);
        register_all(&mut fixture, &one_machine(), &[agent]);
        let window = editor_peer(2, "ses_0000000a", (3101, 3100), "C:\\work\\shop", true);
        register_all(&mut fixture, &editor_machine(), &[window]);

        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(5003, 5002, HOME),
                &hook_input("home"),
                &SyntheticProcessTable::new(),
            )
            .expect("a binding");
        assert_eq!(bound, HookBinding::Bound("ses_00000001".to_owned()));
    }

    #[test]
    fn a_session_with_no_capability_row_is_a_candidate_as_before() {
        let mut fixture = Fixture::new();
        let table = one_machine();
        let mut peer = server_peer(1, "ses_00000001", 1001, 1000, "C:\\projects\\baton");
        peer.capability_row = None;
        register_all(&mut fixture, &table, &[peer]);

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
    }

    // ------------------------------------------ a hook another agent runs (T-072)

    /// The row the GitHub Copilot CLI's server sends: no hook of ours, keyed on its parent.
    fn copilot_cli_row() -> CapabilityRow {
        CapabilityRow {
            agent_id: "copilot".to_owned(),
            support: SupportLevel::Base,
            images_in_results: true,
            stop_hook: false,
            tool_timeout_ms: Some(1_800_000),
            display_name: Some("GitHub Copilot".to_owned()),
            subagent_stop_hook: Some(false),
            session_identity: None,
            user_request_delivery: None,
            cancellation_notifications: Some(true),
        }
    }

    /// Two tabs of one terminal: Claude Code in one, the Copilot CLI in the other, and the
    /// Copilot CLI running a project's Claude Code `Stop` hook through a PowerShell of its own
    /// at the end of its turn (T-072, measured against the CLI 1.0.83).
    fn two_agents_one_terminal() -> SyntheticProcessTable {
        SyntheticProcessTable::new()
            .with(9000, None, "explorer.exe")
            .with(8000, Some(9000), "WindowsTerminal.exe")
            // Tab one: Claude Code and its server.
            .with(7000, Some(8000), "pwsh.exe")
            .with(1000, Some(7000), "claude.exe")
            .with(1001, Some(1000), "handoff-mcp.exe")
            // Tab two: the Copilot CLI, its server, and the hook it runs.
            .with(7100, Some(8000), "pwsh.exe")
            .with(4000, Some(7100), "copilot.exe")
            .with(4001, Some(4000), "handoff-mcp.exe")
            .with(4010, Some(4000), "pwsh.exe")
            .with(4011, Some(4010), "handoff-mcp.exe")
    }

    #[test]
    fn a_hook_the_copilot_cli_runs_is_not_bound_to_a_claude_code_session() {
        let mut fixture = Fixture::new();
        let table = two_agents_one_terminal();
        let claude = server_peer(1, "ses_00000001", 1001, 1000, "C:\\work\\shop");
        let mut copilot = server_peer(2, "ses_00000002", 4001, 4000, "C:\\work\\shop");
        copilot.agent_id = Some("copilot".to_owned());
        copilot.capability_row = Some(copilot_cli_row());
        register_all(&mut fixture, &table, &[claude, copilot]);

        // The terminal is Claude Code's too, and so is the folder: the walk must stop at the
        // Copilot session the hook came from, and neither key may reach Claude Code's.
        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(4011, 4010, "C:\\work\\shop"),
                &hook_input("copilot-session"),
                &table,
            )
            .expect("a binding");
        assert_eq!(bound, HookBinding::None);
        assert_eq!(
            fixture
                .registry
                .get("ses_00000001")
                .expect("the Claude Code session")
                .claude_session_id,
            None
        );

        // Claude Code's own hook still finds its session.
        let table =
            table
                .with(1002, Some(1000), "bash.exe")
                .with(1003, Some(1002), "handoff-mcp.exe");
        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(1003, 1002, "C:\\work\\shop"),
                &hook_input("claude-session"),
                &table,
            )
            .expect("a binding");
        assert_eq!(bound, HookBinding::Bound("ses_00000001".to_owned()));
    }

    #[test]
    fn a_generation_a_hook_runner_shares_with_another_agent_is_still_its() {
        // The rule stops the walk only where no owner runs a hook: a Copilot CLI started from
        // Claude Code's own shell lives under Claude Code, and Claude Code's hook still binds.
        let mut fixture = Fixture::new();
        let table = SyntheticProcessTable::new()
            .with(9000, None, "explorer.exe")
            .with(1000, Some(9000), "claude.exe")
            .with(1001, Some(1000), "handoff-mcp.exe")
            .with(1002, Some(1000), "bash.exe")
            .with(1003, Some(1002), "handoff-mcp.exe")
            .with(4000, Some(1000), "copilot.exe")
            .with(4001, Some(4000), "handoff-mcp.exe");
        let claude = server_peer(1, "ses_00000001", 1001, 1000, "C:\\work\\shop");
        let mut copilot = server_peer(2, "ses_00000002", 4001, 4000, "C:\\work\\shop");
        copilot.capability_row = Some(copilot_cli_row());
        register_all(&mut fixture, &table, &[claude, copilot]);

        let bound = fixture
            .registry
            .bind_hook(
                &fixture.db,
                &hook_identity(1003, 1002, "C:\\work\\shop"),
                &hook_input("claude-session"),
                &table,
            )
            .expect("a binding");
        assert_eq!(bound, HookBinding::Bound("ses_00000001".to_owned()));
    }
}
