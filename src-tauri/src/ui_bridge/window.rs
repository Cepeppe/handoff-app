//! Where the panel sits and how big it is (WIN-01..03, §7.16).
//!
//! Three things live here, and they are together because they are the same fact seen from
//! three sides: a narrow always-on-top panel that the user drags around, that follows its
//! content in height, and that gets out of the way when they click elsewhere.
//!
//! - **The position, per monitor.** WIN-02 remembers where the panel was, and it does so
//!   *per monitor identifier*: a laptop docked at a desk and undocked on a train has two
//!   right answers, and one remembered pair of coordinates would put the window off-screen
//!   on the second. The map lives in the `settings` table, which is the one table
//!   `delete_all` keeps (LOG-04).
//! - **The collapse of WIN-03.** The window decides *when* — it is the side that knows
//!   whether the user is looking at it — so what is here is only the fact that the focus
//!   changed and the setting behind the R-10 fallback.
//! - **The height.** `super::resize_to_content` is what the frontend calls; the clamping
//!   lives there, next to the width the configuration fixes.
//!
//! # Why the writes are not where the moves are
//!
//! A drag emits a `Moved` event per frame, and a settings write per frame would be a
//! transaction per frame. The map is therefore kept in memory and flushed at the three
//! moments the position has finished changing: the window loses the focus, the window is
//! hidden, the application quits. What that costs in the worst case (a crash while
//! dragging) is one remembered position; what it saves is a write loop on the user's disk
//! every time they move the panel.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Monitor, PhysicalPosition, Window};

use crate::log::{settings, Db};

/// The `settings` key holding the position of the panel on each monitor (WIN-02).
pub const POSITIONS_KEY: &str = "window.positions";

/// The `settings` key behind the R-10 fallback collapse.
///
/// Off by default, and it must stay off by default: the timer collapses a panel the user is
/// reading, which is only ever worth it on a platform where the blur event has proved
/// unreliable.
pub const COLLAPSE_FALLBACK_KEY: &str = "window.collapseFallback";

/// How long after the last interaction the fallback collapses the panel (R-10).
pub const COLLAPSE_FALLBACK_MS: u32 = 3_000;

/// Where the panel was left, in the physical pixels of the monitor it was left on.
///
/// Physical and not logical, because the monitor's own position and size are physical and
/// the two are compared: mixing the units is how a window ends up half a screen away on a
/// display whose scale factor is not 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    /// Distance from the left edge of the virtual desktop.
    pub x: i32,
    /// Distance from its top edge.
    pub y: i32,
}

/// A rectangle in physical pixels: a monitor, or the window on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// How a monitor is named in the settings table (WIN-02).
///
/// The operating system's name when there is one — `\\.\DISPLAY1` on Windows, the model
/// name on macOS — because it survives the monitor being moved around in the desktop
/// arrangement, which its coordinates do not. Its geometry otherwise, which is stable
/// enough for a machine with one screen and is the only thing left to identify it by.
#[must_use]
pub fn monitor_key(name: Option<&str>, bounds: Rect) -> String {
    match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => name.to_owned(),
        None => format!(
            "{}x{}+{}+{}",
            bounds.width, bounds.height, bounds.x, bounds.y
        ),
    }
}

/// The position to actually use for a window of `size` on `monitor`.
///
/// A remembered position is not trusted as it is: the monitor may have been resized, the
/// arrangement may have changed, and a panel whose title bar is off-screen cannot be
/// dragged back (it has no title bar to grab, `decorations: false`). It is therefore pushed
/// back inside the monitor's own rectangle. A window taller than the monitor is pinned to
/// the top-left corner rather than to a negative coordinate.
#[must_use]
pub fn clamp_into(position: Position, monitor: Rect, size: (u32, u32)) -> Position {
    let fit = |start: i32, extent: u32, monitor_start: i32, monitor_extent: u32| {
        let last = monitor_start
            .saturating_add(i32::try_from(monitor_extent).unwrap_or(i32::MAX))
            .saturating_sub(i32::try_from(extent).unwrap_or(i32::MAX));
        start.clamp(monitor_start.min(last), last.max(monitor_start))
    };
    Position {
        x: fit(position.x, size.0, monitor.x, monitor.width),
        y: fit(position.y, size.1, monitor.y, monitor.height),
    }
}

/// Where the window goes when its **width** changes (§7.6: the expanded view and back).
///
/// The window is undecorated and never resizable, so a width change is something the
/// application does to a window the user has placed: the least surprising thing it can do is
/// leave the edge they aimed at where it is. A panel dragged against the right of the screen
/// keeps its right edge and grows leftwards; one on the left keeps its left edge and grows
/// rightwards. Which of the two it is is decided by the window's own centre against the
/// monitor's — not by the distance to each edge, which flips on a window that is already
/// wider than half the screen.
///
/// Whatever comes out is pushed back inside the monitor by [`clamp_into`], so a window near
/// an edge cannot be widened off the screen it is on. The height is not touched: the
/// frontend remeasures its content and `super::resize_to_content` applies the new one.
#[must_use]
pub fn anchored_position(current: Rect, monitor: Rect, next_width: u32) -> Position {
    let width = i64::from(current.width);
    let next = i64::from(next_width);
    let centre = i64::from(current.x) + width / 2;
    let monitor_centre = i64::from(monitor.x) + i64::from(monitor.width) / 2;
    let x = if centre > monitor_centre {
        // The right edge stays: the left one moves by the whole difference.
        i64::from(current.x) + width - next
    } else {
        i64::from(current.x)
    };
    let start = Position {
        x: i32::try_from(x).unwrap_or(if x < 0 { i32::MIN } else { i32::MAX }),
        y: current.y,
    };
    clamp_into(start, monitor, (next_width, current.height))
}

/// The rectangle a Tauri monitor occupies.
fn bounds_of(monitor: &Monitor) -> Rect {
    Rect {
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width,
        height: monitor.size().height,
    }
}

/// The part of a monitor a window may occupy: everything but the taskbar and the docks.
///
/// Used where the window is being *moved by the application* rather than by the user — the
/// width change of §7.6 — so that an expanded window does not end up under the taskbar. The
/// remembered positions of WIN-02 keep using the whole monitor ([`bounds_of`]): a user who
/// dragged the panel half over their taskbar meant to.
#[must_use]
pub fn work_area_of(monitor: &Monitor) -> Rect {
    let area = monitor.work_area();
    Rect {
        x: area.position.x,
        y: area.position.y,
        width: area.size.width,
        height: area.size.height,
    }
}

/// Where the panel was left, on each monitor it has been left on (WIN-02).
///
/// In memory, with the `settings` table behind it: `super::Ui` owns the connection, because
/// it is the window's own connection to the log and this is not the only thing the window
/// writes through it.
#[derive(Default)]
pub struct Geometry {
    positions: Mutex<BTreeMap<String, Position>>,
    unsaved: AtomicBool,
}

impl std::fmt::Debug for Geometry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Geometry")
            .field("monitors", &self.positions.lock().map(|it| it.len()).ok())
            .finish_non_exhaustive()
    }
}

impl Geometry {
    /// Reads what a previous run left behind.
    ///
    /// A settings row that cannot be read is logged and dropped: the panel then opens where
    /// the window manager puts it, which is a worse first impression and not a failure.
    pub fn load(&self, db: &Db) {
        let restored = settings::get::<BTreeMap<String, Position>>(db, POSITIONS_KEY)
            .unwrap_or_else(|error| {
                tracing::warn!(error = %error, "the remembered window positions could not be read");
                None
            })
            .unwrap_or_default();
        *self.positions.lock().expect("the positions mutex") = restored;
    }

    /// Remembers where `window` is now, on the monitor it is now on. Writes nothing.
    pub fn remember(&self, window: &Window) {
        let Some(key) = current_monitor_key(window) else {
            return;
        };
        let Ok(position) = window.outer_position() else {
            return;
        };
        let mut positions = self.positions.lock().expect("the positions mutex");
        let next = Position {
            x: position.x,
            y: position.y,
        };
        if positions.insert(key, next) != Some(next) {
            self.unsaved.store(true, Ordering::Relaxed);
        }
    }

    /// Puts `window` back where it was left on the monitor it is on (WIN-02).
    ///
    /// Called before the window is shown. A monitor with nothing remembered is left alone:
    /// the window manager's own placement is the right answer for a screen the user has
    /// never dragged the panel on.
    pub fn restore(&self, window: &Window) {
        let Ok(Some(monitor)) = window.current_monitor() else {
            return;
        };
        let bounds = bounds_of(&monitor);
        let key = monitor_key(monitor.name().map(String::as_str), bounds);
        let Some(remembered) = self
            .positions
            .lock()
            .expect("the positions mutex")
            .get(&key)
            .copied()
        else {
            return;
        };
        let size = window
            .outer_size()
            .map_or((0, 0), |size| (size.width, size.height));
        let position = clamp_into(remembered, bounds, size);
        if let Err(error) = window.set_position(PhysicalPosition::new(position.x, position.y)) {
            tracing::warn!(error = %error, "the overlay window refused a remembered position");
        }
    }

    /// Writes the remembered positions, if any changed since the last write.
    ///
    /// `db` is `None` when the log could not be opened: the panel then keeps its position
    /// for as long as the process lives and forgets it afterwards, which is the right
    /// degradation for a preference.
    pub fn flush(&self, db: Option<&Db>) {
        if !self.unsaved.swap(false, Ordering::Relaxed) {
            return;
        }
        let Some(db) = db else {
            return;
        };
        let positions = self.positions.lock().expect("the positions mutex").clone();
        if let Err(error) = settings::set(db, POSITIONS_KEY, &positions) {
            tracing::warn!(error = %error, "the window positions could not be written");
            // Left marked unsaved, so the next flush tries again rather than losing them.
            self.unsaved.store(true, Ordering::Relaxed);
        }
    }

    /// What the map holds right now. For the tests of this module.
    #[cfg(test)]
    fn remembered(&self, key: &str) -> Option<Position> {
        self.positions
            .lock()
            .expect("the positions mutex")
            .get(key)
            .copied()
    }
}

/// How the monitor a window is on is named, when it is on one.
fn current_monitor_key(window: &Window) -> Option<String> {
    let monitor = window.current_monitor().ok().flatten()?;
    Some(monitor_key(
        monitor.name().map(String::as_str),
        bounds_of(&monitor),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn a_monitor_is_named_by_the_system_when_the_system_names_it() {
        assert_eq!(monitor_key(Some(r"\\.\DISPLAY1"), SCREEN), r"\\.\DISPLAY1");
    }

    #[test]
    fn a_nameless_monitor_is_named_by_its_geometry() {
        assert_eq!(monitor_key(None, SCREEN), "1920x1080+0+0");
        // A blank name is no name: it would collide with every other blank one.
        assert_eq!(monitor_key(Some("   "), SCREEN), "1920x1080+0+0");
    }

    #[test]
    fn the_second_monitor_of_a_desktop_has_a_key_of_its_own() {
        let second = Rect {
            x: 1920,
            y: 0,
            width: 2560,
            height: 1440,
        };
        assert_ne!(monitor_key(None, SCREEN), monitor_key(None, second));
    }

    #[test]
    fn a_position_inside_the_monitor_is_kept_as_it_is() {
        let position = Position { x: 400, y: 300 };
        assert_eq!(clamp_into(position, SCREEN, (360, 480)), position);
    }

    #[test]
    fn a_position_that_would_hang_off_the_edge_is_pushed_back_in() {
        // The panel has no title bar to grab (`decorations: false`), so a window pushed off
        // the bottom right of the screen could not be brought back by hand.
        assert_eq!(
            clamp_into(Position { x: 1900, y: 1070 }, SCREEN, (360, 480)),
            Position { x: 1560, y: 600 }
        );
        assert_eq!(
            clamp_into(Position { x: -200, y: -50 }, SCREEN, (360, 480)),
            Position { x: 0, y: 0 }
        );
    }

    #[test]
    fn a_position_on_a_second_monitor_stays_on_that_monitor() {
        let second = Rect {
            x: -1920,
            y: 200,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            clamp_into(Position { x: -1000, y: 400 }, second, (360, 480)),
            Position { x: -1000, y: 400 }
        );
        assert_eq!(
            clamp_into(Position { x: 500, y: 400 }, second, (360, 480)),
            Position { x: -360, y: 400 }
        );
    }

    #[test]
    fn widening_a_panel_on_the_left_keeps_its_left_edge() {
        // 360 -> 720 at x = 200: the window grows rightwards and does not move.
        let panel = Rect {
            x: 200,
            y: 300,
            width: 360,
            height: 480,
        };
        assert_eq!(
            anchored_position(panel, SCREEN, 720),
            Position { x: 200, y: 300 }
        );
    }

    #[test]
    fn widening_a_panel_on_the_right_keeps_its_right_edge() {
        // The usual place for an always-on-top panel. Right edge 1900 before and after.
        let panel = Rect {
            x: 1540,
            y: 300,
            width: 360,
            height: 480,
        };
        assert_eq!(
            anchored_position(panel, SCREEN, 720),
            Position { x: 1180, y: 300 }
        );
    }

    #[test]
    fn narrowing_undoes_the_widening_exactly() {
        // Restore is Expand read backwards: the edge that stayed still stays still again.
        let expanded = Rect {
            x: 1180,
            y: 300,
            width: 720,
            height: 480,
        };
        assert_eq!(
            anchored_position(expanded, SCREEN, 360),
            Position { x: 1540, y: 300 }
        );
        let left = Rect {
            x: 200,
            y: 300,
            width: 720,
            height: 480,
        };
        assert_eq!(
            anchored_position(left, SCREEN, 360),
            Position { x: 200, y: 300 }
        );
    }

    #[test]
    fn a_window_that_would_grow_off_the_screen_is_pushed_back_in() {
        // Against the right edge of a monitor whose work area stops before it: the anchored
        // position is outside, so the clamp brings the whole window back (WIN-02's rule).
        let panel = Rect {
            x: 1860,
            y: 1000,
            width: 360,
            height: 480,
        };
        let work = Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };
        assert_eq!(
            anchored_position(panel, work, 720),
            Position { x: 1200, y: 560 }
        );
    }

    #[test]
    fn a_panel_exactly_in_the_middle_grows_rightwards() {
        // The tie goes to the left edge: `>` and not `>=`, so a centred window has one answer
        // and not two, and Expand followed by Restore puts it back where it was.
        let panel = Rect {
            x: 780,
            y: 0,
            width: 360,
            height: 480,
        };
        assert_eq!(
            anchored_position(panel, SCREEN, 720),
            Position { x: 780, y: 0 }
        );
    }

    #[test]
    fn a_panel_on_a_second_monitor_is_anchored_against_that_monitor() {
        let second = Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        // Left half of the second screen, whose coordinates are negative throughout.
        let panel = Rect {
            x: -1800,
            y: 100,
            width: 360,
            height: 480,
        };
        assert_eq!(
            anchored_position(panel, second, 720),
            Position { x: -1800, y: 100 }
        );
        // Right half of it.
        let right = Rect {
            x: -500,
            y: 100,
            width: 360,
            height: 480,
        };
        assert_eq!(
            anchored_position(right, second, 720),
            Position { x: -860, y: 100 }
        );
    }

    #[test]
    fn a_window_larger_than_the_monitor_is_pinned_to_its_corner() {
        let tiny = Rect {
            x: 10,
            y: 20,
            width: 200,
            height: 200,
        };
        assert_eq!(
            clamp_into(Position { x: 5000, y: 5000 }, tiny, (360, 480)),
            Position { x: 10, y: 20 }
        );
    }

    #[test]
    fn a_flush_with_no_database_forgets_rather_than_failing() {
        // The log could not be opened: the panel keeps its position for as long as the
        // process lives and forgets it afterwards, which is the right end for a preference.
        let geometry = Geometry::default();
        geometry.flush(None);
    }

    #[test]
    fn positions_written_by_one_run_are_read_by_the_next() {
        let path = std::env::temp_dir().join(format!(
            "baton-window-positions-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        {
            let db = Db::open_at(&path).expect("a database");
            let mut positions = BTreeMap::new();
            positions.insert(r"\\.\DISPLAY1".to_owned(), Position { x: 12, y: 34 });
            settings::set(&db, POSITIONS_KEY, &positions).expect("a write");
        }

        let geometry = Geometry::default();
        geometry.load(&Db::open_at(&path).expect("a database"));
        assert_eq!(
            geometry.remembered(r"\\.\DISPLAY1"),
            Some(Position { x: 12, y: 34 })
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_fallback_collapse_setting_round_trips() {
        // R-10: the timer collapses a panel the user may be reading, so the default is the
        // blur event alone and the setting exists for the platform where that is unreliable.
        let db = Db::open_in_memory().expect("a database");
        assert_eq!(
            settings::get::<bool>(&db, COLLAPSE_FALLBACK_KEY).expect("a read"),
            None
        );
        settings::set(&db, COLLAPSE_FALLBACK_KEY, &true).expect("a write");
        assert_eq!(
            settings::get::<bool>(&db, COLLAPSE_FALLBACK_KEY).expect("a read"),
            Some(true)
        );
    }
}
