/**
 * The key-parity check of APP-02.
 *
 * Italian is not a nice-to-have: the requirement calls it non-negotiable, so a key added
 * to English and forgotten in Italian is a defect, not a pending translation. These files
 * are also read by the Rust side (`src-tauri/src/i18n.rs`) for the texts it owns, so this
 * one test covers the tray menu as well as the window.
 */
import { describe, expect, it } from 'vitest';

import enCatalogue from '../locales/en.json';
import itCatalogue from '../locales/it.json';
import { LANGUAGES } from '../i18n';
import { VIEW_NAMES, viewTitleKey } from '../views';

const CATALOGUES: Record<string, Record<string, string>> = {
  en: enCatalogue,
  it: itCatalogue,
};

describe('locale catalogues', () => {
  it('cover exactly the two supported languages', () => {
    expect(Object.keys(CATALOGUES).sort()).toEqual([...LANGUAGES].sort());
  });

  it('carry identical key sets', () => {
    const [reference, ...others] = Object.keys(CATALOGUES);
    const expected = Object.keys(CATALOGUES[reference]).sort();

    for (const language of others) {
      expect(Object.keys(CATALOGUES[language]).sort(), `keys of ${language}`).toEqual(expected);
    }
  });

  it('hold no empty and no whitespace-only text', () => {
    for (const [language, catalogue] of Object.entries(CATALOGUES)) {
      for (const [key, value] of Object.entries(catalogue)) {
        expect(typeof value, `${language}.${key}`).toBe('string');
        expect(value.trim(), `${language}.${key}`).not.toBe('');
      }
    }
  });

  it('name every view and every tray entry', () => {
    // The two lists the skeleton renders. A view or a tray item added without its text
    // would otherwise show its key to the user.
    const required = [
      'app.name',
      'tray.show',
      'tray.newRequest',
      'tray.settings',
      'tray.quit',
      'view.placeholder',
      ...VIEW_NAMES.map(viewTitleKey),
    ];

    for (const [language, catalogue] of Object.entries(CATALOGUES)) {
      for (const key of required) {
        expect(catalogue, `${language} is missing ${key}`).toHaveProperty(key);
      }
    }
  });
});
