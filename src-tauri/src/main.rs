// Prevents an extra console window on Windows in release. The overlay is a tray
// application: a console flashing behind it would be the first thing a user distrusts.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    handoff_app_lib::run()
}
