//! The e2e automation channel (DD-33, TECHNICAL-DESIGN §11.5, implementation decision 5).
//!
//! A second local endpoint, token-protected, through which a test harness plays the person
//! at the machine: it reads what the window would draw and presses the buttons the user
//! presses. It exists so that the end-to-end scenarios of §11.5 can be **run at all** —
//! every one of them needs a user, and there is no other way to be one without a pair of
//! hands and a screenshot of the panel (the manual walk of `docs/dev/smoke.md`).
//!
//! # It is not in the product
//!
//! The whole module is behind `--features e2e`, which no release build enables. §11.5 gives
//! the reason in one sentence — "a hidden control channel in a trust-sensitive app must not
//! ship" — and the guarantee is only worth what proves it, so:
//!
//! - the two names that would betray it, `handoff-e2e` and `e2e.sock`, exist in this module
//!   and nowhere else, and `scripts/check-no-automation.mjs` greps a built binary for them;
//! - CI runs that check against a release build on every push, and the same script run with
//!   `--present` against an e2e build is the positive control that says the check can fail.
//!
//! # What it may do, and what it may not
//!
//! It may do what the window may do, and it reaches it the same way: [`api`] calls
//! [`crate::ui_bridge::commands`], the functions `invoke_handler` registers. That is the
//! point rather than a convenience — a scenario driving a second implementation of "confirm
//! a step" would prove that second implementation works.
//!
//! It may **not** hand out a spec value. `state` answers with the view the webview is given,
//! where every secret-treated value is masked (DET-04); the log-invariant check of §11.2
//! reads the database for values that leaked, and a channel that published them would make
//! that check meaningless.
//!
//! # The four files
//!
//! - [`endpoint`] — the pipe name, the socket path, the token file.
//! - [`server`] — accept, authenticate, frame, answer.
//! - [`api`] — the six methods, and what each of them is an adapter over.
//! - [`webdriver`] — the WebView2 switches a WebDriver needs, added to the window's own when
//!   one started the application (the UI suite of T-055, which drives the window itself).

pub mod api;
pub mod endpoint;
pub mod server;
pub mod webdriver;

pub use endpoint::E2eEndpoint;

use std::sync::Arc;

use tauri::AppHandle;

/// The application behind the six methods (`server::Methods` says why it is a trait).
struct TauriMethods(AppHandle);

impl server::Methods for TauriMethods {
    fn serve<'a>(&'a self, request: &'a api::Request) -> server::MethodFuture<'a> {
        Box::pin(api::serve(&self.0, request))
    }

    fn quit(&self) {
        // The same path the tray's **Quit** takes: `run_return` unwinds, the goodbye of
        // §6.3 reaches the peers, and the process leaves through `lib.rs` (T-037).
        self.0.exit(0);
    }
}

/// Starts the automation channel, or says why it could not start and lets the app run.
///
/// Called from `setup()`, which is the first moment there is an [`AppHandle`] to serve
/// methods with and — deliberately — after the product channel and the store: a harness
/// that connects the instant the endpoint appears must find a store behind it.
///
/// A failure here is never fatal. The app under test still has to come up: a harness that
/// times out waiting for the endpoint reports "the automation channel never appeared", which
/// is a better message than a window that silently never exists.
pub fn start(app: &AppHandle) {
    let endpoint = E2eEndpoint::resolve();
    let token = match endpoint::ensure_token() {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(error = %error, "the automation token could not be written");
            return;
        }
    };
    let methods: Arc<dyn server::Methods> = Arc::new(TauriMethods(app.clone()));
    // Inside `block_on` because the accept loop is a `tokio::spawn`, which panics outside a
    // runtime context, and `setup()` runs on the main thread before the event loop starts —
    // the same reason `lib.rs` starts the product channel that way. Nothing here awaits.
    match tauri::async_runtime::block_on(async { server::start(methods, &endpoint, token) }) {
        Ok(()) => {
            // The endpoint is published rather than left to be re-derived. The product
            // channel is the opposite case on purpose — two independent implementations
            // computing the same digest from the same environment is what proves the
            // derivation (§5.8) — but here both ends are this commit, and a harness that
            // recomputed a SHA-256 over `<user>|<home>` would only add a second thing to
            // get wrong. It doubles as the "the app is up" signal the harness waits on.
            if let Err(error) = endpoint::publish(&endpoint) {
                tracing::error!(error = %error, "the automation endpoint could not be published");
            }
            tracing::info!(
                endpoint = %endpoint.display(),
                "the automation channel is listening (this build carries --features e2e)"
            );
        }
        Err(error) => tracing::error!(error = %error, "the automation channel could not start"),
    }
}
