//! The tray icon and its menu (WIN-04, WIN-05, §7.16).
//!
//! The icon is always present; the menu is `Show`, `New request`, `Settings`, `Quit`, and
//! `Quit` is the *only* way to end the process — closing the window hides it here instead.
//!
//! # The badge (WIN-05)
//!
//! "A badge appears only when there are active handoffs." Tauri's own badge — a title beside
//! the icon — is **unsupported on Windows**, which is the platform this build is for, so the
//! badge is painted into the icon itself: [`badged`] draws a filled dot over the bottom-right
//! corner of the icon's own pixels. That needs no image codec (the icon arrives as RGBA
//! already) and therefore no new dependency to justify to `cargo deny`.
//!
//! The count goes to the tooltip rather than into the dot: two digits rendered into a
//! 32-pixel icon are a smudge, and the tooltip is where a person reads a number anyway.

use tauri::menu::{Menu, MenuEvent, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Wry};

use crate::i18n::{self, Language};

/// The id of the tray icon, so the badge can find it again.
pub const TRAY_ID: &str = "main";

const ID_SHOW: &str = "show";
const ID_NEW_REQUEST: &str = "new-request";
const ID_SETTINGS: &str = "settings";
const ID_QUIT: &str = "quit";

/// The menu items, kept so the menu can be relabelled when the language changes.
///
/// Relabelling in place rather than rebuilding the menu keeps the tray icon and its handler
/// untouched, which matters on Windows where replacing a menu while it is open is visible.
pub struct Handles {
    show: MenuItem<Wry>,
    new_request: MenuItem<Wry>,
    settings: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

impl Handles {
    /// Writes the four labels in `language` (APP-02).
    pub fn relabel(&self, language: Language) -> tauri::Result<()> {
        self.show.set_text(i18n::text(language, "tray.show"))?;
        self.new_request
            .set_text(i18n::text(language, "tray.newRequest"))?;
        self.settings
            .set_text(i18n::text(language, "tray.settings"))?;
        self.quit.set_text(i18n::text(language, "tray.quit"))
    }
}

/// Puts the icon in the tray and returns the handles to its menu items.
pub fn install(app: &AppHandle, language: Language) -> Result<Handles, Box<dyn std::error::Error>> {
    let enabled = true;
    let no_accelerator = None::<&str>;

    let show = MenuItem::with_id(
        app,
        ID_SHOW,
        i18n::text(language, "tray.show"),
        enabled,
        no_accelerator,
    )?;
    let new_request = MenuItem::with_id(
        app,
        ID_NEW_REQUEST,
        i18n::text(language, "tray.newRequest"),
        enabled,
        no_accelerator,
    )?;
    let settings = MenuItem::with_id(
        app,
        ID_SETTINGS,
        i18n::text(language, "tray.settings"),
        enabled,
        no_accelerator,
    )?;
    let quit = MenuItem::with_id(
        app,
        ID_QUIT,
        i18n::text(language, "tray.quit"),
        enabled,
        no_accelerator,
    )?;

    let menu = Menu::with_items(app, &[&show, &new_request, &settings, &quit])?;

    // The bundle icon doubles as the tray icon while the mark is a placeholder; the day it
    // is replaced, the tray gets its own size rather than the window's.
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("the bundle declares no default window icon")?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip(i18n::text(language, "app.name"))
        .menu(&menu)
        .on_menu_event(on_menu_event)
        .build(app)?;

    Ok(Handles {
        show,
        new_request,
        settings,
        quit,
    })
}

/// The dot's diameter, as a fraction of the icon's shorter side.
///
/// Just over a third: small enough to leave the mark recognisable, large enough to be seen
/// at the sixteen physical pixels a Windows tray gives an icon on a 100% display.
const BADGE_FRACTION: f32 = 0.38;

/// The dot's colour, opaque, as R, G, B, A.
///
/// A warning red rather than the product's own accent: the badge has to read as "something
/// is waiting" against an icon whose colours it does not know.
const BADGE_COLOUR: [u8; 4] = [0xd7, 0x26, 0x38, 0xff];

/// The icon's pixels with the "there is work" dot painted over the bottom-right corner.
///
/// Row-major RGBA in, row-major RGBA out, which is what `tauri::image::Image` carries both
/// ways. The dot is drawn opaque over whatever is under it — an icon corner is decoration,
/// and a translucent dot over a dark icon is not a badge — with one pixel of feathering, so
/// that the circle does not read as a square at tray sizes.
///
/// Pixels that are not a `width × height` RGBA buffer come back unchanged: an icon this
/// cannot understand is still a usable icon, and a tray with no icon at all would be a
/// worse answer than a tray with no badge.
#[must_use]
pub fn badged(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let expected = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
    if width == 0 || height == 0 || rgba.len() != expected {
        return rgba.to_vec();
    }

    let side = width.min(height) as f32;
    let radius = side * BADGE_FRACTION / 2.0;
    // Inset by a pixel, so the dot does not bleed off the icon's own edge.
    let centre_x = width as f32 - radius - 1.0;
    let centre_y = height as f32 - radius - 1.0;

    let mut painted = rgba.to_vec();
    for y in 0..height {
        for x in 0..width {
            let dx = (x as f32 + 0.5) - centre_x;
            let dy = (y as f32 + 0.5) - centre_y;
            let distance = dx.mul_add(dx, dy * dy).sqrt();
            // One pixel of feathering: 1.0 well inside the circle, 0.0 well outside it.
            let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let at = ((y as usize * width as usize) + x as usize) * 4;
            for channel in 0..4 {
                let under = f32::from(painted[at + channel]);
                let over = f32::from(BADGE_COLOUR[channel]);
                painted[at + channel] = coverage.mul_add(over - under, under).round() as u8;
            }
        }
    }
    painted
}

/// Shows or hides the badge, and says how many handoffs it stands for (WIN-05).
///
/// `count` is how many handoffs are not final: everything the user still has something to do
/// about, which is what "active" means to the person glancing at the tray. Zero puts the
/// plain icon and the plain tooltip back.
pub fn set_badge(app: &AppHandle, count: usize) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let Some(icon) = app.default_window_icon().cloned() else {
        return;
    };
    let language = super::language_of(app);

    let icon = if count == 0 {
        icon
    } else {
        let painted = badged(icon.rgba(), icon.width(), icon.height());
        tauri::image::Image::new_owned(painted, icon.width(), icon.height())
    };
    if let Err(error) = tray.set_icon(Some(icon)) {
        tracing::warn!(error = %error, "the tray icon refused the badge");
    }

    let tooltip = if count == 0 {
        i18n::text(language, "app.name").to_owned()
    } else {
        i18n::text(language, "tray.tooltipActive").replace("{count}", &count.to_string())
    };
    if let Err(error) = tray.set_tooltip(Some(tooltip)) {
        tracing::warn!(error = %error, "the tray icon refused its tooltip");
    }
}

/// `Quit` is the only exit of the application (WIN-04); the other three open a view.
fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        ID_SHOW => super::show_main_window(app),
        ID_NEW_REQUEST => super::show_view(app, super::VIEW_REQUEST),
        ID_SETTINGS => super::show_view(app, super::VIEW_SETTINGS),
        ID_QUIT => app.exit(0),
        other => tracing::warn!(menu_id = other, "unknown tray menu entry"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frontend's view vocabulary, read as text: the two views the tray asks for have
    /// to be names the frontend knows, or the menu entries would do nothing.
    const VIEWS_TS: &str = include_str!("../../../src/views.ts");

    #[test]
    fn the_views_the_menu_asks_for_are_views_the_frontend_knows() {
        for view in ["request", "settings"] {
            assert!(
                VIEWS_TS.contains(&format!("'{view}'")),
                "src/views.ts does not declare the view {view}"
            );
        }
    }

    #[test]
    fn the_badge_paints_the_corner_and_leaves_the_rest_alone() {
        let icon = vec![0xff_u8; 16 * 16 * 4];
        let painted = badged(&icon, 16, 16);
        assert_eq!(painted.len(), icon.len());
        // The corner furthest from the dot is untouched: the mark stays recognisable.
        assert_eq!(&painted[0..4], &[0xff, 0xff, 0xff, 0xff]);
        // A pixel inside the dot is the badge colour, opaquely.
        let pixel = |x: usize, y: usize| {
            let at = (y * 16 + x) * 4;
            painted[at..at + 4].to_vec()
        };
        assert_eq!(pixel(12, 12), BADGE_COLOUR.to_vec());
        // And the dot is inset, so it does not bleed off the icon's own edge.
        assert_eq!(pixel(15, 15), vec![0xff, 0xff, 0xff, 0xff]);
    }

    #[test]
    fn pixels_that_are_not_an_image_come_back_as_they_went_in() {
        let odd = vec![1_u8, 2, 3];
        assert_eq!(badged(&odd, 4, 4), odd);
        assert_eq!(badged(&[], 0, 0), Vec::<u8>::new());
    }

    #[test]
    fn the_badge_is_the_same_size_whatever_the_icon_resolution_is() {
        for side in [16_u32, 32, 64, 128] {
            let icon = vec![0x00_u8; (side * side * 4) as usize];
            let painted = badged(&icon, side, side);
            let lit = painted
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[3] > 0)
                .count() as f32;
            let share = lit / (side * side) as f32;
            // A disc of that fraction covers about π/4 · fraction² of the square, plus the
            // feathered ring, which weighs more on a small icon than on a large one. The
            // point of the check is that the dot scales with the icon rather than staying a
            // fixed number of pixels, which is what would make it invisible at 128.
            let ideal = std::f32::consts::PI / 4.0 * BADGE_FRACTION * BADGE_FRACTION;
            assert!(
                (ideal..ideal + 0.05).contains(&share),
                "a {side}px icon has {share} of its area lit, expected about {ideal}"
            );
        }
    }

    #[test]
    fn every_menu_entry_has_a_text_in_both_languages() {
        for key in [
            "tray.show",
            "tray.newRequest",
            "tray.settings",
            "tray.quit",
            "tray.tooltipActive",
        ] {
            for language in [Language::En, Language::It] {
                assert_ne!(i18n::text(language, key), key);
            }
        }
    }
}
