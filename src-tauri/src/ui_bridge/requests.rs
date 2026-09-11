//! Putting a queued request in front of an agent (§7.7, OPEN-05, OPEN-04a, FM-21, FM-31).
//!
//! The queue decides *what* is waiting and for whom; this is the side that acts on it, and
//! it is here because all three actions are Tauri's: the clipboard, the notification and the
//! window of another process. `lib.rs` says why the core may not reach for any of them.
//!
//! The sequence of §7.7 is the whole module:
//!
//! 1. render the entry in the user's language (`requests::text` picks the OPEN-05 sentence
//!    or the FM-31 resume sentence from the entry itself);
//! 2. put it on the clipboard, which is the fast path;
//! 3. bring that session's terminal to the front, best effort (`requests::focus`);
//! 4. when the window could not be found, notify — "paste it into session …" (FM-21).
//!
//! None of it is load-bearing. The Stop hook delivers the same queue at the end of the
//! agent's next turn (OPEN-06), so a clipboard a virus scanner is holding, a terminal that
//! is not a window, or a request that arrives before the window exists all cost a detour and
//! never the request.

use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt as _;
use tauri_plugin_notification::NotificationExt as _;

use crate::format::channel::AncestorProcess;
use crate::i18n::{text, Language};
use crate::log::Db;
use crate::requests::focus::TerminalFocus;
use crate::requests::queue::{Queue, RequestObserver, RequestReadyForSession, UserRequest};
use crate::requests::text::render_for;
use crate::sessions::Registry;

/// The notification of OPEN-05, when the terminal could not be brought forward.
pub const NOTIFY_PASTE_KEY: &str = "request.notifyPaste";

/// The notification of the case OPEN-05 does not print: the clipboard itself refused.
///
/// Saying "copied" then would be a sentence the user can act on and that is not true; what
/// is true is that the request is queued and the agent will be told at the end of its turn
/// (OPEN-06), and that the window can copy it again ("Copy request again", §7.6).
pub const NOTIFY_QUEUED_KEY: &str = "request.notifyQueued";

/// The delivery side of the queue: clipboard, terminal focus, notification.
///
/// Cloned into the queue as its [`RequestObserver`] and kept by `ui_bridge` for the request
/// sheet, which delivers what the user just typed without going through the observer —
/// `Queue::create` deliberately announces nothing, because the person is looking at the
/// window that called it.
#[derive(Clone)]
pub struct RequestDelivery {
    inner: Arc<Inner>,
}

struct Inner {
    /// The application, once `setup()` has run. Shared with [`super::Notifier`], so filling
    /// it there fills it here.
    app: Arc<OnceLock<AppHandle>>,
    /// Who is connected, and what their process chain is (§7.5).
    registry: Arc<Mutex<Registry>>,
    /// The queue, to record that the clipboard carried an entry.
    ///
    /// Weak, and filled after construction, because the queue owns this object as its
    /// observer: an `Arc` here would be a cycle that never drops. `lib.rs` builds the two in
    /// that order and calls [`RequestDelivery::attach_queue`] between them.
    queue: OnceLock<Weak<Queue>>,
    /// How a terminal window is raised, or [`crate::requests::NoTerminalFocus`] where there
    /// is none.
    focus: Box<dyn TerminalFocus>,
}

impl std::fmt::Debug for RequestDelivery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RequestDelivery")
            .finish_non_exhaustive()
    }
}

impl RequestDelivery {
    /// The delivery of a running app: `app` is the handle [`super::Notifier`] shares.
    #[must_use]
    pub fn new(
        app: Arc<OnceLock<AppHandle>>,
        registry: Arc<Mutex<Registry>>,
        focus: Box<dyn TerminalFocus>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                app,
                registry,
                queue: OnceLock::new(),
                focus,
            }),
        }
    }

    /// Gives it the queue it reports deliveries to. Called once, from `lib.rs`.
    pub fn attach_queue(&self, queue: &Arc<Queue>) {
        if self.inner.queue.set(Arc::downgrade(queue)).is_err() {
            tracing::warn!("the request delivery was given a queue twice");
        }
    }

    /// Copies `request`, raises the terminal of `session_ref` when there is one, and notifies
    /// when the raise did not happen (§7.7).
    ///
    /// `db` is the caller's connection, as everywhere in this queue: the request sheet writes
    /// on the window's, the channel dispatch on the registry's. Nothing here opens a
    /// transaction, so joining the caller's is exactly what is wanted.
    pub fn deliver(&self, db: &Db, request: &UserRequest, session_ref: Option<&str>) {
        let Some(app) = self.inner.app.get() else {
            // The listener registers sessions before `setup()` runs (§7.2), so a request can
            // be handed to a session while there is still no window to copy from. OPEN-06 is
            // what covers it, and the tab is already there to copy it again.
            tracing::debug!("a request was ready before the window existed");
            return;
        };
        let language = super::language_of(app);
        let sentence = render_for(language, request);

        let copied = match app.clipboard().write_text(sentence) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(error = %error, "the request could not be put on the clipboard");
                false
            }
        };
        if copied {
            self.mark_delivered(db, &request.id);
        }

        let session = session_ref.and_then(|session_ref| self.session(session_ref));
        let raised = session.as_ref().is_some_and(|session| {
            self.inner
                .focus
                .focus(&session.chain, session.folder.as_deref())
        });

        if let (Some(key), Some(session)) = (what_to_say(copied, raised), session) {
            self.notify(app, language, key, &session.label);
        }
    }

    /// The clipboard carried it, so the queue may say so (OPEN-05).
    fn mark_delivered(&self, db: &Db, id: &str) {
        let Some(queue) = self.inner.queue.get().and_then(Weak::upgrade) else {
            tracing::debug!("the request delivery has no queue to record a copy in");
            return;
        };
        if let Err(error) = queue.delivered_by_clipboard(db, id) {
            tracing::warn!(error = %error, request_id = id, "a clipboard delivery was not recorded");
        }
    }

    /// What the delivery needs to know about a session: where to point the focus — its chain,
    /// and the name of its folder, which picks the window among an editor's (T-070) — and what
    /// to call it in the notification (OPEN-02).
    fn session(&self, session_ref: &str) -> Option<SessionTarget> {
        let registry = self
            .inner
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let session = registry.get(session_ref)?;
        Some(SessionTarget {
            chain: session.pid_chain.clone(),
            folder: session
                .project_folder()
                .map(|folder| crate::sessions::registry::base_name(folder).to_owned()),
            label: session.display_name(),
        })
    }

    /// One system notification, named after the session it is about.
    fn notify(&self, app: &AppHandle, language: Language, key: &str, session: &str) {
        let body = text(language, key).replace("{session}", session);
        if let Err(error) = app
            .notification()
            .builder()
            .title(text(language, "app.name"))
            .body(body)
            .show()
        {
            // Notifications need an installed application on Windows; a development run
            // simply has none, and the request is queued either way.
            tracing::debug!(error = %error, "the request notification was not shown");
        }
    }
}

/// What a notification and a focus attempt need from the registry.
struct SessionTarget {
    chain: Vec<AncestorProcess>,
    /// The name of the session's folder, which picks its window among an editor's (T-070).
    folder: Option<String>,
    label: String,
}

/// Which notification a delivery ends in, if any (OPEN-05, FM-21).
///
/// Nothing is said when the terminal is in front of the user with the sentence already on
/// the clipboard: they are looking at the place they are about to paste into, and a
/// notification over it would be noise. Nothing is said either when there is no session to
/// name — OPEN-04a's "no active session" is on the screen they just typed into.
fn what_to_say(copied: bool, raised: bool) -> Option<&'static str> {
    match (copied, raised) {
        (true, true) => None,
        (true, false) => Some(NOTIFY_PASTE_KEY),
        (false, _) => Some(NOTIFY_QUEUED_KEY),
    }
}

impl RequestObserver for RequestDelivery {
    fn request_ready(&self, db: &Db, ready: &RequestReadyForSession) {
        self.deliver(db, &ready.request, Some(&ready.session_ref));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_notification_sentences_exist_in_both_languages() {
        for key in [NOTIFY_PASTE_KEY, NOTIFY_QUEUED_KEY] {
            for language in [Language::En, Language::It] {
                let sentence = text(language, key);
                assert_ne!(sentence, key, "{language} has no text for {key}");
                assert!(
                    sentence.contains("{session}"),
                    "{language}.{key} does not name the session"
                );
            }
        }
    }

    #[test]
    fn a_terminal_that_came_forward_with_the_text_copied_is_told_nothing() {
        // The user is looking at the window they are about to paste into.
        assert_eq!(what_to_say(true, true), None);
    }

    #[test]
    fn a_terminal_that_could_not_be_found_is_answered_by_the_notification_of_open_05() {
        assert_eq!(what_to_say(true, false), Some(NOTIFY_PASTE_KEY));
    }

    #[test]
    fn a_clipboard_that_refused_never_claims_the_text_was_copied() {
        // FM-21 prints one sentence for both halves of the row; saying "copied" when it was
        // not is a sentence the user would act on and that is not true. The request is
        // queued either way, and the Stop hook delivers it (OPEN-06).
        assert_eq!(what_to_say(false, true), Some(NOTIFY_QUEUED_KEY));
        assert_eq!(what_to_say(false, false), Some(NOTIFY_QUEUED_KEY));
    }

    #[test]
    fn the_paste_sentence_is_the_one_of_77() {
        // §7.7 prints it, and §1.3 makes a text of the design normative.
        assert_eq!(
            text(Language::En, NOTIFY_PASTE_KEY).replace("{session}", "Claude Code · baton"),
            "Request copied: paste it into session Claude Code · baton"
        );
    }
}
