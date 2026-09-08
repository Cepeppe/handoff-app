//! User-opened requests (§7.7, OPEN-03..08).
//!
//! The queue behind the global shortcut and the tray entry "New request": the text the
//! user types or pastes, the terminal it should reach, and the linking of a request to the
//! session that adopts it (§12.4). Clipboard and window focus are side effects, so they
//! arrive through traits rather than through Tauri types.
// TASK: T-038
