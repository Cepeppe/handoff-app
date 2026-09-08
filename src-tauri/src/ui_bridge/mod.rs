//! The bridge between the core and Tauri (§7.6, §7.16).
//!
//! The Tauri commands the frontend invokes, the events it listens to, and the *only*
//! implementations of the side-effect traits the core declares (clipboard, notification,
//! focus, opener, window). Everything that names `AppHandle`, `WebviewWindow` or a Tauri
//! plugin belongs here; nothing in this module holds state that the core does not own.
// TASK: T-028 (the first commands and events), T-036 (bridge and view model)
