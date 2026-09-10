//! `target/security-report.json`: the machine-readable report of the §11.7 suite.
//!
//! §11.7 asks for gates that are crossed or not — certain precision and recall, suspected
//! recall — and for a number that is watched per release rather than bounded, the suspected
//! false-positive rate (R-07). Every check of this suite records its own section as it
//! finishes, with its status, what it measured and, where it has one, the threshold it was
//! held to, so one file answers "what did the security suite say about this build" without
//! reading a log. CI prints it and keeps it as an artifact of the run.
//!
//! The checks run in parallel threads of one process, so every write goes through one lock
//! and rewrites the whole file. A file left by an earlier run is replaced rather than merged
//! (its `run` differs), so no section is ever a stale one from another build; a section that
//! is missing is a check that did not finish, which is what a failed run looks like. A check
//! records **before** it asserts wherever it can, so a failing gate is in the report with
//! its numbers rather than absent.

use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};

use serde_json::{json, Value};

use handoff_app_lib::format::patterns::patterns_version;
use handoff_app_lib::log::Timestamp;

/// The name of the report, under Cargo's target directory.
pub const FILE_NAME: &str = "security-report.json";

/// Raised when a section changes shape in a way a reader of an older report would misread.
const REPORT_VERSION: u32 = 1;

/// One value per process: what tells this run's sections from an earlier run's.
static RUN: LazyLock<String> =
    LazyLock::new(|| format!("{}-{}", std::process::id(), Timestamp::now()));

static WRITING: Mutex<()> = Mutex::new(());

/// `<target>/security-report.json`: `CARGO_TARGET_DIR` when it is set, `src-tauri/target`
/// otherwise.
#[must_use]
pub fn path() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| manifest.join("target"), PathBuf::from);
    let target = if target.is_absolute() {
        target
    } else {
        manifest.join(target)
    };
    target.join(FILE_NAME)
}

/// `"passed"` or `"failed"`, for a section whose status is one condition.
#[must_use]
pub fn status(passed: bool) -> &'static str {
    if passed {
        "passed"
    } else {
        "failed"
    }
}

/// Writes `body` as the section `name` of this run's report.
///
/// A child process (`child.rs`) records nothing: its parent is the check, and the parent
/// records what it read.
///
/// # Panics
///
/// When the file cannot be written: a report that silently stopped being written would be
/// read as the last one that was.
pub fn record(name: &str, body: Value) {
    if crate::child::is_child() {
        return;
    }
    let _writing = WRITING.lock().unwrap_or_else(PoisonError::into_inner);
    let path = path();
    let mut report = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|report| report["run"].as_str() == Some(RUN.as_str()))
        .unwrap_or_else(fresh);
    report["sections"][name] = body;
    report["updated_at"] = json!(Timestamp::now().to_string());

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("{}: {error}", parent.display()));
    }
    // Written beside it and then renamed over it, so a reader never meets half a file.
    let partial = path.with_extension(format!("json.{}", std::process::id()));
    let mut text = serde_json::to_string_pretty(&report).expect("the report serialises");
    text.push('\n');
    std::fs::write(&partial, text).unwrap_or_else(|error| panic!("{}: {error}", partial.display()));
    std::fs::rename(&partial, &path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

/// The head of a new report: what build and what machine the sections are about.
fn fresh() -> Value {
    json!({
        "report": "Baton security suite (TECHNICAL-DESIGN §11.7)",
        "report_version": REPORT_VERSION,
        "run": RUN.as_str(),
        "app_version": env!("CARGO_PKG_VERSION"),
        "patterns_version": patterns_version(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "sections": {},
    })
}
