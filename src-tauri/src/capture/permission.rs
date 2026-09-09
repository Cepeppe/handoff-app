//! The screen-recording permission, asked before a capture rather than during one (FM-17).
//!
//! CAP-04 requests the permission in onboarding, with an explanation and a deep link to the
//! Screen Recording pane, because granting it needs the application to be restarted. FM-17
//! is what happens when it was refused there, or revoked afterwards: the capture must not
//! reach the platform at all, because on macOS the first `CGDisplayCreateImage` of a process
//! without the permission is what raises the system prompt — in the middle of a handoff, on
//! top of the user's work, which is the one thing CAP-04 exists to prevent. So the flow asks
//! first and, when the answer is no, shows the explanation in the preview area instead.
//!
//! `CGPreflightScreenCaptureAccess` is the documented way to ask without prompting (macOS
//! 10.15 and later). It is declared here rather than pulled in with a Core Graphics binding
//! crate: it is one C function returning one boolean, and the crate that would provide it
//! is only in this tree at all as a transitive dependency of the capture library.
//!
//! Everywhere else there is no such permission and the answer is always [`Permission::Granted`]:
//! Windows lets any process of the session read the screen, which is why NFR-03's promise
//! there is ours to keep rather than the platform's.

/// Whether this process may read the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// It may. The only answer on Windows.
    Granted,
    /// macOS has not been given the screen-recording permission (FM-17).
    Denied,
}

impl Permission {
    /// Whether a capture may be attempted.
    #[must_use]
    pub fn is_granted(self) -> bool {
        self == Self::Granted
    }
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    /// Answers whether the calling process already has the permission, **without** asking
    /// the user for it. `CGRequestScreenCaptureAccess` is the one that prompts, and nothing
    /// in this application ever calls it: the user is sent to the settings pane instead,
    /// because the permission only takes effect after a restart (CAP-04).
    fn CGPreflightScreenCaptureAccess() -> bool;
}

/// Whether the screen may be read right now (CAP-04, FM-17).
#[must_use]
pub fn screen_recording() -> Permission {
    #[cfg(target_os = "macos")]
    {
        // Safe: the function takes no argument, returns a `bool`, and is present on every
        // macOS this application supports (§1.5 gives 12 as the floor, the call arrived in
        // 10.15).
        if unsafe { CGPreflightScreenCaptureAccess() } {
            Permission::Granted
        } else {
            Permission::Denied
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Permission::Granted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn there_is_no_such_permission_off_macos() {
        assert_eq!(screen_recording(), Permission::Granted);
        assert!(screen_recording().is_granted());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn the_preflight_answers_without_prompting() {
        // Whichever way it answers on the machine running the suite, the point is that it
        // answers: a call that raised the system prompt would hang a headless runner, which
        // is exactly the failure FM-17 is about.
        let answer = screen_recording();
        assert!(answer == Permission::Granted || answer == Permission::Denied);
    }
}
