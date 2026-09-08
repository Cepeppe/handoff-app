//! Language resolution and the texts the Rust side owns (APP-02, §7.16).
//!
//! The UI language is the setting when the user chose one, else the system language when it
//! is `en` or `it`, else English.
//!
//! The strings are **not** written here. `src/locales/en.json` and `src/locales/it.json` are
//! the single catalogue of the product, and this module compiles the very same two files
//! into the binary. Two reasons, both practical: the tray menu and the window would
//! otherwise be translated in two places and drift the first time a word changes, and the
//! frontend's key-parity test (`src/__tests__/locales.test.ts`) then covers the tray as
//! well — a key added in English and forgotten in Italian fails the build wherever it is
//! used.
//!
//! The resolution itself is written twice, here and in `src/i18n.ts`, because each side
//! resolves before it can talk to the other: the frontend has the system preference list
//! and resolves at startup, the tray is built before the webview exists. The rule is four
//! lines and both are tested against the same cases; what keeps them honest is that the
//! frontend reports the language it resolved (`set_ui_language`), so what the window and
//! the tray show always comes from one decision.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// The English catalogue, the same file the frontend imports.
const EN_JSON: &str = include_str!("../../src/locales/en.json");

/// The Italian catalogue, the same file the frontend imports.
const IT_JSON: &str = include_str!("../../src/locales/it.json");

/// The two languages of APP-02.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// English, the default of APP-02 and the fallback of every unsupported system.
    #[default]
    En,
    /// Italian, which APP-02 calls non-negotiable.
    It,
}

impl Language {
    /// The tag the frontend and the settings use (`"en"`, `"it"`).
    pub fn tag(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::It => "it",
        }
    }

    /// The language a BCP 47 tag names, or `None` when it is neither of the two.
    ///
    /// Only the primary subtag is compared, so `it-IT` is Italian and `IT` is too: the
    /// value comes from an operating system or from a stored setting, and neither promises
    /// a case or a region.
    pub fn from_tag(tag: &str) -> Option<Self> {
        let primary = tag
            .split('-')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        match primary.as_str() {
            "en" => Some(Language::En),
            "it" => Some(Language::It),
            _ => None,
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.tag())
    }
}

/// The rule of §7.16: the setting, else the first system language we speak, else English.
///
/// `system` is the platform's preference list, most wanted first. Where its head is `en` or
/// `it` this is exactly the design's sentence; where it is not, walking on finds a language
/// the user asked for rather than falling straight to English.
pub fn resolve(setting: Option<&str>, system: &[&str]) -> Language {
    if let Some(chosen) = setting.and_then(exact_tag) {
        return chosen;
    }
    system
        .iter()
        .find_map(|tag| Language::from_tag(tag))
        .unwrap_or_default()
}

/// A stored setting names a language exactly (`"en"`, `"it"`), never a region.
fn exact_tag(setting: &str) -> Option<Language> {
    match setting {
        "en" => Some(Language::En),
        "it" => Some(Language::It),
        _ => None,
    }
}

/// The text of `key` in `language`.
///
/// A key the active catalogue does not carry falls back to English and then to the key
/// itself: the parity test makes both impossible in a committed state, and a visible key in
/// the tray menu is easier to notice and to fix than an empty entry.
pub fn text(language: Language, key: &str) -> &'static str {
    catalogue(language)
        .get(key)
        .or_else(|| catalogue(Language::En).get(key))
        .map(String::as_str)
        .unwrap_or_else(|| leaked_key(key))
}

/// The catalogue of a language, parsed once.
fn catalogue(language: Language) -> &'static BTreeMap<String, String> {
    static EN: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    static IT: OnceLock<BTreeMap<String, String>> = OnceLock::new();

    match language {
        Language::En => EN.get_or_init(|| parse(EN_JSON, "en")),
        Language::It => IT.get_or_init(|| parse(IT_JSON, "it")),
    }
}

/// The catalogues are compiled in, so a parse failure is a broken build, not a runtime
/// condition: it is better to say which file and stop than to run with an empty menu.
fn parse(source: &str, name: &str) -> BTreeMap<String, String> {
    serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("src/locales/{name}.json is not a flat string map: {error}"))
}

/// The last resort of [`text`]: the key itself, kept alive for the `'static` signature.
///
/// It is reached only for a key no catalogue carries, which the parity test forbids, so
/// the leak is bounded by the number of programming mistakes rather than by anything a
/// user can do.
fn leaked_key(key: &str) -> &'static str {
    Box::leak(key.to_string().into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_wins_over_the_system() {
        assert_eq!(resolve(Some("it"), &["en-US"]), Language::It);
        assert_eq!(resolve(Some("en"), &["it-IT"]), Language::En);
    }

    #[test]
    fn a_setting_naming_no_supported_language_is_ignored() {
        assert_eq!(resolve(Some("de"), &["it-IT"]), Language::It);
        assert_eq!(resolve(Some(""), &["it-IT"]), Language::It);
        assert_eq!(resolve(None, &["it-IT"]), Language::It);
    }

    #[test]
    fn the_system_language_is_read_without_its_region_or_case() {
        assert_eq!(resolve(None, &["it-IT"]), Language::It);
        assert_eq!(resolve(None, &["IT"]), Language::It);
        assert_eq!(resolve(None, &["en-GB"]), Language::En);
    }

    #[test]
    fn english_is_the_fallback() {
        assert_eq!(resolve(None, &["de-DE", "fr-FR"]), Language::En);
        assert_eq!(resolve(None, &[]), Language::En);
        assert_eq!(Language::default(), Language::En);
    }

    #[test]
    fn the_preference_list_is_walked_as_the_frontend_walks_it() {
        // `src/__tests__/i18n.test.ts` pins the same two cases on the other side.
        assert_eq!(resolve(None, &["fr-FR", "it-IT", "en-US"]), Language::It);
        assert_eq!(resolve(None, &["fr-FR", "en-US", "it-IT"]), Language::En);
    }

    #[test]
    fn both_catalogues_answer_every_text_the_rust_side_prints() {
        // The tray menu is the whole list today. A key missing here would put a raw
        // `tray.show` in the user's menu.
        for key in [
            "app.name",
            "tray.show",
            "tray.newRequest",
            "tray.settings",
            "tray.quit",
        ] {
            for language in [Language::En, Language::It] {
                let value = text(language, key);
                assert_ne!(value, key, "{language} has no text for {key}");
                assert!(!value.trim().is_empty(), "{language}.{key} is blank");
            }
        }
    }

    #[test]
    fn the_two_catalogues_carry_the_same_keys() {
        // The frontend test says the same thing; it is repeated here because the Rust side
        // reads the files at compile time and would otherwise ship a half-translated menu
        // from a checkout whose frontend tests were never run.
        let en: Vec<&String> = catalogue(Language::En).keys().collect();
        let it: Vec<&String> = catalogue(Language::It).keys().collect();
        assert_eq!(en, it);
    }

    #[test]
    fn an_unknown_key_comes_back_as_itself() {
        assert_eq!(text(Language::It, "nothing.here"), "nothing.here");
    }

    #[test]
    fn the_tag_round_trips() {
        for language in [Language::En, Language::It] {
            assert_eq!(Language::from_tag(language.tag()), Some(language));
        }
        assert_eq!(Language::from_tag("de"), None);
        assert_eq!(Language::from_tag(""), None);
    }
}
