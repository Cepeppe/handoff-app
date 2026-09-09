//! User-opened requests (§7.7, OPEN-03..08).
//!
//! The queue behind the global shortcut and the tray entry "New request": the text the
//! user types or pastes, the terminal it should reach, and the linking of a request to the
//! session that adopts it (§12.4). Clipboard and window focus are side effects, so they
//! arrive through traits rather than through Tauri types.
//!
//! - [`queue`] is the queue itself: what is waiting, for whom, and what answers it.
//! - [`text`] is the sentence that reaches the agent, in the two languages of APP-02.
//! - [`focus`] is the best-effort half of OPEN-05: bringing the session's terminal window to
//!   the front so the user only has to paste. It is a trait for the same reason the
//!   observer is one — the platform call belongs to `ui_bridge`, not to the queue.

pub mod focus;
pub mod queue;
pub mod text;

pub use focus::{NoTerminalFocus, PlatformFocus, TerminalFocus};
pub use queue::{
    NoRequestObserver, OpenLink, Queue, RequestObserver, RequestReadyForSession, UserRequest,
};
pub use text::{render_for, render_request_text, render_resume_text};
