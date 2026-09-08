/**
 * GUIDE-05 and PRIN-07: no percentage progress bar and no estimated time, anywhere.
 *
 * The requirement is about the product, not about one component, so the check is over the
 * whole frontend source rather than over a rendered tree: a later view that reaches for a
 * `<progress>` element or a progress role fails the build here, which is the only place
 * that sees all of them.
 */
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';

import { describe, expect, it } from 'vitest';

// Resolved from the working directory rather than from `import.meta.url`: under the jsdom
// environment the module URL is an `http://localhost` one and `fileURLToPath` refuses it.
// Vitest runs with the project root as the working directory.
const SOURCE_DIR = resolve(process.cwd(), 'src');

/** The shapes a progress indicator takes in HTML, whatever names the code gives it. */
const FORBIDDEN: ReadonlyArray<{ what: string; pattern: RegExp }> = [
  { what: 'a <progress> element', pattern: /<progress[\s/>]/i },
  { what: 'a progressbar role', pattern: /role\s*=\s*["'{]?\s*progressbar/i },
  { what: 'an aria-valuenow attribute', pattern: /aria-valuenow/i },
];

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      // The tests name the forbidden shapes on purpose; the check is about what ships.
      return entry === '__tests__' ? [] : sourceFiles(path);
    }
    return /\.(svelte|ts|css|html)$/.test(entry) ? [path] : [];
  });
}

describe('the frontend', () => {
  it('contains no progress bar of any shape', () => {
    expect(existsSync(SOURCE_DIR), `${SOURCE_DIR} is not the frontend source folder`).toBe(true);
    const files = sourceFiles(SOURCE_DIR);
    expect(files.length).toBeGreaterThan(0);

    const offenders = files.flatMap((path) => {
      const text = readFileSync(path, 'utf8');
      return FORBIDDEN.filter(({ pattern }) => pattern.test(text)).map(
        ({ what }) => `${path}: ${what}`,
      );
    });

    expect(offenders).toEqual([]);
  });
});
