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
//! useless and rude. [`nearest_owner`] is that rule, and it is the part worth a test — the
//! platform half cannot be exercised without a display.
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
    /// `chain` is the session's completed ancestor chain, nearest generation first.
    ///
    /// Returns whether a window was actually raised. `false` is an ordinary answer, not an
    /// error: the caller notifies and the hook delivers anyway (FM-21, OPEN-06).
    fn focus(&self, chain: &[AncestorProcess]) -> bool;
}

/// The focus of a build that has none: every unit test, and any platform without an
/// implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoTerminalFocus;

impl TerminalFocus for NoTerminalFocus {
    fn focus(&self, _chain: &[AncestorProcess]) -> bool {
        false
    }
}

/// The platform's own implementation: Win32 on Windows, AppKit on macOS, nothing elsewhere.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlatformFocus;

impl TerminalFocus for PlatformFocus {
    fn focus(&self, chain: &[AncestorProcess]) -> bool {
        #[cfg(windows)]
        {
            windows_focus::raise(chain)
        }
        #[cfg(target_os = "macos")]
        {
            macos_focus::raise(chain)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = chain;
            false
        }
    }
}

/// The first generation of `chain` that owns something, and what it owns.
///
/// `owners` is what the platform found, as `(pid, thing)` pairs in whatever order it
/// enumerated them; a pid may appear more than once (a process with several windows) and the
/// first entry for the winning pid is the one taken.
///
/// The walk is over the **chain**, not over the owners: the order that decides is "how near
/// is this process to the agent", and the platform's enumeration order says nothing about
/// that. A chain whose nearest generations own nothing falls through to the next one, which
/// is how a terminal that hosts the shell as a child is reached at all.
pub fn nearest_owner<T: Clone>(chain: &[AncestorProcess], owners: &[(u32, T)]) -> Option<T> {
    chain.iter().find_map(|ancestor| {
        owners
            .iter()
            .find(|(pid, _)| *pid == ancestor.pid)
            .map(|(_, owned)| owned.clone())
    })
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
        AllowSetForegroundWindow, EnumWindows, GetForegroundWindow, GetWindowThreadProcessId,
        IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    use crate::format::channel::AncestorProcess;

    /// One visible top-level window and the process that owns it.
    type Owner = (u32, isize);

    pub(super) fn raise(chain: &[AncestorProcess]) -> bool {
        let owners = visible_windows();
        let Some(handle) = super::nearest_owner(chain, &owners) else {
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
}

/// AppKit: the nearest chain member that is a running application with windows, activated.
///
/// `NSRunningApplication` only knows about applications, not about every process, so the
/// shell and the agent simply answer `nil` and the walk continues to the terminal that hosts
/// them — which is the rule of [`super::nearest_owner`] arriving at the same place from the
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

    #[test]
    fn the_nearest_generation_that_owns_a_window_is_the_one_taken() {
        // The terminal owns a window and so does the desktop shell; the terminal is nearer.
        let owners = [(400_u32, "desktop"), (300, "terminal")];
        assert_eq!(nearest_owner(&chain(), &owners), Some("terminal"));
    }

    #[test]
    fn the_desktop_shell_is_never_reached_while_something_nearer_owns_a_window() {
        // The failure this guards is the one that makes the feature worse than useless: a
        // walk from the far end activates `explorer.exe` for every session of the machine.
        let owners = [(400_u32, "desktop"), (200, "shell"), (300, "terminal")];
        assert_eq!(nearest_owner(&chain(), &owners), Some("shell"));
    }

    #[test]
    fn a_process_with_several_windows_contributes_the_first_of_them() {
        let owners = [(300_u32, "main"), (300, "second")];
        assert_eq!(nearest_owner(&chain(), &owners), Some("main"));
    }

    #[test]
    fn a_chain_that_owns_nothing_answers_nothing() {
        let owners = [(999_u32, "somebody else")];
        assert_eq!(nearest_owner(&chain(), &owners), None);
        assert_eq!(nearest_owner::<&str>(&chain(), &[]), None);
    }

    #[test]
    fn an_empty_chain_answers_nothing() {
        let owners = [(300_u32, "terminal")];
        assert_eq!(nearest_owner(&[], &owners), None);
    }

    #[test]
    fn the_focus_of_a_build_that_has_none_says_so() {
        assert!(!NoTerminalFocus.focus(&chain()));
    }
}
