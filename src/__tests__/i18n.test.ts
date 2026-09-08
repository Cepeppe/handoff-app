/**
 * The language resolution rule of §7.16: setting → system if `en` or `it` → English.
 */
import { afterEach, describe, expect, it } from 'vitest';

import {
  DEFAULT_LANGUAGE,
  isLanguage,
  language,
  resolveLanguage,
  setLanguage,
  t,
} from '../i18n';

afterEach(() => setLanguage(DEFAULT_LANGUAGE));

describe('resolveLanguage', () => {
  it('takes the setting when it names a supported language', () => {
    expect(resolveLanguage('it', ['en-US'])).toBe('it');
    expect(resolveLanguage('en', ['it-IT'])).toBe('en');
  });

  it('ignores a setting that names no supported language', () => {
    expect(resolveLanguage('de', ['it-IT'])).toBe('it');
    expect(resolveLanguage('', ['it-IT'])).toBe('it');
    expect(resolveLanguage(null, ['it-IT'])).toBe('it');
    expect(resolveLanguage(undefined, ['it-IT'])).toBe('it');
  });

  it('follows the system language, region and case notwithstanding', () => {
    expect(resolveLanguage(null, ['it-IT'])).toBe('it');
    expect(resolveLanguage(null, ['IT'])).toBe('it');
    expect(resolveLanguage(null, ['en-GB'])).toBe('en');
  });

  it('falls back to English when the system speaks neither', () => {
    expect(resolveLanguage(null, ['de-DE', 'fr-FR'])).toBe('en');
    expect(resolveLanguage(null, [])).toBe('en');
    expect(DEFAULT_LANGUAGE).toBe('en');
  });

  it('walks the preference list rather than reading only its head', () => {
    // A machine set to French with Italian behind it gets Italian: "follows the system
    // language" means the most wanted language we can actually speak.
    expect(resolveLanguage(null, ['fr-FR', 'it-IT', 'en-US'])).toBe('it');
    expect(resolveLanguage(null, ['fr-FR', 'en-US', 'it-IT'])).toBe('en');
  });
});

describe('isLanguage', () => {
  it('accepts the two supported tags and nothing else', () => {
    expect(isLanguage('en')).toBe(true);
    expect(isLanguage('it')).toBe(true);
    expect(isLanguage('it-IT')).toBe(false);
    expect(isLanguage('de')).toBe(false);
    expect(isLanguage(null)).toBe(false);
    expect(isLanguage(7)).toBe(false);
  });
});

describe('t', () => {
  it('answers in the active language', () => {
    setLanguage('en');
    expect(language()).toBe('en');
    expect(t('tray.quit')).toBe('Quit');

    setLanguage('it');
    expect(language()).toBe('it');
    expect(t('tray.quit')).toBe('Esci');
  });

  it('returns the key itself when no catalogue has it', () => {
    expect(t('nothing.here')).toBe('nothing.here');
  });
});
