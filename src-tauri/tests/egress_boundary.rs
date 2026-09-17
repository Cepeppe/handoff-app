//! The network boundary, read from the sources (§7.13, DD-32, NET-01, NET-02, PRIN-05).
//!
//! "The app makes no network call except the update check, through one module" is the
//! promise NFR-05 asks a user to be able to *check*, and three of the four things that hold
//! it are configuration: `clippy.toml` disallows the types, `deny.toml` says who may bring a
//! client into the tree, and the webview's CSP has no `connect-src`. Configuration is exactly
//! what an `#[allow]`, a `--no-deny-warnings` or a forgotten CI step switches off in silence.
//!
//! This suite is the fourth thing, and it is the one that watches the other three:
//!
//! - no module but `src/net/egress.rs` names a client or a raw socket, whatever attributes it
//!   carries;
//! - the exemption is written once, in that file, and nowhere else;
//! - the types it scans for are the ones `clippy.toml` actually disallows, so the two lists
//!   cannot drift apart;
//! - `tauri.conf.json` keeps `default-src 'self'` and no `connect-src`;
//! - every webview — the configured window and the ones built in code — starts the WebView2
//!   runtime with the same arguments, and they switch the runtime's own background traffic
//!   off (T-052: the runtime is another program, so nothing above can see what it sends);
//! - **nothing calls `egress::get`**, which is implementation decision 8's "zero network connections" as a
//!   fact about the sources rather than a sentence in a document.
//!
//! # What is scanned
//!
//! The shipped part of `src/`: every line outside a `#[cfg(test)]` item, and no comment. A
//! test may name a client — the one in `egress.rs` does — and a module comment that explains
//! the boundary is documentation, not a connection. The scan is the same one
//! `tests/timers.rs` performs for `tokio::time`, for the same reason: what is worth catching
//! is a *new* line, which is a diff and not a state.

use std::fs;
use std::path::{Path, PathBuf};

use handoff_app_lib::ui_bridge::WEBVIEW2_BROWSER_ARGS;

/// The one file allowed to name a client or a socket.
const EGRESS: &str = "src/net/egress.rs";

/// The exemption, spelled as `egress.rs` spells it.
const EXEMPTION: &str = "allow(clippy::disallowed_types)";

/// What a module may not name.
///
/// The crate names catch a client however it is spelled (`reqwest::Client`,
/// `reqwest::blocking`, a `use` of either); the two socket types catch the layer under it,
/// whichever of `std` and `tokio` it is reached through. Every entry is checked against
/// `clippy.toml` below, so this list cannot quietly stop covering the rule.
const FORBIDDEN: &[&str] = &["reqwest", "hyper", "TcpStream", "TcpListener"];

/// The call that would make this build reach the network.
///
/// There is no caller: the update check is deferred (implementation decision 8), so this is
/// what "the app makes zero network connections" means in the sources. It is the strongest form of the firewall test of NET-02 that a suite can run.
// TASK: T-078 — delete this assertion when the update check gains its caller; the rest of
// this file outlives it, and so does the `network_events` row the caller will write.
const EGRESS_CALL: &str = "egress::get";

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `src/`, relative to the crate root, sorted.
fn sources() -> Vec<String> {
    let root = crate_root();
    let mut found = Vec::new();
    walk(&root.join("src"), &root, &mut found);
    found.sort();
    found
}

fn walk(dir: &Path, root: &Path, found: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, root, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let relative = path
                .strip_prefix(root)
                .expect("every source is under the crate root")
                .to_string_lossy()
                .replace('\\', "/");
            found.push(relative);
        }
    }
}

/// The lines of `text` that end up in the shipped application, comments dropped.
///
/// `#[cfg(test)]` items are skipped brace by brace rather than assuming the test module is
/// last, exactly as `tests/timers.rs` does; a line that starts with `//` is prose about the
/// boundary and opens nothing.
fn shipped_code(text: &str) -> Vec<&str> {
    let mut kept = Vec::new();
    let mut skipping = false;
    let mut depth = 0usize;
    let mut opened = false;
    for line in text.lines() {
        let code = line.trim_start();
        if !skipping && code.starts_with("#[cfg(test)]") {
            skipping = true;
            depth = 0;
            opened = false;
        }
        if !skipping {
            if !code.starts_with("//") {
                kept.push(line);
            }
            continue;
        }
        depth += code.matches('{').count();
        opened |= depth > 0;
        depth = depth.saturating_sub(code.matches('}').count());
        if (opened && depth == 0) || (!opened && code.ends_with(';')) {
            skipping = false;
        }
    }
    kept
}

/// `file → the lines that name something of `needles``.
fn mentions(needles: &[&str]) -> Vec<(String, String)> {
    let root = crate_root();
    let mut found = Vec::new();
    for file in sources() {
        let text = fs::read_to_string(root.join(&file)).expect("a source file reads");
        for line in shipped_code(&text) {
            if needles.iter().any(|needle| line.contains(needle)) {
                found.push((file.clone(), line.trim().to_owned()));
            }
        }
    }
    found
}

#[test]
fn no_module_but_the_egress_point_names_a_client_or_a_socket() {
    let outside: Vec<String> = mentions(FORBIDDEN)
        .into_iter()
        .filter(|(file, _)| file != EGRESS)
        .map(|(file, line)| format!("{file}: {line}"))
        .collect();
    assert!(
        outside.is_empty(),
        "only {EGRESS} may open a connection (§7.13, DD-32); found:\n  {}",
        outside.join("\n  ")
    );
}

#[test]
fn the_egress_point_is_the_one_that_names_them() {
    // The negative above passes on a day somebody deletes `egress.rs` too, and then it is
    // asserting nothing. This is the control: the boundary exists and is where it says.
    let inside = mentions(FORBIDDEN)
        .into_iter()
        .filter(|(file, _)| file == EGRESS)
        .count();
    assert!(
        inside > 0,
        "{EGRESS} names no client at all, so the scan above proves nothing"
    );
}

#[test]
fn the_exemption_is_written_in_one_file_and_it_is_that_one() {
    // A module-level `#[allow]` is how the clippy rule is switched off, so the exemption
    // itself is a thing to count. One, here, with the reason in the comment above it.
    let files: Vec<String> = mentions(&[EXEMPTION])
        .into_iter()
        .map(|(file, _)| file)
        .collect();
    assert_eq!(
        files,
        vec![EGRESS.to_owned()],
        "the disallowed-types exemption of §7.13 belongs to {EGRESS} alone"
    );
}

#[test]
fn the_scan_would_find_a_client_added_to_another_module() {
    // A grep for a string nobody writes passes for ever, including on the day the grep
    // breaks. Every case the scan has to get right is here: a plain use, a use behind a
    // `#[cfg]`, a test that is allowed to name one, and a comment that only talks about it.
    let planted = "use reqwest::blocking::Client;\n\
                   fn phone_home() { let _ = std::net::TcpStream::connect(\"a.test:80\"); }\n\
                   #[cfg(test)]\n\
                   mod tests {\n\
                       fn allowed() { let _ = reqwest::Client::new(); }\n\
                   }\n\
                   // reqwest is named in this comment and opens nothing\n";
    let code = shipped_code(planted).join("\n");
    assert!(code.contains("use reqwest::blocking::Client;"));
    assert!(code.contains("TcpStream::connect"));
    assert!(!code.contains("fn allowed"));
    assert!(!code.contains("this comment"));
    assert_eq!(
        FORBIDDEN
            .iter()
            .filter(|needle| code.contains(*needle))
            .count(),
        2,
        "the scan should see the client and the socket, and nothing else"
    );
}

#[test]
fn the_scanned_types_are_the_ones_clippy_disallows() {
    // Two lists in two files describing one rule. `clippy.toml` is what the compiler
    // enforces; `FORBIDDEN` is what this suite reads the sources for. A path added there and
    // forgotten here would leave a type nobody checks, and the reverse would leave this
    // suite guarding something the build does not.
    const CLIPPY_TOML: &str = include_str!("../clippy.toml");
    let paths: Vec<&str> = regex::Regex::new(r#"path = "([^"]+)""#)
        .expect("a valid pattern")
        .captures_iter(CLIPPY_TOML)
        .map(|found| found.get(1).expect("the path group").as_str())
        .collect();
    assert!(
        paths.len() >= 6,
        "clippy.toml names almost nothing, so it is the parse that is broken: {paths:?}"
    );
    for path in &paths {
        assert!(
            FORBIDDEN.iter().any(|needle| path.contains(needle)),
            "clippy.toml disallows `{path}`, which this suite does not look for"
        );
    }
    for needle in FORBIDDEN {
        assert!(
            paths.iter().any(|path| path.contains(needle)),
            "this suite looks for `{needle}`, which clippy.toml does not disallow"
        );
    }
}

#[test]
fn the_webview_cannot_reach_the_network_either() {
    // §7.13: "the frontend's CSP is `default-src 'self'` with no `connect-src`, so the
    // webview cannot reach the network either". `img-src 'self' blob:` is beside it since
    // T-046 and grants nothing that could leave the machine; anything that
    // widened `connect-src`, or `default-src` itself, would.
    const TAURI_CONF: &str = include_str!("../tauri.conf.json");
    let config: serde_json::Value =
        serde_json::from_str(TAURI_CONF).expect("tauri.conf.json is valid JSON");
    let csp = config["app"]["security"]["csp"]
        .as_str()
        .expect("the configuration declares a CSP");
    assert!(
        csp.contains("default-src 'self'"),
        "the CSP of §7.13 is `default-src 'self'`; it is `{csp}`"
    );
    assert!(
        !csp.contains("connect-src"),
        "the CSP names `connect-src`, so the webview may open a connection: `{csp}`"
    );
}

#[test]
fn every_configured_window_starts_the_runtime_with_its_background_traffic_off() {
    // The CSP above keeps the *page* off the network and says nothing about the WebView2
    // runtime under it, which is another program (`msedgewebview2.exe`) with requests of its
    // own: measured on 2026-09-10, an idle Baton's webview reached Microsoft addresses within
    // seconds. The switches are what the owner decided in T-052 (`docs/verify-trust.md`).
    const TAURI_CONF: &str = include_str!("../tauri.conf.json");
    let config: serde_json::Value =
        serde_json::from_str(TAURI_CONF).expect("tauri.conf.json is valid JSON");
    let windows = config["app"]["windows"]
        .as_array()
        .expect("the configuration declares windows");
    assert!(!windows.is_empty(), "the configuration declares no window");
    for window in windows {
        // One string for every webview of the profile: WebView2 refuses a second one whose
        // arguments differ, so a drift is a selection overlay that cannot open.
        assert_eq!(
            window["additionalBrowserArgs"].as_str(),
            Some(WEBVIEW2_BROWSER_ARGS),
            "window {} does not start the runtime with ui_bridge::WEBVIEW2_BROWSER_ARGS",
            window["label"]
        );
    }
    // wry *replaces* its own default with any value given, so the value has to carry it on.
    assert!(
        WEBVIEW2_BROWSER_ARGS
            .starts_with("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection "),
        "wry's default switches are lost: `{WEBVIEW2_BROWSER_ARGS}`"
    );
    let args: Vec<&str> = WEBVIEW2_BROWSER_ARGS.split_whitespace().collect();
    for switch in [
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-domain-reliability",
        "--no-pings",
    ] {
        assert!(args.contains(&switch), "{switch} is not among {args:?}");
    }
}

#[test]
fn every_webview_built_in_code_is_given_the_same_arguments() {
    // A window made by a builder rather than by the configuration gets wry's default unless
    // it is told otherwise, and WebView2 then refuses to create it. Count the builders and
    // the arguments file by file; the control is that at least one builder exists.
    let root = crate_root();
    let mut builders = 0;
    for file in sources() {
        let text = fs::read_to_string(root.join(&file)).expect("a source file reads");
        let code = shipped_code(&text).join("\n");
        let built = code.matches("WebviewWindowBuilder::new(").count();
        let given = code
            .matches(".additional_browser_args(WEBVIEW2_BROWSER_ARGS)")
            .count();
        assert_eq!(
            built, given,
            "{file} builds {built} webview(s) and gives {given} of them WEBVIEW2_BROWSER_ARGS"
        );
        builders += built;
    }
    assert!(
        builders > 0,
        "no webview builder was found, so the scan above proves nothing"
    );
}

#[test]
fn nothing_in_this_build_calls_the_egress_point() {
    // implementation decision 8: the app makes **zero** network connections until T-078 writes the update
    // check. The module and its lint rules ship; the caller does not.
    let callers: Vec<String> = mentions(&[EGRESS_CALL])
        .into_iter()
        .map(|(file, line)| format!("{file}: {line}"))
        .collect();
    assert!(
        callers.is_empty(),
        "this build promises zero network connections (implementation decision 8), and something calls \
         `{EGRESS_CALL}`:\n  {}",
        callers.join("\n  ")
    );
}
