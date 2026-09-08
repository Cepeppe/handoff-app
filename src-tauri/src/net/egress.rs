//! The single network egress point (§7.13, NET-01, NET-02, UPD-01, UPD-02, DD-32).
//!
//! `get(url)` will be the only function in the whole application that opens a network
//! connection, and it records `{ at, domain, bytes_sent, purpose }` in `network_events`
//! before sending, so the Network settings page can list every connection the app has ever
//! made (NET-01).
//!
//! Until then the application makes **zero** network connections: the update check is
//! deferred together with the public release (`TASKS.md` §0.4 item 8, T-078), so this
//! module has no caller and no HTTP client is a dependency of the crate at all. The
//! enforcement exists from the first commit rather than from the first connection:
//!
//! - `clippy.toml` disallows `reqwest`, `hyper` and the raw TCP types everywhere, and
//!   `cargo clippy -D warnings` is a CI gate;
//! - `deny.toml` refuses `reqwest` and `hyper` as dependencies at all, so they cannot even
//!   enter the tree;
//! - the frontend CSP is `default-src 'self'` with no `connect-src`, so the webview cannot
//!   reach the network either.
//!
//! When the update check returns, both rules are narrowed to allow the client **in this
//! file only**, and nowhere else.
// TASK: T-051 (the module, the lint exception and the zero-egress test), T-078 (the caller)
