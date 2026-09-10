#!/usr/bin/env node
/**
 * The version a release of Baton carries, and the check that a tag names it (T-054).
 *
 * The number is written twice. `src-tauri/Cargo.toml` is the one that counts: `tauri.conf.json`
 * names no version of its own, so Tauri writes the crate's into the installer — the setup's
 * file name, its file properties, the `DisplayVersion` of the uninstall key — and
 * `CARGO_PKG_VERSION` is what the application reports to every peer (`app_version` in the
 * `hello` result, the crash files, a runbook's `origin`). `package.json` carries the same
 * number for the frontend. This prints the version the two agree on, and refuses when they do
 * not.
 *
 * Given a tag, it also refuses unless the tag is exactly `v<version>`. That is the first thing
 * the release workflow does, before a Windows minute is spent: a setup whose number is not its
 * release's cannot be corrected once somebody has downloaded it, and the in-place update of
 * FM-24 names the server it moves aside after the version the installation recorded.
 *
 * Usage:
 *
 *   node scripts/release-version.mjs [--root <dir>]           # prints the version
 *   node scripts/release-version.mjs [--root <dir>] <tag>     # and checks that the tag is v<version>
 *
 * Exit codes: 0 consistent · 1 not consistent · 2 the arguments are wrong.
 */
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Semantic Versioning 2.0.0, without the leading `v`. */
const SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

export class VersionError extends Error {}

/** The `version` of the `[package]` table of a Cargo manifest, and no other table's. */
export function cargoVersion(manifest) {
  let inPackage = false;
  for (const raw of manifest.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.startsWith('[')) {
      inPackage = line === '[package]';
      continue;
    }
    if (!inPackage) continue;
    const version = /^version\s*=\s*"([^"]*)"/.exec(line)?.[1];
    if (version !== undefined) return version;
  }
  throw new VersionError('src-tauri/Cargo.toml has no version in its [package] table');
}

/** The one version `src-tauri/Cargo.toml` and `package.json` agree on. */
export function applicationVersion(root) {
  const cargo = cargoVersion(readFileSync(join(root, 'src-tauri', 'Cargo.toml'), 'utf8'));
  const npm = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version;
  if (cargo !== npm) {
    throw new VersionError(
      `src-tauri/Cargo.toml says ${cargo} and package.json says ${npm}: a release needs both at the same version`,
    );
  }
  if (!SEMVER.test(cargo)) throw new VersionError(`${cargo} is not a semantic version`);
  return cargo;
}

/** Refuses a tag that is not exactly `v<version>`. */
export function checkTag(tag, version) {
  if (tag !== `v${version}`) {
    throw new VersionError(
      `tag ${tag} does not match the application version ${version}; the tag of this release is v${version}`,
    );
  }
}

function usage(message) {
  process.stderr.write(`release-version: ${message}\n`);
  process.stderr.write('usage: node scripts/release-version.mjs [--root <dir>] [<tag>]\n');
  return 2;
}

function main(argv) {
  let root = join(dirname(fileURLToPath(import.meta.url)), '..');
  const positional = [];
  for (let at = 0; at < argv.length; at += 1) {
    if (argv[at] === '--root') {
      const value = argv[at + 1];
      if (value === undefined) return usage('--root needs a folder');
      root = resolve(value);
      at += 1;
    } else if (argv[at].startsWith('--')) {
      return usage(`unknown option ${argv[at]}`);
    } else {
      positional.push(argv[at]);
    }
  }
  if (positional.length > 1) return usage('one tag at most');

  try {
    const version = applicationVersion(root);
    if (positional.length === 1) checkTag(positional[0], version);
    process.stdout.write(`${version}\n`);
    return 0;
  } catch (error) {
    if (!(error instanceof VersionError)) throw error;
    process.stderr.write(`release-version: ${error.message}\n`);
    return 1;
  }
}

// Importable, runnable as a script. Compared as resolved paths: on Windows `import.meta.url`
// is a `file:///C:/...` URL and `argv[1]` a backslashed path.
if (process.argv[1] !== undefined && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  process.exitCode = main(process.argv.slice(2));
}
