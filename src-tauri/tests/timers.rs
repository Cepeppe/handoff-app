//! The timer registry: what the application is allowed to wake up for (WIN-06, NFR-14).
//!
//! "At rest the app is an icon and a listening socket" is a promise about a process that is
//! doing nothing, and the way it is broken is never deliberate: somebody adds a poll, a
//! refresh, a "check every few seconds", and nothing fails. A background wake-up costs a
//! laptop battery and costs nobody a test.
//!
//! So every timer is declared here, with the requirement it serves, and this suite fails on
//! any that is not. It reads the sources rather than the running program on purpose: a
//! runtime probe would have to reproduce the state each timer is armed in, and the thing
//! worth catching is a *new* one, which is a diff and not a state.
//!
//! # What is scanned
//!
//! The shipped part of `src/`: everything outside a `#[cfg(test)]` item. A test that waits
//! for a deadline is not the application waking a laptop up, and counting those would turn
//! the registry into a list of test conveniences that nobody reads.
//!
//! # What counts as a timer
//!
//! Every way of arming one from `tokio::time`, plus `thread::sleep`. Three of them are not
//! periodic and are declared anyway: a bound on one operation is not a wake-up at rest, but
//! it is a place a future edit could turn into one, and a registry with holes is not a
//! registry.
//!
//! # The rule the entries have to satisfy
//!
//! At rest — no channel connection, no handoff being verified — the process arms **nothing**.
//! Each entry below says what has to be true for its timer to exist, and the two that could
//! be armed with nothing happening (the store's actor loop, which runs from startup, and the
//! accept loop, which runs from startup) are the two that must be conditional. They are:
//! `until_verifying_deadline` waits for ever when no handoff is being verified (VER-06), and
//! the accept penalty is armed only after a `hello` was refused (§6.2).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// One declared timer: where it is, what arms it, and why the app is allowed to wake up.
struct Timer {
    /// The source file, relative to `src-tauri/`.
    file: &'static str,
    /// The call that arms it, spelled as the source spells it.
    call: &'static str,
    /// How many times that call appears in that file.
    count: usize,
    /// What has to be happening for it to exist at all.
    armed_when: &'static str,
}

/// Every timer this application is allowed to arm.
const REGISTRY: &[Timer] = &[
    Timer {
        file: "src/store/actor.rs",
        call: "tokio::time::sleep",
        count: 1,
        armed_when: "a handoff is awaiting verification: the 30-minute window of VER-06. \
                     `until_verifying_deadline` waits for ever when there is none, so an \
                     application at rest arms nothing here.",
    },
    Timer {
        file: "src/channel/listener.rs",
        call: "tokio::time::sleep",
        count: 1,
        armed_when: "a peer is connected: the silence budget after which the app pings it \
                     (§6.6). One per connection, reset by every line that arrives, and there \
                     are no connections at rest.",
    },
    Timer {
        file: "src/channel/listener.rs",
        call: "tokio::time::sleep_until",
        count: 1,
        armed_when: "a `hello` was refused and the next accept is being delayed (§6.2, \
                     FM-11). Nothing arms it on a run where every peer authenticates.",
    },
    Timer {
        file: "src/channel/listener.rs",
        call: "tokio::time::timeout",
        count: 1,
        armed_when: "a connection has been accepted and has two seconds to send its `hello` \
                     (§6.2). A bound on one read, not a wake-up.",
    },
    Timer {
        file: "src/e2e/server.rs",
        call: "tokio::time::sleep",
        count: 1,
        armed_when: "the automation channel of DD-33 refused a token and is holding the \
                     connection for a second before closing it — the same penalty the \
                     product listener imposes (§6.2). It exists only in an `--features e2e` \
                     build, which no release enables, and only after a wrong token; it is \
                     declared here all the same, because this registry is what a person \
                     reads when they ask what wakes the app up, and a site the suite is \
                     blind to is a site nobody re-reads.",
    },
];

/// Everything that arms a timer, in the spelling the sources use. `tokio::time` is the only
/// import path in this crate: `use tokio::time::sleep` would hide a site from this suite, so
/// the convention is the fully qualified call, which is also what the code does today.
const TIMER_CALLS: &[&str] = &[
    "tokio::time::sleep_until",
    "tokio::time::sleep",
    "tokio::time::interval_at",
    "tokio::time::interval",
    "tokio::time::timeout_at",
    "tokio::time::timeout",
    "thread::sleep",
];

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

/// The lines of `text` that end up in the shipped application: everything outside a
/// `#[cfg(test)]` item.
///
/// A test may sleep as much as it likes — a suite that waits for a deadline is not the
/// application waking a laptop up — so counting those would make the registry a list of
/// test conveniences and nobody would read it. The scan therefore skips each `#[cfg(test)]`
/// item, brace by brace, rather than assuming the test module is last.
fn shipped_lines(text: &str) -> Vec<&str> {
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
            kept.push(line);
            continue;
        }
        depth += code.matches('{').count();
        opened |= depth > 0;
        depth = depth.saturating_sub(code.matches('}').count());
        // The item is over at its closing brace, or at the semicolon of a braceless one
        // (`#[cfg(test)] use …;`).
        if (opened && depth == 0) || (!opened && code.ends_with(';')) {
            skipping = false;
        }
    }
    kept
}

/// How many times each timer call appears in the shipped part of each source file.
fn armed_timers() -> BTreeMap<(String, &'static str), usize> {
    let root = crate_root();
    let mut counts = BTreeMap::new();
    for file in sources() {
        let text = fs::read_to_string(root.join(&file)).expect("a source file reads");
        for line in shipped_lines(&text) {
            // The registry declares call sites, not the prose about them: a module comment
            // naming `tokio::time::sleep` is documentation and arms nothing.
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            // Longest first, so `sleep_until` is not counted as a `sleep`.
            if let Some(call) = TIMER_CALLS.iter().find(|call| code.contains(*call)) {
                *counts.entry((file.clone(), *call)).or_insert(0) += 1;
            }
        }
    }
    counts
}

#[test]
fn every_timer_in_the_sources_is_declared_in_the_registry() {
    let armed = armed_timers();
    let declared: BTreeMap<(String, &str), usize> = REGISTRY
        .iter()
        .map(|timer| ((timer.file.to_owned(), timer.call), timer.count))
        .collect();

    let undeclared: Vec<String> = armed
        .iter()
        .filter(|(key, _)| !declared.contains_key(*key))
        .map(|((file, call), count)| format!("{file}: {count} × {call}"))
        .collect();
    assert!(
        undeclared.is_empty(),
        "a timer nobody declared. WIN-06 and NFR-14 say the app is an icon and a listening \
         socket at rest: add it to REGISTRY in tests/timers.rs with what arms it, or do not \
         arm it.\n{}",
        undeclared.join("\n")
    );

    let miscounted: Vec<String> = armed
        .iter()
        .filter_map(|(key, count)| {
            let expected = declared.get(key)?;
            (expected != count).then(|| {
                format!(
                    "{}: {expected} declared, {count} in the source, call {}",
                    key.0, key.1
                )
            })
        })
        .collect();
    assert!(
        miscounted.is_empty(),
        "a declared timer gained or lost a call site:\n{}",
        miscounted.join("\n")
    );
}

#[test]
fn the_registry_declares_nothing_that_is_no_longer_there() {
    // The other direction: a timer that was removed leaves an entry behind, and the next
    // reader trusts it. A registry is only worth reading if it is exactly the truth.
    let armed = armed_timers();
    let stale: Vec<String> = REGISTRY
        .iter()
        .filter(|timer| !armed.contains_key(&(timer.file.to_owned(), timer.call)))
        .map(|timer| format!("{}: {}", timer.file, timer.call))
        .collect();
    assert!(
        stale.is_empty(),
        "REGISTRY declares a timer that is not in the sources any more:\n{}",
        stale.join("\n")
    );
}

#[test]
fn nothing_polls_on_an_interval() {
    // The shape of the mistake this suite exists for. Every timer above is armed by
    // something that happened — a connection, a refusal, a verification window — and is
    // waited on once; an `interval` is a wake-up that repeats whether or not anything is
    // going on, which is what WIN-06 rules out.
    let intervals: Vec<String> = armed_timers()
        .keys()
        .filter(|(_, call)| call.contains("interval"))
        .map(|(file, call)| format!("{file}: {call}"))
        .collect();
    assert!(
        intervals.is_empty(),
        "an interval timer: the app must not poll at rest (WIN-06, NFR-14).\n{}",
        intervals.join("\n")
    );
}

#[test]
fn the_registry_reads_the_sources_it_thinks_it_reads() {
    // A path that stopped resolving would make every assertion above pass over an empty
    // set, which is the one way this suite could be green and worthless.
    let sources = sources();
    assert!(
        sources.len() > 30,
        "only {} sources found under src/",
        sources.len()
    );
    assert!(sources.contains(&"src/store/actor.rs".to_owned()));
    assert!(sources.contains(&"src/channel/listener.rs".to_owned()));
    assert!(
        !armed_timers().is_empty(),
        "no timer found at all, so the scan is not reading the code"
    );
}

#[test]
fn a_timer_inside_a_test_module_is_not_the_applications() {
    // The rule the scan applies, on a source of its own so that a change to it fails here
    // rather than silently widening what the registry covers.
    let source = concat!(
        "fn armed() { tokio::time::sleep(d).await; }
",
        "#[cfg(test)]
",
        "mod tests {
",
        "    fn waits() { tokio::time::sleep(d).await; }
",
        "    fn nested() { if x { tokio::time::interval(d); } }
",
        "}
",
        "fn after() { tokio::time::timeout(d, f).await; }
",
    );
    let kept = shipped_lines(source).join(
        "
",
    );
    assert!(kept.contains("fn armed"));
    assert!(
        kept.contains("fn after"),
        "the scan resumes after the test module"
    );
    assert!(!kept.contains("fn waits"));
    assert!(
        !kept.contains("interval"),
        "a brace inside the module does not end it"
    );

    // A `#[cfg(test)]` item with no braces at all.
    let braceless = "#[cfg(test)]
use std::fs;
fn armed() { tokio::time::sleep(d).await; }
";
    let kept = shipped_lines(braceless).join(
        "
",
    );
    assert!(!kept.contains("use std::fs"));
    assert!(kept.contains("fn armed"));
}

#[test]
fn every_entry_says_what_arms_it() {
    for timer in REGISTRY {
        assert!(
            timer.armed_when.len() > 40,
            "{}: {} has no reason worth reading",
            timer.file,
            timer.call
        );
        assert!(timer.count > 0);
    }
}
