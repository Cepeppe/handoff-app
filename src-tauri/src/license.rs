//! The licence entry point (§7.14, LIC-01, LIC-02).
//!
//! v1 has no licence verification: the app starts and works in full (LIC-01). What LIC-02
//! asks for is that a later local check should touch one module and nothing else, so this
//! one owns the answer and every feature gate reads it.
//!
//! Two properties this module keeps, and that a future check must keep as well:
//!
//! - it makes **no network call**. The single egress point of the application is
//!   `net::egress` (§7.13, PRIN-05); a licence check that opened a connection would break
//!   the promise the Network page and the firewall test of NET-02 are built on.
//! - it is called **once**, from `run()`, and the value is passed down. A gate that called
//!   back into this module would turn a future check into an unpredictable number of
//!   checks.

use std::fmt;

/// What the running installation is entitled to. One variant in v1, by LIC-01.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Entitlement {
    /// Everything is available.
    Full,
}

impl fmt::Display for Entitlement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Entitlement::Full => f.write_str("full"),
        }
    }
}

/// The one call site is `run()` (§7.2).
pub fn check() -> Entitlement {
    Entitlement::Full
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_is_entitled_to_everything() {
        assert_eq!(check(), Entitlement::Full);
        assert_eq!(check().to_string(), "full");
    }
}
