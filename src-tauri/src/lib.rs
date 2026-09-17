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

/// The automation channel of DD-33, compiled only under `--features e2e`: the second socket
/// through which the harness of §11.5 plays the user. No release build enables the feature,
/// and `scripts/check-no-automation.mjs` is what proves the names are absent from one.
#[cfg(feature = "e2e")]
pub mod e2e;

/// Starts the application: the startup sequence of §7.2, in its order.
///
/// ```text
/// single-instance lock            a second launch brings the running panel forward
/// crash::install                  the panic hook and the ring it empties (TEL-02)
/// license::check                  the one call site of LIC-01, LIC-02
/// log::Db::open_app_data          the database and its migrations (§7.11)
/// channel::start                  ~/.handoff/{runbooks/} and channel.token, then listen
/// Store::load / Registry::open    tabs restored, sessions marked disconnected (FM-13)
/// ui_bridge::init                 tray and shortcut, from setup() (WIN-05, OPEN-03)
/// install::scan_agents            from the window's first mount, with the FM-23 path check
/// net::updater::check_if_enabled  the update call site (UPD-01), a no-op in this build
/// install::cleanup                the superseded server binaries of a Windows update (FM-24)
/// ```
///
/// Two steps are not on this side of the process, a departure from §7.2 made in T-040:
/// the **agent scan** and the **registered-path check** are called by the
/// window when it first mounts, because each of them produces a sentence for a person and
/// at `setup()` no webview is listening — a notice pushed from here is dropped. The **crash
/// notice** of §7.14 is the third of that family and is asked for in the same place
/// ([`ui_bridge::crash`]). What stays here is everything that has to happen whether or not
/// anybody ever opens the window.
///
/// The order of the first six is load-bearing rather than decorative: the migrations run
/// before anything opens a second connection to the same file, the token and the folder
/// exist before a peer can connect, and the listener is up before the store is restored so
/// that a server which started before the app registers on its first retry (SRV-20, FM-02)
/// — its traffic waits in the event queue until the dispatch is running.
pub fn run() {
    // The ring buffer the panic hook empties into the crash file. Held for the process
    // lifetime; nothing else reads it.
    let _recent = crash::install(paths::app_data_dir());

    // LIC-01, LIC-02: the single call site. Every feature gate in v1 is a no-op that reads
    // this value, so a future local check changes `license.rs` and nothing else.
    let entitlement = license::check();
    tracing::info!(entitlement = %entitlement, "starting");

    // The core half of the window (§7.6). It has to exist before the channel does — the
    // registry and the store take it as their observer — and it is given the `AppHandle` in
    // `setup()`, which is the first moment there is one; `ui_bridge::events` says why.
    let notifier = ui_bridge::Notifier::new();

    // §7.2: the database and its migrations, before anything reads or writes it. Three
    // connections to the one file, opened here so the migrations run exactly once and in
    // the sequence's own order: the session registry's and the store's (§7.11 sets the
    // pragmas for that traffic pattern) and the window's, which remembers the panel's
    // position per monitor (WIN-02) whether or not the channel ever came up.
    let databases = Databases::open();
    let window_db = databases.window;

    // §7.2: the shared folder, the token, the listener, and then the state the database
    // holds. A server that connects while the UI is still starting is registered all the
    // same, because registration is the channel's business and not the window's (SRV-20).
    let (channel, core) = start_channel(&notifier, databases.registry, databases.store);

    // §7.2, UPD-01: the update call site, in the place the sequence puts it. This build
    // makes no network connection at all (implementation decision 8) and the function says so.
    tracing::debug!(status = %net::updater::check_if_enabled(), "update check");

    // §7.2, FM-24: the binaries a Windows update renamed out of the way, once the sessions
    // that were using them have gone. Nothing to do on macOS, where an update replaces the
    // bundle rather than renaming a file inside it.
    if cfg!(windows) {
        clean_up_superseded_binaries();
    }

    let mut ui = ui_bridge::Ui::with_notifier(notifier);
    if let Some(db) = window_db {
        ui = ui.with_settings(db);
    }

    // DD-33, T-055: an e2e build started by `msedgedriver` gives its windows the driver's
    // WebView2 switches beside its own, which not every runtime build does by itself
    // (`e2e::webdriver`). A release build compiles none of this, and the context is built
    // exactly as before.
    #[cfg_attr(not(feature = "e2e"), allow(unused_mut))]
    let mut context = tauri::generate_context!();
    #[cfg(feature = "e2e")]
    e2e::webdriver::merge_driver_arguments(context.config_mut());

    let app = tauri::Builder::default()
        // First, as the plugin requires: a second launch must reach the running instance
        // before that instance has finished starting. The overlay is one window and one
        // process (MULTI-04, APP-01), so a second launch only brings it forward.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            ui_bridge::show_main_window(app);
        }))
        // The plugins the commands of §7.6 and §7.7 need. None of them is granted to the
        // webview: `capabilities/main.json` lists no clipboard, opener, notification or
        // shortcut permission, because the frontend never calls them — it calls
        // `copy_value`, `open_url`, `open_secret_file`, `create_request` and `set_shortcut`,
        // and this side decides what may be copied, opened, said and registered.
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        // INST-06: the folder picker of the project scope, in the Agents settings page. Like
        // the others it is never granted to the webview — the window calls
        // `pick_project_folder`, and this side is what opens a dialog.
        .plugin(tauri_plugin_dialog::init())
        // OPEN-05: the notification that carries a request when its terminal could not be
        // brought forward (FM-21).
        .plugin(tauri_plugin_notification::init())
        // OPEN-03: the combination that opens the request sheet from anywhere. The plugin
        // registers nothing by itself; `ui_bridge::shortcut` does that from `setup()`, once
        // the settings connection can say whether the user chose another one.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // APP-01: the login entry. The plugin writes nothing by itself either — it is
        // `ui_bridge::general` that decides, from the answer onboarding stored — and the
        // entry it writes carries `--hidden`, so a launch at login leaves the panel in the
        // tray "doing nothing until an agent asks for a handoff".
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![ui_bridge::general::HIDDEN_ARG]),
        ))
        .manage(ui)
        .manage(ui_bridge::CoreState(core))
        .invoke_handler(tauri::generate_handler![
            ui_bridge::resize_to_content,
            ui_bridge::set_ui_language,
            ui_bridge::set_wide_layout,
            ui_bridge::commands::list_handoffs,
            ui_bridge::commands::get_handoff_view,
            ui_bridge::commands::act,
            ui_bridge::commands::copy_value,
            ui_bridge::commands::copy_request_text,
            ui_bridge::commands::copy_handoff_id,
            ui_bridge::commands::reveal_value,
            ui_bridge::commands::open_url,
            ui_bridge::commands::open_secret_file,
            ui_bridge::commands::scan_typed_text,
            ui_bridge::commands::sessions,
            ui_bridge::commands::create_request,
            ui_bridge::commands::open_requests,
            ui_bridge::commands::shortcut_status,
            ui_bridge::commands::set_shortcut,
            ui_bridge::commands::dismiss_shortcut_question,
            ui_bridge::commands::session_picker,
            ui_bridge::commands::answer_session_picker,
            ui_bridge::commands::window_settings,
            ui_bridge::commands::set_collapse_fallback,
            ui_bridge::commands::show_window,
            ui_bridge::capture::capture_settings,
            ui_bridge::capture::capture_full_screen,
            ui_bridge::capture::start_region_capture,
            ui_bridge::capture::selection_setup,
            ui_bridge::capture::region_captured,
            ui_bridge::capture::cancel_region_capture,
            ui_bridge::capture::capture_preview,
            ui_bridge::capture::discard_capture,
            ui_bridge::preview::analyze_capture,
            ui_bridge::preview::edit_preview,
            ui_bridge::preview::preview_text,
            ui_bridge::preview::send_screenshot,
            ui_bridge::crash::crash_notice,
            ui_bridge::crash::open_crashes_folder,
            ui_bridge::install::onboarding,
            ui_bridge::install::finish_onboarding,
            ui_bridge::install::agents,
            ui_bridge::install::scan_agents,
            ui_bridge::install::consent_plan,
            ui_bridge::install::install_agent,
            ui_bridge::install::uninstall_agent,
            ui_bridge::install::repair_token,
            ui_bridge::install::pick_project_folder,
            ui_bridge::install::open_screen_recording_settings,
            ui_bridge::general::general_settings,
            ui_bridge::general::set_language,
            ui_bridge::general::set_autostart,
            ui_bridge::log::log_entries,
            ui_bridge::log::log_detail,
            ui_bridge::log::delete_log_entry,
            ui_bridge::log::delete_log,
            ui_bridge::log::export_log,
            ui_bridge::network::network_events,
            ui_bridge::runbooks::runbooks,
            ui_bridge::runbooks::runbook_proposals,
            ui_bridge::runbooks::open_runbooks_folder,
            ui_bridge::runbooks::delete_runbook,
            ui_bridge::runbooks::resolve_runbook_proposal
        ])
        // WIN-02, WIN-03, WIN-04: the close button hides the window to the tray, the focus
        // change is what the panel collapses on, and a move is remembered per monitor.
        .on_window_event(ui_bridge::on_window_event)
        .setup(|app| {
            ui_bridge::init(app.handle())?;
            // DD-33: the harness of §11.5 needs an `AppHandle` to press a button with, and
            // this is the first moment there is one. Compiled out of every build that does
            // not ask for `--features e2e`.
            #[cfg(feature = "e2e")]
            e2e::start(app.handle());
            Ok(())
        })
        .build(context)
        .expect("error while starting the Baton application");

    // `run_return` and not `run`: `App::run` exits the process itself and never comes back,
    // so everything after it — including the goodbye below — would be code that cannot run.
    // The tray's `Quit` calls `AppHandle::exit`, which unwinds the event loop to here.
    let code = app.run_return(|app, event| {
        // The panel's last position, while the window still exists to be asked (WIN-02).
        if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
            ui_bridge::on_exit_requested(app);
        }
    });

    // The peers are owed the `app.shutdown` of §6.3 before the socket goes away: a server
    // that is told why its channel went down degrades to text mode at once, while one left
    // to infer it from an EOF spends the backoff schedule finding out.
    if let Some(handle) = channel {
        tracing::info!(endpoint = %handle.endpoint().display(), "telling the peers the app is leaving");
        tauri::async_runtime::block_on(handle.shutdown("the user quit the app"));
    }

    std::process::exit(code);
}

/// The three connections to `handoff.sqlite` the startup sequence opens (§7.2, §7.11).
///
/// Opened together, and before anything that uses one: `Db::open_at` runs the migrations,
/// so the first of them is what brings the file up to `current_schema_version()`, and the
/// other two must not be racing it. Each is optional on its own — a database that cannot be
/// opened costs the feature that needs it and not the launch (FM-28, PRIN-10) — and each
/// failure is reported once, here, naming which half of the app will be without it.
struct Databases {
    /// The session registry's, written from the channel dispatch task (§7.5).
    registry: Option<log::Db>,
    /// The handoff store's, written from its actor task (§7.4).
    store: Option<log::Db>,
    /// The window's: settings, remembered geometry, the FM-22 picker's binding (§7.16).
    window: Option<log::Db>,
}

impl Databases {
    fn open() -> Self {
        let open = |what: &str| match log::Db::open_app_data() {
            Ok(db) => Some(db),
            Err(error) => {
                tracing::error!(error = %error, "the database could not be opened for the {what}");
                None
            }
        };
        Self {
            registry: open("session registry"),
            store: open("handoff store"),
            window: open("window"),
        }
    }
}

/// The last step of §7.2: the server binaries a Windows update left beside ours (FM-24).
fn clean_up_superseded_binaries() {
    let Some(folder) = install::cleanup::bundle_folder() else {
        tracing::debug!("no bundle folder to clean up");
        return;
    };
    let cleaned = install::cleanup::remove_superseded_binaries(&folder);
    if cleaned.found_something() {
        tracing::info!(
            count = cleaned.removed,
            "superseded server binaries were deleted"
        );
    }
}

/// Starts the channel, or reports why it could not start and lets the app run without it.
///
/// A listener that cannot bind is a bad day, not a reason to deny the user the window: the
/// overlay still shows what the log holds, the settings screen still repairs the token, and
/// every agent call degrades to text mode, which is a supported way to work (SRV-14).
fn start_channel(
    notifier: &ui_bridge::Notifier,
    registry_db: Option<log::Db>,
    store_db: Option<log::Db>,
) -> (Option<channel::ChannelHandle>, Option<ui_bridge::Core>) {
    match tauri::async_runtime::block_on(channel::start()) {
        Ok((handle, events)) => {
            tracing::info!(endpoint = %handle.endpoint().display(), "the channel is listening");
            // Inside `block_on` because `store::spawn` puts the actor on the runtime, and
            // `tokio::spawn` panics outside a runtime's context. Nothing here awaits; what
            // the block provides is the context, which the builder has not started yet.
            let built = tauri::async_runtime::block_on(async {
                state_of_the_app(&handle, notifier, registry_db, store_db)
            });
            match built {
                Some((dispatch, deliveries, core)) => {
                    tauri::async_runtime::spawn(dispatch.run(events, deliveries));
                    (Some(handle), Some(core))
                }
                // Without the store there is nothing to answer a peer with, but the queue
                // still has to be read: an unread event stream is back-pressure on the
                // socket, and a peer blocked on a write is worse than a peer in text mode.
                // The window opens all the same, with an empty tab strip.
                None => {
                    tauri::async_runtime::spawn(drain(events));
                    (Some(handle), None)
                }
            }
        }
        Err(error) => {
            tracing::error!(error = %error, "the channel could not start; agents will use text mode");
            (None, None)
        }
    }
}

/// `store::load_active_handoffs()` and the session registry of §7.2, and the dispatch that
/// joins them to the channel.
///
/// Two connections to the same file: the registry writes `sessions` from the dispatch task
/// and the store writes everything else from its own actor task, which is the traffic
/// pattern the pragmas of `log::db` are set for.
///
/// The two halves of "restore on launch" are one call each, and both are the constructors'
/// own business rather than a step written out here (FM-13):
///
/// - [`sessions::Registry::open`] closes the `sessions` rows a killed process left flagged
///   connected, so every restored handoff starts out **detached** and stays that way until
///   a server registers again (SRV-21, SRV-22, §8.3);
/// - [`store::Store::load`] brings back every handoff, open and closed, with no attached
///   call — a call is a connection and none of them survived — and the actor re-reads
///   `verifying_since` on its first pass, so a verification window that ran out while the
///   app was down is closed at once rather than at the next command (VER-06).
fn state_of_the_app(
    handle: &channel::ChannelHandle,
    notifier: &ui_bridge::Notifier,
    registry_db: Option<log::Db>,
    store_db: Option<log::Db>,
) -> Option<(
    channel::Dispatch,
    tokio::sync::mpsc::Receiver<store::Delivery>,
    ui_bridge::Core,
)> {
    let registry_db = registry_db?;
    let store_db = store_db?;

    // The registry is shared with the window rather than owned by the dispatch alone:
    // §7.6's `sessions_changed` carries no payload because the view re-reads the registry,
    // and whether a session is still connected is what tells the "detached" row of §8.4
    // from the rest (SRV-21).
    let registry = sessions::Registry::open(&registry_db, Box::new(notifier.clone()))
        .inspect_err(
            |error| tracing::error!(error = %error, "the session registry could not start"),
        )
        .ok()?;
    let registry = std::sync::Arc::new(std::sync::Mutex::new(registry));

    // The delivery of OPEN-05 — clipboard, terminal focus, notification — and the queue it
    // observes. Two objects rather than one because they own each other: the queue announces
    // what is ready and the delivery records what it copied, so the delivery is built first,
    // handed to the queue as its observer, and given the queue back (weakly) afterwards.
    let delivery = ui_bridge::RequestDelivery::new(
        notifier.handle(),
        std::sync::Arc::clone(&registry),
        Box::new(requests::PlatformFocus),
    );
    // The queue of §7.7, shared by the store (which links a request inside the transition
    // that answers it) and the dispatch (which hands it to sessions and reads it for every
    // hook). No session is connected yet, so every assignment left in the table names a
    // session of the previous run (FM-34, `requests::queue::requeue_on_start`).
    let queue = std::sync::Arc::new(requests::Queue::new(Box::new(delivery.clone())));
    delivery.attach_queue(&queue);
    if let Err(error) = queue.requeue_on_start(&registry_db) {
        tracing::error!(error = %error, "the queued requests of the previous run could not be re-queued");
    }

    // The runbook writer of §7.12, over `~/.handoff/runbooks/` (RUN-03a). It is built here
    // rather than asked for the folder on every write because the environment is settled by
    // now, and it is the store that tells it a handoff has finished (RUN-01, RUN-09).
    let store = store::Store::load(
        store_db,
        Box::new(runbooks::RunbookWriter::in_handoff_home()),
        Box::new(std::sync::Arc::clone(&queue)),
    )
    .inspect_err(|error| tracing::error!(error = %error, "the handoff store could not be restored"))
    .ok()?
    .watched_by(Box::new(notifier.clone()));

    let (store, deliveries) = store::spawn(store);
    let core = ui_bridge::Core {
        store: store.clone(),
        registry: std::sync::Arc::clone(&registry),
        queue: std::sync::Arc::clone(&queue),
        delivery,
    };
    Some((
        channel::Dispatch::new(registry_db, registry, store, handle.clone(), queue),
        deliveries,
        core,
    ))
}

async fn drain(mut events: tokio::sync::mpsc::Receiver<channel::ChannelEvent>) {
    while let Some(event) = events.recv().await {
        tracing::debug!(event = ?event, "channel event with no store to consume it");
    }
}
