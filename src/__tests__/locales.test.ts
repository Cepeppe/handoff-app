/**
 * The texts of the product: key parity, and no text written anywhere else (APP-02).
 *
 * Italian is not a nice-to-have: the requirement calls it non-negotiable, so a key added
 * to English and forgotten in Italian is a defect, not a pending translation. These files
 * are also read by the Rust side (`src-tauri/src/i18n.rs`) for the texts it owns, so this
 * one test covers the tray menu as well as the window.
 *
 * The second half is the other side of the same rule. Parity proves that everything *in*
 * the catalogue exists twice; it says nothing about a sentence that never reached the
 * catalogue at all. `hard-coded-text.ts` walks the components for those, and the three
 * cases below prove it would find one — a lint nobody has watched fail is a lint that
 * passes for the wrong reason.
 */
import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import { describe, expect, it } from 'vitest';

import enCatalogue from '../locales/en.json';
import itCatalogue from '../locales/it.json';
import { LANGUAGES } from '../i18n';
import { VIEW_NAMES, viewTitleKey } from '../views';
import { hardCodedText, hardCodedTextIn, svelteFiles } from './hard-coded-text';

const CATALOGUES: Record<string, Record<string, string>> = {
  en: enCatalogue,
  it: itCatalogue,
};

// Resolved from the working directory rather than from `import.meta.url`: under the jsdom
// environment the module URL is an `http://localhost` one and `fileURLToPath` refuses it.
// Vitest runs with the project root as the working directory.
const SOURCE_DIR = resolve(process.cwd(), 'src');

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

  it('carry every key the components ask for', () => {
    // The other direction of the same promise: a `t('…')` whose key is in no catalogue
    // draws the key itself, which the fallback in `t` makes visible rather than blank.
    const keys = new Set(Object.keys(CATALOGUES.en));
    const missing: string[] = [];
    for (const path of svelteFiles(SOURCE_DIR)) {
      for (const match of readFileSync(path, 'utf8').matchAll(/\bt\(\s*'([a-zA-Z][\w.]*)'/g)) {
        const key = match[1];
        if (!keys.has(key)) {
          missing.push(`${path}: ${key}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });
});

describe('the components', () => {
  it('write no user-visible text of their own', () => {
    // APP-02, from the side parity cannot see: `src/locales/*.json` is the only place a
    // sentence is written, so a component draws expressions and nothing else.
    expect(existsSync(SOURCE_DIR), `${SOURCE_DIR} is not the frontend source folder`).toBe(true);
    expect(svelteFiles(SOURCE_DIR).length).toBeGreaterThan(0);
    expect(hardCodedTextIn(SOURCE_DIR)).toEqual([]);
  });

  it('would be caught with a sentence between two tags', () => {
    const found = hardCodedText('<p class="empty">Nothing to do yet.</p>');
    expect(found).toEqual([{ what: 'a text node', text: 'Nothing to do yet.' }]);
  });

  it('would be caught with a sentence in an attribute a person reads', () => {
    const found = hardCodedText('<button type="button" aria-label="Close">{t(\'x\')}</button>');
    expect(found).toEqual([{ what: 'an attribute', text: 'aria-label="Close"' }]);
  });

  it('would be caught with a sentence assigned in the script', () => {
    const source = [
      '<script lang="ts">',
      "  let problem = 'that did not work';",
      '</script>',
      '<p>{problem}</p>',
    ].join('\n');
    expect(hardCodedText(source)).toEqual([
      { what: 'a sentence in the script', text: 'that did not work' },
    ]);
  });

  it('allows the ids, the symbols and the punctuation between two expressions', () => {
    // The "allowlist for ids and symbols": a class, a role, a form value and the ` · `
    // separator carry no language and are the same text in both catalogues.
    const source = [
      '<script lang="ts">',
      "  // A comment is prose and is not a text: this sentence must not fail the lint.",
      "  const kind = 'not_registered';",
      "  const combination = 'Control+Alt+H';",
      "  const pattern = /[^'\"]{2,}/u;",
      '</script>',
      '<li class="agent" data-kind={kind} role="group">',
      "  <input type=\"radio\" name=\"scope\" value=\"user\" />",
      "  {t('install.files')}: {kind} · {combination} · {String(pattern)}",
      '</li>',
    ].join('\n');
    expect(hardCodedText(source)).toEqual([]);
  });
});

