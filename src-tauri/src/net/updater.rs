//! The update check of the startup sequence (§7.2, §7.13, UPD-01, UPD-02).
//!
//! §7.2 calls this once, after the agent scan and before the Windows cleanup, and §7.13
//! gives it exactly one way out of the process: [`super::egress`], the single point that
//! may open a connection.
//!
//! **In this build it opens nothing and asks nothing.** The public release is deferred
//! together with the update check (`TASKS.md` §0.4 item 8): UPD-01 and UPD-02 are
//! suspended, the domain of OI-12 is not decided, and the app makes zero network
//! connections — which is what the firewall test of NET-02 asserts and what `deny.toml`
//! and `clippy.toml` enforce from the dependency side.
//!
//! The call site exists all the same, in the order the sequence puts it, so that turning
//! the check on is one function body and not a change to `run()`.
// TASK: T-051 (the egress module, its lint exception and the zero-egress test),
// T-078 (the hosted endpoint and the caller that reaches it)

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

    #[test]
    fn this_build_checks_nothing() {
        // NET-02 and §0.4 item 8: zero network connections until T-078 gives this a caller.
        // The test is the record of that promise at the call site, beside the ones
        // `deny.toml` and `clippy.toml` make about the dependency graph.
        assert_eq!(check_if_enabled(), UpdateCheck::Disabled);
    }
}
