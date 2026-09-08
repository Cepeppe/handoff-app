//! The tray icon and its menu (WIN-04, WIN-05, §7.16).
//!
//! The icon is always present; the menu is `Show`, `New request`, `Settings`, `Quit`, and
//! `Quit` is the *only* way to end the process — closing the window hides it here instead.
//! The badge that appears when handoffs are active comes later, with the store that knows
//! how many there are.
// TASK: T-037 — the badge over the icon, from the count of active handoffs (WIN-05).

use tauri::menu::{Menu, MenuEvent, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Wry};

use crate::i18n::{self, Language};

/// The id of the tray icon, so a later task can find it again to set the badge.
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

/// `Quit` is the only exit of the application (WIN-04); the other three open a view.
fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        ID_SHOW => super::show_main_window(app),
        ID_NEW_REQUEST => super::show_view(app, "request"),
        ID_SETTINGS => super::show_view(app, "settings"),
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
    fn every_menu_entry_has_a_text_in_both_languages() {
        for key in ["tray.show", "tray.newRequest", "tray.settings", "tray.quit"] {
            for language in [Language::En, Language::It] {
                assert_ne!(i18n::text(language, key), key);
            }
        }
    }
}
