/**
 * The real Kilo CLI, run non-interactively against the real server and the built app (T-081,
 * §11.5, §13 M7, and the `DEVIATIONS.md` entry that makes Kilo Code the fifth agent): the Kilo
 * Code twin of `opencode.ts`, whose fork Kilo's CLI is. It answers the same `AgentRun`, so a
 * scenario reads the same whichever agent it ran against.
 *
 * Every switch below was measured by the Kilo Code canary of `handoff-mcp` (T-081);
 * `handoff-mcp/test/canary/agents/kilo-code/workspace.ts` says why each one is there. It is
 * copied here rather than imported, because nothing in this repository may reach into the other
 * one (§3.1 rule 3):
 *
 * - **Our server is declared in `KILO_CONFIG_CONTENT`**, Kilo's inline configuration, so a run
 *   writes nothing into a file Kilo reads — and Kilo rewrites the files it reads (T-080).
 * - **`XDG_CONFIG_HOME` points at an empty folder of the run**, so the user's global
 *   configuration never loads. The login lives in Kilo's data folder, which the variable does
 *   not move.
 * - **Project configuration and Claude Code's files are switched off**, sharing and self-update
 *   too, and every `KILO_*` variable of the parent is dropped, with the editor pointers
 *   `VSCODE_PID` and `WORKSPACE_FOLDER_PATHS`: Kilo hands its whole environment to our server,
 *   and a suite started from a shell inside an editor would otherwise start a server that looks
 *   for that editor.
 * - **`PWD` is the run's project folder**, as for OpenCode: an OpenCode fork starts its servers
 *   in `PWD` when the variable is set.
 * - **The native `kilo.exe` is started**, past npm's shim and its Node launcher, so that the
 *   server's parent is the process the harness started (T-080).
 * - **The session is deleted afterwards** (`kilo session delete`), by the id the run printed:
 *   `kilo run` has no ephemeral mode, and `kilo session list` shows the user's sessions too.
 *
 * The entry holds the values the installer writes (`src-tauri/src/install/kilo_code.rs`): a
 * local server whose command is the pinned binary alone, `HANDOFF_AGENT = "kilo-code"`, and a
 * `timeout` in milliseconds that mirrors `HANDOFF_TOOL_TIMEOUT_MS`. The one value the installer
 * does not write is `HANDOFF_HOME`, which isolates the run (`TASKS.md` §0.4 item 4). That the
 * real Kilo reads the entry the installer *does* write is the preflight at the bottom of this
 * file.
 *
 * Kilo prints OpenCode's JSON events (`--format json`), so they are read by `readOpenCodeRun`.
 * The VS Code surface runs the same binary as `kilo serve` and starts its servers only for a
 * task typed into its panel, which no script can do: it is the numbered walk of
 * `docs/agents/kilo-code.md`, done by hand.
 */
import { execFileSync, spawn } from 'node:child_process';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import {
  DEFAULT_TIMEOUT_MS,
  DEFAULT_TOOL_TIMEOUT_MS,
  MCP_SERVER_NAME,
  transcriptIds,
  type AgentOptions,
  type AgentRun,
  type AgentRunner,
} from './agent.ts';
import type { Workspace } from './app.ts';
import { check, type Assertion } from './classify.ts';
import { readOpenCodeRun } from './opencode.ts';
import { REPO_ROOT, serverBinary } from './paths.ts';

/**
 * The model a Kilo Code scenario runs on unless `HANDOFF_E2E_KILO_CODE_MODEL` says otherwise: a
 * free model of the Kilo Gateway, so a run spends nothing, and one that makes its tool calls.
 * On the Gateway's free automatic model, `kilo/kilo-auto/free`, which the canary ran on, the model
 * it picked (`nex-agi/nex-n2.5-pro:free`) answered one scenario of every full run without calling
 * the tool — it printed an invented report line, or the call itself as text — in three runs of
 * 2026-09-12; on this one the subset passed 5/5 on the first attempt. Nothing in the adapter
 * names a provider or a model: only the harness pins one.
 */
export const KILO_CODE_DEFAULT_MODEL = 'kilo/inclusionai/ling-3.0-flash-vl:free';

/** The session title, so that Kilo does not name a session after its prompt. */
export const KILO_CODE_SESSION_TITLE = 'baton e2e';

/** The switches every run sets, whatever the parent had. */
export const KILO_CODE_ISOLATION_ENV: Readonly<Record<string, string>> = {
  KILO_DISABLE_PROJECT_CONFIG: '1',
  KILO_DISABLE_CLAUDE_CODE: '1',
  KILO_DISABLE_SHARE: '1',
  KILO_DISABLE_AUTOUPDATE: '1',
};

/** Variables of the parent that would tell the server about the harness rather than about Kilo. */
const DROPPED_PARENT_ENV: ReadonlySet<string> = new Set([
  'CLAUDECODE',
  'VSCODE_PID',
  'WORKSPACE_FOLDER_PATHS',
]);

/**
 * The entry the installer writes into an empty machine, byte for byte: the golden file of the
 * Rust suite (`tests/install_golden.rs` fails if `install::kilo_code` writes anything else).
 */
export const INSTALLED_ENTRY = join(
  REPO_ROOT,
  'src-tauri',
  'tests',
  'fixtures',
  'install',
  'kilo-code-empty',
  'out',
  '.config',
  'kilo',
  'kilo.json',
);

/** Our entry, with the installer's values and the run's `HANDOFF_HOME`. */
export function kiloCodeEntry(workspace: Workspace, toolTimeoutMs: number): Record<string, unknown> {
  return {
    type: 'local',
    command: [serverBinary()],
    environment: {
      HANDOFF_AGENT: 'kilo-code',
      HANDOFF_HOME: workspace.home,
      HANDOFF_TOOL_TIMEOUT_MS: String(toolTimeoutMs),
    },
    timeout: toolTimeoutMs,
  };
}

/** The `kilo run` command line, the prompt last. */
export function kiloCodeArgs(options: AgentOptions): string[] {
  return [
    'run',
    '--pure',
    '--format',
    'json',
    '--title',
    KILO_CODE_SESSION_TITLE,
    '-m',
    options.model ?? process.env['HANDOFF_E2E_KILO_CODE_MODEL'] ?? KILO_CODE_DEFAULT_MODEL,
    options.prompt,
  ];
}

/** The parent's environment without the harness's own markers and without any `KILO_*` variable. */
function cleanEnvironment(parent: NodeJS.ProcessEnv): Record<string, string> {
  const child: Record<string, string> = {};
  for (const [name, value] of Object.entries(parent)) {
    if (value === undefined) continue;
    const upper = name.toUpperCase();
    if (DROPPED_PARENT_ENV.has(upper)) continue;
    if (upper.startsWith('KILO_')) continue;
    child[name] = value;
  }
  return child;
}

/**
 * The environment of the `kilo` child. Kilo hands its whole environment to the servers it
 * starts, which is how `USERDOMAIN` and `USERNAME` — the pipe name is derived from them — reach
 * ours, and why neither `CLAUDECODE` nor an editor's pointers may be in it.
 */
export function kiloCodeEnvironment(
  workspace: Workspace,
  toolTimeoutMs: number,
  parent: NodeJS.ProcessEnv = process.env,
): Record<string, string> {
  const child = cleanEnvironment(parent);
  child['PWD'] = workspace.project;
  child['XDG_CONFIG_HOME'] = join(workspace.root, 'kilo-config');
  child['KILO_CONFIG_CONTENT'] = JSON.stringify({
    mcp: { [MCP_SERVER_NAME]: kiloCodeEntry(workspace, toolTimeoutMs) },
  });
  for (const [name, value] of Object.entries(KILO_CODE_ISOLATION_ENV)) child[name] = value;
  child['HANDOFF_HOME'] = workspace.home;
  return child;
}

/** How to start `kilo`: the program, and whether a shell is needed. */
interface KiloCommand {
  readonly command: string;
  readonly shell: boolean;
}

/** Every `kilo` on `PATH`, in the order the system would pick them. */
function kilosOnPath(): string[] {
  try {
    return execFileSync(process.platform === 'win32' ? 'where' : 'which', ['kilo'], {
      encoding: 'utf8',
      windowsHide: true,
    })
      .split(/\r?\n/u)
      .map((line) => line.trim())
      .filter((line) => line !== '');
  } catch {
    return [];
  }
}

/** Whether there is a `kilo` to run at all (`main.ts` stops with exit 2 when there is not). */
export function kiloOnPath(): boolean {
  return kilosOnPath().length > 0;
}

/**
 * Picks the way to start Kilo, by the canary's rule: a native executable is started directly;
 * npm's shim — `kilo.cmd` on Windows, a symlink to the Node launcher elsewhere — is bypassed for
 * the binary of the platform package, nested under the CLI package or hoisted beside it, and is
 * used itself, through a shell on Windows, only when that binary is not there.
 */
export function resolveKilo(): KiloCommand {
  const windows = process.platform === 'win32';
  const usable = kilosOnPath().filter((path) => !windows || /\.(?:exe|cmd|bat)$/iu.test(path));
  const first = usable[0];
  if (first === undefined) return { command: 'kilo', shell: windows };
  if (windows && /\.exe$/iu.test(first)) return { command: first, shell: false };

  let launcher = join(dirname(first), 'node_modules', '@kilocode', 'cli', 'bin', 'kilo');
  if (!windows) {
    try {
      launcher = realpathSync(first);
    } catch {
      launcher = first;
    }
  }
  const cli = dirname(dirname(launcher));
  const binary = windows ? 'kilo.exe' : 'kilo';
  const platform = windows ? 'windows' : process.platform;
  const base = `cli-${platform}-${process.arch}`;
  for (const name of process.arch === 'x64' ? [base, `${base}-baseline`] : [base]) {
    for (const candidate of [
      join(cli, 'node_modules', '@kilocode', name, 'bin', binary),
      join(dirname(cli), name, 'bin', binary),
    ]) {
      if (existsSync(candidate)) return { command: candidate, shell: false };
    }
  }
  return { command: first, shell: /\.(?:cmd|bat)$/iu.test(first) };
}

/** The variable npm's launcher would have set for the binary: its tree-sitter resources. */
function launcherEnvironment(kilo: KiloCommand): Record<string, string> {
  if (kilo.shell) return {};
  const folder = join(dirname(kilo.command), 'tree-sitter');
  return existsSync(join(folder, 'tree-sitter.wasm')) ? { KILO_TREE_SITTER_WASM_DIR: folder } : {};
}

/** Removes a run's session from the user's Kilo history, by the id the run printed. Best effort. */
function deleteSession(
  kilo: KiloCommand,
  sessionId: string,
  cwd: string,
  env: Record<string, string>,
): void {
  try {
    execFileSync(kilo.command, ['session', 'delete', sessionId], {
      cwd,
      env,
      shell: kilo.shell,
      windowsHide: true,
      stdio: 'ignore',
      timeout: 60_000,
    });
  } catch {
    // A session left in a list is not a failed scenario.
  }
}

/**
 * Starts one Kilo run and answers a promise of everything it produced — not awaited by the
 * caller straight away, exactly like `startAgent`.
 */
export function startKiloCode(workspace: Workspace, options: AgentOptions): Promise<AgentRun> {
  const kilo = resolveKilo();
  const env = {
    ...kiloCodeEnvironment(workspace, options.toolTimeoutMs ?? DEFAULT_TOOL_TIMEOUT_MS),
    ...launcherEnvironment(kilo),
  };
  mkdirSync(env['XDG_CONFIG_HOME'] ?? join(workspace.root, 'kilo-config'), { recursive: true });
  const started = Date.now();

  return new Promise<AgentRun>((resolve, reject) => {
    const child = spawn(kilo.command, kiloCodeArgs(options), {
      cwd: workspace.project,
      env,
      shell: kilo.shell,
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill('SIGKILL');
    }, options.timeoutMs ?? DEFAULT_TIMEOUT_MS);
    timer.unref();

    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk: string) => (stdout += chunk));
    child.stderr.on('data', (chunk: string) => (stderr += chunk));
    child.on('error', (cause) => {
      clearTimeout(timer);
      reject(cause);
    });
    child.on('close', (code) => {
      clearTimeout(timer);
      const run: AgentRun = {
        ...readOpenCodeRun(stdout, stderr, code, timedOut, Date.now() - started),
        agent: 'kilo-code',
      };
      // The session is deleted below, so this file is the only record of the run: it is in the
      // run's root, and `HANDOFF_E2E_KEEP=1` keeps it.
      writeFileSync(
        join(workspace.root, `agent-kilo-code-${String(run.sessionId ?? 'unknown')}.jsonl`),
        stdout,
        'utf8',
      );
      transcriptIds.push(run.sessionId ?? 'unknown');
      if (run.sessionId !== undefined) deleteSession(kilo, run.sessionId, workspace.project, env);
      resolve(run);
    });
  });
}

/** Kilo Code's CLI, as the Kilo Code subset of the suite runs it. */
export const KILO_CODE: AgentRunner = {
  id: 'kilo-code',
  displayName: 'Kilo Code',
  stopHook: false,
  start: startKiloCode,
  tool: (name) => `${name} (a tool of the MCP server ${MCP_SERVER_NAME})`,
};

function asRecord(value: unknown): Readonly<Record<string, unknown>> | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

/**
 * The preflight of the Kilo Code subset: the real Kilo reads the entry the installer writes.
 *
 * The scenarios declare the server inline, because a run must not touch the user's `kilo.json`;
 * so on their own they would prove the values and not the file. This puts the golden `kilo.json`
 * in a throw-away configuration folder and asks `kilo debug config` — no model, only Kilo's own
 * parser — what it read: a local server, the fixed path alone as its command, our two variables
 * and thirty minutes in milliseconds. Kilo rewrites the copy as it loads it (T-080), which is why
 * it is a copy.
 */
export function kiloReadsTheInstalledEntry(): Assertion {
  const what = 'the real Kilo reads the entry the installer writes (kilo debug config)';
  const root = mkdtempSync(join(tmpdir(), 'baton-e2e-kilo-config-'));
  try {
    mkdirSync(join(root, 'kilo'), { recursive: true });
    copyFileSync(INSTALLED_ENTRY, join(root, 'kilo', 'kilo.json'));
    const kilo = resolveKilo();
    const env = {
      ...cleanEnvironment(process.env),
      ...launcherEnvironment(kilo),
      PWD: root,
      XDG_CONFIG_HOME: root,
      ...KILO_CODE_ISOLATION_ENV,
    };
    const printed = execFileSync(kilo.command, ['debug', 'config', '--pure'], {
      cwd: root,
      encoding: 'utf8',
      env,
      shell: kilo.shell,
      windowsHide: true,
      timeout: 90_000,
    });
    const config = asRecord(
      JSON.parse(printed.slice(printed.indexOf('{'), printed.lastIndexOf('}') + 1)) as unknown,
    );
    const entry = asRecord(asRecord(config?.['mcp'])?.[MCP_SERVER_NAME]);
    const command = entry?.['command'];
    const environment = asRecord(entry?.['environment']);
    const ok =
      entry?.['type'] === 'local' &&
      Array.isArray(command) &&
      command.length === 1 &&
      command[0] === '/apps/Baton/handoff-mcp' &&
      environment?.['HANDOFF_AGENT'] === 'kilo-code' &&
      environment['HANDOFF_TOOL_TIMEOUT_MS'] === '1800000' &&
      entry['timeout'] === 1_800_000;
    return check('INST-08', what, 'protocol', ok, `mcp.handoff: ${JSON.stringify(entry ?? null)}`);
  } catch (cause) {
    return check('INST-08', what, 'protocol', false, String(cause));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
