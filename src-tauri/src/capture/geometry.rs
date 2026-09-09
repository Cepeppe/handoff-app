//! Where the monitors are, and what a selection on one of them means on the others.
//!
//! Everything here is arithmetic over integers and a scale factor: no platform call, no
//! image, nothing to mock. That is deliberate. The two facts CAP-02 rests on — "the monitor
//! where the cursor is" and "the region is cropped from the composite of all monitors" —
//! are exactly the ones a machine with one screen can never exercise, and A-25 lists the
//! multi-monitor geometry as the assumption to verify. So the geometry is a set of pure
//! functions with the awkward layouts written down as tests: a secondary screen to the left
//! of the primary one (negative origins), two screens at different scale factors, a
//! selection dragged off one screen and onto another.
//!
//! # The coordinate space
//!
//! One space, used everywhere in this module and in `capture`: the **virtual desktop in
//! physical pixels**. Each monitor occupies a rectangle in it, the primary one does not
//! have to be at the origin, and coordinates left of or above the primary monitor are
//! negative. It is the space `xcap` reports monitors in, the space Tauri places a window in
//! with a `PhysicalPosition`, and the space a captured image's pixels are in — so nothing
//! in the capture path ever converts between two of them.
//!
//! The one place another unit appears is the selection overlay, whose webview measures a
//! drag in **CSS pixels of its own window**. [`Selection::to_virtual`] is the only
//! conversion, it happens on this side, and its unit tests are the reason it is written
//! here rather than in the frontend.

use serde::{Deserialize, Serialize};

/// The identifier a backend gives a monitor. Opaque: only equality is ever asked of it.
pub type MonitorId = u32;

/// A point on the virtual desktop, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// A rectangle on the virtual desktop, in physical pixels.
///
/// The size is unsigned: an empty rectangle is one with a zero side, and there is no such
/// thing as a negative one. Callers that compute edges use [`Rect::between`], which returns
/// `None` rather than an inside-out rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The rectangle spanning two corners, in whichever order they were given.
    ///
    /// `None` when the two corners share a row or a column: a drag that never moved is not
    /// a region, and a zero-pixel image is refused by every encoder downstream.
    #[must_use]
    pub fn between(a: Point, b: Point) -> Option<Self> {
        let left = a.x.min(b.x);
        let top = a.y.min(b.y);
        let width = u32::try_from(a.x.max(b.x) - left).ok()?;
        let height = u32::try_from(a.y.max(b.y) - top).ok()?;
        (width > 0 && height > 0).then_some(Self::new(left, top, width, height))
    }

    /// The first column beyond the rectangle.
    #[must_use]
    pub fn right(&self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    /// The first row beyond the rectangle.
    #[must_use]
    pub fn bottom(&self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    /// Whether the point is inside, the right and bottom edges being outside.
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        i64::from(point.x) >= i64::from(self.x)
            && i64::from(point.x) < self.right()
            && i64::from(point.y) >= i64::from(self.y)
            && i64::from(point.y) < self.bottom()
    }

    /// The part of `self` that is also in `other`, or `None` when they do not overlap.
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        let width = u32::try_from(right - i64::from(left)).ok()?;
        let height = u32::try_from(bottom - i64::from(top)).ok()?;
        (width > 0 && height > 0).then_some(Self::new(left, top, width, height))
    }

    /// The smallest rectangle holding both.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self::new(
            left,
            top,
            u32::try_from(right - i64::from(left)).unwrap_or(u32::MAX),
            u32::try_from(bottom - i64::from(top)).unwrap_or(u32::MAX),
        )
    }
}

/// One monitor as a backend reports it (§7.8: geometry and scale factor).
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    /// The backend's own identifier.
    pub id: MonitorId,
    /// The name the platform gives it, for the log and for a picker that may never exist.
    pub name: String,
    /// Where it is on the virtual desktop, in physical pixels.
    pub bounds: Rect,
    /// Physical pixels per logical pixel. Never zero, never negative, never `NaN`.
    pub scale_factor: f64,
    /// Whether the platform calls it the primary one.
    pub is_primary: bool,
}

impl Monitor {
    /// A scale factor that can be multiplied by.
    ///
    /// A backend that fails to read the DPI reports something unusable rather than nothing
    /// — `0.0` on the paths that divide, `NaN` on the ones that do not check. Both would
    /// turn a selection into an empty or an infinite rectangle, so they become 1.0 here,
    /// which maps CSS pixels onto physical ones and is exactly right on an unscaled screen.
    #[must_use]
    pub fn usable_scale(&self) -> f64 {
        if self.scale_factor.is_finite() && self.scale_factor > 0.0 {
            self.scale_factor
        } else {
            1.0
        }
    }
}

/// A rectangle as the selection overlay measured it: CSS pixels of one monitor's window.
///
/// The values are `f64` and may be negative or beyond the window, because a drag that
/// leaves the monitor it started on keeps being reported in the coordinates of that window
/// (the pointer is captured). That is not an error to reject: it is a selection spanning
/// two monitors, which CAP-02 asks for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LogicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What the user dragged, and on which monitor's overlay they dragged it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    /// The monitor whose overlay window reported the drag.
    pub monitor: MonitorId,
    /// The rectangle, in that window's CSS pixels.
    pub rect: LogicalRect,
}

impl Selection {
    /// The selection on the virtual desktop, in physical pixels.
    ///
    /// The conversion is `origin + css × scale` with the **drawn-on** monitor's scale, and
    /// that is right even for the part of the drag that left the monitor: the overlay's CSS
    /// space extends linearly past its own edges, so a point 100 CSS pixels beyond the
    /// right edge of a 150 % screen is 150 physical pixels beyond it on the virtual desktop
    /// — whatever the scale of the screen that happens to be there.
    ///
    /// `None` when the drag has no area, which is what a click without a drag produces.
    #[must_use]
    pub fn to_virtual(&self, monitor: &Monitor) -> Option<Rect> {
        let scale = monitor.usable_scale();
        let origin = |value: f64, base: i32| -> Option<i32> {
            let scaled = (value * scale).round();
            if !scaled.is_finite() {
                return None;
            }
            i64::from(base)
                .checked_add(scaled as i64)
                .and_then(|sum| i32::try_from(sum).ok())
        };
        let left = origin(self.rect.x, monitor.bounds.x)?;
        let top = origin(self.rect.y, monitor.bounds.y)?;
        let right = origin(self.rect.x + self.rect.width, monitor.bounds.x)?;
        let bottom = origin(self.rect.y + self.rect.height, monitor.bounds.y)?;
        Rect::between(Point::new(left, top), Point::new(right, bottom))
    }
}

/// The rectangle every monitor fits into, or `None` when there are no monitors.
#[must_use]
pub fn virtual_bounds(monitors: &[Monitor]) -> Option<Rect> {
    monitors
        .iter()
        .map(|monitor| monitor.bounds)
        .reduce(|whole, one| whole.union(&one))
}

/// The monitor the point is on (CAP-02).
///
/// The first that contains it, so an overlap — which Windows allows while a display is
/// being rearranged — resolves to one monitor rather than to none. When the point is on no
/// monitor at all, which a stale cursor position between two hot-plug events can produce,
/// the answer is the primary one, and the first one if the platform names no primary: a
/// full-screen capture of *some* screen is the degraded answer, and no capture at all is
/// not (PRIN-10).
#[must_use]
pub fn monitor_at(monitors: &[Monitor], point: Point) -> Option<&Monitor> {
    monitors
        .iter()
        .find(|monitor| monitor.bounds.contains(point))
        .or_else(|| monitors.iter().find(|monitor| monitor.is_primary))
        .or_else(|| monitors.first())
}

/// The monitor with that id.
#[must_use]
pub fn monitor_with(monitors: &[Monitor], id: MonitorId) -> Option<&Monitor> {
    monitors.iter().find(|monitor| monitor.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout this machine has: one 2560×1440 screen at 125 %.
    fn primary() -> Monitor {
        Monitor {
            id: 1,
            name: "\\\\.\\DISPLAY1".to_owned(),
            bounds: Rect::new(0, 0, 2560, 1440),
            scale_factor: 1.25,
            is_primary: true,
        }
    }

    /// A second screen to the **left** of it, unscaled: the case that produces negative
    /// origins, and the one a single-screen machine can never meet.
    fn to_the_left() -> Monitor {
        Monitor {
            id: 2,
            name: "\\\\.\\DISPLAY2".to_owned(),
            bounds: Rect::new(-1920, -120, 1920, 1080),
            scale_factor: 1.0,
            is_primary: false,
        }
    }

    #[test]
    fn a_drag_that_never_moved_is_not_a_region() {
        assert_eq!(Rect::between(Point::new(4, 9), Point::new(4, 90)), None);
        assert_eq!(Rect::between(Point::new(4, 9), Point::new(40, 9)), None);
    }

    #[test]
    fn a_rectangle_is_the_same_whichever_corner_was_grabbed_first() {
        let down_right = Rect::between(Point::new(10, 20), Point::new(110, 220));
        let up_left = Rect::between(Point::new(110, 220), Point::new(10, 20));
        assert_eq!(down_right, Some(Rect::new(10, 20, 100, 200)));
        assert_eq!(up_left, down_right);
    }

    #[test]
    fn the_virtual_desktop_covers_a_screen_placed_left_of_the_primary_one() {
        // The union has to start at the negative origin, not at 0: a composite allocated
        // from a bounds box that begins at the primary monitor would drop every pixel of
        // the second screen and would do it silently.
        let bounds = virtual_bounds(&[primary(), to_the_left()]).expect("two monitors");
        assert_eq!(bounds, Rect::new(-1920, -120, 4480, 1560));
        assert_eq!(virtual_bounds(&[]), None);
    }

    #[test]
    fn the_cursor_names_the_monitor_it_is_on() {
        let monitors = [primary(), to_the_left()];
        assert_eq!(
            monitor_at(&monitors, Point::new(-4, -4)).map(|monitor| monitor.id),
            Some(2)
        );
        assert_eq!(
            monitor_at(&monitors, Point::new(0, 0)).map(|monitor| monitor.id),
            Some(1)
        );
        // The right edge belongs to the next screen, not to this one.
        assert_eq!(
            monitor_at(&monitors, Point::new(2560, 0)).map(|monitor| monitor.id),
            Some(1),
            "a point on no monitor falls back to the primary one"
        );
        assert_eq!(monitor_at(&[], Point::new(0, 0)), None);
    }

    #[test]
    fn a_cursor_on_no_monitor_falls_back_to_the_first_when_none_is_primary() {
        let mut only = to_the_left();
        only.is_primary = false;
        assert_eq!(
            monitor_at(&[only], Point::new(9_000, 9_000)).map(|monitor| monitor.id),
            Some(2)
        );
    }

    #[test]
    fn a_selection_is_scaled_by_the_monitor_it_was_drawn_on() {
        // 125 %: a 400 × 200 CSS rectangle at (80, 40) is 500 × 250 physical pixels at
        // (100, 50). Getting this wrong is invisible on an unscaled screen and cuts a fifth
        // off every capture on this one.
        let selection = Selection {
            monitor: 1,
            rect: LogicalRect {
                x: 80.0,
                y: 40.0,
                width: 400.0,
                height: 200.0,
            },
        };
        assert_eq!(
            selection.to_virtual(&primary()),
            Some(Rect::new(100, 50, 500, 250))
        );
    }

    #[test]
    fn a_selection_on_a_screen_with_a_negative_origin_lands_on_it() {
        let selection = Selection {
            monitor: 2,
            rect: LogicalRect {
                x: 10.0,
                y: 10.0,
                width: 100.0,
                height: 50.0,
            },
        };
        assert_eq!(
            selection.to_virtual(&to_the_left()),
            Some(Rect::new(-1910, -110, 100, 50))
        );
    }

    #[test]
    fn a_drag_that_left_the_monitor_keeps_that_monitors_scale() {
        // Pointer capture keeps reporting in the window that took the press, so the
        // rectangle runs past the right edge of the 125 % screen. The part beyond it is
        // still `origin + css × 1.25`, because that window's CSS space is what the numbers
        // are in — reaching for the neighbour's scale factor is the mistake this pins.
        let selection = Selection {
            monitor: 1,
            rect: LogicalRect {
                x: 2000.0,
                y: 100.0,
                width: 400.0,
                height: 100.0,
            },
        };
        assert_eq!(
            selection.to_virtual(&primary()),
            Some(Rect::new(2500, 125, 500, 125))
        );
    }

    #[test]
    fn a_drag_towards_the_top_left_is_the_same_rectangle() {
        let selection = Selection {
            monitor: 1,
            rect: LogicalRect {
                x: 480.0,
                y: 240.0,
                width: -400.0,
                height: -200.0,
            },
        };
        assert_eq!(
            selection.to_virtual(&primary()),
            Some(Rect::new(100, 50, 500, 250))
        );
    }

    #[test]
    fn a_click_without_a_drag_selects_nothing() {
        let selection = Selection {
            monitor: 1,
            rect: LogicalRect {
                x: 10.0,
                y: 10.0,
                width: 0.0,
                height: 0.4,
            },
        };
        assert_eq!(selection.to_virtual(&primary()), None);
    }

    #[test]
    fn a_scale_factor_the_platform_could_not_read_maps_one_to_one() {
        let mut broken = primary();
        broken.scale_factor = 0.0;
        assert_eq!(broken.usable_scale(), 1.0);
        broken.scale_factor = f64::NAN;
        assert_eq!(broken.usable_scale(), 1.0);
        broken.scale_factor = -2.0;
        assert_eq!(broken.usable_scale(), 1.0);

        let selection = Selection {
            monitor: 1,
            rect: LogicalRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        };
        assert_eq!(
            selection.to_virtual(&broken),
            Some(Rect::new(0, 0, 100, 100))
        );
    }

    #[test]
    fn a_selection_that_would_overflow_the_desktop_is_refused_rather_than_wrapped() {
        let selection = Selection {
            monitor: 1,
            rect: LogicalRect {
                x: 0.0,
                y: 0.0,
                width: f64::from(i32::MAX),
                height: 10.0,
            },
        };
        assert_eq!(selection.to_virtual(&primary()), None);
    }

    #[test]
    fn two_screens_overlap_only_where_they_actually_do() {
        let a = Rect::new(-1920, -120, 1920, 1080);
        let b = Rect::new(-100, 0, 2560, 1440);
        assert_eq!(a.intersection(&b), Some(Rect::new(-100, 0, 100, 960)));
        assert_eq!(a.intersection(&Rect::new(4000, 0, 100, 100)), None);
        // Touching edges are not an overlap.
        assert_eq!(a.intersection(&Rect::new(0, -120, 10, 10)), None);
    }
}
