/**
 * Instants as a person reads them (APP-02).
 *
 * Everything the core hands the window is RFC 3339 in UTC with milliseconds
 * (`log::time::Timestamp`), because that is the one shape the database compares as text and
 * the two implementations agree on. It is exactly the wrong thing to put in front of
 * somebody: `2026-09-08T12:00:00.000Z` is a fact, not a date.
 *
 * So the Log and the Runbooks pages render it through here, in the language the window is
 * running in — the same rune `t()` reads, so switching the language repaints these too. A
 * string that is not an instant comes back as it is rather than as `Invalid Date`: it can
 * only reach here from a row an older version wrote, and showing it is more useful than
 * hiding it.
 *
 * No text is written here (the format is the platform's), so nothing of this belongs in the
 * catalogue.
 */
import { language } from './i18n';

/** `iso` as a date and a time in the active language, or `iso` itself if it is not one. */
export function moment(iso: string): string {
  const at = new Date(iso);
  return Number.isNaN(at.getTime()) ? iso : at.toLocaleString(language());
}
