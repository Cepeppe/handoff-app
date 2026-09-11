//! Bringing the agent's terminal to the front (§7.7, OPEN-05, FM-21, A-18).
//!
//! The fast path of a user-opened request is the clipboard: the sentence is copied and the
//! user pastes it into the session they were about to talk to. Putting that window in front
//! of them saves a `Alt`+`Tab`, and that is *all* it does — it is best effort by design, it
//! never types anything (PRIN-06), and its failure is answered by the notification of
//! OPEN-05 and by the Stop hook of OPEN-06. A18 is "to verify", so nothing may depend on it.
//!
//! # What "the terminal" is
//!
//! The session's completed ancestor chain (DD-22), nearest generation to the server first.
//! The server is a child of the agent, the agent is a child of a shell, the shell is hosted
//! by a terminal: the first generation that *owns a visible top-level window* is the window
//! the user is looking at. Windows Terminal hosts the shell as a child process, so its own
//! pid is in the chain; VS Code's integrated terminal likewise ends at the editor's window.
//!
//! Nearest-first matters. Every chain ends in the desktop shell (`explorer.exe` on Windows,
//! `loginwindow` on macOS), which owns windows and belongs to every session on the machine:
//! a walk from the far end would activate the desktop for every request, which is both
//! useless and rude. [`nearest_window`] is that rule, and it is the part worth a test — the
//! platform half cannot be exercised without a display.
//!
//! # Which of its windows (T-070)
//!
//! One process can own several windows, and an editor is the case that matters: every window
//! of Cursor or VS Code belongs to the editor's one main process, so the chain says *which
//! program* and not *which window* (A-18, measured in T-038). A session an editor started is one
//! window's, and its project folder is that window's folder, which the editor puts in the title
//! bar (`<file> - <folder> - Cursor`). So among the windows of the generation the walk stops at,
//! the one whose title names the session's folder is taken ([`title_names_folder`]), and the
//! first one — the most recently active, in the platform's z-order — when none does. The rule is
//! the session's and not an editor's: a Claude Code session in an editor's terminal gets the
//! right window by the same test, and a terminal whose titles name no folder gets exactly the
//! window it got before.
//!
//! # Why a trait
//!
//! `lib.rs`: the core never names a platform API it can be tested without. The queue tells
//! `ui_bridge` that a request can be put in front of a session; `ui_bridge` owns the
//! clipboard and calls this. A test substitutes [`NoTerminalFocus`], which is also what a
//! platform we have no implementation for gets — the notification then carries the request.

use crate::format::channel::AncestorProcess;

/// Brings the window of a session's process chain to the front, if it can (OPEN-05).
pub trait TerminalFocus: Send + Sync {
    /// `chain` is the session's completed ancestor chain, nearest generation first, and
    /// `folder` the name of the folder the session works on, when it has one: what picks one
    /// window among several of the same process.
    ///
    /// Returns whether a window was actually raised. `false` is an ordinary answer, not an
    /// error: the caller notifies and the hook delivers anyway (FM-21, OPEN-06).
    fn focus(&self, chain: &[AncestorProcess], folder: Option<&str>) -> bool;
}

/// The focus of a build that has none: every unit test, and any platform without an
/// implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoTerminalFocus;

impl TerminalFocus for NoTerminalFocus {
    fn focus(&self, _chain: &[AncestorProcess], _folder: Option<&str>) -> bool {
        false
    }
}

/// The platform's own implementation: Win32 on Windows, AppKit on macOS, nothing elsewhere.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlatformFocus;

impl TerminalFocus for PlatformFocus {
    fn focus(&self, chain: &[AncestorProcess], folder: Option<&str>) -> bool {
        #[cfg(windows)]
        {
            windows_focus::raise(chain, folder)
        }
        #[cfg(target_os = "macos")]
        {
            // AppKit activates an application with all of its windows; picking one of them
            // would take the Accessibility permission this application does not ask for.
            let _ = folder;
            macos_focus::raise(chain)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (chain, folder);
            false
        }
    }
}

/// One visible top-level window the platform found: the pid that owns it, the window itself,
/// and its title.
pub type OwnedWindow<T> = (u32, T, String);

/// The window to bring forward: the nearest generation of `chain` that owns one, and of that
/// generation's windows the one whose title names `folder`, else the first of them.
///
/// `windows` is what the platform found, in whatever order it enumerated them — on Windows the
/// z-order, most recently active first; a pid may appear more than once.
///
/// The walk is over the **chain**, not over the windows: the order that decides is "how near
/// is this process to the agent", and the platform's enumeration order says nothing about
/// that. A chain whose nearest generations own nothing falls through to the next one, which
/// is how a terminal that hosts the shell as a child is reached at all. The folder only ever
/// chooses *within* the generation the walk stopped at: a farther process whose title happens
/// to name the folder never wins over a nearer one.
pub fn nearest_window<T: Clone>(
    chain: &[AncestorProcess],
    windows: &[OwnedWindow<T>],
    folder: Option<&str>,
) -> Option<T> {
    chain.iter().find_map(|ancestor| {
        let mut owned = windows
            .iter()
            .filter(|(pid, _, _)| *pid == ancestor.pid)
            .peekable();
        let first = owned.peek()?.1.clone();
        let named = folder.and_then(|folder| {
            owned
                .find(|(_, _, title)| title_names_folder(title, folder))
                .map(|(_, window, _)| window.clone())
        });
        Some(named.unwrap_or(first))
    })
}

/// Whether a window title names `folder` as one of its parts (T-070).
///
/// Editors of the VS Code family join the parts of a window title with ` - ` — the default
/// `window.title` is the file, the folder and the editor's name — and a part is compared whole
/// and without case, so `api` does not name the window of `api-gateway`. An em or an en dash
/// between spaces separates parts as well, which is what other programs write.
#[must_use]
pub fn title_names_folder(title: &str, folder: &str) -> bool {
    let folder = folder.trim().to_lowercase();
    if folder.is_empty() {
        return false;
    }
    title
        .replace(" \u{2014} ", " - ")
        .replace(" \u{2013} ", " - ")
        .split(" - ")
        .any(|part| part.trim().to_lowercase() == folder)
}

/// Win32: enumerate the visible top-level windows, take the nearest one the chain owns, and
/// go through the foreground dance (§7.7).
///
/// `SetForegroundWindow` refuses a process that has not been given the right to steal the
/// focus. Two things make it work from a tray application the user has just interacted with:
/// `AllowSetForegroundWindow`, which passes our own right on to the target process, and
/// attaching our input queue to the target's thread, which is the documented way round the
/// same restriction. Both are asked for and neither is required to succeed — this is a
/// best-effort path and a refusal ends in the notification of OPEN-05.
#[cfg(windows)]
mod windows_focus {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, TRUE};
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, EnumWindows, GetForegroundWindow, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow,
        SW_RESTORE,
    };

    use crate::format::channel::AncestorProcess;

    /// One visible top-level window and the process that owns it.
    type Owner = (u32, isize);

    pub(super) fn raise(chain: &[AncestorProcess], folder: Option<&str>) -> bool {
        // Titles are read for the windows of the session's own processes only: the rest of the
        // desktop has nothing to say about which of those to raise.
        let windows: Vec<super::OwnedWindow<isize>> = visible_windows()
            .into_iter()
            .filter(|(pid, _)| chain.iter().any(|ancestor| ancestor.pid == *pid))
            .map(|(pid, handle)| (pid, handle, title_of(handle)))
            .collect();
        let Some(handle) = super::nearest_window(chain, &windows, folder) else {
            tracing::debug!("no window of the session's process chain is on screen");
            return false;
        };
        let window = HWND(handle as *mut core::ffi::c_void);

        // The target has to be allowed to come forward: we are the process with the
        // foreground right (the user just pressed our shortcut), and this hands it over.
        let mut pid = 0_u32;
        let target_thread = unsafe { GetWindowThreadProcessId(window, Some(&raw mut pid)) };
        if pid != 0 {
            if let Err(error) = unsafe { AllowSetForegroundWindow(pid) } {
                tracing::debug!(error = %error, "the terminal was not granted the foreground right");
            }
        }

        // A minimised window is restored first: raising it without this leaves the taskbar
        // button flashing and nothing on screen to paste into.
        if unsafe { IsIconic(window) }.as_bool() {
            let _ = unsafe { ShowWindow(window, SW_RESTORE) };
        }

        // The input-queue attachment, and its undo. `AttachThreadInput` is refused when the
        // two threads are one (nothing to do) or when the target has gone; either way the
        // call below is still worth making.
        let ours = unsafe { GetCurrentThreadId() };
        let attached = target_thread != 0
            && target_thread != ours
            && unsafe { AttachThreadInput(ours, target_thread, true) }.as_bool();
        let raised = unsafe { SetForegroundWindow(window) }.as_bool();
        if attached {
            let _ = unsafe { AttachThreadInput(ours, target_thread, false) };
        }

        // `SetForegroundWindow` answers "the request was accepted", not "the window is
        // there now", so the foreground window itself is what is reported.
        let front = unsafe { GetForegroundWindow() };
        let done = raised && front.0 as isize == handle;
        tracing::debug!(raised, done, "the terminal was asked to come forward");
        done
    }

    /// Every visible top-level window, with the pid that owns it.
    fn visible_windows() -> Vec<Owner> {
        let mut found: Vec<Owner> = Vec::new();
        let sink = std::ptr::from_mut(&mut found);
        // The callback writes through `sink` and nothing else touches `found` until
        // `EnumWindows` has returned, which it does synchronously.
        if let Err(error) = unsafe { EnumWindows(Some(collect), LPARAM(sink as isize)) } {
            tracing::debug!(error = %error, "the top-level windows could not be enumerated");
        }
        found
    }

    /// The `EnumWindows` callback: append `(pid, hwnd)` for every visible window.
    unsafe extern "system" fn collect(window: HWND, sink: LPARAM) -> BOOL {
        // Safety: the pointer is the `Vec` `visible_windows` is filling, alive for the whole
        // of the `EnumWindows` call and reached from no other thread.
        let found = unsafe { &mut *(sink.0 as *mut Vec<Owner>) };
        if unsafe { IsWindowVisible(window) }.as_bool() {
            let mut pid = 0_u32;
            let _thread = unsafe { GetWindowThreadProcessId(window, Some(&raw mut pid)) };
            if pid != 0 {
                found.push((pid, window.0 as isize));
            }
        }
        TRUE
    }

    /// A window's title as its title bar shows it; empty for a window with none.
    ///
    /// The windows asked about belong to other processes, and for those `GetWindowTextW` reads
    /// the caption Windows keeps rather than sending the window a message, so a program that has
    /// stopped answering cannot hold the walk.
    fn title_of(handle: isize) -> String {
        let window = HWND(handle as *mut core::ffi::c_void);
        let mut buffer = [0_u16; 512];
        let copied = unsafe { GetWindowTextW(window, &mut buffer) };
        let length = usize::try_from(copied).unwrap_or(0).min(buffer.len());
        String::from_utf16_lossy(&buffer[..length])
    }
}

/// AppKit: the nearest chain member that is a running application with windows, activated.
///
/// `NSRunningApplication` only knows about applications, not about every process, so the
/// shell and the agent simply answer `nil` and the walk continues to the terminal that hosts
/// them — which is the rule of [`super::nearest_window`] arriving at the same place from the
/// other side. `Regular` is the activation policy of an application that appears in the Dock
/// and owns windows; an `Accessory` (our own tray application is one) is not what the user
/// wants brought forward.
///
/// This branch is compiled by the macOS CI leg and has never been run: macOS is deferred
/// (`TASKS.md` §0.4 item 7) and the manual matrix of A-18 is a Windows one for now.
#[cfg(target_os = "macos")]
mod macos_focus {
    use objc2_app_kit::{
        NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication,
    };

    use crate::format::channel::AncestorProcess;

    pub(super) fn raise(chain: &[AncestorProcess]) -> bool {
        for ancestor in chain {
            // `pid_t` is `i32` on every Apple platform; naming the alias would mean a
            // direct dependency on `libc` for one integer type.
            let Ok(pid) = i32::try_from(ancestor.pid) else {
                continue;
            };
            let Some(application) =
                NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            else {
                continue;
            };
            if application.activationPolicy() != NSApplicationActivationPolicy::Regular {
                continue;
            }
            // `ActivateAllWindows` rather than the deprecated `ignoringOtherApps`: the user
            // pressed our shortcut a moment ago, so the system already lets us yield the
            // activation, and what they want is the terminal's windows in front.
            let raised =
                application.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
            tracing::debug!(raised, "the terminal was asked to come forward");
            return raised;
        }
        tracing::debug!("no application of the session's process chain could be activated");
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ancestor(pid: u32, name: &str) -> AncestorProcess {
        AncestorProcess {
            pid,
            name: name.to_owned(),
        }
    }

    fn window(pid: u32, which: &'static str, title: &str) -> OwnedWindow<&'static str> {
        (pid, which, title.to_owned())
    }

    /// The chain a Claude Code session has on Windows, abridged: the agent, the shell, the
    /// terminal that hosts it, and the desktop every session on the machine shares.
    fn chain() -> Vec<AncestorProcess> {
        vec![
            ancestor(100, "node.exe"),
            ancestor(200, "pwsh.exe"),
            ancestor(300, "WindowsTerminal.exe"),
            ancestor(400, "explorer.exe"),
        ]
    }

    /// The chain of a session Cursor's editor started: the window's extension host, the editor's
    /// main process, and the desktop (T-069 measured the editor second in the chain).
    fn editor_chain() -> Vec<AncestorProcess> {
        vec![
            ancestor(3100, "Cursor.exe"),
            ancestor(3000, "Cursor.exe"),
            ancestor(400, "explorer.exe"),
        ]
    }

    #[test]
    fn the_nearest_generation_that_owns_a_window_is_the_one_taken() {
        // The terminal owns a window and so does the desktop shell; the terminal is nearer.
        let windows = [window(400, "desktop", ""), window(300, "terminal", "pwsh")];
        assert_eq!(nearest_window(&chain(), &windows, None), Some("terminal"));
    }

    #[test]
    fn the_desktop_shell_is_never_reached_while_something_nearer_owns_a_window() {
        // The failure this guards is the one that makes the feature worse than useless: a
        // walk from the far end activates `explorer.exe` for every session of the machine.
        let windows = [
            window(400, "desktop", ""),
            window(200, "shell", "pwsh"),
            window(300, "terminal", "pwsh"),
        ];
        assert_eq!(nearest_window(&chain(), &windows, None), Some("shell"));
    }

    #[test]
    fn a_process_with_several_windows_contributes_the_first_of_them() {
        let windows = [window(300, "main", "pwsh"), window(300, "second", "pwsh")];
        assert_eq!(nearest_window(&chain(), &windows, None), Some("main"));
    }

    #[test]
    fn a_chain_that_owns_nothing_answers_nothing() {
        let windows = [window(999, "somebody else", "")];
        assert_eq!(nearest_window(&chain(), &windows, None), None);
        assert_eq!(nearest_window::<&str>(&chain(), &[], None), None);
    }

    #[test]
    fn an_empty_chain_answers_nothing() {
        let windows = [window(300, "terminal", "pwsh")];
        assert_eq!(nearest_window(&[], &windows, Some("baton")), None);
    }

    #[test]
    fn of_an_editors_windows_the_one_naming_the_sessions_folder_is_taken() {
        // Every window of the editor belongs to its one main process (A-18, T-038), and the
        // most recently used one comes first in the z-order: without the title it would win.
        let windows = [
            window(3000, "the other window", "main.rs - shop - Cursor"),
            window(3000, "this window", "RUN-TASK.md - baton - Cursor"),
            window(400, "desktop", ""),
        ];
        assert_eq!(
            nearest_window(&editor_chain(), &windows, Some("baton")),
            Some("this window")
        );
        assert_eq!(
            nearest_window(&editor_chain(), &windows, Some("Baton")),
            Some("this window"),
            "a folder is compared without case"
        );
    }

    #[test]
    fn a_folder_no_title_names_leaves_the_first_window_as_it_was() {
        let windows = [
            window(3000, "first", "main.rs - shop - Cursor"),
            window(3000, "second", "notes.md - blog - Cursor"),
        ];
        assert_eq!(
            nearest_window(&editor_chain(), &windows, Some("baton")),
            Some("first")
        );
        assert_eq!(
            nearest_window(&editor_chain(), &windows, None),
            Some("first")
        );
    }

    #[test]
    fn a_title_never_pulls_the_walk_past_a_nearer_generation() {
        // The terminal is nearer than the desktop, whatever the desktop's windows are called.
        let windows = [
            window(400, "desktop", "baton"),
            window(300, "terminal", "Windows PowerShell"),
        ];
        assert_eq!(
            nearest_window(&chain(), &windows, Some("baton")),
            Some("terminal")
        );
    }

    #[test]
    fn a_folder_is_a_whole_part_of_the_title() {
        assert!(title_names_folder("main.rs - baton - Cursor", "baton"));
        assert!(title_names_folder("baton - Cursor", "BATON"));
        assert!(title_names_folder("baton", "baton"));
        assert!(title_names_folder(
            "\u{25cf} main.rs \u{2014} baton \u{2014} Visual Studio Code",
            "baton"
        ));
        assert!(!title_names_folder("main.rs - baton-app - Cursor", "baton"));
        assert!(!title_names_folder("main.rs - baton - Cursor", ""));
        assert!(!title_names_folder("Cursor", "baton"));
    }

    #[test]
    fn the_focus_of_a_build_that_has_none_says_so() {
        assert!(!NoTerminalFocus.focus(&chain(), Some("baton")));
    }
}
