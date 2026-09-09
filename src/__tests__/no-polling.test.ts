/**
 * WIN-06 and NFR-14, the window's half: at rest the app is an icon and a listening socket.
 *
 * `src-tauri/tests/timers.rs` is the registry for the Rust process; this is the same
 * promise over the webview, which lives in that process and burns the same battery. The
 * mistake it exists to catch is `setInterval`: a repaint every second, a "refresh the tab
 * strip" poll, a countdown — none of which fails anything, and all of which wake a machine
 * whose panel is hidden.
 *
 * The frontend is event-driven by construction (`onNotice`, `onTabsChanged`,
 * `onSessionsChanged`, `onWindowFocus`), so it needs no timer to stay current. The two
 * one-shot `setTimeout`s it does use are declared below, each armed by something the user
 * did and each firing once.
 */
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';

import { describe, expect, it } from 'vitest';

// Resolved from the working directory rather than from `import.meta.url`, for the reason
// `no-progress-bar.test.ts` gives: under jsdom the module URL is an `http://localhost` one.
const SOURCE_DIR = resolve(process.cwd(), 'src');

/** Every one-shot timer the frontend is allowed to arm, and what arms it. */
const DECLARED_TIMEOUTS: ReadonlyArray<{ file: string; count: number; armedWhen: string }> = [
  {
    file: 'overlay/collapse.svelte.ts',
    count: 1,
    armedWhen:
      'R-10: the collapse fallback, and only when the user switched the setting on. It is ' +
      'cleared and re-armed by an interaction, and never armed at all by default.',
  },
  {
    file: 'overlay/state.svelte.ts',
    count: 1,
    armedWhen:
      'DET-04: the ten seconds a revealed secret stays on screen, armed by the press of ' +
      '**Show** and cleared when the value is hidden again.',
  },
];

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      // The tests name the forbidden shapes on purpose; the check is about what ships.
      return entry === '__tests__' ? [] : sourceFiles(path);
    }
    return /\.(svelte|ts)$/.test(entry) ? [path] : [];
  });
}

/** `path` as the declarations spell it: relative to `src/`, forward slashes. */
function relative(path: string): string {
  return path.slice(SOURCE_DIR.length + 1).replace(/\\/gu, '/');
}

/** How many times `pattern` is called in each file, ignoring type positions and comments. */
function callsPerFile(pattern: RegExp): Map<string, number> {
  const counts = new Map<string, number>();
  for (const path of sourceFiles(SOURCE_DIR)) {
    for (const line of readFileSync(path, 'utf8').split(/\r?\n/u)) {
      const code = line.trim();
      // `ReturnType<typeof setTimeout>` is a type and arms nothing; a comment about a timer
      // is documentation. Both are ordinary in this codebase.
      if (code.startsWith('*') || code.startsWith('//') || /typeof\s+set/u.test(code)) {
        continue;
      }
      const found = code.match(pattern);
      if (found !== null) {
        counts.set(relative(path), (counts.get(relative(path)) ?? 0) + found.length);
      }
    }
  }
  return counts;
}

describe('the frontend at rest', () => {
  it('is the source folder this suite thinks it is', () => {
    expect(existsSync(SOURCE_DIR), `${SOURCE_DIR} is not the frontend source folder`).toBe(true);
    expect(sourceFiles(SOURCE_DIR).length).toBeGreaterThan(20);
  });

  it('polls on no interval at all', () => {
    // The whole point. Everything the window shows arrives as an event from the core.
    expect([...callsPerFile(/\bsetInterval\s*\(/gu).keys()]).toEqual([]);
  });

  it('arms only the one-shot timers that are declared, each with what arms it', () => {
    const armed = callsPerFile(/\bsetTimeout\s*\(/gu);
    const declared = new Map(DECLARED_TIMEOUTS.map((timer) => [timer.file, timer.count]));

    expect(
      [...armed.keys()].filter((file) => !declared.has(file)),
      'a timer nobody declared: add it to DECLARED_TIMEOUTS with what arms it, or do not arm it',
    ).toEqual([]);
    expect([...armed.entries()].sort()).toEqual([...declared.entries()].sort());

    for (const timer of DECLARED_TIMEOUTS) {
      expect(timer.armedWhen.length, `${timer.file} has no reason worth reading`).toBeGreaterThan(
        40,
      );
    }
  });
});
