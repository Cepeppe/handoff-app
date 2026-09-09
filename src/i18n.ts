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
 * covers the whole product and a translation is never half-applied. That rule is enforced
 * by `__tests__/locales.test.ts`, which also refuses a sentence written into a component.
 *
 * The language *in force* lives in `i18n.svelte.ts`, because the settings page changes it
 * with the window open and that makes it reactive state. It is re-exported here, so this
 * module stays the one import a component needs.
 */
import { DEFAULT_LANGUAGE, isLanguage, language, type Language } from './i18n.svelte';
import en from './locales/en.json';
import it from './locales/it.json';

export { DEFAULT_LANGUAGE, isLanguage, language, LANGUAGES, setLanguage } from './i18n.svelte';
export type { Language } from './i18n.svelte';

const CATALOGUES: Record<Language, Record<string, string>> = { en, it };

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
 * The setting is what `bridge().generalSettings()` answers with, `null` while the user has
 * never chosen one: "System" in the settings page is the absence of a setting, not a third
 * value, so a machine that changes its system language follows it afterwards.
 */
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

/**
 * The text of `key` in the active language, with `{name}` placeholders filled in.
 *
 * A key missing from the active catalogue falls back to English and then to the key itself:
 * the parity test makes both impossible in a committed state, and a visible key beats a
 * blank line in the window while a translation is being written.
 *
 * The substitution runs once over the catalogue text and never over what it substituted, so
 * a step whose own words contain `{total}` stays the agent's words — the same rule
 * `requests::text` follows on the Rust side.
 */
export function t(key: string, params?: Readonly<Record<string, string | number>>): string {
  const text = CATALOGUES[language()][key] ?? CATALOGUES[DEFAULT_LANGUAGE][key] ?? key;
  if (params === undefined) {
    return text;
  }
  return text.replace(/\{(\w+)\}/g, (whole, name: string) => {
    const value = params[name];
    // A placeholder nobody supplied is left as it is rather than printed as `undefined`:
    // it is a mistake in the caller, and showing it names the key that is wrong.
    return value === undefined ? whole : String(value);
  });
}

/** The catalogue of a language, for tests and for the parity check. */
export function catalogue(lang: Language): Record<string, string> {
  return CATALOGUES[lang];
}
