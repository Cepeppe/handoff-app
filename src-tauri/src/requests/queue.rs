//! The queue of things waiting to reach an agent (§7.7, OPEN-03..08, FM-20, FM-31, FM-34).
//!
//! Two kinds of entry, one table and one delivery machinery:
//!
//! - a **request**, which the user typed in the request sheet: it asks for a spec, its id
//!   is the id the handoff will take over (DD-13), and it is answered when a handoff adopts
//!   it or is linked to it (OPEN-08);
//! - a **resume**, queued when the user picks a parked or deferred handoff up in the overlay
//!   while its agent is elsewhere (RESP-07, FM-31): it opens nothing and is answered when a
//!   call finally attaches to the handoff it names.
//!
//! Both reach an agent through the same two paths — the clipboard, which is the fast one
//! (OPEN-05), and the Stop hook at the end of a turn, which is the safety net (OPEN-06).
//! Neither path is in this module: the queue decides *what* is waiting for whom and says so
//! through [`RequestObserver`]; putting text on a clipboard, finding a terminal window and
//! showing a notification are side effects, and `lib.rs` explains why the core never has
//! them. `hook::decide` reads the queue directly, because the hook's answer *is* the
//! delivery.
//!
//! # Where the assignment of a request to a session comes from, and where it goes
//!
//! A request may be queued with no session at all (OPEN-04a). It is then given to the first
//! session that registers ([`Queue::deliver_to_first_session`]), and given back to the
//! unassigned queue if that session detaches before it produced a spec (FM-34,
//! [`Queue::requeue_on_detach`]). A crash detaches every session at once without asking, so
//! [`Queue::requeue_on_start`] does the same thing for all of them when the app comes back:
//! an open request is assigned only to a session that is connected *now*, and a
//! `session_ref` from a previous run names nothing.

use crate::ids;
use crate::log::user_requests::{self, DeliveredVia};
use crate::log::{Db, Result, Timestamp};

use super::text::resume_queue_text;

/// One entry of the queue: exactly the row §7.7 persists (`log::user_requests`).
pub use crate::log::user_requests::UserRequestRow as UserRequest;

/// A request that has a session to go to (OPEN-05, OPEN-04a, FM-31).
///
/// It carries the whole entry rather than the two fields §7.7 names, because the text that
/// goes on the clipboard is rendered from it — the id belongs in the sentence (OPEN-05) and
/// a resume is a different sentence (`requests::text`) — and because the handler has to
/// mark what it delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestReadyForSession {
    /// The session it should be put in front of.
    pub session_ref: String,
    /// What is waiting.
    pub request: UserRequest,
}

/// Told that something in the queue can be put in front of a session.
///
/// The overlay answers by rendering the text in the user's language, putting it on the
/// clipboard and bringing that session's terminal to the front (OPEN-05, FM-21). It returns
/// nothing: a clipboard that refuses must not undo a queued request, and the hook still
/// delivers it at the end of the turn (OPEN-06).
///
/// `db` is the caller's connection, for the same reason every method of [`Queue`] takes one:
/// recording that the clipboard carried an entry is a write about the announcement that has
/// just been made, and it belongs on the connection that made it rather than on a fourth one
/// opened for the purpose. No caller holds a transaction open across this call.
pub trait RequestObserver: Send + Sync {
    /// `ready` is waiting for a session that is connected now.
    fn request_ready(&self, db: &Db, ready: &RequestReadyForSession);
}

/// The observer of a queue with no window behind it: every test that is not about the
/// clipboard, and every build before T-038.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoRequestObserver;

impl RequestObserver for NoRequestObserver {
    fn request_ready(&self, _db: &Db, _ready: &RequestReadyForSession) {}
}

/// What a `handoff.open` found in the queue (OPEN-08, DD-13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenLink {
    /// The agent quoted this request's id, so the handoff *is* it (DD-13).
    Adopted(UserRequest),
    /// The agent quoted nothing and this was the oldest request the session had open, so
    /// the handoff answers it without taking its id (OPEN-08, FM-20).
    Oldest(UserRequest),
    /// Nothing in the queue for this handoff.
    None,
}

impl OpenLink {
    /// The request either way, when there is one.
    #[must_use]
    pub fn request(&self) -> Option<&UserRequest> {
        match self {
            Self::Adopted(request) | Self::Oldest(request) => Some(request),
            Self::None => None,
        }
    }
}

/// The queue of §7.7.
///
/// It owns no connection: every method takes the caller's, exactly as `sessions::Registry`
/// does, so the store writes the queue inside its own transition and the dispatch writes it
/// on the connection the registry uses (§7.11 expects the two).
pub struct Queue {
    observer: Box<dyn RequestObserver>,
}

impl std::fmt::Debug for Queue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Queue").finish_non_exhaustive()
    }
}

impl Queue {
    /// A queue that tells `observer` when something can be put in front of a session.
    #[must_use]
    pub fn new(observer: Box<dyn RequestObserver>) -> Self {
        Self { observer }
    }

    /// Queues what the user typed in the request sheet and returns its id (OPEN-04, OPEN-05).
    ///
    /// The id is a handoff id, because it becomes one (DD-13). The tab in "waiting for spec"
    /// is the store's to open and the clipboard is the sheet's to fill, so this emits
    /// nothing: the user is looking at the window that called it.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the row cannot be written.
    pub fn create(
        &self,
        db: &Db,
        text: &str,
        session: Option<&str>,
        now: &Timestamp,
    ) -> Result<String> {
        let id = ids::new_request_id();
        user_requests::upsert(
            db,
            &UserRequest {
                id: id.clone(),
                session_ref: session.map(ToOwned::to_owned),
                text: text.to_owned(),
                created_at: now.clone(),
                delivered_via: None,
                linked_handoff_id: None,
                about_handoff_id: None,
            },
        )?;
        tracing::info!(
            request_id = id,
            session_ref = session,
            "a user request was queued"
        );
        Ok(id)
    }

    /// Queues a request to come back to a handoff the user resumed from the overlay
    /// (RESP-07, FM-31), and asks for it to be copied.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the row cannot be written.
    pub fn queue_resume_request(
        &self,
        db: &Db,
        handoff_id: &str,
        session: Option<&str>,
        now: &Timestamp,
    ) -> Result<String> {
        let id = ids::new_request_id();
        let request = UserRequest {
            id: id.clone(),
            session_ref: session.map(ToOwned::to_owned),
            text: resume_queue_text(handoff_id),
            created_at: now.clone(),
            delivered_via: None,
            linked_handoff_id: None,
            about_handoff_id: Some(handoff_id.to_owned()),
        };
        user_requests::upsert(db, &request)?;
        tracing::info!(
            request_id = id,
            handoff_id,
            session_ref = session,
            "a resume request was queued"
        );
        if let Some(session_ref) = session {
            self.observer.request_ready(
                db,
                &RequestReadyForSession {
                    session_ref: session_ref.to_owned(),
                    request,
                },
            );
        }
        Ok(id)
    }

    /// The request a `handoff.open` answers (DD-13, OPEN-08).
    ///
    /// `request_id` is what the agent quoted, already checked by the store against the ids
    /// it holds. Quoting an id no request carries is not an error: the handoff still takes
    /// the id — that is what the "waiting for spec" tab of §7.7 is — and there is simply
    /// nothing in the queue to close.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the rows cannot be read.
    pub fn link_on_open(
        &self,
        db: &Db,
        session: Option<&str>,
        request_id: Option<&str>,
    ) -> Result<OpenLink> {
        if let Some(request_id) = request_id {
            return Ok(match user_requests::get(db, request_id)? {
                Some(request) if request.linked_handoff_id.is_none() => OpenLink::Adopted(request),
                _ => OpenLink::None,
            });
        }
        // OPEN-08: "the first new handoff of that session". Nothing counts the handoffs —
        // the oldest *open* request is the first one a handoff has not answered yet, so the
        // second handoff of the session finds the next one, or nothing.
        let Some(session_ref) = session else {
            return Ok(OpenLink::None);
        };
        Ok(
            match user_requests::oldest_open_for_session(db, session_ref)? {
                Some(request) => OpenLink::Oldest(request),
                None => OpenLink::None,
            },
        )
    }

    /// Records that `handoff_id` answers `request_id` and nothing else (OPEN-08, FM-20).
    ///
    /// Whatever this handoff was answering becomes collectable again: the **Change** control
    /// of §7.7 exists so that a mis-link costs one click, and a request consumed by the
    /// handoff that turned out not to be about it would be gone for good.
    ///
    /// Returns whether the queue holds that request at all.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the update fails.
    pub fn relink(&self, db: &Db, handoff_id: &str, request_id: &str) -> Result<bool> {
        user_requests::unlink_handoff(db, handoff_id)?;
        user_requests::link(db, request_id, handoff_id)
    }

    /// The user abandoned the tab a request had opened, so the request is over (OPEN-04).
    ///
    /// It is closed the way a resume is closed (`close_resumes_about`): the entry points at
    /// the handoff that answers it, which here is the handoff it *became* — a user-opened
    /// request and its tab share one id (DD-13). From then on it is out of
    /// `oldest_open_for_session`, out of the hook's items, and out of `list_unassigned_open`.
    ///
    /// Returns whether the queue held that request at all; an agent-opened handoff has none.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the update fails.
    pub fn abandoned(&self, db: &Db, request_id: &str) -> Result<bool> {
        let closed = user_requests::link(db, request_id, request_id)?;
        if closed {
            tracing::info!(request_id, "a queued request was abandoned by the user");
        }
        Ok(closed)
    }

    /// An agent's call attached to `handoff_id`, so the resume requests about it are
    /// answered (FM-31).
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the update fails.
    pub fn resumed(&self, db: &Db, handoff_id: &str) -> Result<usize> {
        user_requests::close_resumes_about(db, handoff_id)
    }

    /// Gives the unassigned queue to a session that has just registered (OPEN-04a).
    ///
    /// Returns how many were handed over. Each one is announced, so the overlay copies it
    /// and notifies (OPEN-05); the Stop hook of that session delivers it again at the end of
    /// its next turn if the user never pasted it (OPEN-06).
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the rows cannot be read or written.
    pub fn deliver_to_first_session(&self, db: &Db, session_ref: &str) -> Result<usize> {
        let waiting = user_requests::list_unassigned_open(db)?;
        let mut handed = 0;
        for mut request in waiting {
            if !user_requests::assign(db, &request.id, session_ref)? {
                continue;
            }
            request.session_ref = Some(session_ref.to_owned());
            self.observer.request_ready(
                db,
                &RequestReadyForSession {
                    session_ref: session_ref.to_owned(),
                    request,
                },
            );
            handed += 1;
        }
        if handed > 0 {
            tracing::info!(
                session_ref,
                count = handed,
                "the queued requests were given to a session that registered"
            );
        }
        Ok(handed)
    }

    /// A session went away with requests still waiting for a spec (FM-34).
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the update fails.
    pub fn requeue_on_detach(&self, db: &Db, session_ref: &str) -> Result<usize> {
        let requeued = user_requests::unassign_open_of(db, session_ref)?;
        if requeued > 0 {
            tracing::info!(
                session_ref,
                count = requeued,
                "the open requests of a detached session went back to the queue"
            );
        }
        Ok(requeued)
    }

    /// The same, for every session at once, at startup.
    ///
    /// No session is connected when the app starts — `sessions::Registry::open` has just
    /// closed the rows a killed process left behind — so every assignment in the table names
    /// a session of a previous run. Without this a request queued before a crash would stay
    /// addressed to a session that will never come back.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the update fails.
    pub fn requeue_on_start(&self, db: &Db) -> Result<usize> {
        let requeued = user_requests::unassign_all_open(db)?;
        if requeued > 0 {
            tracing::info!(
                count = requeued,
                "the open requests of the previous run went back to the queue"
            );
        }
        Ok(requeued)
    }

    /// What is open for a session: its own requests and the unassigned ones (§7.5).
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the rows cannot be read.
    pub fn open_for_session(&self, db: &Db, session_ref: &str) -> Result<Vec<UserRequest>> {
        user_requests::list_open(db, Some(session_ref))
    }

    /// One entry, if the queue holds it.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the row cannot be read.
    pub fn get(&self, db: &Db, id: &str) -> Result<Option<UserRequest>> {
        user_requests::get(db, id)
    }

    /// Records that the clipboard carried a request (OPEN-05).
    ///
    /// The hook's own deliveries are recorded by `log::hook_blocks::commit_decision`, in the
    /// same transaction as the blocks that announced them.
    ///
    /// # Errors
    ///
    /// [`crate::log::StoreError::Persistence`] when the update fails.
    pub fn delivered_by_clipboard(&self, db: &Db, id: &str) -> Result<bool> {
        user_requests::mark_delivered(db, id, DeliveredVia::Clipboard)
    }
}

/// The queue as the store reaches it (`store::runbook_sink`).
///
/// `Arc` because the queue has two callers on two tasks: the store's actor, which links a
/// request inside the transition that answers it, and the channel dispatch, which hands the
/// queue to a session that registers and reads it for every hook. Neither owns it.
///
/// Every method but the first swallows its error into a log line, which is what the trait
/// promises: a queue entry that cannot be written must not undo the handoff that was.
impl crate::store::Requests for std::sync::Arc<Queue> {
    fn link_on_open(&self, db: &Db, session: Option<&str>, request_id: Option<&str>) -> OpenLink {
        Queue::link_on_open(self, db, session, request_id).unwrap_or_else(|error| {
            tracing::error!(error = %error, "the request queue could not be read for an open");
            OpenLink::None
        })
    }

    fn linked(&self, db: &Db, handoff_id: &str, request_id: &str) {
        if let Err(error) = Queue::relink(self, db, handoff_id, request_id) {
            tracing::error!(error = %error, handoff_id, request_id, "a request could not be linked");
        }
    }

    fn request_resume(&self, db: &Db, handoff_id: &str, session_ref: Option<&str>) {
        if let Err(error) =
            Queue::queue_resume_request(self, db, handoff_id, session_ref, &Timestamp::now())
        {
            tracing::error!(error = %error, handoff_id, "a resume request could not be queued");
        }
    }

    fn resumed(&self, db: &Db, handoff_id: &str) {
        if let Err(error) = Queue::resumed(self, db, handoff_id) {
            tracing::error!(error = %error, handoff_id, "a resume request could not be closed");
        }
    }

    fn abandoned(&self, db: &Db, request_id: &str) {
        if let Err(error) = Queue::abandoned(self, db, request_id) {
            tracing::error!(error = %error, request_id, "an abandoned request could not be closed");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::log::testing::{at, handoff, session};
    use crate::log::{handoffs, sessions};

    /// An observer that remembers what it was told.
    #[derive(Debug, Default)]
    struct Recording {
        ready: Mutex<Vec<RequestReadyForSession>>,
    }

    impl RequestObserver for std::sync::Arc<Recording> {
        fn request_ready(&self, _db: &Db, ready: &RequestReadyForSession) {
            self.ready
                .lock()
                .expect("the recorder is not poisoned")
                .push(ready.clone());
        }
    }

    fn queue() -> (Queue, std::sync::Arc<Recording>) {
        let recorder = std::sync::Arc::new(Recording::default());
        (
            Queue::new(Box::new(std::sync::Arc::clone(&recorder))),
            recorder,
        )
    }

    fn db_with_two_sessions() -> Db {
        let db = Db::open_in_memory().expect("a database");
        sessions::register(&db, &session("ses_00000001")).expect("a session");
        sessions::register(&db, &session("ses_00000002")).expect("another session");
        db
    }

    fn announced(recorder: &Recording) -> Vec<String> {
        recorder
            .ready
            .lock()
            .expect("the recorder is not poisoned")
            .iter()
            .map(|ready| ready.request.id.clone())
            .collect()
    }

    #[test]
    fn a_request_quoted_by_its_id_is_the_one_adopted() {
        let db = db_with_two_sessions();
        let (queue, _) = queue();
        let mine = queue
            .create(
                &db,
                "add the webhook",
                Some("ses_00000001"),
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("a request");

        let link = queue
            .link_on_open(&db, Some("ses_00000001"), Some(&mine))
            .expect("a link");
        assert_eq!(
            link.request().map(|request| request.id.as_str()),
            Some(mine.as_str())
        );
        assert!(matches!(link, OpenLink::Adopted(_)));
    }

    #[test]
    fn a_request_the_user_gave_up_on_is_out_of_the_queue() {
        // OPEN-04: pressing Abandon on a tab that never got its spec must stop the hook
        // asking for one (OPEN-06) and stop the next handoff being linked to it (OPEN-08).
        let db = db_with_two_sessions();
        let (queue, _) = queue();
        let given_up = queue
            .create(
                &db,
                "first",
                Some("ses_00000001"),
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("a request");
        let kept = queue
            .create(
                &db,
                "second",
                Some("ses_00000001"),
                &at("2026-09-08T11:01:00Z"),
            )
            .expect("another request");
        // The tab `Store::open_request` writes beside the entry: the entry is closed by
        // pointing at the handoff it became, so that handoff has to exist (DD-13).
        handoffs::upsert(&db, &handoff(&given_up)).expect("the tab");

        assert!(queue.abandoned(&db, &given_up).expect("the queue holds it"));

        let open = queue
            .open_for_session(&db, "ses_00000001")
            .expect("the open ones");
        assert_eq!(
            open.iter()
                .map(|entry| entry.id.clone())
                .collect::<Vec<_>>(),
            vec![kept.clone()]
        );
        // And the oldest open one is now the second, not the abandoned first.
        let link = queue
            .link_on_open(&db, Some("ses_00000001"), None)
            .expect("a link");
        assert_eq!(
            link.request().map(|request| request.id.as_str()),
            Some(kept.as_str())
        );
    }

    #[test]
    fn abandoning_a_handoff_the_queue_never_held_changes_nothing() {
        // Every agent-opened handoff takes this path when the user abandons it.
        let db = db_with_two_sessions();
        let (queue, _) = queue();
        assert!(!queue
            .abandoned(&db, "hf_0000000001")
            .expect("no such entry"));
    }

    #[test]
    fn an_open_without_a_request_id_takes_the_oldest_of_that_session() {
        // OPEN-08 and §12.4: two open requests in one session, the older one wins.
        let db = db_with_two_sessions();
        let (queue, _) = queue();
        let older = queue
            .create(
                &db,
                "first",
                Some("ses_00000001"),
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("a request");
        let newer = queue
            .create(
                &db,
                "second",
                Some("ses_00000001"),
                &at("2026-09-08T12:00:00Z"),
            )
            .expect("a request");
        let elsewhere = queue
            .create(
                &db,
                "another session",
                Some("ses_00000002"),
                &at("2026-09-08T10:00:00Z"),
            )
            .expect("a request");

        let link = queue
            .link_on_open(&db, Some("ses_00000001"), None)
            .expect("a link");
        assert!(matches!(link, OpenLink::Oldest(_)));
        let picked = link.request().expect("a request").id.clone();
        assert_eq!(picked, older);
        assert_ne!(
            picked, elsewhere,
            "another session's request is not offered"
        );

        // Once the first handoff answered it, the next one finds the second request.
        handoffs::upsert(&db, &handoff("hf_0123456789")).expect("the handoff");
        assert!(queue.relink(&db, "hf_0123456789", &older).expect("linked"));
        let next = queue
            .link_on_open(&db, Some("ses_00000001"), None)
            .expect("a link");
        assert_eq!(next.request().expect("a request").id, newer);
    }

    #[test]
    fn an_open_with_nothing_queued_links_nothing() {
        let db = db_with_two_sessions();
        let (queue, _) = queue();
        assert_eq!(
            queue
                .link_on_open(&db, Some("ses_00000001"), None)
                .expect("a link"),
            OpenLink::None
        );
        // An id no request carries: the handoff still takes it, the queue holds nothing.
        assert_eq!(
            queue
                .link_on_open(&db, Some("ses_00000001"), Some("hf_0123456789"))
                .expect("a link"),
            OpenLink::None
        );
        // And a session with nothing of its own is not given someone else's.
        queue
            .create(
                &db,
                "theirs",
                Some("ses_00000002"),
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("a request");
        assert_eq!(
            queue
                .link_on_open(&db, Some("ses_00000001"), None)
                .expect("a link"),
            OpenLink::None
        );
    }

    #[test]
    fn relinking_a_handoff_frees_the_request_it_was_answering() {
        // FM-20 and §12.4: the correction costs one click, so it must be undoable.
        let db = db_with_two_sessions();
        let (queue, _) = queue();
        let wrong = queue
            .create(
                &db,
                "the wrong one",
                Some("ses_00000001"),
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("a request");
        let right = queue
            .create(
                &db,
                "the right one",
                Some("ses_00000001"),
                &at("2026-09-08T12:00:00Z"),
            )
            .expect("a request");
        handoffs::upsert(&db, &handoff("hf_0123456789")).expect("the handoff");

        assert!(queue.relink(&db, "hf_0123456789", &wrong).expect("linked"));
        assert!(queue
            .relink(&db, "hf_0123456789", &right)
            .expect("relinked"));

        let open: Vec<String> = queue
            .open_for_session(&db, "ses_00000001")
            .expect("the queue")
            .into_iter()
            .map(|request| request.id)
            .collect();
        assert_eq!(open, [wrong], "the request it left is collectable again");
        assert_eq!(
            queue
                .get(&db, &right)
                .expect("a read")
                .expect("a row")
                .linked_handoff_id
                .as_deref(),
            Some("hf_0123456789")
        );
    }

    #[test]
    fn an_unassigned_request_goes_to_the_first_session_that_registers() {
        // OPEN-04a: the window opens with no session, and the queue waits.
        let db = db_with_two_sessions();
        let (queue, recorder) = queue();
        let waiting = queue
            .create(&db, "no session yet", None, &at("2026-09-08T11:00:00Z"))
            .expect("a request");

        assert_eq!(
            queue
                .deliver_to_first_session(&db, "ses_00000001")
                .expect("a delivery"),
            1
        );
        assert_eq!(announced(&recorder), std::slice::from_ref(&waiting));
        assert_eq!(
            queue
                .get(&db, &waiting)
                .expect("a read")
                .expect("a row")
                .session_ref
                .as_deref(),
            Some("ses_00000001")
        );

        // The second session that registers finds nothing left to take.
        assert_eq!(
            queue
                .deliver_to_first_session(&db, "ses_00000002")
                .expect("a delivery"),
            0
        );
        assert_eq!(announced(&recorder).len(), 1);
    }

    #[test]
    fn a_detached_session_gives_its_open_requests_back_but_not_its_history() {
        // FM-34, and the line either side of it: what was answered stays answered.
        let db = db_with_two_sessions();
        let (queue, _) = queue();
        let still_open = queue
            .create(
                &db,
                "waiting for a spec",
                Some("ses_00000001"),
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("a request");
        let answered = queue
            .create(
                &db,
                "already answered",
                Some("ses_00000001"),
                &at("2026-09-08T11:30:00Z"),
            )
            .expect("a request");
        handoffs::upsert(&db, &handoff("hf_0123456789")).expect("the handoff");
        queue
            .relink(&db, "hf_0123456789", &answered)
            .expect("linked");

        assert_eq!(
            queue
                .requeue_on_detach(&db, "ses_00000001")
                .expect("a requeue"),
            1
        );
        assert_eq!(
            queue
                .get(&db, &still_open)
                .expect("a read")
                .expect("a row")
                .session_ref,
            None
        );
        assert_eq!(
            queue
                .get(&db, &answered)
                .expect("a read")
                .expect("a row")
                .session_ref
                .as_deref(),
            Some("ses_00000001"),
            "a request that was answered keeps the record of where it went"
        );
    }

    #[test]
    fn a_restart_gives_every_open_request_back_to_the_unassigned_queue() {
        let db = db_with_two_sessions();
        let (queue, _) = queue();
        queue
            .create(
                &db,
                "one",
                Some("ses_00000001"),
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("a request");
        queue
            .create(
                &db,
                "two",
                Some("ses_00000002"),
                &at("2026-09-08T11:30:00Z"),
            )
            .expect("a request");

        assert_eq!(queue.requeue_on_start(&db).expect("a requeue"), 2);
        assert_eq!(
            queue
                .open_for_session(&db, "ses_00000001")
                .expect("the queue")
                .len(),
            2,
            "both are unassigned, so both are offered to whoever comes back"
        );
    }

    #[test]
    fn a_resume_request_names_its_handoff_is_copied_and_is_closed_when_a_call_attaches() {
        // FM-31 and RESP-07: queued, copied, and answered by the agent coming back.
        let db = db_with_two_sessions();
        let (queue, recorder) = queue();
        handoffs::upsert(&db, &handoff("hf_0123456789")).expect("the handoff");
        let id = queue
            .queue_resume_request(
                &db,
                "hf_0123456789",
                Some("ses_00000001"),
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("a resume request");

        assert_eq!(announced(&recorder), std::slice::from_ref(&id));
        let row = queue.get(&db, &id).expect("a read").expect("a row");
        assert_eq!(row.about_handoff_id.as_deref(), Some("hf_0123456789"));
        assert_eq!(row.text, "Resume handoff hf_0123456789");

        // A resume is never adopted or linked by a new handoff: it is about one that exists.
        assert_eq!(
            queue
                .link_on_open(&db, Some("ses_00000001"), None)
                .expect("a link"),
            OpenLink::None
        );

        assert_eq!(queue.resumed(&db, "hf_0123456789").expect("closed"), 1);
        assert!(queue
            .open_for_session(&db, "ses_00000001")
            .expect("the queue")
            .is_empty());
        // And a second attach has nothing left to close.
        assert_eq!(queue.resumed(&db, "hf_0123456789").expect("closed"), 0);
    }

    #[test]
    fn the_queue_of_a_session_is_its_own_and_the_unassigned_oldest_first() {
        let db = db_with_two_sessions();
        let (queue, _) = queue();
        let unassigned = queue
            .create(&db, "for anyone", None, &at("2026-09-08T10:00:00Z"))
            .expect("a request");
        let mine = queue
            .create(
                &db,
                "mine",
                Some("ses_00000001"),
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("a request");
        queue
            .create(
                &db,
                "theirs",
                Some("ses_00000002"),
                &at("2026-09-08T09:00:00Z"),
            )
            .expect("a request");

        let open: Vec<String> = queue
            .open_for_session(&db, "ses_00000001")
            .expect("the queue")
            .into_iter()
            .map(|request| request.id)
            .collect();
        assert_eq!(open, [unassigned, mine]);
    }

    #[test]
    fn the_clipboard_path_is_recorded_on_the_request_it_carried() {
        let db = db_with_two_sessions();
        let (queue, _) = queue();
        let id = queue
            .create(
                &db,
                "copied",
                Some("ses_00000001"),
                &at("2026-09-08T11:00:00Z"),
            )
            .expect("a request");
        assert!(queue.delivered_by_clipboard(&db, &id).expect("marked"));
        assert_eq!(
            queue
                .get(&db, &id)
                .expect("a read")
                .expect("a row")
                .delivered_via,
            Some(DeliveredVia::Clipboard)
        );
    }
}
