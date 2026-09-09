//! The three events the core sends the window (§7.6).
//!
//! The core never names Tauri (`lib.rs`), so it talks to the window through the observer
//! traits declared beside the state that changes: [`crate::store::HandoffsObserver`] and
//! [`crate::sessions::SessionsObserver`]. [`Notifier`] is the one implementation of both,
//! and it is also what the commands use to push a [`Notice`].
//!
//! # Why the handle arrives late
//!
//! The channel listener, the registry and the store are all started **before**
//! `tauri::Builder::run` (§7.2: a server that connects while the UI is starting is
//! registered all the same), so the observers have to exist before there is an `AppHandle`
//! to emit through. The notifier is therefore created empty and filled in `setup()`; an
//! event fired in that window is dropped, which is correct — there is no window yet, and
//! the view re-reads the core when it mounts.
//!
//! # Why two of the three carry no payload worth the name
//!
//! `handoff_changed` carries an id and `sessions_changed` carries nothing, because the view
//! re-reads the core: one source of truth, and a projection that travels is a projection
//! that can be stale by the time it is drawn. `notice` is the exception, because it *is* the
//! payload — a sentence with nothing behind it to re-read.

use std::sync::OnceLock;

use serde::Serialize;
use tauri::{AppHandle, Emitter as _};

use crate::i18n::{text, Language};
use crate::sessions::SessionsObserver;
use crate::store::HandoffsObserver;

/// A tab changed and the window should re-read it. Payload: the `hf_` id.
pub const EVENT_HANDOFF_CHANGED: &str = "handoff_changed";

/// The set of sessions, or something shown about one of them, changed. No payload.
pub const EVENT_SESSIONS_CHANGED: &str = "sessions_changed";

/// One sentence for the user. Payload: [`Notice`].
pub const EVENT_NOTICE: &str = "notice";

/// How loudly a notice is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeKind {
    /// Something happened that the user asked for.
    Info,
    /// Something did not happen, and the user can do something about it.
    Warning,
    /// Something failed.
    Error,
}

/// A sentence the window shows and then forgets.
///
/// Unlike everything else that crosses this boundary it carries a **rendered** text, not a
/// catalogue key. A notice is pushed by the Rust side with nothing behind it for the window
/// to look up, exactly like the tray menu, so it is rendered here in the language the
/// frontend reported (`set_ui_language`) and from the same two catalogue files — the texts
/// still live only in `src/locales/{en,it}.json` (T-028).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// How loudly.
    pub kind: NoticeKind,
    /// What to say, already in the user's language.
    pub text: String,
}

/// The one bridge from a core observer to `AppHandle::emit`.
///
/// Cloned into the registry, into the store and into the Tauri state; the handle inside is
/// shared, so filling it in `setup()` fills it for all of them.
#[derive(Debug, Default, Clone)]
pub struct Notifier {
    app: std::sync::Arc<OnceLock<AppHandle>>,
}

impl Notifier {
    /// A notifier with no window behind it yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Gives it the application to emit through. Called once, from `setup()`.
    pub fn attach(&self, app: AppHandle) {
        if self.app.set(app).is_err() {
            tracing::warn!("the notifier was attached twice");
        }
    }

    /// Emits `event` with `payload`, or drops it when there is no window yet.
    fn emit<P: Serialize + Clone>(&self, event: &str, payload: P) {
        let Some(app) = self.app.get() else {
            tracing::debug!(event, "an event fired before the window existed");
            return;
        };
        if let Err(error) = app.emit(event, payload) {
            tracing::warn!(error = %error, event, "an event reached no window");
        }
    }

    /// Says something to the user, in the language the window is showing.
    pub fn notice(&self, kind: NoticeKind, key: &str) {
        let language = self
            .app
            .get()
            .map_or(Language::default(), super::language_of);
        self.emit(
            EVENT_NOTICE,
            Notice {
                kind,
                text: text(language, key).to_owned(),
            },
        );
    }
}

impl HandoffsObserver for Notifier {
    fn handoff_changed(&self, handoff_id: &str) {
        self.emit(EVENT_HANDOFF_CHANGED, handoff_id.to_owned());
    }
}

impl SessionsObserver for Notifier {
    fn sessions_changed(&self) {
        self.emit(EVENT_SESSIONS_CHANGED, ());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frontend half of the bridge, read as text so the two sides can be compared.
    const BRIDGE_TS: &str = include_str!("../../../src/bridge.ts");

    #[test]
    fn the_frontend_listens_to_every_event_this_module_emits() {
        // The two sides spell each name independently; a rename on one alone would leave a
        // tab that never repaints, silently.
        for event in [EVENT_HANDOFF_CHANGED, EVENT_SESSIONS_CHANGED, EVENT_NOTICE] {
            assert!(
                BRIDGE_TS.contains(&format!("'{event}'")),
                "src/bridge.ts does not mention {event}"
            );
        }
    }

    #[test]
    fn an_event_fired_before_the_window_exists_is_dropped_and_not_a_panic() {
        // The listener and the store start before `tauri::Builder::run` (§7.2), so this is
        // the ordinary state of the first moments of a launch.
        let notifier = Notifier::new();
        notifier.handoff_changed("hf_0000000001");
        notifier.sessions_changed();
        notifier.notice(NoticeKind::Warning, "notice.actionRefused");
    }

    #[test]
    fn every_notice_key_has_a_text_in_both_languages() {
        for key in super::super::NOTICE_KEYS {
            for language in [Language::En, Language::It] {
                let value = text(language, key);
                assert_ne!(value, key, "{language} has no text for {key}");
                assert!(!value.trim().is_empty(), "{language}.{key} is blank");
            }
        }
    }
}
