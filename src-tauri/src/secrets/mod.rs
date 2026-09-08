//! Reserved extension point: writing secrets into `.env`-style files (§12.2, SEC-03).
//!
//! Closed for v1 and compiled only under `--features secrets-write`, which no v1 build
//! enables. The design reserves the point rather than opening it: in v1 the app writes
//! files only in its own data directory, in `~/.handoff/`, as backups of agent
//! configuration during install and uninstall, and to a path the user picks in an export
//! dialog. No module can write into a project file, and the trust story (Network page,
//! preview, log) has to exist before that changes. v1 offers **Open file** instead
//! (SEC-02).
pub mod writer;
