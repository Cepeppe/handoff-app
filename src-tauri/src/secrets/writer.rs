//! The v2 writer of §12.2: deliberately empty.
//!
//! When it exists it will be limited to `.env`-style files, gated by a settings opt-in and
//! by the `secrets-write` feature, and it will show the user the path and the variable
//! name only — never the value.
