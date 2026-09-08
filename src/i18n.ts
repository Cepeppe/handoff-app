/**
 * UI language and texts (APP-02, §7.16).
 *
 * Two supported languages, English and Italian, and one resolution rule: the setting the
 * user chose, else the system language when it is one of the two, else English.
 *
 * The strings live in `locales/en.json` and `locales/it.json`, flat `"area.name"` keys.
 * Those two files are the *only* place a user-visible text is written: the Rust side reads
 * the very same files at compile time (`src-tauri/src/i18n.rs`) for the texts it owns —
 * the tray menu today, notifications and the crash notice later — so one key-parity test
 * covers the whole product and a translation is never half-applied.
 */
import en from './locales/en.json';
import it from './locales/it.json';

/** The two languages of APP-02. Italian is non-negotiable; a third would be a design change. */
export const LANGUAGES = ['en', 'it'] as const;

export type Language = (typeof LANGUAGES)[number];

/** English, the fallback of APP-02 and of every unsupported system language. */
export const DEFAULT_LANGUAGE: Language = 'en';

const CATALOGUES: Record<Language, Record<string, string>> = { en, it };

/** Whether a value — a stored setting, a payload from Rust — names a supported language. */
export function isLanguage(value: unknown): value is Language {
  return typeof value === 'string' && (LANGUAGES as readonly string[]).includes(value);
}

/**
 * The language the UI runs in: the setting, else the system, else English (§7.16).
 *
 * `systemLanguages` is the platform's own preference list, most wanted first, which is what
 * `navigator.languages` gives inside the webview; its first entry is the system language.
 * Each entry is a BCP 47 tag, so only the primary subtag is compared (`it-IT` is Italian).
 * Walking the list rather than reading only its head matters for the one case where the two
 * differ: a machine set to French with Italian behind it gets Italian instead of English,
 * which is the intent of "follows the system language" — where the head is `en` or `it`,
 * the walk and the design's sentence give the same answer.
 *
 * The setting is `null` until the settings store exists.
 */
// TASK: T-041 — pass the stored language setting here instead of `null`.
export function resolveLanguage(
  setting: string | null | undefined,
  systemLanguages: readonly string[],
): Language {
  if (isLanguage(setting)) {
    return setting;
  }
  for (const tag of systemLanguages) {
    const primary = tag.split('-')[0]?.toLowerCase();
    if (isLanguage(primary)) {
      return primary;
    }
  }
  return DEFAULT_LANGUAGE;
}

/** What the browser reports as the system preference list, in order, safe under jsdom. */
export function systemLanguages(): readonly string[] {
  if (typeof navigator === 'undefined') {
    return [];
  }
  return navigator.languages ?? (navigator.language ? [navigator.language] : []);
}

let active: Language = DEFAULT_LANGUAGE;

/** The language the UI is currently showing. */
export function language(): Language {
  return active;
}

/**
 * Switches the UI language. Callers tell the Rust side themselves (`bridge().setUiLanguage`).
 *
 * The language is resolved once before the application is mounted, so this holds a plain
 * variable rather than a rune: a call after mounting changes what `t` returns but re-renders
 * nothing.
 */
// TASK: T-041 — the settings page switches the language while the window is open, so this
// state has to become a rune (`$state`) at that point.
export function setLanguage(next: Language): void {
  active = next;
}

/**
 * The text of `key` in the active language.
 *
 * A key missing from the active catalogue falls back to English and then to the key itself:
 * the parity test makes both impossible in a committed state, and a visible key beats a
 * blank line in the window while a translation is being written.
 */
export function t(key: string): string {
  return CATALOGUES[active][key] ?? CATALOGUES[DEFAULT_LANGUAGE][key] ?? key;
}

/** The catalogue of a language, for tests and for the parity check. */
export function catalogue(lang: Language): Record<string, string> {
  return CATALOGUES[lang];
}
