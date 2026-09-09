//! The seam through which the window learns that a tab changed (§7.6).
//!
//! The store is the single owner of handoff state and it never touches Tauri (`lib.rs`), so
//! the `handoff_changed(id)` event of §7.6 leaves through this trait: `ui_bridge` implements
//! it over `AppHandle::emit`, a test implements it over a counter.
//!
//! It carries the id and nothing else, for the same reason
//! [`crate::sessions::SessionsObserver`] carries no payload at all: the view re-reads the
//! store, so there is one source of truth and it is not the event. A payload would also have
//! to be a projection, and a projection that travels is a projection that can be stale by
//! the time it is drawn.
//!
//! It is called from [`super::actor::Store::commit`] — the one place a transition is put
//! back after the write — so a refused write notifies nobody (FM-28), and every path that
//! changes a handoff notifies exactly once, including the ones no request is waiting for
//! (a timer, a disconnect).

/// Told that the handoff named by `handoff_id` is no longer what the window last drew.
pub trait HandoffsObserver: Send {
    /// A transition of `handoff_id` has been persisted.
    fn handoff_changed(&self, handoff_id: &str);
}

/// The observer of a store nobody is watching: every test that is not about the event, and
/// the startup window before the window exists.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoWatchers;

impl HandoffsObserver for NoWatchers {
    fn handoff_changed(&self, _handoff_id: &str) {}
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Arc, Mutex};

    use super::HandoffsObserver;

    /// An observer that remembers, in order, which handoffs it was told about.
    #[derive(Debug, Default)]
    pub(crate) struct RecordingWatcher {
        pub(crate) changed: Mutex<Vec<String>>,
    }

    impl HandoffsObserver for Arc<RecordingWatcher> {
        fn handoff_changed(&self, handoff_id: &str) {
            self.changed
                .lock()
                .expect("the recording watcher is not poisoned")
                .push(handoff_id.to_owned());
        }
    }
}
