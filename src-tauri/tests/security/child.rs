//! Running one test of this binary again, in a process of its own.
//!
//! Two checks of §11.7 are about what the application leaves **outside** itself — the
//! tracing output a refused token produces, and the file a panic produces — and both come
//! from process-wide machinery: the subscriber and the panic hook `crash::install` sets up
//! once at startup, exactly as `run()` does. Installed inside this test binary, they would
//! collect every other test's log lines and every other test's panics. So such a test starts
//! this same executable again with only itself selected, the child does what the app does,
//! and the parent reads what the child left behind: its stderr, which is where the
//! production subscriber writes, and the files in the folder it was given.
//!
//! The child learns that it is one from [`CHILD_VARIABLE`], and prints [`finished`] as its
//! last line. A test name mistyped in the parent would otherwise select zero tests, exit 0,
//! and hand the parent an empty output to call clean.

use std::ffi::OsStr;
use std::process::{Command, Output};

/// Set in a child's environment to the name of the test it is running.
pub const CHILD_VARIABLE: &str = "BATON_SECURITY_CHILD";

/// Whether this process is a child started by [`run`].
#[must_use]
pub fn is_child() -> bool {
    std::env::var_os(CHILD_VARIABLE).is_some()
}

/// The line a child prints when its body has run to the end.
#[must_use]
pub fn finished(test: &str) -> String {
    format!("security child finished: {test}")
}

/// Runs the test `test` (its full path, `module::name`) of this binary in a child process,
/// with `envs` added to its environment, and returns what it printed.
///
/// # Panics
///
/// When the child cannot be started, fails, or never reached its last line.
#[must_use]
pub fn run(test: &str, envs: &[(&str, &OsStr)]) -> Output {
    let exe = std::env::current_exe().expect("the test binary knows its own path");
    let mut command = Command::new(exe);
    command
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_VARIABLE, test)
        // The production subscriber colours its output for a terminal; the parent reads it
        // as text. The two test-isolation variables of §0.4 item 4 are cleared so that a
        // value the parent's environment happens to hold cannot move the child's files.
        .env("NO_COLOR", "1")
        .env_remove("HANDOFF_HOME")
        .env_remove("HANDOFF_APP_DATA_DIR");
    for (name, value) in envs {
        command.env(name, value);
    }
    let output = command.output().expect("the test binary starts again");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains(&finished(test)),
        "the child run of {test} did not finish ({}):\n--- stdout\n{stdout}\n--- stderr\n{stderr}",
        output.status
    );
    output
}
