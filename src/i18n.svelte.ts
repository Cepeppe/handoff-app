/**
 * The language the interface is showing (APP-02, §7.16).
 *
 * It is one variable, and it lives apart from {@link module:i18n} for one reason: the
 * settings page changes the language while the window is open, so every label already on
 * screen has to be repainted. That makes it reactive state, and reactive state has to be
 * declared in a `.svelte.ts` module — a plain `.ts` file cannot carry a rune. `i18n.ts`
 * re-exports everything here, so no component imports this file directly and the language
 * still reads as one module from the outside.
 *
 * `t()` reads {@link language} on every call. A component that renders a text therefore
 * subscribes to the language by the act of rendering it, and a switch redraws the whole
 * window with nothing to remember and nothing to reload (APP-02: "changeable in settings").
 */

/** The two languages of APP-02. Italian is non-negotiable; a third would be a design change. */
export const LANGUAGES = ['en', 'it'] as const;

export type Language = (typeof LANGUAGES)[number];

/** English, the fallback of APP-02 and of every unsupported system language. */
export const DEFAULT_LANGUAGE: Language = 'en';

/** Whether a value — a stored setting, a payload from Rust — names a supported language. */
export function isLanguage(value: unknown): value is Language {
  return typeof value === 'string' && (LANGUAGES as readonly string[]).includes(value);
}

let active = $state<Language>(DEFAULT_LANGUAGE);

/** The language the UI is currently showing. Reactive: reading it subscribes to it. */
export function language(): Language {
  return active;
}

/**
 * Switches the UI language, repainting every label that is on screen.
 *
 * Callers tell the Rust side themselves (`bridge().setUiLanguage`), because the tray menu
 * and the window are two surfaces of one decision and only the frontend makes it: it is
 * the side that can see the system's preference list (§7.16).
 */
export function setLanguage(next: Language): void {
  active = next;
}
