//! The network boundary (§7.13, NET-01, NET-02, PRIN-05).
//!
//! Everything that could open a connection lives under this module, and `egress` is the
//! only place in the codebase allowed to do it. See `egress.rs`.
pub mod egress;
