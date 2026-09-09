//! Screen capture and the region selection overlay (§7.8, CAP-01..06, PRIN-04).
//!
//! Every capture is decided by the user, one at a time; nothing observes the screen
//! continuously (PRIN-04). The backend is a trait so that the unit tests and the e2e suite
//! can substitute a fixture image (`--features fake-capture`, `--features e2e`).
//!
//! # What is here and what is not
//!
//! This module produces one [`Capture`] per press of the Screenshot button and stops there.
//! It owns no window, no `AppHandle` and no state at all: the two-choice popover, the
//! hiding of the overlay (CAP-03), the transparent selection windows of DD-29 and the
//! preview that shows the result are `ui_bridge::capture`, because all four are Tauri. What
//! crosses between them is [`Selection`] going in and [`Capture`] coming out.
//!
//! The pixels then belong to the OCR of §7.9 (T-047) and the detector of §7.10 (T-048);
//! until those exist the preview draws the raw capture and nothing is sent anywhere.
//!
//! # Nothing runs between captures
//!
//! NFR-03 and PRIN-04 are not a promise about intent, they are a promise about the process:
//! between two screenshots there must be no thread, no timer and no handle to the screen.
//! So [`XcapBackend`] is a zero-sized type that opens the platform's capture API inside one
//! call and drops it before returning, and a test at the bottom of this file reads these
//! sources and fails on a spawn, a timer or a `static mut` appearing in them. The one wait
//! in the whole flow — the moment the compositor needs to actually remove the overlay after
//! it hides — is in `ui_bridge::capture`, where `tests/timers.rs` can see it.

pub mod geometry;
pub mod permission;
mod xcap_backend;

#[cfg(feature = "fake-capture")]
pub mod fake;

use image::RgbaImage;

use crate::log::time::Timestamp;

pub use geometry::{
    monitor_at, monitor_with, virtual_bounds, LogicalRect, Monitor, MonitorId, Point, Rect,
    Selection,
};
pub use xcap_backend::XcapBackend;

/// Why a capture did not happen.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The platform reported no monitor at all.
    #[error("no monitor answered")]
    NoMonitors,
    /// The selection named a monitor that is no longer there — a display unplugged while
    /// the overlay was open.
    #[error("monitor {0} is not there any more")]
    UnknownMonitor(MonitorId),
    /// The drag had no area, or fell entirely outside every monitor.
    #[error("the selected region is empty")]
    EmptyRegion,
    /// macOS has not been granted the screen-recording permission (FM-17, CAP-04).
    #[error("the screen-recording permission has not been granted")]
    PermissionDenied,
    /// Anything the platform's capture API refused.
    #[error("{0}")]
    Backend(String),
}

/// One monitor's pixels, with the geometry they were taken at.
#[derive(Debug, Clone)]
pub struct Shot {
    /// The monitor, as the backend described it at the moment of the capture.
    pub monitor: Monitor,
    /// Its pixels, top-left aligned with `monitor.bounds`.
    pub image: RgbaImage,
}

impl Shot {
    /// The part of the virtual desktop these pixels actually cover.
    ///
    /// The image is what is trusted, not the monitor's declared size: a backend that
    /// returns a buffer one row short of what `EnumDisplaySettings` promised must not make
    /// the composite read past its end.
    #[must_use]
    fn covered(&self) -> Rect {
        Rect::new(
            self.monitor.bounds.x,
            self.monitor.bounds.y,
            self.image.width().min(self.monitor.bounds.width),
            self.image.height().min(self.monitor.bounds.height),
        )
    }
}

/// What one press of the Screenshot button produced (§7.8).
#[derive(Debug, Clone)]
pub struct Capture {
    /// The pixels, at full resolution: OCR runs on these (CAP-05) and the downscale to
    /// 1600 px happens on the way out, in T-048's burn-in.
    pub image: RgbaImage,
    /// The monitor the capture came from, or the one the region's top-left corner was on.
    pub origin_monitor: MonitorId,
    /// When it was taken.
    pub taken_at: Timestamp,
}

impl Capture {
    /// The capture as a PNG, which is how it reaches the preview.
    ///
    /// # Errors
    ///
    /// [`CaptureError::Backend`] with the encoder's message. There is no useful recovery:
    /// the alternative to a PNG is showing the user nothing.
    pub fn to_png(&self) -> Result<Vec<u8>, CaptureError> {
        let mut png = Vec::new();
        self.image
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|error| CaptureError::Backend(error.to_string()))?;
        Ok(png)
    }
}

/// Where the pixels come from (§7.8).
///
/// Three calls and no lifetime of its own: an implementation may hold a fixture path, and
/// must not hold a connection to the screen between calls (PRIN-04).
pub trait Backend: Send + Sync {
    /// Every monitor, with its geometry and its scale factor.
    ///
    /// # Errors
    ///
    /// [`CaptureError::Backend`] when the platform refuses to enumerate them.
    fn list_monitors(&self) -> Result<Vec<Monitor>, CaptureError>;

    /// One monitor's pixels.
    ///
    /// # Errors
    ///
    /// [`CaptureError::UnknownMonitor`] when it is gone, [`CaptureError::Backend`] when the
    /// capture itself fails.
    fn capture_monitor(&self, id: MonitorId) -> Result<Shot, CaptureError>;

    /// Every monitor's pixels, for a region that may span several of them (CAP-02).
    ///
    /// # Errors
    ///
    /// [`CaptureError::NoMonitors`], or [`CaptureError::Backend`] from the first monitor
    /// that refuses.
    fn capture_all(&self) -> Result<Vec<Shot>, CaptureError>;
}

/// The monitor the cursor is on, whole (CAP-02).
///
/// # Errors
///
/// [`CaptureError::NoMonitors`] when the platform lists none, and whatever the backend
/// answers otherwise.
pub fn full_screen(backend: &dyn Backend, cursor: Point) -> Result<Capture, CaptureError> {
    let monitors = backend.list_monitors()?;
    let id = monitor_at(&monitors, cursor)
        .ok_or(CaptureError::NoMonitors)?
        .id;
    let shot = backend.capture_monitor(id)?;
    Ok(Capture {
        image: shot.image,
        origin_monitor: id,
        taken_at: Timestamp::now(),
    })
}

/// The rectangle the user dragged, cropped from the composite of every monitor (CAP-02).
///
/// # Errors
///
/// [`CaptureError::UnknownMonitor`] when the monitor the drag started on has gone,
/// [`CaptureError::EmptyRegion`] when the drag has no area or lands on no screen, and
/// whatever the backend answers otherwise.
pub fn region(backend: &dyn Backend, selection: Selection) -> Result<Capture, CaptureError> {
    let monitors = backend.list_monitors()?;
    let drawn_on = monitor_with(&monitors, selection.monitor)
        .ok_or(CaptureError::UnknownMonitor(selection.monitor))?;
    let wanted = selection
        .to_virtual(drawn_on)
        .ok_or(CaptureError::EmptyRegion)?;
    let desktop = virtual_bounds(&monitors).ok_or(CaptureError::NoMonitors)?;
    // A drag can run past the edge of the desktop — the pointer is captured, so the last
    // position reported is wherever the mouse was, not wherever a screen is. The part that
    // is on a screen is what the user selected.
    let cropped = wanted
        .intersection(&desktop)
        .ok_or(CaptureError::EmptyRegion)?;

    let shots = backend.capture_all()?;
    let origin = monitor_at(&monitors, Point::new(cropped.x, cropped.y))
        .map_or(selection.monitor, |monitor| monitor.id);
    Ok(Capture {
        image: compose(&shots, cropped),
        origin_monitor: origin,
        taken_at: Timestamp::now(),
    })
}

/// The pixels of `region`, taken from whichever monitors cover it.
///
/// Anything no monitor covers — the gap between two screens of different heights, which the
/// virtual desktop's bounding box includes and no screen fills — stays as the transparent
/// black the buffer starts as. That is the honest answer: there is nothing there to show.
fn compose(shots: &[Shot], region: Rect) -> RgbaImage {
    let mut canvas = RgbaImage::new(region.width, region.height);
    let canvas_width = region.width as usize;
    for shot in shots {
        let covered = shot.covered();
        let Some(overlap) = covered.intersection(&region) else {
            continue;
        };
        let source_width = shot.image.width() as usize;
        let source = shot.image.as_raw();
        let target: &mut [u8] = &mut canvas;
        let (Ok(source_x), Ok(source_y), Ok(target_x), Ok(target_y)) = (
            usize::try_from(overlap.x - covered.x),
            usize::try_from(overlap.y - covered.y),
            usize::try_from(overlap.x - region.x),
            usize::try_from(overlap.y - region.y),
        ) else {
            continue;
        };
        let span = overlap.width as usize * 4;
        for row in 0..overlap.height as usize {
            let from = ((source_y + row) * source_width + source_x) * 4;
            let to = ((target_y + row) * canvas_width + target_x) * 4;
            target[to..to + span].copy_from_slice(&source[from..from + span]);
        }
    }
    canvas
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use image::Rgba;

    use super::*;

    /// A backend over a layout written down here, filling each monitor with one colour so
    /// that a composed pixel says which screen it came from.
    struct Painted {
        monitors: Vec<Monitor>,
        colours: Vec<Rgba<u8>>,
    }

    impl Painted {
        fn shot(&self, index: usize) -> Shot {
            let monitor = self.monitors[index].clone();
            let image = RgbaImage::from_pixel(
                monitor.bounds.width,
                monitor.bounds.height,
                self.colours[index],
            );
            Shot { monitor, image }
        }
    }

    impl Backend for Painted {
        fn list_monitors(&self) -> Result<Vec<Monitor>, CaptureError> {
            Ok(self.monitors.clone())
        }

        fn capture_monitor(&self, id: MonitorId) -> Result<Shot, CaptureError> {
            let index = self
                .monitors
                .iter()
                .position(|monitor| monitor.id == id)
                .ok_or(CaptureError::UnknownMonitor(id))?;
            Ok(self.shot(index))
        }

        fn capture_all(&self) -> Result<Vec<Shot>, CaptureError> {
            Ok((0..self.monitors.len()).map(|i| self.shot(i)).collect())
        }
    }

    fn monitor(id: MonitorId, bounds: Rect, scale: f64, primary: bool) -> Monitor {
        Monitor {
            id,
            name: format!("monitor {id}"),
            bounds,
            scale_factor: scale,
            is_primary: primary,
        }
    }

    /// Two screens side by side at different scale factors, the left one at a negative
    /// origin: the layout every mapping mistake shows up in.
    fn two_screens() -> Painted {
        Painted {
            monitors: vec![
                monitor(1, Rect::new(0, 0, 400, 300), 1.25, true),
                monitor(2, Rect::new(-200, 0, 200, 300), 1.0, false),
            ],
            colours: vec![Rgba([10, 20, 30, 255]), Rgba([200, 100, 50, 255])],
        }
    }

    #[test]
    fn full_screen_takes_the_monitor_under_the_cursor() {
        let backend = two_screens();
        let capture = full_screen(&backend, Point::new(-100, 40)).expect("a capture");
        assert_eq!(capture.origin_monitor, 2);
        assert_eq!(capture.image.dimensions(), (200, 300));
        assert_eq!(*capture.image.get_pixel(0, 0), Rgba([200, 100, 50, 255]));
    }

    #[test]
    fn a_region_inside_one_screen_is_that_screens_pixels() {
        let backend = two_screens();
        let capture = region(
            &backend,
            Selection {
                monitor: 1,
                rect: LogicalRect {
                    x: 8.0,
                    y: 8.0,
                    width: 80.0,
                    height: 40.0,
                },
            },
        )
        .expect("a capture");
        // 125 %: 80 × 40 CSS pixels are 100 × 50 physical ones.
        assert_eq!(capture.image.dimensions(), (100, 50));
        assert_eq!(capture.origin_monitor, 1);
        assert_eq!(*capture.image.get_pixel(0, 0), Rgba([10, 20, 30, 255]));
    }

    #[test]
    fn a_region_spanning_two_screens_carries_pixels_from_both() {
        // Dragged on the primary screen, leftwards past its origin and onto the second one.
        // The composite has to place each screen's pixels at its own offset, and the seam
        // has to fall exactly at x = 0 of the virtual desktop.
        let backend = two_screens();
        let capture = region(
            &backend,
            Selection {
                monitor: 1,
                rect: LogicalRect {
                    x: -80.0,
                    y: 0.0,
                    width: 160.0,
                    height: 40.0,
                },
            },
        )
        .expect("a capture");
        assert_eq!(capture.image.dimensions(), (200, 50));
        assert_eq!(
            capture.origin_monitor, 2,
            "the top-left corner is on the second screen"
        );
        assert_eq!(
            *capture.image.get_pixel(0, 0),
            Rgba([200, 100, 50, 255]),
            "the left half comes from the second screen"
        );
        assert_eq!(
            *capture.image.get_pixel(99, 0),
            Rgba([200, 100, 50, 255]),
            "up to the seam"
        );
        assert_eq!(
            *capture.image.get_pixel(100, 0),
            Rgba([10, 20, 30, 255]),
            "and the right half from the primary one"
        );
    }

    #[test]
    fn the_part_of_a_drag_that_is_on_no_screen_is_dropped() {
        // The desktop is 600 × 300 here; a drag that runs off the right edge keeps only
        // what a screen actually covers, rather than producing a band of nothing.
        let backend = two_screens();
        let capture = region(
            &backend,
            Selection {
                monitor: 1,
                rect: LogicalRect {
                    x: 280.0,
                    y: 0.0,
                    width: 200.0,
                    height: 40.0,
                },
            },
        )
        .expect("a capture");
        assert_eq!(capture.image.dimensions(), (50, 50));
    }

    #[test]
    fn a_region_that_lands_on_no_screen_at_all_is_refused() {
        let backend = two_screens();
        let error = region(
            &backend,
            Selection {
                monitor: 1,
                rect: LogicalRect {
                    x: 4_000.0,
                    y: 0.0,
                    width: 100.0,
                    height: 40.0,
                },
            },
        )
        .expect_err("nothing is there");
        assert!(matches!(error, CaptureError::EmptyRegion));
    }

    #[test]
    fn a_drag_on_a_monitor_that_has_gone_names_it() {
        let backend = two_screens();
        let error = region(
            &backend,
            Selection {
                monitor: 9,
                rect: LogicalRect {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
            },
        )
        .expect_err("no monitor 9");
        assert!(matches!(error, CaptureError::UnknownMonitor(9)));
    }

    #[test]
    fn a_gap_between_two_screens_of_different_heights_stays_empty() {
        // The bounding box of the desktop includes rows no screen fills. Composing them as
        // whatever was last in the buffer would put another window's pixels there.
        let backend = Painted {
            monitors: vec![
                monitor(1, Rect::new(0, 0, 100, 200), 1.0, true),
                monitor(2, Rect::new(100, 0, 100, 100), 1.0, false),
            ],
            colours: vec![Rgba([1, 2, 3, 255]), Rgba([4, 5, 6, 255])],
        };
        let capture = region(
            &backend,
            Selection {
                monitor: 1,
                rect: LogicalRect {
                    x: 0.0,
                    y: 0.0,
                    width: 200.0,
                    height: 200.0,
                },
            },
        )
        .expect("a capture");
        assert_eq!(capture.image.dimensions(), (200, 200));
        assert_eq!(*capture.image.get_pixel(150, 50), Rgba([4, 5, 6, 255]));
        assert_eq!(
            *capture.image.get_pixel(150, 150),
            Rgba([0, 0, 0, 0]),
            "no screen covers this, so nothing is drawn there"
        );
    }

    #[test]
    fn a_backend_that_returns_fewer_pixels_than_it_promised_is_not_read_past() {
        // The failure this guards is a panic inside the composite, on a machine nobody can
        // reproduce, because a driver answered one row short.
        let short = Painted {
            monitors: vec![monitor(1, Rect::new(0, 0, 100, 100), 1.0, true)],
            colours: vec![Rgba([9, 9, 9, 255])],
        };
        let mut shot = short.shot(0);
        shot.image = RgbaImage::from_pixel(100, 60, Rgba([9, 9, 9, 255]));
        assert_eq!(shot.covered(), Rect::new(0, 0, 100, 60));
        let composed = compose(&[shot], Rect::new(0, 0, 100, 100));
        assert_eq!(composed.dimensions(), (100, 100));
        assert_eq!(*composed.get_pixel(0, 99), Rgba([0, 0, 0, 0]));
    }

    #[test]
    fn a_capture_encodes_as_a_png() {
        let capture = Capture {
            image: RgbaImage::from_pixel(4, 4, Rgba([1, 2, 3, 255])),
            origin_monitor: 1,
            taken_at: Timestamp::now(),
        };
        let png = capture.to_png().expect("a PNG");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }

    /// The shipped part of this module's sources: everything before the test module.
    ///
    /// The cut matters here more than usual, because the rule below is a list of the very
    /// shapes it forbids — without it this test would fail on itself, which is the one way
    /// a source-reading check can be red for a reason nobody can fix.
    fn module_sources() -> Vec<(PathBuf, String)> {
        let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/capture");
        let mut found = Vec::new();
        for entry in fs::read_dir(&here).expect("the capture module is a directory") {
            let path = entry.expect("a directory entry").path();
            if path.extension().is_some_and(|ext| ext == "rs") {
                let text = fs::read_to_string(&path).expect("a source file reads");
                let shipped = text
                    .split_once("#[cfg(test)]")
                    .map_or(text.clone(), |(before, _)| before.to_owned());
                found.push((path, shipped));
            }
        }
        assert!(
            found.len() >= 4,
            "the scan found only {} files",
            found.len()
        );
        assert!(
            found
                .iter()
                .any(|(path, text)| path.ends_with("mod.rs") && !text.contains("thread::spawn")),
            "the cut at the test module did not happen, so this suite reads its own rule"
        );
        found
    }

    #[test]
    fn nothing_here_outlives_a_capture() {
        // The acceptance criterion of this task, and PRIN-04 with it: between two
        // screenshots the process holds nothing that watches the screen. A backend with a
        // handle, a worker thread draining frames or a cached monitor list would each be
        // invisible to every other test in this file — they would all still pass.
        let forbidden = [
            "thread::spawn",
            "tokio::spawn",
            "tauri::async_runtime::spawn",
            "tokio::time::",
            "thread::sleep",
            "static mut",
            "OnceLock",
            "lazy_static",
        ];
        let mut found = Vec::new();
        for (path, text) in module_sources() {
            for line in text.lines() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                for shape in forbidden {
                    if code.contains(shape) {
                        found.push(format!("{}: {code}", path.display()));
                    }
                }
            }
        }
        assert!(
            found.is_empty(),
            "the capture module must hold nothing between two captures (PRIN-04, NFR-03):\n{}",
            found.join("\n")
        );
    }

    #[test]
    fn the_production_backend_carries_no_state() {
        // The other half of the same rule, from the type system: a backend that could hold
        // a handle is a backend that will, one refactor later.
        assert_eq!(std::mem::size_of::<XcapBackend>(), 0);
    }
}
