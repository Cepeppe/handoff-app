//! Build script: the vendored format material, then Tauri.
//!
//! The crate embeds the schemas, the channel protocol and the certain-secret patterns of
//! the pinned `handoff-mcp` release with `include_str!` (§3.4, §3.5), so `vendor/` has to
//! be filled before anything compiles — not only before a bundle is built. Without this
//! check the failure is a wall of `couldn't read ../vendor/…` from the middle of the crate;
//! with it, it is one line naming the command that fixes it.
//!
//! What this does **not** do is re-run `node scripts/fetch-server.mjs --check`. That script
//! is the release gate of §3.5 and it verifies more than presence — the version against
//! `server.lock.json`, the declared format versions, the platform binary and its copy under
//! `src-tauri/binaries/`. It also refuses a platform the lock pins no binary for, which is
//! every macOS host while macOS is deferred (`TASKS.md` §0.4 item 7), so calling it from
//! here would make `cargo test` impossible on the macOS CI leg. It runs where it can do its
//! whole job instead: `beforeDevCommand` and `beforeBuildCommand` in `tauri.conf.json`, and
//! its own step in `ci.yml`.

use std::path::Path;

/// Every vendored file the crate embeds. Kept in step with the `include_str!` calls of
/// `src/format/schema.rs` and `src/format/patterns.rs`; a test asserts the two lists agree.
const EMBEDDED: &[&str] = &[
    "schemas/handoff-spec.v1.schema.json",
    "schemas/handoff-outcome.v1.schema.json",
    "schemas/handoff-runbook.v1.schema.json",
    "protocol/channel/channel.v1.schema.json",
    "patterns/certain-secrets.v1.json",
];

/// `handoff-app/vendor/handoff-mcp/format/`, relative to this manifest.
const FORMAT_DIR: &str = "../vendor/handoff-mcp/format";

fn main() {
    let format = Path::new(FORMAT_DIR);
    for name in EMBEDDED {
        let path = format.join(name);
        if !path.exists() {
            panic!(
                "{} is missing.\n\
                 The app embeds the format of the pinned handoff-mcp release; fill vendor/ \
                 with `node scripts/fetch-server.mjs` (or `node scripts/fetch-server.mjs \
                 --format-only` where the lock pins no binary for this platform), or link a \
                 local build with the workspace root's `scripts/dev-link`.",
                path.display()
            );
        }
        println!("cargo:rerun-if-changed={}", path.display());
    }

    tauri_build::build()
}
