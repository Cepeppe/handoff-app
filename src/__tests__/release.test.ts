// @vitest-environment node
/**
 * Guards the release pipeline of the application (T-054).
 *
 * Everything here runs off-line and in about a second. The two scripts `release.yml` calls
 * are run through their command line, and the workflow itself is read as text, because the
 * parts of a release that can go wrong quietly — a tag that is not the version, a gate that
 * reads a binary other than the one shipped, a positive control nobody builds any more, a
 * report that stopped being attached, a job that can write to the repository without needing
 * to — are decisions written in that file and nowhere else. The shape follows
 * `handoff-mcp`'s `test/unit/release.test.ts`, which guards the server's release.
 */
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterAll, describe, expect, it } from 'vitest';

// `import.meta.url` is not a file URL under vitest (T-028), so every path starts at the root.
const ROOT = process.cwd();
const RELEASE_VERSION = join(ROOT, 'scripts', 'release-version.mjs');
const CHANGELOG_SECTION = join(ROOT, 'scripts', 'changelog-section.mjs');

const release = readFileSync(join(ROOT, '.github', 'workflows', 'release.yml'), 'utf8').replace(
  /\r\n/g,
  '\n',
);
const packageJson = JSON.parse(readFileSync(join(ROOT, 'package.json'), 'utf8')) as {
  version: string;
};

const temporary: string[] = [];

function temporaryDir(): string {
  const dir = mkdtempSync(join(tmpdir(), 'baton-release-'));
  temporary.push(dir);
  return dir;
}

afterAll(() => {
  for (const dir of temporary) rmSync(dir, { recursive: true, force: true });
});

function run(script: string, args: string[]): { status: number; stdout: string; stderr: string } {
  const result = spawnSync(process.execPath, [script, ...args], { encoding: 'utf8' });
  return { status: result.status ?? -1, stdout: result.stdout, stderr: result.stderr };
}

/**
 * The slice of the workflow after `from` and before `to`. A marker that is not there is a
 * failure of the test rather than an empty string quietly satisfying every assertion.
 */
function between(text: string, from: string, to?: string): string {
  const parts = text.split(from);
  if (parts.length < 2) throw new Error(`the workflow has no ${JSON.stringify(from)}`);
  const after = parts.slice(1).join(from);
  if (to === undefined) return after;
  const end = after.split(to);
  if (end.length < 2) throw new Error(`no ${JSON.stringify(to)} after ${JSON.stringify(from)}`);
  return end[0] ?? '';
}

/** A repository root holding only the two files the version is read from. */
function manifests(cargo: string, npm: string): string {
  const root = temporaryDir();
  mkdirSync(join(root, 'src-tauri'));
  writeFileSync(join(root, 'src-tauri', 'Cargo.toml'), cargo);
  writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'x', version: npm }));
  return root;
}

const cargoManifest = (version: string) =>
  `[package]\nname = "handoff-app"\nversion = "${version}"\n\n[dependencies]\nserde = { version = "1" }\n`;

describe('release-version.mjs', () => {
  it('prints the version Cargo.toml and package.json agree on', () => {
    const result = run(RELEASE_VERSION, []);
    expect(result.stderr).toBe('');
    expect(result.status).toBe(0);
    expect(result.stdout.trim()).toBe(packageJson.version);
  });

  it('accepts the tag of that version and no other', () => {
    expect(run(RELEASE_VERSION, [`v${packageJson.version}`]).status).toBe(0);

    const other = run(RELEASE_VERSION, ['v0.9.0-not-this-one']);
    expect(other.stderr).toContain('does not match the application version');
    expect(other.status).toBe(1);

    const bare = run(RELEASE_VERSION, [packageJson.version]);
    expect(bare.stderr).toContain(`the tag of this release is v${packageJson.version}`);
    expect(bare.status).toBe(1);
  });

  it('refuses when the two manifests disagree', () => {
    const root = manifests(cargoManifest('1.2.3'), '1.2.4');
    const result = run(RELEASE_VERSION, ['--root', root]);
    expect(result.stderr).toContain('src-tauri/Cargo.toml says 1.2.3 and package.json says 1.2.4');
    expect(result.status).toBe(1);
  });

  it('reads the version of the [package] table, not a dependency line that looks like one', () => {
    const cargo =
      '[dependencies]\nversion = "9.9.9"\n\n[package]\nname = "handoff-app"\nversion = "0.9.0-test.1"\n';
    const root = manifests(cargo, '0.9.0-test.1');
    const result = run(RELEASE_VERSION, ['--root', root, 'v0.9.0-test.1']);
    expect(result.stderr).toBe('');
    expect(result.stdout.trim()).toBe('0.9.0-test.1');
    expect(result.status).toBe(0);
  });

  it('refuses a number that is not a semantic version', () => {
    const root = manifests(cargoManifest('1.2'), '1.2');
    const result = run(RELEASE_VERSION, ['--root', root]);
    expect(result.stderr).toContain('1.2 is not a semantic version');
    expect(result.status).toBe(1);
  });

  it('refuses arguments it does not understand', () => {
    expect(run(RELEASE_VERSION, ['--tag', 'v1.0.0']).status).toBe(2);
    expect(run(RELEASE_VERSION, ['v1.0.0', 'v2.0.0']).status).toBe(2);
  });
});

describe('changelog-section.mjs', () => {
  function changelog(text: string): string {
    const file = join(temporaryDir(), 'CHANGELOG.md');
    writeFileSync(file, text);
    return file;
  }

  it('prints the section of one version, without its heading', () => {
    const result = run(CHANGELOG_SECTION, [
      '1.0.0',
      changelog('# Changelog\n\n## [Unreleased]\n\n- next\n\n## [1.0.0] - 2026-01-01\n\n- first\n'),
    ]);
    expect(result.status).toBe(0);
    expect(result.stdout).toBe('- first\n');
  });

  it('fails on a version the changelog does not have', () => {
    const result = run(CHANGELOG_SECTION, ['4.2.0']);
    expect(result.stderr).toContain('has no section for 4.2.0');
    expect(result.status).not.toBe(0);
  });

  it('fails on a section with nothing in it', () => {
    const result = run(CHANGELOG_SECTION, [
      '1.0.0',
      changelog('# Changelog\n\n## [1.0.0] - 2026-01-01\n\n## [0.9.0] - 2025-12-01\n\n- old\n'),
    ]);
    expect(result.stderr).toContain('is empty');
    expect(result.status).not.toBe(0);
  });

  it('does not stop at a heading written inside a fenced block', () => {
    const result = run(CHANGELOG_SECTION, [
      '1.0.0',
      changelog(
        '# Changelog\n\n## [1.0.0] - 2026-01-01\n\n- first\n\n```md\n## [0.1.0]\n```\n\n- last\n\n## [0.9.0] - 2025-12-01\n\n- old\n',
      ),
    ]);
    expect(result.status).toBe(0);
    expect(result.stdout).toContain('- first');
    expect(result.stdout).toContain('- last');
    expect(result.stdout).not.toContain('- old');
  });

  it('reads the changelog of this repository', () => {
    const text = readFileSync(join(ROOT, 'CHANGELOG.md'), 'utf8');
    expect(text).toMatch(/^## \[Unreleased\]$/m);
  });
});

describe('release.yml', () => {
  it('runs on version tags only, and is never cancelled half way', () => {
    expect(release).toMatch(/\non:\n {2}push:\n {4}tags: \['v\*'\]\n\n/);
    expect(release).toContain('cancel-in-progress: false');
  });

  it('refuses a tag that is not the version, and a version without notes, before building', () => {
    const guard = between(release, '\n  guard:', '\n  security:');
    expect(guard).toContain('node scripts/release-version.mjs "$GITHUB_REF_NAME"');
    expect(guard).toContain('node scripts/changelog-section.mjs "$VERSION"');
    expect(guard).toContain('runs-on: ubuntu-latest');
    for (const job of ['security', 'windows']) {
      expect(between(release, `\n  ${job}:`, '\n    steps:')).toContain('needs: guard');
    }
    expect(between(release, '\n  publish:', '\n    steps:')).toContain(
      'needs: [guard, security, windows]',
    );
  });

  it('builds the setup from the pinned server, checked the strict way', () => {
    const job = between(release, '\n  windows:', '\n  publish:');
    expect(job).toContain('HANDOFF_MCP_READ_TOKEN: ${{ secrets.HANDOFF_MCP_READ_TOKEN }}');
    expect(job).toContain('run: node scripts/fetch-server.mjs --check');
    expect(job).toContain('run: pnpm tauri build\n');
    expect(job).not.toContain('--debug');
  });

  it('gates the binary the setup was packed from, before the control is built over it', () => {
    const job = between(release, '\n  windows:', '\n  publish:');
    const order = [
      'run: pnpm tauri build\n',
      'run: node scripts/check-no-automation.mjs src-tauri/target/release/handoff-app.exe',
      'cp "$setup" "release-assets/Baton-${VERSION}-win32-x64-setup.exe"',
      'run: pnpm tauri build --features e2e --no-bundle',
      'run: node scripts/check-no-automation.mjs --present src-tauri/target/release/handoff-app.exe',
    ];
    const at = order.map((step) => job.indexOf(step));
    for (const [index, position] of at.entries()) {
      expect(position, `${order[index]} is in the job`).toBeGreaterThan(-1);
    }
    expect([...at].sort((a, b) => a - b)).toEqual(at);
  });

  it('keeps the setup out of the folder the control build empties', () => {
    // The control build runs `beforeBuildCommand`, and its `vite build` empties `dist/`
    // (`emptyOutDir` in vite.config.ts): the first rehearsal of this workflow set the setup
    // aside under `dist/` and had nothing left to upload.
    // The steps only: the comment that explains the rule names the folder too.
    const steps = between(release, '\n  windows:', '\n  publish:')
      .split('\n')
      .filter((line) => !line.trimStart().startsWith('#'))
      .join('\n');
    expect(steps).not.toMatch(/\bdist\//);
    expect(steps).toContain('path: release-assets/*-win32-x64-setup.exe');
  });

  it('attaches the report of the security suite run on the tagged commit', () => {
    const security = between(release, '\n  security:', '\n  windows:');
    expect(security).toContain('cargo test --test security -- --nocapture');
    expect(security).toContain('name: security-report');
    expect(security).toContain('save-if: false');
    const publish = between(release, '\n  publish:');
    expect(publish).toContain('name: security-report');
    expect(publish).toContain('"dist/assets/Baton-${VERSION}-security-report.json"');
  });

  it('drafts the release with the checksums and the changelog notes, and publishes nothing', () => {
    const publish = between(release, '\n  publish:');
    expect(publish).toContain('sha256sum Baton-* > SHA256SUMS');
    expect(publish).toContain('node scripts/changelog-section.mjs "$VERSION" > dist/notes.md');
    expect(publish).toContain('--notes-file dist/notes.md');
    expect(publish).toContain('gh release create "v$VERSION" --draft');
    expect(publish).toContain('--verify-tag');
    expect(release).not.toMatch(/gh release (edit|upload)|--draft=false|npm publish/);
  });

  it('lets only the job that drafts the release write to the repository', () => {
    expect(release).toMatch(/^permissions:\n {2}contents: read$/m);
    expect(release.match(/contents: write/g)).toHaveLength(1);
    expect(between(release, '\n  publish:')).toContain('contents: write');
  });

  it('has no macOS job until the macOS release exists', () => {
    expect(release).not.toMatch(/runs-on: macos/);
    expect(release).toContain('TASK: T-060');
  });
});
