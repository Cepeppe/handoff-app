/**
 * The views the overlay window can show (§7.6, DD-10).
 *
 * The request sheet, the settings and the onboarding are modes of the one window rather
 * than separate windows, so "never two windows" (MULTI-04) holds structurally and the
 * always-on-top and focus rules are managed once. This module is the whole routing
 * vocabulary: a name here is a mode of the single overlay window, never a second window.
 */

/** The five modes of §7.6. Order is the order the dev switcher lists them in. */
export const VIEW_NAMES = ['overlay', 'request', 'settings', 'onboarding', 'preview'] as const;

export type ViewName = (typeof VIEW_NAMES)[number];

/** The view the window opens on. The step view lives inside `overlay` (T-036). */
export const DEFAULT_VIEW: ViewName = 'overlay';

/** The i18n key holding the title of a view, so the two never drift apart. */
export function viewTitleKey(view: ViewName): string {
  return `view.${view}`;
}

/** Whether a value coming from the Rust side names a view. */
export function isViewName(value: unknown): value is ViewName {
  return typeof value === 'string' && (VIEW_NAMES as readonly string[]).includes(value);
}
