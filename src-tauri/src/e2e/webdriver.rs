//! What `msedgedriver` needs from the window it drives, in a build that is driven (T-055).
//!
//! The WebDriver suite of §11.4 starts this application through `tauri-driver` and
//! `msedgedriver`. The driver hands the WebView2 it wants to reach switches of its own —
//! `--remote-debugging-port=0` above all, and `--enable-automation` — through the
//! `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` environment variable, and then waits for the runtime
//! to write the port it opened into a `DevToolsActivePort` file.
//!
//! This application passes switches of its own as well, [`crate::ui_bridge::WEBVIEW2_BROWSER_ARGS`]
//! (T-052: the runtime's background networking off). What the runtime does with the two
//! depends on its build. The 152 of the development machine merges them; the 151 the CI runner
//! image carried on 2026-09-10 kept the application's and dropped the driver's — measured, from
//! the browser process's own command line — so no port was ever opened and every session
//! failed after sixty seconds with "DevToolsActivePort file doesn't exist". So an e2e build
//! merges them itself, before the window exists, and the window the suite drives is the
//! product's window with the driver's switches added, whatever the runtime would have done.
//!
//! Nothing here exists in a release build: the module is behind `--features e2e` like the rest
//! of [`super`], and it acts only when the variable is set, which no one launching an e2e build
//! by hand has reason to do.
//!
//! The region-selection overlays are left as they are. WebView2 refuses a second webview in
//! one profile with different switches (T-052), so under a driver they would not open — and no
//! suite opens them, since a harness cannot drag a rectangle across a screen (`capture::fake`).

use tauri::utils::config::WindowConfig;

/// The variable `msedgedriver` hands its switches to the application's WebView2 in.
pub const DRIVER_ARGUMENTS: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";

/// The one switch whose values two lists have to share rather than overwrite.
const DISABLE_FEATURES: &str = "--disable-features=";

/// Adds the driver's switches, when a driver set any, to every window the configuration
/// declares. Called from `lib.rs` before the application is built.
pub fn merge_driver_arguments(config: &mut tauri::Config) {
    let Ok(driver) = std::env::var(DRIVER_ARGUMENTS) else {
        return;
    };
    merge_into(&mut config.app.windows, &driver);
    tracing::info!(
        "the windows carry the WebView2 switches of the WebDriver that started them (this build carries --features e2e)"
    );
}

/// [`merge_driver_arguments`] without the environment: `driver` added to each window's own.
pub fn merge_into(windows: &mut [WindowConfig], driver: &str) {
    for window in windows {
        let own = window.additional_browser_args.take().unwrap_or_default();
        window.additional_browser_args = Some(merged(&own, driver));
    }
}

/// `own` followed by the switches of `driver` it does not already carry, with a single
/// `--disable-features=` holding the features of both, in that order.
///
/// A single list and not two, because Chromium reads the last `--disable-features=` and
/// forgets the ones before it: appended as they came, the driver's list would switch
/// SmartScreen and the OOUI features back on for the whole session. The switches of both lists
/// are separated by spaces and carry none of their own, so a split on whitespace is a parse.
#[must_use]
pub fn merged(own: &str, driver: &str) -> String {
    let mut features: Vec<&str> = Vec::new();
    let mut switches: Vec<&str> = Vec::new();
    for switch in own.split_whitespace().chain(driver.split_whitespace()) {
        if let Some(list) = switch.strip_prefix(DISABLE_FEATURES) {
            for feature in list.split(',').filter(|feature| !feature.is_empty()) {
                if !features.contains(&feature) {
                    features.push(feature);
                }
            }
        } else if !switches.contains(&switch) {
            switches.push(switch);
        }
    }
    let mut result = String::new();
    if !features.is_empty() {
        result.push_str(DISABLE_FEATURES);
        result.push_str(&features.join(","));
    }
    for switch in switches {
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(switch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_bridge::WEBVIEW2_BROWSER_ARGS;

    /// What `msedgedriver` 152 put in the variable, as the browser process's command line
    /// showed it on the development machine (T-055), shortened to the switches that matter.
    const DRIVER: &str = "--allow-pre-commit-input --disable-background-networking \
        --disable-features=IgnoreDuplicateNavs,Prewarm --enable-automation --enable-logging \
        --remote-debugging-port=0 --test-type=webdriver";

    fn switches(list: &str) -> Vec<&str> {
        list.split_whitespace().collect()
    }

    #[test]
    fn the_driver_switches_join_ours() {
        let all = merged(WEBVIEW2_BROWSER_ARGS, DRIVER);
        for switch in [
            "--remote-debugging-port=0",
            "--enable-automation",
            "--test-type=webdriver",
            "--disable-component-update",
            "--disable-domain-reliability",
            "--no-pings",
        ] {
            assert!(
                switches(&all).contains(&switch),
                "{switch} is missing from {all}"
            );
        }
    }

    #[test]
    fn one_feature_list_carries_the_features_of_both() {
        let all = merged(WEBVIEW2_BROWSER_ARGS, DRIVER);
        let lists: Vec<&str> = switches(&all)
            .into_iter()
            .filter(|switch| switch.starts_with(DISABLE_FEATURES))
            .collect();
        assert_eq!(
            lists,
            vec!["--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,IgnoreDuplicateNavs,Prewarm"]
        );
    }

    #[test]
    fn a_switch_both_lists_carry_is_there_once() {
        let all = merged(WEBVIEW2_BROWSER_ARGS, DRIVER);
        assert_eq!(all.matches("--disable-background-networking").count(), 1);
    }

    #[test]
    fn with_nothing_from_a_driver_the_switches_are_ours() {
        assert_eq!(merged(WEBVIEW2_BROWSER_ARGS, ""), WEBVIEW2_BROWSER_ARGS);
    }

    #[test]
    fn every_declared_window_gets_them() {
        let mut windows = vec![
            WindowConfig {
                additional_browser_args: Some(WEBVIEW2_BROWSER_ARGS.to_owned()),
                ..WindowConfig::default()
            },
            WindowConfig::default(),
        ];
        merge_into(&mut windows, DRIVER);
        for window in &windows {
            let all = window
                .additional_browser_args
                .as_deref()
                .unwrap_or_default();
            assert!(
                switches(all).contains(&"--remote-debugging-port=0"),
                "{all}"
            );
        }
    }
}
