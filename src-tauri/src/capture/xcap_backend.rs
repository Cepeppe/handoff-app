//! The production backend: `xcap` over the platform's own capture API (A-25).
//!
//! `xcap` is the one dependency of this module and it is a thin one: on Windows it is GDI
//! (`BitBlt` from the screen device context), on macOS `CGDisplayCreateImage`. It is asked
//! for its **default** features on purpose — `wgc` would swap the Windows path for
//! `Windows.Graphics.Capture`, which draws the operating system's yellow "this is being
//! recorded" border around every monitor it touches, and the promise of §7.8 is a capture
//! the user asked for and nothing else on screen.
//!
//! Every call opens what it needs and drops it before returning: [`XcapBackend`] is a
//! zero-sized type, and the suite in the parent module fails if it ever stops being one.

use image::RgbaImage;

use super::geometry::{Monitor, MonitorId, Rect};
use super::{Backend, CaptureError, Shot};

/// The screen, through `xcap`.
#[derive(Debug, Default, Clone, Copy)]
pub struct XcapBackend;

impl XcapBackend {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Everything `xcap` reports about one monitor, or the reason it would not.
///
/// `xcap` answers every property with a `Result` of its own, because each one is a separate
/// platform call. A monitor that cannot describe itself is dropped rather than guessed at:
/// a display unplugged between the enumeration and the query is the ordinary cause, and a
/// list without it is exactly right.
fn describe(monitor: &xcap::Monitor) -> Option<Monitor> {
    let width = monitor.width().ok()?;
    let height = monitor.height().ok()?;
    Some(Monitor {
        id: monitor.id().ok()?,
        name: monitor
            .name()
            .ok()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "?".to_owned()),
        bounds: Rect::new(monitor.x().ok()?, monitor.y().ok()?, width, height),
        scale_factor: f64::from(monitor.scale_factor().ok()?),
        is_primary: monitor.is_primary().unwrap_or(false),
    })
}

fn monitors() -> Result<Vec<(xcap::Monitor, Monitor)>, CaptureError> {
    let found: Vec<(xcap::Monitor, Monitor)> = xcap::Monitor::all()
        .map_err(|error| CaptureError::Backend(error.to_string()))?
        .into_iter()
        .filter_map(|monitor| describe(&monitor).map(|described| (monitor, described)))
        .collect();
    if found.is_empty() {
        return Err(CaptureError::NoMonitors);
    }
    Ok(found)
}

fn take(monitor: &xcap::Monitor, described: Monitor) -> Result<Shot, CaptureError> {
    let image: RgbaImage = monitor
        .capture_image()
        .map_err(|error| CaptureError::Backend(error.to_string()))?;
    Ok(Shot {
        monitor: described,
        image,
    })
}

impl Backend for XcapBackend {
    fn list_monitors(&self) -> Result<Vec<Monitor>, CaptureError> {
        Ok(monitors()?
            .into_iter()
            .map(|(_, described)| described)
            .collect())
    }

    fn capture_monitor(&self, id: MonitorId) -> Result<Shot, CaptureError> {
        let (monitor, described) = monitors()?
            .into_iter()
            .find(|(_, described)| described.id == id)
            .ok_or(CaptureError::UnknownMonitor(id))?;
        take(&monitor, described)
    }

    fn capture_all(&self) -> Result<Vec<Shot>, CaptureError> {
        monitors()?
            .into_iter()
            .map(|(monitor, described)| take(&monitor, described))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backend_is_a_handle_to_nothing() {
        // It is constructed per capture and dropped with it; a backend that could be
        // constructed once and kept is the shape PRIN-04 rules out.
        let backend = XcapBackend::new();
        assert_eq!(std::mem::size_of_val(&backend), 0);
    }

    #[test]
    fn a_real_monitor_describes_itself_completely() {
        // The one thing a unit test can honestly assert about a real screen: the
        // enumeration works and every monitor it returns has a geometry the composite can
        // use. `NoMonitors` is accepted rather than failed, because a runner without a
        // display is a fact about the machine and not about this code — and `describe`
        // dropping a monitor it could not read would show up here as an empty list on the
        // developer machine, which is the case worth catching.
        match XcapBackend::new().list_monitors() {
            Ok(monitors) => {
                assert!(!monitors.is_empty());
                for monitor in &monitors {
                    assert!(monitor.bounds.width > 0 && monitor.bounds.height > 0);
                    assert!(monitor.usable_scale() > 0.0);
                    assert!(!monitor.name.is_empty());
                }
            }
            Err(CaptureError::NoMonitors) => {}
            Err(other) => panic!("the platform refused to list its monitors: {other}"),
        }
    }
}
