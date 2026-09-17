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
//! every macOS host while macOS is deferred (implementation decision 7), so calling it from
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

/// The Windows application manifest, embedded into every binary. See [`windows_manifest`].
const MANIFEST: &str = "windows-app-manifest.xml";

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
                 local build with `scripts/workspace/dev-link`.",
                path.display()
            );
        }
        println!("cargo:rerun-if-changed={}", path.display());
    }

    windows_manifest();

    // The manifest is embedded by the linker instead of by Tauri's resource, so Tauri is
    // asked not to produce one; everything else it puts in that resource — the icons, the
    // version information — is unchanged and still reaches the application binary alone.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
    )
    .expect("failed to run tauri-build");
}

/// Embeds `windows-app-manifest.xml` into **every** binary this crate produces.
///
/// `tauri-build` links its resource with `cargo:rustc-link-arg-bins`, which reaches the
/// application and not the test harnesses, and Cargo has no flag that reaches the unit-test
/// binary of a library (`rustc-link-arg-tests` covers `tests/` and not that one). Asking
/// the linker to embed the manifest itself covers all of them at once, with no duplicate
/// resource to reconcile.
///
/// It matters because the manifest is what declares the dependency on Common Controls
/// **version 6**: `muda`, `tray-icon` and `tauri-runtime-wry` call `TaskDialogIndirect`,
/// which `comctl32.dll` exports only in v6. Without it Windows loads v5.82 and the binary
/// dies before `main` with STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) — no output, no
/// backtrace, no failing test to read.
fn windows_manifest() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = Path::new(MANIFEST).canonicalize().unwrap_or_else(|error| {
        panic!("{MANIFEST} cannot be read: {error}");
    });
    println!("cargo:rerun-if-changed={MANIFEST}");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}
