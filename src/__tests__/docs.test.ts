// @vitest-environment node
/**
 * Guards the user documentation of `docs/` (T-052: NET-02, NFR-05, NFR-09, NFR-16).
 *
 * Three things are checked. The link check of `scripts/check-links.mjs` can actually fail,
 * so it is run once against a tree built to break it — a green check that cannot go red is
 * worth nothing. Every page exists in English and in Italian, and each index links every page
 * of its language. And the statements the task exists for are really on the pages: the
 * firewall test with its zero-connection expectation, the process-tree command — the same in
 * both languages, and never the `-OwningProcess` form that silently answers nothing — the
 * WebView2 switches the windows are really started with, and the `ocrs` attribution word for
 * word as it ships beside the models.
 */
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import { afterAll, describe, expect, it } from 'vitest';

// `import.meta.url` is not a file URL under vitest (T-028), so every path starts at the root.
const ROOT = process.cwd();
const DOCS = join(ROOT, 'docs');
const CHECKER = join(ROOT, 'scripts', 'check-links.mjs');

/** The pages the T-052 deliverables name, plus the index that links them. */
const PAGES = [
  'index.md',
  'install-macos.md',
  'install-windows.md',
  'consent-screen.md',
  'using-the-overlay.md',
  'screenshots-and-privacy.md',
  'runbooks.md',
  'log-and-export.md',
  'verify-trust.md',
  'crash-reports.md',
  'troubleshooting.md',
  'third-party-notices.md',
] as const;

/** A page, with its line endings normalised: the check is about the words. */
function read(...path: string[]): string {
  return readFileSync(join(DOCS, ...path), 'utf8').replace(/\r\n/g, '\n');
}

interface Run {
  readonly status: number;
  readonly stdout: string;
  readonly stderr: string;
}

function check(root: string): Run {
  try {
    const stdout = execFileSync(process.execPath, [CHECKER, root], {
      encoding: 'utf8',
      stdio: 'pipe',
    });
    return { status: 0, stdout, stderr: '' };
  } catch (error) {
    const failure = error as { status?: number; stdout?: string; stderr?: string };
    return {
      status: failure.status ?? -1,
      stdout: failure.stdout ?? '',
      stderr: failure.stderr ?? '',
    };
  }
}

const temporaries: string[] = [];

function tree(files: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), 'baton-links-'));
  temporaries.push(root);
  for (const [name, content] of Object.entries(files)) {
    const path = join(root, name);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, content, 'utf8');
  }
  return root;
}

afterAll(() => {
  for (const root of temporaries) rmSync(root, { recursive: true, force: true });
});

describe('the link checker catches what it claims to', () => {
  it('reports a missing page and an anchor no heading produces', () => {
    const root = tree({
      'docs/a.md': '# A\n\n## Real heading\n\n[gone](b.md) and [x](#imaginary-heading)\n',
    });
    const result = check(root);
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('docs/a.md:5: "b.md" does not exist');
    expect(result.stderr).toContain('"#imaginary-heading" has no matching heading in this file');
  });

  it('ignores links in code, external links, and the build output of target/', () => {
    const root = tree({
      'docs/a.md': '# A\n\n`[not a link](nowhere.md)` and [real](https://example.invalid/x)\n',
      'src-tauri/target/doc/b.md': '[gone](missing.md)\n',
    });
    const result = check(root);
    expect(result.status).toBe(0);
    expect(result.stdout).toContain('0 relative link(s) in 1 markdown file(s)');
  });
});

describe('the documentation of this repository', () => {
  it('has no broken relative link in any markdown file', () => {
    const result = check(ROOT);
    expect(result.stderr).toBe('');
    expect(result.status).toBe(0);
  });

  it('has every page in English and in Italian, and nothing else', () => {
    const listed = (dir: string) =>
      readdirSync(dir)
        .filter((name) => name.endsWith('.md'))
        .sort();
    const expected = [...PAGES].sort();
    expect(listed(DOCS)).toEqual(expected);
    expect(listed(join(DOCS, 'it'))).toEqual(expected);
  });

  it.each(PAGES.filter((page) => page !== 'index.md'))(
    'links %s from both index pages',
    (page) => {
      expect(read('index.md')).toContain(`](${page})`);
      expect(read('it', 'index.md')).toContain(`](${page})`);
    },
  );

  it('is linked from the README', () => {
    expect(readFileSync(join(ROOT, 'README.md'), 'utf8')).toContain('](docs/index.md)');
  });
});

describe('what the pages must say', () => {
  it('states the zero-connection expectation of the firewall test (NET-02)', () => {
    expect(read('verify-trust.md')).toContain(
      "In this build the expected number of connections from Baton's own programs is **zero**.",
    );
    expect(read('it', 'verify-trust.md')).toContain(
      'In questa versione il numero di connessioni atteso dai programmi di Baton è **zero**.',
    );
    for (const page of [read('verify-trust.md'), read('it', 'verify-trust.md')]) {
      expect(page).toContain('New-NetFirewallRule');
      expect(page).toContain('Remove-NetFirewallRule');
    }
  });

  it('gives one process-tree command, the same in both languages, in a form that sees', () => {
    const block = (page: string) =>
      /```powershell\n(# Baton network watch[\s\S]*?)```/.exec(page)?.[1];
    const english = block(read('verify-trust.md'));
    expect(english).toBeTruthy();
    expect(block(read('it', 'verify-trust.md'))).toBe(english);
    // The whole tree, walked by parent: a firewall rule on Baton's own executables cannot
    // see WebView2, which is a different program (the owner's answer in T-052).
    expect(english).toContain('ParentProcessId');
    expect(english).toContain('Get-NetTCPConnection');
    expect(english).toContain('Get-NetUDPEndpoint');
    // `-OwningProcess` with an array of pids answers nothing at all, silently, on this
    // PowerShell (a type mismatch the error preference hides): measured while writing it.
    expect(english).not.toContain('-OwningProcess');
  });

  it('names every switch the windows start WebView2 with', () => {
    const config = JSON.parse(
      readFileSync(join(ROOT, 'src-tauri', 'tauri.conf.json'), 'utf8'),
    ) as { app: { windows: { additionalBrowserArgs: string }[] } };
    const switches = config.app.windows[0].additionalBrowserArgs
      .split(' ')
      .filter((arg) => !arg.startsWith('--disable-features='));
    expect(switches.length).toBeGreaterThanOrEqual(4);
    for (const page of [read('verify-trust.md'), read('it', 'verify-trust.md')]) {
      for (const arg of switches) expect(page).toContain(`\`${arg}\``);
    }
  });

  it('gives the SmartScreen steps of an unsigned build (NFR-09)', () => {
    expect(read('install-windows.md')).toContain('**More info**');
    expect(read('install-windows.md')).toContain('**Run anyway**');
    expect(read('it', 'install-windows.md')).toContain('**Ulteriori informazioni**');
    expect(read('it', 'install-windows.md')).toContain('**Esegui comunque**');
  });

  it('carries the ocrs attribution word for word, as it ships beside the models', () => {
    const licence = readFileSync(join(ROOT, 'src-tauri', 'models', 'ocrs', 'LICENSE'), 'utf8')
      .replace(/\r\n/g, '\n')
      .trim();
    expect(read('third-party-notices.md')).toContain(licence);
    expect(read('it', 'third-party-notices.md')).toContain(licence);
  });

  it('lists the crates of the shipped build in both notices', () => {
    for (const page of [read('third-party-notices.md'), read('it', 'third-party-notices.md')]) {
      const generated = /<!-- crates:begin[^\n]*-->\n([\s\S]*?)<!-- crates:end -->/.exec(page);
      expect(generated).toBeTruthy();
      const rows = (generated?.[1] ?? '').split('\n').filter((line) => line.startsWith('| '));
      // A header row, then one row per crate; the graph has several hundred.
      expect(rows.length).toBeGreaterThan(100);
      expect(page).toMatch(/\n\| ocrs \| [^|]+ \| [^|]*MIT[^|]* \|\n/);
      expect(page).toMatch(/\n\| rten \| [^|]+ \| [^|]*MIT[^|]* \|\n/);
      // The HTTP client is behind a feature the shipped build does not enable (T-051).
      expect(page).not.toMatch(/\n\| reqwest \|/);
    }
  });
});
