// @vitest-environment node
/**
 * Guards the job of `ci.yml` that decides whether a push needs the Windows jobs (T-079).
 *
 * The rule is a shell script inside the workflow, so that whoever changes it reads it where it
 * acts. This cuts that very script out of the file and runs it the way GitHub does, under
 * `bash -eo pipefail`, with `git` and `gh` replaced by functions that answer what each case
 * needs and write down how they were called. A wrong `false` is the one mistake the classifier
 * can make that no run ever shows: a push that changed code, green, with its Windows jobs
 * skipped. So every way to reach `false` is here, and so is every way found to reach it
 * wrongly. The wiring around the script, which jobs wait for it and on what, is read as text,
 * the way `release.test.ts` reads `release.yml`.
 */
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterAll, describe, expect, it } from 'vitest';

// `import.meta.url` is not a file URL under vitest (T-028), so every path starts at the root.
const ROOT = process.cwd();

const ci = readFileSync(join(ROOT, '.github', 'workflows', 'ci.yml'), 'utf8').replace(
  /\r\n/g,
  '\n',
);

/** The condition the three Windows jobs share, exactly as it is written. */
const WINDOWS_CONDITION = [
  '    if: >-',
  "      (github.event_name == 'workflow_dispatch' && inputs.windows) ||",
  "      (github.event_name != 'workflow_dispatch' && needs.changes.outputs.code == 'true')",
].join('\n');

const BEFORE = 'a'.repeat(40);
const AFTER = 'b'.repeat(40);

const temporary: string[] = [];

afterAll(() => {
  for (const dir of temporary) rmSync(dir, { recursive: true, force: true });
});

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

/** The `run: |` block of the step `classify`, dedented: the script GitHub runs. */
function classifier(): string {
  const lines = between(ci, '\n  changes:', '\n  docs:').split('\n');
  const step = lines.indexOf('        id: classify');
  const run = step === -1 ? -1 : lines.indexOf('        run: |', step);
  if (run === -1) throw new Error('the changes job has no classify step with a script');
  const body: string[] = [];
  for (const line of lines.slice(run + 1)) {
    if (line.trim() !== '' && !line.startsWith(' '.repeat(10))) break;
    body.push(line.slice(10));
  }
  return body.join('\n');
}

/**
 * GitHub runs a `run:` script as `bash --noprofile --norc -eo pipefail <file>`, and so does
 * this. On Windows that is Git for Windows' bash, the one `shell: bash` names: the `bash` a
 * Windows PATH finds first is often WSL's launcher in System32, which starts another system.
 */
function bash(): string {
  if (process.platform !== 'win32') return 'bash';
  const found = [process.env.ProgramW6432, process.env.ProgramFiles]
    .filter((dir): dir is string => dir !== undefined && dir !== '')
    .map((dir) => join(dir, 'Git', 'bin', 'bash.exe'))
    .find((path) => existsSync(path));
  if (found === undefined) throw new Error('no Git for Windows bash under Program Files');
  return found;
}

interface Case {
  /** `github.event_name`; a push when absent. */
  readonly event?: string;
  /** The commit compared with; an ordinary one when absent. */
  readonly before?: string;
  readonly forced?: boolean;
  /** What `git diff --name-only` prints, one path per entry, or `fail` for a diff that fails. */
  readonly diff?: readonly string[] | 'fail';
  /** What `gh run list … --jq` prints, the number of successful runs, or `fail`. */
  readonly passed?: string;
}

interface Answer {
  readonly code: string;
  /** What the step printed: the answer and its reason. */
  readonly said: string;
  /** Every `git` and `gh` command the step ran, in order. */
  readonly calls: readonly string[];
}

/** Runs the classifier over one case, with `git` and `gh` answering what the case says. */
function classify(input: Case): Answer {
  const dir = mkdtempSync(join(tmpdir(), 'baton-ci-'));
  temporary.push(dir);
  // Git for Windows' bash takes a Windows path written with forward slashes as it is.
  const at = (name: string) => join(dir, name).replaceAll('\\', '/');
  writeFileSync(at('output'), '');
  writeFileSync(at('calls'), '');
  const fakes = [
    'git() {',
    '  echo "git $*" >> "$CALLS"',
    '  if [ "$FAKE_DIFF" = fail ]; then return 128; fi',
    '  printf "%s" "$FAKE_DIFF"',
    '}',
    'gh() {',
    '  echo "gh $*" >> "$CALLS"',
    '  if [ "$FAKE_PASSED" = fail ]; then return 1; fi',
    '  printf "%s\\n" "$FAKE_PASSED"',
    '}',
  ].join('\n');
  writeFileSync(at('step.sh'), `${fakes}\n${classifier()}\n`);

  const result = spawnSync(bash(), ['--noprofile', '--norc', '-eo', 'pipefail', at('step.sh')], {
    encoding: 'utf8',
    env: {
      ...process.env,
      EVENT: input.event ?? 'push',
      FORCED: input.forced === true ? 'true' : 'false',
      BEFORE: input.before ?? BEFORE,
      AFTER,
      GH_TOKEN: 'not-a-token',
      GITHUB_REPOSITORY: 'Cepeppe/handoff-app',
      // Never the real one: under the `windows` job this suite runs inside a step of its own.
      GITHUB_OUTPUT: at('output'),
      CALLS: at('calls'),
      FAKE_DIFF: input.diff === 'fail' ? 'fail' : (input.diff ?? []).join('\n'),
      FAKE_PASSED: input.passed ?? '1',
    },
  });
  expect(result.error).toBeUndefined();
  expect(result.status, result.stderr).toBe(0);

  const answers = readFileSync(at('output'), 'utf8')
    .split('\n')
    .filter((line) => line !== '');
  expect(answers).toHaveLength(1);
  const code = /^code=(true|false)$/.exec(answers[0] ?? '')?.[1];
  expect(code, answers[0]).toBeDefined();
  return {
    code: code ?? '',
    said: result.stdout.trim(),
    calls: readFileSync(at('calls'), 'utf8')
      .split('\n')
      .filter((line) => line !== ''),
  };
}

describe('the classifier of ci.yml', () => {
  it('lets documentation alone skip the Windows jobs, on top of a commit that passed', () => {
    const answer = classify({
      diff: [
        'docs/verify-trust.md',
        'docs/it/verify-trust.md',
        'docs/dev/testing.md',
        'README.md',
        'CHANGELOG.md',
        'LICENSE',
      ],
      passed: '1',
    });
    expect(answer.code).toBe('false');
    expect(answer.calls).toEqual([
      `git diff --name-only --no-renames ${BEFORE} ${AFTER}`,
      `gh run list --repo Cepeppe/handoff-app --workflow ci.yml --event push --commit ${BEFORE} --json conclusion --jq map(select(.conclusion == "success")) | length`,
    ]);
  });

  const code: [string, string[]][] = [
    ['a change to the code beside the pages', ['docs/index.md', 'src-tauri/src/lib.rs']],
    ['the workflow itself', ['.github/workflows/ci.yml']],
    ['the English notices, which the windows job checks', ['docs/third-party-notices.md']],
    ['the Italian notices, checked the same way', ['docs/it/third-party-notices.md']],
    ['a README below the root', ['tests/ui/README.md']],
    ['a licence that ships inside the bundle', ['src-tauri/models/ocrs/LICENSE']],
    ['a folder whose name only begins like docs', ['docsx/index.md']],
    ['a docs folder that is not the root one', ['src/docs/index.md']],
    // Listed under both paths because the diff runs with `--no-renames`, which the case above
    // pins: with rename detection only `docs/moved.md` would be listed.
    ['a file moved into docs/', ['docs/moved.md', 'src/moved.ts']],
    ['a page moved out of docs/', ['docs/moved.md', 'scripts/moved.md']],
  ];
  it.each(code)('calls %s code', (_what, diff) => {
    expect(classify({ diff, passed: '1' }).code).toBe('true');
  });

  it('calls a diff that lists nothing, or cannot be read, code', () => {
    expect(classify({ diff: [] }).code).toBe('true');
    expect(classify({ diff: 'fail' }).code).toBe('true');
  });

  it('calls documentation code on top of a commit whose own run did not pass', () => {
    // A newer push cancels the older run, and a run can be red: either way no Windows job has
    // passed the code under the pages, and skipping would make `main` green over it.
    for (const passed of ['0', '', 'fail']) {
      const answer = classify({ diff: ['docs/index.md'], passed });
      expect(answer.code).toBe('true');
      expect(answer.said).toContain(`no push run of this workflow succeeded on ${BEFORE}`);
    }
  });

  it('calls code, without reading a diff, what has no commit to compare with', () => {
    for (const before of ['', '0'.repeat(40)]) {
      const answer = classify({ before, diff: ['docs/index.md'] });
      expect(answer.code).toBe('true');
      expect(answer.calls).toEqual([]);
    }
    const forced = classify({ forced: true, diff: ['docs/index.md'] });
    expect(forced.code).toBe('true');
    expect(forced.calls).toEqual([]);
  });

  it('calls a dispatch code and reads nothing: its Windows jobs follow its input instead', () => {
    const answer = classify({ event: 'workflow_dispatch', diff: ['docs/index.md'] });
    expect(answer.code).toBe('true');
    expect(answer.calls).toEqual([]);
  });

  it('reads a pull request from the merge base, and asks about the base', () => {
    const answer = classify({ event: 'pull_request', diff: ['docs/index.md'], passed: '1' });
    expect(answer.code).toBe('false');
    expect(answer.calls[0]).toBe(`git diff --name-only --no-renames ${BEFORE}...${AFTER}`);
    expect(answer.calls[1]).toContain(`--commit ${BEFORE}`);
  });

  it('takes what it reads from its environment, never from an expression', () => {
    // An expression inside a script is pasted in before bash parses it; through `env:` it is
    // data, whatever a branch or a path is called.
    expect(classifier()).not.toContain('${{');
  });
});

describe('ci.yml around the classifier', () => {
  it('makes the three Windows jobs wait for it, and a dispatch follow its input', () => {
    for (const job of ['windows', 'security', 'ui']) {
      const head = between(ci, `\n  ${job}:`, '\n    steps:');
      expect(head).toContain('\n    needs: changes\n');
      expect(head).toContain(WINDOWS_CONDITION);
    }
  });

  it('leaves the macOS leg as it was: dispatch only, waiting for nothing', () => {
    const head = between(ci, '\n  macos:', '\n    steps:');
    expect(head).toContain("\n    if: github.event_name == 'workflow_dispatch'\n");
    expect(head).not.toMatch(/^ {4}needs:/m);
  });

  it('checks the documentation on Linux on every push and pull request, whatever the answer', () => {
    const docs = between(ci, '\n  docs:', '\n  windows:');
    expect(docs).toContain('runs-on: ubuntu-latest');
    expect(docs).toContain("\n    if: github.event_name != 'workflow_dispatch'\n");
    expect(docs).not.toMatch(/^ {4}needs:/m);
    expect(docs).toContain('run: pnpm check:links');
    expect(ci.match(/run: pnpm check:links/g)).toHaveLength(1);
  });

  it('runs, in the docs job, every suite that reads a file the classifier lets through', () => {
    const command = /run: pnpm test (.+)\n/.exec(between(ci, '\n  docs:', '\n  windows:'))?.[1];
    const readers = readdirSync(join(ROOT, 'src', '__tests__'))
      .filter((name) => name.endsWith('.test.ts'))
      .filter((name) =>
        /join\(ROOT, '(?:docs'|README\.md'|CHANGELOG\.md'|LICENSE)/.test(
          readFileSync(join(ROOT, 'src', '__tests__', name), 'utf8'),
        ),
      );
    expect(readers).toEqual(expect.arrayContaining(['docs.test.ts', 'release.test.ts']));
    for (const name of readers) {
      expect(command?.split(' ')).toContain(`src/__tests__/${name}`);
    }
  });

  it('keeps the concurrency group, and lets the classifier alone read the runs', () => {
    expect(ci).toContain(
      'group: ci-${{ github.event_name }}-${{ github.ref }}\n  cancel-in-progress: true',
    );
    expect(ci).toMatch(/^permissions:\n {2}contents: read$/m);
    expect(ci.match(/^ +actions: read$/gm)).toHaveLength(1);
    expect(between(ci, '\n  changes:', '\n  docs:')).toMatch(/^ +actions: read$/m);
  });
});
