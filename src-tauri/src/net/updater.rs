//! The update check of the startup sequence (§7.2, §7.13, UPD-01, UPD-02).
//!
//! §7.2 calls this once, after the agent scan and before the Windows cleanup, and §7.13
//! gives it exactly one way out of the process: [`super::egress`], the single point that
//! may open a connection.
//!
//! **In this build it opens nothing and asks nothing.** The public release is deferred
//! together with the update check (`TASKS.md` §0.4 item 8): UPD-01 and UPD-02 are
//! suspended, the domain of OI-12 is not decided, and the app makes zero network
//! connections — which is what the firewall test of NET-02 asserts, what `deny.toml` and
//! `clippy.toml` enforce from the dependency side, and what `tests/egress_boundary.rs`
//! asserts from the source side: **no module of this crate calls `egress::get`**, and this
//! is the module that will be the first to.
//!
//! The call site exists all the same, in the order the sequence puts it, so that turning
//! the check on is one function body and not a change to `run()`.
// TASK: T-078 — the hosted endpoint, the `enabled` setting, the 24 h cache and the caller
// that reaches `egress::get(db, url, egress::PURPOSE_UPDATE_CHECK)`. Turning the client on
// is `net` in the `default` feature list of `Cargo.toml`.

/// What the startup sequence did about updates.
///
/// Reported rather than returned as `()` so that the log line in `run()` says which of the
/// two silences this was, and so that the day UPD-01 returns the caller has somewhere to
/// put "there is a newer version" without changing the sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateCheck {
    /// No request was made. The only answer this build gives.
    Disabled,
}

impl std::fmt::Display for UpdateCheck {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => out.write_str("disabled"),
        }
    }
}

/// The §7.2 call site: checks for a new version when the build has one to check against.
///
/// It never blocks the startup sequence and it never fails: a check that cannot be made is
/// not a reason to keep the user out of their overlay (PRIN-10).
#[must_use]
pub fn check_if_enabled() -> UpdateCheck {
    UpdateCheck::Disabled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::network_events;
    use crate::log::Db;

    #[test]
    fn this_build_checks_nothing_and_leaves_the_network_page_empty() {
        // NET-02 and §0.4 item 8: zero network connections until T-078 gives this a caller.
        // The two halves are the promise: the sequence's answer, and the table the Network
        // page reads — which the single egress point of §7.13 is the only writer of, so a
        // launch that ran this line has nothing to show.
        let db = Db::open_in_memory().expect("a database");
        assert_eq!(check_if_enabled(), UpdateCheck::Disabled);
        assert!(network_events::list(&db).expect("the events").is_empty());
    }

    #[test]
    fn the_purpose_this_call_site_will_record_is_the_one_the_page_can_name() {
        // §7.13 and F-14 give the row a stable `purpose`, and the Network page turns exactly
        // this key into a sentence. Naming it here keeps the two ends of the only connection
        // the design foresees spelled once.
        assert_eq!(super::super::egress::PURPOSE_UPDATE_CHECK, "update-check");
    }
}
