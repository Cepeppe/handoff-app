//! Baton — the overlay application of the contextual handoff system (TECHNICAL-DESIGN §7).
//!
//! The window shows the work a coding agent hands to the person at the machine, guides it
//! one step at a time and sends a structured outcome back through the internal channel.
//! Everything trust-sensitive — the screenshots, the detectors, the log, the runbooks, the
//! installation adapters — lives here rather than in the open server (§2.4, ARCH-03).
//!
//! # Module layout
//!
//! One module per box of the §3.3 tree. Most of them are stubs today: each carries the
//! design section it implements and the task that fills it, so that the layout is decided
//! once and no later task has to move code between modules.
//!
//! Two modules sit beside that tree rather than in it, because what they carry is not the
//! app's to define: [`format`] is the Rust side of the schemas, the pattern file and the
//! channel protocol of the pinned `handoff-mcp` release (§3.4), and [`ids`] is the shape of
//! the identifiers §4.1 fixes. Everything else reads them.
//!
//! # The rule that shapes all of them: no `AppHandle` in the core
//!
//! Core logic — the state machine, the store, the session registry, the hook decision, the
//! detectors, the runbook writer, the channel codec — never depends on `tauri::AppHandle`
//! or on any other Tauri type. Side effects (clipboard, notification, window focus,
//! opening a URL, capturing the screen, the socket itself) are reached through traits
//! declared next to the logic that needs them; the Tauri implementations of those traits
//! live in [`ui_bridge`] and nowhere else. Two reasons, both load-bearing:
//!
//! - `cargo test` has to exercise the whole core with no webview and no display, which is
//!   what CI does on every push and what makes the app testable at all (§11.2);
//! - the e2e automation channel of DD-33 plays the user by driving those same traits with
//!   fakes (`--features e2e`, T-043), so a core that reached for an `AppHandle` would have
//!   to be duplicated to be automated.
//!
//! A module that needs a side effect declares the trait; `ui_bridge` implements it over
//! Tauri; tests implement it over a fake.

pub mod capture;
pub mod channel;
pub mod crash;
pub mod format;
pub mod hook;
pub mod i18n;
pub mod ids;
pub mod install;
pub mod license;
pub mod log;
pub mod net;
pub mod ocr;
pub mod paths;
pub mod redaction;
pub mod requests;
pub mod runbooks;
pub mod sessions;
pub mod store;
pub mod ui_bridge;

/// The v2 extension point of §12.2, compiled only under `--features secrets-write`. It is
/// off in every v1 build and the module below it is deliberately empty.
#[cfg(feature = "secrets-write")]
pub mod secrets;

/// Starts the application.
///
/// The full startup sequence of §7.2 — the database and its migrations, the `~/.handoff/`
/// folder and the token, the listener, the restored tabs, the shortcut, the agent scan — is
/// assembled by T-042 as its modules appear. What is here is what the earlier tasks own:
/// the panic hook (TEL-02) and the licence entry point (LIC-01, LIC-02), which touch
/// `main()` and would otherwise mean editing the startup path twice, plus the single
/// instance, the tray and the window rules of §7.16.
pub fn run() {
    // The ring buffer the panic hook empties into the crash file. Held for the process
    // lifetime; nothing else reads it.
    let _recent = crash::install(paths::app_data_dir());

    // LIC-01, LIC-02: the single call site. Every feature gate in v1 is a no-op that reads
    // this value, so a future local check changes `license.rs` and nothing else.
    let entitlement = license::check();
    tracing::info!(entitlement = %entitlement, "starting");

    // §7.2: the shared folder, the token and the listener, before the window exists. A
    // server that connects while the UI is still starting is registered all the same,
    // because registration is the channel's business and not the window's (SRV-20).
    let channel = start_channel();

    tauri::Builder::default()
        // First, as the plugin requires: a second launch must reach the running instance
        // before that instance has finished starting. The overlay is one window and one
        // process (MULTI-04, APP-01), so a second launch only brings it forward.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            ui_bridge::show_main_window(app);
        }))
        .manage(ui_bridge::Ui::default())
        .invoke_handler(tauri::generate_handler![
            ui_bridge::resize_to_content,
            ui_bridge::set_ui_language
        ])
        // WIN-04: the close button hides the window to the tray; `Quit` in the tray menu is
        // the only way out.
        .on_window_event(ui_bridge::on_window_event)
        .setup(|app| ui_bridge::init(app.handle()))
        .run(tauri::generate_context!())
        .expect("error while running the Baton application");

    // The peers are owed the `app.shutdown` of §6.3 before the socket goes away: a server
    // that is told why its channel went down degrades to text mode at once, while one left
    // to infer it from an EOF spends the backoff schedule finding out.
    if let Some(handle) = channel {
        tauri::async_runtime::block_on(handle.shutdown("the user quit the app"));
    }
}

/// Starts the channel, or reports why it could not start and lets the app run without it.
///
/// A listener that cannot bind is a bad day, not a reason to deny the user the window: the
/// overlay still shows what the log holds, the settings screen still repairs the token, and
/// every agent call degrades to text mode, which is a supported way to work (SRV-14).
fn start_channel() -> Option<channel::ChannelHandle> {
    match tauri::async_runtime::block_on(channel::start()) {
        Ok((handle, events)) => {
            tracing::info!(endpoint = %handle.endpoint().display(), "the channel is listening");
            // The store actor of T-034 is what consumes this stream: it completes the
            // ancestor chain, registers the session, answers `hook.stop` and drives the
            // handoff state machine. Until it exists the events are drained and counted, so
            // that a peer is never blocked by an unread queue and a `cargo tauri dev`
            // session still shows a server connecting.
            // TASK: T-034 — replace this drain with the store actor.
            tauri::async_runtime::spawn(drain_until_the_store_exists(events));
            Some(handle)
        }
        Err(error) => {
            tracing::error!(error = %error, "the channel could not start; agents will use text mode");
            None
        }
    }
}

// TASK: T-034
async fn drain_until_the_store_exists(
    mut events: tokio::sync::mpsc::Receiver<channel::ChannelEvent>,
) {
    while let Some(event) = events.recv().await {
        tracing::debug!(event = ?event, "channel event with no store to consume it");
    }
}
