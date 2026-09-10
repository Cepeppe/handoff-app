// Prints the `CHANGELOG.md` section of one version, which is the body of its draft release
// (T-054). Copied from `handoff-mcp/build/changelog-section.mjs` (T-011), which does the same
// job for the server, rather than rewritten: the two repositories keep one changelog format.
//
// The changelog is Keep a Changelog, so a version is the heading `## [<version>] - <date>`
// and its section runs to the next `## ` heading. The heading itself is not printed: the
// release page already shows the tag and the date.
//
// Failing loudly matters more than the output. A tag whose section is missing would
// otherwise produce a release with an empty body and nobody would notice until someone
// went looking for the notes, so an unknown version and an empty section are both errors,
// and the release workflow runs this before it builds anything.
//
// Usage: node scripts/changelog-section.mjs <version> [path-to-changelog]
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');

class ChangelogError extends Error {}

function fail(message) {
  throw new ChangelogError(message);
}

/** The heading of a released version; `Unreleased` is deliberately not one of them. */
function headingOf(version) {
  return `## [${version}]`;
}

/**
 * The body of the section, with its surrounding blank lines removed. `lines` is scanned
 * rather than split by a regular expression so that a `## ` written inside a fenced block
 * cannot end the section early.
 */
function changelogSection(text, version) {
  const lines = text.split(/\r?\n/);
  const start = lines.findIndex((line) => line.startsWith(headingOf(version)));
  if (start === -1) {
    const known = lines
      .filter((line) => line.startsWith('## ['))
      .map((line) => line.slice(4, line.indexOf(']')));
    fail(`CHANGELOG.md has no section for ${version} (it has ${known.join(', ') || 'none'})`);
  }

  const body = [];
  let fenced = false;
  for (const line of lines.slice(start + 1)) {
    if (line.startsWith('```')) fenced = !fenced;
    if (!fenced && line.startsWith('## ')) break;
    body.push(line);
  }

  const section = body.join('\n').trim();
  if (section === '') fail(`the ${version} section of CHANGELOG.md is empty`);
  return section;
}

function main(argv) {
  const [version, path] = argv;
  if (version === undefined) fail('usage: node scripts/changelog-section.mjs <version> [path]');
  const file = path ?? join(repoRoot, 'CHANGELOG.md');
  process.stdout.write(`${changelogSection(readFileSync(file, 'utf8'), version)}\n`);
}

try {
  main(process.argv.slice(2));
} catch (error) {
  if (!(error instanceof ChangelogError)) throw error;
  console.error(`changelog-section: ${error.message}`);
  process.exitCode = 2;
}
