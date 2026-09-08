//! Crash files and the recent-log ring behind them (§7.14, TEL-01, TEL-02).
//!
//! A panic writes `crashes/<timestamp>.txt` into the app data directory and nothing else
//! happens: nothing is uploaded, ever, and no telemetry code exists (TEL-01, TEL-02). The
//! next launch offers to open the folder so the user can send the file by hand — that part
//! is UI and arrives with T-037.
//!
//! The file carries the version, the OS, the panic and its location, the backtrace, and
//! the **last 50 log lines with identifiers only**. That last clause is what shapes this
//! module: the lines come from a ring buffer fed by a `tracing` layer that keeps a field
//! only when its name is an identifier, an instant, a duration, a size or a count. A step
//! text, a note, a spec value or a secret has no name of that shape, so it cannot reach a
//! file the user may end up mailing to us. The allow-list is the one `handoff-mcp` uses in
//! `src/log.ts` (R-19), copied on purpose: the two sides log the same events, and a field
//! that is safe on one side is safe on the other.
//!
//! The convention that makes this work, and that every caller has to follow: **the message
//! of an event is a static string and every variable travels in a field.**
//! `info!(handoff_id = %id, "handoff opened")` is right; `info!("opened {note}")` puts user
//! text in the message, where no filter can see it.

use std::collections::VecDeque;
use std::fmt;
use std::fs;
use std::io;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::EnvFilter;

/// The folder under the app data directory that holds the crash files (§7.2).
pub const CRASHES_FOLDER_NAME: &str = "crashes";

/// How many log lines the crash file carries (§7.14).
pub const RECENT_LINES: usize = 50;

/// The verbosity when `RUST_LOG` says nothing.
const DEFAULT_LOG_FILTER: &str = "info";

/// Field names allowed by their suffix: an identifier, an instant, a duration, a size or a
/// count. The shape of the name is what makes the value safe — `steps_count` is a size,
/// `steps` would have been the steps themselves.
const FIELD_SUFFIXES: [&str; 7] = ["_id", "_ms", "_at", "_bytes", "_count", "_ref", "_version"];

/// Field names allowed as they are: codes, states and the small closed vocabularies the app
/// and the server already exchange. None of them can hold user or agent text.
const FIELD_NAMES: [&str; 17] = [
    "attempt",
    "code",
    "count",
    "entitlement",
    "event",
    "kind",
    "level",
    "method",
    "ok",
    "pid",
    "ppid",
    "problems",
    "reason",
    "role",
    "round",
    "state",
    "status",
];

/// Whether a field may appear in a crash file: it is in the vocabulary, or it carries one
/// of the suffixes with at least one character before it.
pub fn is_recordable_field(name: &str) -> bool {
    if FIELD_NAMES.contains(&name) {
        return true;
    }
    FIELD_SUFFIXES
        .iter()
        .any(|suffix| name.len() > suffix.len() && name.ends_with(suffix))
}

/// The last [`RECENT_LINES`] formatted log lines. Shared between the `tracing` layer that
/// fills it and the panic hook that empties it.
#[derive(Debug)]
pub struct RecentLog {
    capacity: usize,
    lines: Mutex<VecDeque<String>>,
}

impl RecentLog {
    /// A ring holding at most `capacity` lines.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            lines: Mutex::new(VecDeque::with_capacity(capacity)),
        }
    }

    /// Appends a line, dropping the oldest one when the ring is full. A poisoned lock is
    /// ignored rather than propagated: losing log lines must never turn into a second
    /// panic inside the panic hook.
    pub fn push(&self, line: String) {
        let Ok(mut lines) = self.lines.lock() else {
            return;
        };
        if lines.len() == self.capacity {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    /// The lines, oldest first.
    pub fn lines(&self) -> Vec<String> {
        match self.lines.lock() {
            Ok(lines) => lines.iter().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }
}

impl Default for RecentLog {
    fn default() -> Self {
        Self::new(RECENT_LINES)
    }
}

/// The `tracing` layer that fills a [`RecentLog`], dropping every field that is not an
/// identifier.
#[derive(Debug)]
pub struct RecentLogLayer {
    log: Arc<RecentLog>,
}

impl RecentLogLayer {
    pub fn new(log: Arc<RecentLog>) -> Self {
        Self { log }
    }
}

impl<S: Subscriber> Layer<S> for RecentLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut recorded = RecordedFields::default();
        event.record(&mut recorded);
        let metadata = event.metadata();
        self.log.push(format_line(
            metadata.level().as_str(),
            metadata.target(),
            recorded.message.as_deref(),
            &recorded.fields,
        ));
    }
}

/// `LEVEL target: message field=value …`
fn format_line(
    level: &str,
    target: &str,
    message: Option<&str>,
    fields: &[(String, String)],
) -> String {
    let mut line = format!("{level} {target}");
    if let Some(message) = message {
        line.push_str(": ");
        line.push_str(message);
    }
    for (name, value) in fields {
        line.push(' ');
        line.push_str(name);
        line.push('=');
        line.push_str(value);
    }
    line
}

#[derive(Default)]
struct RecordedFields {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl RecordedFields {
    fn record(&mut self, name: &str, value: String) {
        if name == "message" {
            self.message = Some(value);
        } else if is_recordable_field(name) {
            self.fields.push((name.to_string(), value));
        }
    }
}

impl Visit for RecordedFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field.name(), value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record(field.name(), format!("{value:?}"));
    }
}

/// Everything a crash file holds, so that the rendering can be tested without a panic.
#[derive(Debug, Clone)]
pub struct CrashReport {
    pub at: SystemTime,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub panic: String,
    pub location: Option<String>,
    pub backtrace: String,
    pub recent: Vec<String>,
}

/// The text of a crash file.
pub fn render_report(report: &CrashReport) -> String {
    let mut out = String::new();
    out.push_str("Baton crash report\n");
    out.push_str(&format!("at: {}\n", format_instant(report.at)));
    out.push_str(&format!("version: {}\n", report.version));
    out.push_str(&format!("os: {} {}\n", report.os, report.arch));
    out.push_str(&format!("\npanic: {}\n", report.panic));
    if let Some(location) = &report.location {
        out.push_str(&format!("location: {location}\n"));
    }
    out.push_str(&format!(
        "\nlast {} log lines (identifiers only):\n",
        RECENT_LINES
    ));
    if report.recent.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for line in &report.recent {
            out.push_str(&format!("  {line}\n"));
        }
    }
    out.push_str("\nbacktrace:\n");
    out.push_str(report.backtrace.trim_end());
    out.push('\n');
    out
}

/// Writes a crash file into `dir`, creating the folder if needed, and returns its path.
/// A second crash inside the same second gets a numbered name rather than overwriting the
/// first one.
pub fn write_report(dir: &Path, at: SystemTime, body: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = file_stamp(at);
    for attempt in 0..100 {
        let name = if attempt == 0 {
            format!("{stamp}.txt")
        } else {
            format!("{stamp}-{attempt}.txt")
        };
        let path = dir.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                io::Write::write_all(&mut file, body.as_bytes())?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many crash files for the same instant",
    ))
}

/// Installs the panic hook: every panic appends a crash file to `crashes_dir` and then the
/// previous hook runs, so the usual message still reaches stderr.
pub fn install_panic_hook(crashes_dir: PathBuf, recent: Arc<RecentLog>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let report = CrashReport {
            at: SystemTime::now(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            panic: panic_message(info),
            location: info.location().map(|location| location.to_string()),
            backtrace: std::backtrace::Backtrace::force_capture().to_string(),
            recent: recent.lines(),
        };
        // A failure here is ignored on purpose: a crash file that cannot be written must
        // not turn one panic into two.
        let _ = write_report(&crashes_dir, report.at, &render_report(&report));
        previous(info);
    }));
}

/// Installs the tracing subscriber and the panic hook, and returns the ring they share.
/// Called once, from `run()`.
pub fn install(app_data_dir: PathBuf) -> Arc<RecentLog> {
    let recent = Arc::new(RecentLog::default());
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    // `try_init` rather than `init`: a second call (a test binary, a future embedded run)
    // must not panic on its way in.
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
        .with(RecentLogLayer::new(Arc::clone(&recent)))
        .try_init();
    install_panic_hook(app_data_dir.join(CRASHES_FOLDER_NAME), Arc::clone(&recent));
    recent
}

fn panic_message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "panic with a payload of an unknown type".to_string()
    }
}

/// `2026-09-08T14:32:05Z`. Written here rather than pulled from a date crate: the only two
/// instants this application formats without a database are the crash stamp and its file
/// name, and both are UTC seconds.
fn format_instant(at: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(at);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// The same instant as a file name: `:` is not allowed in a Windows path.
fn file_stamp(at: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(at);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}-{minute:02}-{second:02}Z")
}

/// Civil date and time from a `SystemTime`, in UTC. `days_to_civil` is Howard Hinnant's
/// algorithm, valid for any proleptic Gregorian date.
fn utc_parts(at: SystemTime) -> (i64, u32, u32, u32, u32, u32) {
    let seconds = match at.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs() as i64,
        // Before 1970: only reachable from a clock set backwards, and the arithmetic below
        // handles it, so the sign is kept rather than clamped.
        Err(error) => -(error.duration().as_secs() as i64),
    };
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = days_to_civil(days);
    (
        year,
        month,
        day,
        (time_of_day / 3_600) as u32,
        ((time_of_day % 3_600) / 60) as u32,
        (time_of_day % 60) as u32,
    )
}

fn days_to_civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_position + 2) / 5 + 1;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(epoch_seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(epoch_seconds)
    }

    #[test]
    fn identifiers_instants_durations_sizes_and_counts_are_recordable() {
        for name in [
            "handoff_id",
            "session_ref",
            "elapsed_ms",
            "created_at",
            "image_bytes",
            "steps_count",
            "protocol_version",
            "code",
            "state",
            "entitlement",
        ] {
            assert!(is_recordable_field(name), "{name} should be recordable");
        }
    }

    #[test]
    fn anything_that_could_hold_a_spec_value_is_dropped() {
        for name in [
            "note", "step", "steps", "value", "secret", "spec", "text", "path", "url",
            // The suffixes need something in front of them: a field named exactly `_id`
            // says nothing and would be an easy way to smuggle a value through.
            "_id", "_at",
        ] {
            assert!(!is_recordable_field(name), "{name} should be dropped");
        }
    }

    #[test]
    fn a_line_carries_the_message_and_only_the_recordable_fields() {
        let line = format_line(
            "INFO",
            "handoff_app_lib::store",
            Some("handoff opened"),
            &[("handoff_id".to_string(), "hf_01".to_string())],
        );
        assert_eq!(
            line,
            "INFO handoff_app_lib::store: handoff opened handoff_id=hf_01"
        );
    }

    #[test]
    fn the_ring_keeps_the_last_lines_and_drops_the_oldest() {
        let log = RecentLog::new(3);
        for index in 0..5 {
            log.push(format!("line {index}"));
        }
        assert_eq!(log.lines(), vec!["line 2", "line 3", "line 4"]);
    }

    #[test]
    fn the_ring_of_a_default_log_is_the_designed_size() {
        let log = RecentLog::default();
        for index in 0..(RECENT_LINES + 10) {
            log.push(format!("line {index}"));
        }
        let lines = log.lines();
        assert_eq!(lines.len(), RECENT_LINES);
        assert_eq!(lines[0], "line 10");
    }

    #[test]
    fn an_instant_is_formatted_as_utc() {
        assert_eq!(format_instant(at(0)), "1970-01-01T00:00:00Z");
        assert_eq!(format_instant(at(1_757_342_400)), "2025-09-08T14:40:00Z");
        // A leap day, and the second before midnight.
        assert_eq!(format_instant(at(1_709_251_199)), "2024-02-29T23:59:59Z");
        assert_eq!(file_stamp(at(1_709_251_199)), "2024-02-29T23-59-59Z");
    }

    #[test]
    fn a_report_names_the_version_the_os_the_panic_and_the_recent_lines() {
        let report = CrashReport {
            at: at(1_757_342_400),
            version: "0.1.0".to_string(),
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
            panic: "called `Option::unwrap()` on a `None` value".to_string(),
            location: Some("src/store/mod.rs:42:9".to_string()),
            backtrace: "   0: handoff_app_lib::store::open\n".to_string(),
            recent: vec!["INFO handoff_app_lib: starting entitlement=full".to_string()],
        };
        let text = render_report(&report);
        assert!(text.starts_with("Baton crash report\nat: 2025-09-08T14:40:00Z\n"));
        assert!(text.contains("version: 0.1.0\n"));
        assert!(text.contains("os: windows x86_64\n"));
        assert!(text.contains("panic: called `Option::unwrap()` on a `None` value\n"));
        assert!(text.contains("location: src/store/mod.rs:42:9\n"));
        assert!(text.contains("  INFO handoff_app_lib: starting entitlement=full\n"));
        assert!(text.trim_end().ends_with("0: handoff_app_lib::store::open"));
    }

    #[test]
    fn a_report_with_no_log_lines_says_so() {
        let report = CrashReport {
            at: at(0),
            version: "0.1.0".to_string(),
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
            panic: "boom".to_string(),
            location: None,
            backtrace: String::new(),
            recent: Vec::new(),
        };
        let text = render_report(&report);
        assert!(text.contains("  (none)\n"));
        assert!(!text.contains("location:"));
    }

    #[test]
    fn two_crashes_in_the_same_second_do_not_overwrite_each_other() {
        let dir = std::env::temp_dir().join(format!("baton-crash-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let first = write_report(&dir, at(1_757_342_400), "first").expect("first report");
        let second = write_report(&dir, at(1_757_342_400), "second").expect("second report");

        assert_ne!(first, second);
        assert_eq!(fs::read_to_string(&first).expect("read first"), "first");
        assert_eq!(fs::read_to_string(&second).expect("read second"), "second");
        assert_eq!(
            first.file_name().and_then(|name| name.to_str()),
            Some("2025-09-08T14-40-00Z.txt")
        );

        fs::remove_dir_all(&dir).expect("clean up");
    }
}
