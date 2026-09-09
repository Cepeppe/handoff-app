//! A backend that returns a file instead of the screen (`--features fake-capture`).
//!
//! It exists for the e2e suite of §11.5. A harness with nobody at the keyboard cannot drag
//! a rectangle across a screen it cannot see, and a scenario that asserted anything about
//! the pixels of the machine it happens to run on would be a scenario that fails on the
//! next machine — so E2E-3 plants a fixture with a fake key in it and drives the rest of
//! the pipeline over that. The `e2e` feature enables this one; nothing else does, and a
//! release build has neither.
//!
//! The fixture is one image standing in for one monitor at the origin, whatever the machine
//! actually has: the region flow then crops from it exactly as it would from a real screen,
//! including the scale factor, which the harness sets to reproduce a high-DPI capture on a
//! runner that has none.

use std::path::{Path, PathBuf};

use image::RgbaImage;

use super::geometry::{Monitor, MonitorId, Rect};
use super::{Backend, CaptureError, Shot};

/// The id of the one monitor a fixture stands for.
pub const FIXTURE_MONITOR: MonitorId = 1;

/// The settings key the e2e channel writes to choose the fixture (`settings` `set`).
///
/// It is a settings row rather than an injected value like `e2e.verifying_timeout_ms`
/// because the choice has to survive the window reload a scenario may cause, and because
/// the settings table is the door the automation channel already has (§11.5).
pub const FIXTURE_KEY: &str = "e2e.capture_fixture";

/// The settings key that gives the fixture a scale factor, so a scenario can reproduce a
/// high-DPI screen on a runner that has none. Absent means 1.0.
pub const FIXTURE_SCALE_KEY: &str = "e2e.capture_fixture_scale";

/// A capture backend reading one PNG from disk.
#[derive(Debug, Clone)]
pub struct FakeCapture {
    fixture: PathBuf,
    scale_factor: f64,
}

impl FakeCapture {
    /// A backend answering with the PNG at `fixture`.
    #[must_use]
    pub fn new(fixture: impl AsRef<Path>, scale_factor: f64) -> Self {
        Self {
            fixture: fixture.as_ref().to_path_buf(),
            scale_factor,
        }
    }

    /// The fixture's pixels, read on every call: a backend that cached them would be state
    /// held between two captures, which the parent module forbids.
    fn read(&self) -> Result<RgbaImage, CaptureError> {
        let bytes = std::fs::read(&self.fixture).map_err(|error| {
            CaptureError::Backend(format!("{}: {error}", self.fixture.display()))
        })?;
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).map_err(
            |error| CaptureError::Backend(format!("{}: {error}", self.fixture.display())),
        )?;
        Ok(image.to_rgba8())
    }

    fn shot(&self) -> Result<Shot, CaptureError> {
        let image = self.read()?;
        Ok(Shot {
            monitor: Monitor {
                id: FIXTURE_MONITOR,
                name: self.fixture.display().to_string(),
                bounds: Rect::new(0, 0, image.width(), image.height()),
                scale_factor: self.scale_factor,
                is_primary: true,
            },
            image,
        })
    }
}

impl Backend for FakeCapture {
    fn list_monitors(&self) -> Result<Vec<Monitor>, CaptureError> {
        Ok(vec![self.shot()?.monitor])
    }

    fn capture_monitor(&self, id: MonitorId) -> Result<Shot, CaptureError> {
        if id != FIXTURE_MONITOR {
            return Err(CaptureError::UnknownMonitor(id));
        }
        self.shot()
    }

    fn capture_all(&self) -> Result<Vec<Shot>, CaptureError> {
        Ok(vec![self.shot()?])
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use image::Rgba;

    use super::super::{full_screen, region, LogicalRect, Point, Selection};
    use super::*;

    /// A PNG of this test binary's own, removed when it is dropped.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(width: u32, height: u32) -> Self {
            let path = std::env::temp_dir().join(format!(
                "handoff-capture-{}-{}.png",
                std::process::id(),
                crate::ids::new_session_ref()
            ));
            let mut image = RgbaImage::from_pixel(width, height, Rgba([7, 8, 9, 255]));
            // One marked pixel, so a crop can say which part of the fixture it took.
            image.put_pixel(3, 2, Rgba([250, 0, 0, 255]));
            image
                .save_with_format(&path, image::ImageFormat::Png)
                .expect("the fixture is written");
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn the_fixture_is_what_a_full_screen_capture_returns() {
        let fixture = Fixture::new(40, 20);
        let backend = FakeCapture::new(&fixture.0, 1.0);
        let capture = full_screen(&backend, Point::new(0, 0)).expect("a capture");
        assert_eq!(capture.image.dimensions(), (40, 20));
        assert_eq!(capture.origin_monitor, FIXTURE_MONITOR);
        assert_eq!(*capture.image.get_pixel(3, 2), Rgba([250, 0, 0, 255]));
    }

    #[test]
    fn a_region_is_cropped_from_the_fixture_at_the_declared_scale() {
        let fixture = Fixture::new(40, 20);
        let backend = FakeCapture::new(&fixture.0, 2.0);
        let capture = region(
            &backend,
            Selection {
                monitor: FIXTURE_MONITOR,
                rect: LogicalRect {
                    x: 1.0,
                    y: 1.0,
                    width: 4.0,
                    height: 2.0,
                },
            },
        )
        .expect("a capture");
        // At 200 % a 4 × 2 CSS rectangle at (1, 1) is 8 × 4 pixels at (2, 2), so the marked
        // pixel of the fixture lands at (1, 0) of the crop.
        assert_eq!(capture.image.dimensions(), (8, 4));
        assert_eq!(*capture.image.get_pixel(1, 0), Rgba([250, 0, 0, 255]));
    }

    #[test]
    fn a_fixture_that_is_not_there_says_so_rather_than_capturing_the_screen() {
        let backend = FakeCapture::new("no-such-fixture.png", 1.0);
        let error = backend.list_monitors().expect_err("there is no such file");
        assert!(matches!(error, CaptureError::Backend(_)));
    }
}
