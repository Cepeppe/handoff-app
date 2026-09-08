//! User-opened requests (§7.7, OPEN-03..08).
//!
//! The queue behind the global shortcut and the tray entry "New request": the text the
//! user types or pastes, the terminal it should reach, and the linking of a request to the
//! session that adopts it (§12.4). Clipboard and window focus are side effects, so they
//! arrive through traits rather than through Tauri types.
//!
//! - [`queue`] is the queue itself: what is waiting, for whom, and what answers it.
//! - [`text`] is the sentence that reaches the agent, in the two languages of APP-02.
// TASK: T-038 — the request sheet, the global shortcut, the clipboard and the terminal
// focus, over the seams `queue` declares.

pub mod queue;
pub mod text;

pub use queue::{
    NoRequestObserver, OpenLink, Queue, RequestObserver, RequestReadyForSession, UserRequest,
};
pub use text::{render_for, render_request_text, render_resume_text};
