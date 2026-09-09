#!/usr/bin/env node
/**
 * The automation channel must not be in a shipped binary (DD-33, TECHNICAL-DESIGN §11.5).
 *
 * §11.5 states the rule and its reason in one sentence — "a hidden control channel in a
 * trust-sensitive app must not ship" — and gives the check: CI asserts the symbol is absent
 * from release binaries. A feature flag is a promise; this is what makes it a fact.
 *
 * What is searched for are the two names the endpoint cannot exist without, in the encodings
 * a Rust binary stores a string literal in:
 *
 *   handoff-e2e   the named-pipe prefix
 *   e2e.sock      the socket file name
 *
 * Both live in `src-tauri/src/e2e/endpoint.rs` and nowhere else, so a build that has the
 * feature off contains neither, and a build that has it on contains both. Rust puts a `&str`
 * in `.rodata` as plain UTF-8 with no terminator, so a byte search finds them; UTF-16 is
 * searched too, because a Windows resource or a widened literal would store them that way.
 *
 * Usage:
 *
 *   node scripts/check-no-automation.mjs <binary>              # they must be absent
 *   node scripts/check-no-automation.mjs --present <binary>    # they must be present
 *
 * The second form is the positive control, and it is not decoration: a check that greps for
 * a string nobody ever writes passes for ever, including on the day the grep itself breaks.
 * CI runs both — absent from the release build, present in the e2e build — so the two runs
 * together say the check can still fail.
 *
 * Exit codes: 0 the binary is as it should be · 1 it is not · 2 the arguments are wrong.
 */
import { readFileSync, statSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The names that betray the automation channel. Both are in `e2e/endpoint.rs`. */
export const AUTOMATION_MARKERS = ['handoff-e2e', 'e2e.sock'];

/**
 * Every occurrence of `needle` in `haystack`, as UTF-8 and as UTF-16LE.
 *
 * Two encodings rather than one: a `&str` literal is UTF-8 in `.rodata`, but a name that
 * reached a Windows API, a manifest or a resource table can be widened on the way, and a
 * check that only looked for one of the two would report a clean binary that is not.
 */
export function occurrences(haystack, needle) {
  const encodings = [Buffer.from(needle, 'utf8'), Buffer.from(needle, 'utf16le')];
  const found = [];
  for (const pattern of encodings) {
    let at = haystack.indexOf(pattern);
    while (at !== -1) {
      found.push(at);
      at = haystack.indexOf(pattern, at + 1);
    }
  }
  return found;
}

/** Which markers a binary carries, and where the first occurrence of each one is. */
export function scan(bytes, markers = AUTOMATION_MARKERS) {
  return markers.map((marker) => {
    const at = occurrences(bytes, marker);
    return { marker, count: at.length, first: at[0] };
  });
}

function usage(message) {
  process.stderr.write(`check-no-automation: ${message}\n`);
  process.stderr.write(
    'usage: node scripts/check-no-automation.mjs [--present] <path to a built binary>\n',
  );
  return 2;
}

function main(argv) {
  const expectPresent = argv.includes('--present');
  const paths = argv.filter((argument) => !argument.startsWith('--'));
  if (paths.length !== 1) return usage('one binary, please');
  const [binary] = paths;

  let bytes;
  try {
    if (!statSync(binary).isFile()) return usage(`${binary} is not a file`);
    bytes = readFileSync(binary);
  } catch (cause) {
    return usage(`${binary} could not be read (${cause.message})`);
  }

  const results = scan(bytes);
  const carried = results.filter((result) => result.count > 0);
  const size = `${(bytes.length / (1024 * 1024)).toFixed(1)} MB`;

  if (expectPresent) {
    const missing = results.filter((result) => result.count === 0);
    if (missing.length > 0) {
      process.stderr.write(
        `check-no-automation: ${binary} (${size}) should carry the automation channel but ` +
          `${missing.map((result) => result.marker).join(' and ')} ` +
          `${missing.length === 1 ? 'is' : 'are'} not in it.\n` +
          'Either it was built without --features e2e, or this check no longer finds what it looks for.\n',
      );
      return 1;
    }
    process.stderr.write(
      `check-no-automation: ${binary} (${size}) carries the automation channel, as an e2e build must.\n`,
    );
    return 0;
  }

  if (carried.length > 0) {
    process.stderr.write(
      `check-no-automation: ${binary} (${size}) CARRIES THE AUTOMATION CHANNEL.\n` +
        carried
          .map(
            (result) =>
              `  ${result.marker}: ${result.count} occurrence(s), first at byte ${result.first}\n`,
          )
          .join('') +
        'A release build must not contain it (TECHNICAL-DESIGN §11.5, DD-33). Build without\n' +
        '--features e2e, and check that nothing outside src/e2e/ names the endpoint.\n',
    );
    return 1;
  }

  process.stderr.write(
    `check-no-automation: ${binary} (${size}) carries no automation channel. ` +
      `Searched for ${AUTOMATION_MARKERS.join(', ')} as UTF-8 and UTF-16.\n`,
  );
  return 0;
}

// Importable for the unit test, runnable as a script. Compared as resolved paths rather
// than by suffix: on Windows `import.meta.url` is a `file:///C:/...` URL and `argv[1]` a
// backslashed path, and a suffix test between the two is a coin toss.
if (process.argv[1] !== undefined && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  process.exitCode = main(process.argv.slice(2));
}
