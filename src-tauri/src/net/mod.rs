//! The network boundary (§7.13, NET-01, NET-02, PRIN-05, DD-32).
//!
//! Everything that could open a connection lives under this module, and [`egress`] is the
//! only place in the codebase allowed to do it — enforced by `clippy.toml`, by `deny.toml`,
//! by `tests/egress_boundary.rs` and by the webview's CSP, which `egress` documents.
//!
//! [`updater`] is the one caller the design foresees, and in this build it asks for nothing:
//! the update check is deferred with the public release (`TASKS.md` §0.4 item 8, T-078), so
//! the app makes **zero** network connections and the `net` feature that carries the HTTP
//! client is off by default.
pub mod egress;
pub mod updater;
