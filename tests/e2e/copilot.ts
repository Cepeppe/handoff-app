/**
 * The real GitHub Copilot CLI, run non-interactively against the real server and the built app
 * (T-072, §11.5, §13 M7, ADPT-06 item 3): the Copilot twin of `codex.ts`, `opencode.ts` and
 * `cursor.ts`. It answers the same `AgentRun`, so a scenario reads the same whichever agent it
 * ran against.
 *
 * Every switch below was measured by the Copilot canary of `handoff-mcp` (T-072);
 * `handoff-mcp/test/canary/agents/copilot/workspace.ts` says why each one is there. It is copied
 * here rather than imported, because nothing in this repository may reach into the other one
 * (§3.1 rule 3):
 *
 * - **The whole Copilot folder is the run's.** `COPILOT_HOME` moves `~/.copilot` — its
 *   `mcp-config.json`, its `config.json`, its sessions and its logs — into the run's root, while
 *   the CLI still signs in through the gh login, which lives elsewhere. So the user's own servers,
 *   hooks and history never meet a scenario, and what a run leaves behind goes with the run.
 * - **The home folder is the run's** as well (`USERPROFILE`, `HOME`), so that nothing the CLI
 *   looks for in a home folder can be the user's.
 * - **One allowance.** `--allow-tool=handoff` approves our server's tools and nothing else;
 *   `--deny-tool` refuses the shell and every file write, the built-in GitHub server is off, and
 *   `--allow-all-tools` is never passed.
 * - **The server is warm.** Under `-p` the CLI does not wait for its servers before the first
 *   model call (T-071), and the pinned server's first start on a machine can take seconds while
 *   it is scanned — measured: 20 s to answer `initialize` the first time, 135 ms after — so it is
 *   started once with `--version` before every run.
 *
 * The entry is the one the installer writes (`src-tauri/src/install/copilot.rs`), read from the
 * golden file of the Rust suite with three values changed: the pinned binary as the command, the
 * run's `HANDOFF_HOME`, which isolates it (implementation decision 4), and the scenario's tool
 * timeout as both `timeout` and `HANDOFF_TOOL_TIMEOUT_MS` — the CLI honours the entry's
 * `timeout` and cancels a call past it, so a scenario's `toolTimeoutMs` is passed on, as it is
 * for Codex. That the real CLI reads the golden file itself is the preflight at the bottom.
 *
 * `copilot -p --output-format json` prints one session event per line, each `{ type, data }`:
 * `assistant.message` with the model's text, `tool.execution_start` and
 * `tool.execution_complete` around each call — `mcpServerName` and `mcpToolName` say when it is a
 * tool of an MCP server — and a closing `result` with the exit code and the usage and no text:
 * the run's reply is its last assistant message. There is no `--max-turns`: the harness timeout
 * is the bound. Every run spends from the Copilot account, which the owner keeps on the Free
 * plan: the subset runs by hand, and rarely.
 */
import { execFileSync, spawn } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import {
  DEFAULT_TIMEOUT_MS,
  DEFAULT_TOOL_TIMEOUT_MS,
  MCP_SERVER_NAME,
  parseOutcome,
  transcriptIds,
  type AgentOptions,
  type AgentRun,
  type AgentRunner,
  type ToolResult,
  type ToolUse,
  type TranscriptMessage,
} from './agent.ts';
import type { Workspace } from './app.ts';
import { check, type Assertion } from './classify.ts';
import { REPO_ROOT, serverBinary } from './paths.ts';

/**
 * The model a Copilot scenario runs on unless `HANDOFF_E2E_COPILOT_MODEL` says otherwise:
 * `auto`, Copilot's own choice and the one the Free plan offers without question (T-071).
 */
export const COPILOT_DEFAULT_MODEL = 'auto';

/** The `HANDOFF_AGENT` the installer writes for Copilot, and the capability table's key. */
export const COPILOT_AGENT_ID = 'copilot';

/** The `clientInfo.name` of the Copilot CLI and of VS Code, whose chat is Copilot's (T-072). */
export const COPILOT_CLI_CLIENT_NAME = 'copilot-cli';
export const COPILOT_VSCODE_CLIENT_NAME = 'Visual Studio Code';

/** What a run refuses outright: the shell and every file write. */
export const COPILOT_DENIED_TOOLS = ['shell', 'write'] as const;

/** What the installer writes into an empty machine: the golden files of `tests/install_golden.rs`. */
const INSTALLED = join(REPO_ROOT, 'src-tauri', 'tests', 'fixtures', 'install', 'copilot-empty', 'out');

/** The Copilot CLI's file, `~/.copilot/mcp-config.json`, as the installer writes it. */
export const INSTALLED_CLI_ENTRY = join(INSTALLED, '.copilot', 'mcp-config.json');

/** VS Code's file, `%APPDATA%\Code\User\mcp.json`, as the installer writes it. */
export const INSTALLED_VSCODE_ENTRY = join(INSTALLED, 'AppData', 'Roaming', 'Code', 'User', 'mcp.json');

/** The command the golden files name: the fixture's Baton, which a scenario replaces. */
export const GOLDEN_COMMAND = '/apps/Baton/handoff-mcp';

function asRecord(value: unknown): Readonly<Record<string, unknown>> | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function asText(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

function json(value: unknown): string {
  return `${JSON.stringify(value, null, 2)}\n`;
}

/** Our entry in one of the golden files, under `table`. */
function installedEntry(file: string, table: 'mcpServers' | 'servers'): Readonly<Record<string, unknown>> {
  const entry = asRecord(asRecord(asRecord(JSON.parse(readFileSync(file, 'utf8')) as unknown)?.[table])?.[MCP_SERVER_NAME]);
  if (entry === undefined) throw new Error(`${file} declares no "${MCP_SERVER_NAME}" under ${table}`);
  return entry;
}

/** The `env` of an entry, its string values only. */
function envOf(entry: Readonly<Record<string, unknown>>): Record<string, string> {
  const env: Record<string, string> = {};
  for (const [name, value] of Object.entries(asRecord(entry['env']) ?? {})) {
    if (typeof value === 'string') env[name] = value;
  }
  return env;
}

/** The CLI's entry: the installer's, with the pinned binary, the run's home and the scenario's timeout. */
export function copilotCliEntry(workspace: Workspace, toolTimeoutMs: number): Record<string, unknown> {
  const entry = installedEntry(INSTALLED_CLI_ENTRY, 'mcpServers');
  return {
    ...entry,
    command: serverBinary(),
    env: {
      ...envOf(entry),
      HANDOFF_HOME: workspace.home,
      HANDOFF_TOOL_TIMEOUT_MS: String(toolTimeoutMs),
    },
    timeout: toolTimeoutMs,
  };
}

/** VS Code's entry: the installer's, with the pinned binary and the run's home. It has no timeout to raise. */
export function copilotVscodeEntry(workspace: Workspace): Record<string, unknown> {
  const entry = installedEntry(INSTALLED_VSCODE_ENTRY, 'servers');
  return { ...entry, command: serverBinary(), env: { ...envOf(entry), HANDOFF_HOME: workspace.home } };
}

/**
 * The files of a run's Copilot folder, as `name → JSON text`: our server in `mcp-config.json`,
 * and the run's project trusted in `config.json`, so that nothing of it is ignored as untrusted.
 */
export function copilotHomeFiles(workspace: Workspace, toolTimeoutMs: number): Record<string, string> {
  return {
    'mcp-config.json': json({ mcpServers: { [MCP_SERVER_NAME]: copilotCliEntry(workspace, toolTimeoutMs) } }),
    'config.json': json({ trustedFolders: [workspace.project] }),
  };
}

/**
 * The `copilot -p` command line. `--output-format json` prints one session event per line;
 * `--usage-output-file` writes what the run cost. Nothing a run does may reach the user: no
 * question to the user, no custom instructions of the machine, no update, no export of the
 * session to GitHub.
 */
export function copilotArgs(options: AgentOptions, usageFile: string): string[] {
  return [
    '-p',
    options.prompt,
    '--output-format',
    'json',
    `--allow-tool=${MCP_SERVER_NAME}`,
    ...COPILOT_DENIED_TOOLS.map((tool) => `--deny-tool=${tool}`),
    '--disable-builtin-mcps',
    '--no-ask-user',
    '--no-custom-instructions',
    '--no-auto-update',
    '--no-remote-export',
    '--model',
    options.model ?? process.env['HANDOFF_E2E_COPILOT_MODEL'] ?? COPILOT_DEFAULT_MODEL,
    '--usage-output-file',
    usageFile,
  ];
}

/**
 * The parent's environment without `CLAUDECODE` and without any `COPILOT_*` of the parent's —
 * a `COPILOT_GITHUB_TOKEN` among them: a run signs in through the gh login, as the owner's own
 * sessions do (T-071).
 */
function cleanEnvironment(parent: NodeJS.ProcessEnv): Record<string, string> {
  const child: Record<string, string> = {};
  for (const [name, value] of Object.entries(parent)) {
    if (value === undefined) continue;
    if (name === 'CLAUDECODE') continue;
    if (name.toUpperCase().startsWith('COPILOT_')) continue;
    child[name] = value;
  }
  return child;
}

/**
 * The environment of the `copilot` child: `COPILOT_HOME` and the home folder moved to the run's
 * folders, updates off, and the run's `HANDOFF_HOME`, so that nothing the CLI starts can reach
 * `~/.handoff/`. The CLI hands its servers the whole environment and the entry's `env` on top;
 * `USERDOMAIN` and `USERNAME`, which the pipe name is derived from, are in it (T-072).
 */
export function copilotEnvironment(
  workspace: Workspace,
  folders: { readonly copilotHome: string; readonly userHome: string },
  parent: NodeJS.ProcessEnv = process.env,
): Record<string, string> {
  const child = cleanEnvironment(parent);
  child['COPILOT_HOME'] = folders.copilotHome;
  child['COPILOT_AUTO_UPDATE'] = 'false';
  child['HANDOFF_HOME'] = workspace.home;
  child['USERPROFILE'] = folders.userHome;
  child['HOME'] = folders.userHome;
  return child;
}

/** How to start `copilot`: the program, and whether a shell is needed. */
interface CopilotCommand {
  readonly command: string;
  readonly shell: boolean;
}

/** `HANDOFF_E2E_COPILOT`, when it names a program. */
function launcherOverride(): string | undefined {
  const override = process.env['HANDOFF_E2E_COPILOT']?.trim();
  return override === undefined || override === '' ? undefined : override;
}

/** The `copilot` programs on `PATH`, in the order the system would pick them. */
function copilotsOnPath(): string[] {
  try {
    return execFileSync(process.platform === 'win32' ? 'where' : 'which', ['copilot'], {
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

/** Whether there is a Copilot CLI to run at all (`main.ts` stops with exit 2 when there is not). */
export function copilotOnPath(): boolean {
  const override = launcherOverride();
  return override !== undefined ? existsSync(override) : copilotsOnPath().length > 0;
}

/**
 * Picks the way to start the Copilot CLI. On Windows an npm install puts `copilot.cmd` on `PATH`,
 * which reaches the CLI through `cmd.exe` and `npm-loader.js`, whose job is to start the native
 * binary of the platform package — `@github/copilot-<platform>-<arch>/copilot.exe`. A prompt
 * carries JSON, and `cmd.exe` would have its say about the quotes in it, so the shim is bypassed
 * for that binary, looked for where npm nests or hoists it; a shell is used only when neither is
 * there. The canary's rule (T-072).
 */
export function resolveCopilot(): CopilotCommand {
  const windows = process.platform === 'win32';
  const override = launcherOverride();
  const found = override === undefined ? copilotsOnPath() : [override];
  const usable = found.filter((path) => !windows || /\.(?:exe|cmd|bat)$/iu.test(path));
  const first = usable[0];
  if (first === undefined) return { command: 'copilot', shell: windows };
  if (!/\.(?:cmd|bat)$/iu.test(first)) return { command: first, shell: false };

  const binaryPackage = `copilot-${process.platform}-${process.arch}`;
  const modules = join(dirname(first), 'node_modules', '@github');
  for (const candidate of [
    join(modules, 'copilot', 'node_modules', '@github', binaryPackage, 'copilot.exe'),
    join(modules, binaryPackage, 'copilot.exe'),
  ]) {
    if (existsSync(candidate)) return { command: candidate, shell: false };
  }
  return { command: first, shell: true };
}

/** Starts the pinned server once, so that the file is warm when the CLI starts it (T-071). */
function warmServer(): void {
  try {
    execFileSync(serverBinary(), ['--version'], { stdio: 'ignore', windowsHide: true, timeout: 60_000 });
  } catch {
    // A server that cannot even print its version is reported by the run that follows.
  }
}

function readUsage(file: string): unknown {
  try {
    return JSON.parse(readFileSync(file, 'utf8')) as unknown;
  } catch {
    return null;
  }
}

/**
 * Starts one Copilot run and answers a promise of everything it produced — not awaited by the
 * caller straight away, exactly like `startAgent`. Each run has a folder of its own under the
 * run's root, for its Copilot folder, its home folder and its usage file; they go with the root.
 */
export function startCopilot(workspace: Workspace, options: AgentOptions): Promise<AgentRun> {
  const runRoot = mkdtempSync(join(workspace.root, 'copilot-'));
  const copilotHome = join(runRoot, 'copilot-home');
  const userHome = join(runRoot, 'user-home');
  for (const folder of [copilotHome, userHome]) mkdirSync(folder, { recursive: true });
  const files = copilotHomeFiles(workspace, options.toolTimeoutMs ?? DEFAULT_TOOL_TIMEOUT_MS);
  for (const [name, text] of Object.entries(files)) writeFileSync(join(copilotHome, name), text, 'utf8');
  const usageFile = join(runRoot, 'usage.json');
  const copilot = resolveCopilot();
  warmServer();
  const started = Date.now();

  return new Promise<AgentRun>((resolve, reject) => {
    const child = spawn(copilot.command, copilotArgs(options, usageFile), {
      cwd: workspace.project,
      env: copilotEnvironment(workspace, { copilotHome, userHome }),
      shell: copilot.shell,
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
      const run = readCopilotRun(stdout, stderr, code, timedOut, Date.now() - started, readUsage(usageFile));
      writeFileSync(
        join(workspace.root, `agent-copilot-${String(run.sessionId ?? 'unknown')}.jsonl`),
        run.transcript.map((event) => JSON.stringify(event)).join('\n'),
        'utf8',
      );
      transcriptIds.push(run.sessionId ?? 'unknown');
      resolve(run);
    });
  });
}

/**
 * A tool name as Claude Code spells it: `mcp__handoff__<tool>` for a tool of our server, which the
 * CLI itself calls `handoff-<tool>`, and the CLI's own name for anything else.
 */
export function copilotToolName(data: Readonly<Record<string, unknown>>): string {
  const server = asText(data['mcpServerName']);
  const tool = asText(data['mcpToolName']);
  if (server === MCP_SERVER_NAME && tool !== '') return `mcp__${MCP_SERVER_NAME}__${tool}`;
  const name = asText(data['toolName']);
  const prefix = `${MCP_SERVER_NAME}-`;
  if (name.startsWith(prefix)) return `mcp__${MCP_SERVER_NAME}__${name.slice(prefix.length)}`;
  return name;
}

/**
 * The CLI's session events as the `AgentRun` every scenario reads. A line that is not a JSON
 * object with a `type` is dropped here and shows up as a missing result instead. The session id
 * is the closing event's `sessionId` — the name of the run's folder under the Copilot folder's
 * `session-state` — or the first one an event's `data` carries; `usage` is what
 * `--usage-output-file` wrote, else the closing event's own.
 */
export function readCopilotRun(
  stdout: string,
  stderr: string,
  exitCode: number | null,
  timedOut: boolean,
  durationMs: number,
  usage: unknown = null,
): AgentRun {
  const transcript: TranscriptMessage[] = [];
  for (const line of stdout.split(/\r?\n/u)) {
    if (line.trim() === '') continue;
    try {
      const event = asRecord(JSON.parse(line) as unknown);
      if (event === undefined || typeof event['type'] !== 'string') continue;
      transcript.push(event as TranscriptMessage);
    } catch {
      // Reported as a missing result by the scenario's first assertion.
    }
  }

  const toolUses: ToolUse[] = [];
  const toolResults: ToolResult[] = [];
  let sessionId: string | undefined;
  let reply = '';
  for (const event of transcript) {
    const data = asRecord(event['data']) ?? {};
    if (typeof event['sessionId'] === 'string') sessionId = event['sessionId'];
    else if (sessionId === undefined && typeof data['sessionId'] === 'string') sessionId = data['sessionId'];
    if (event.type === 'assistant.message') {
      const content = asText(data['content']);
      if (content.trim() !== '') reply = content;
    } else if (event.type === 'tool.execution_start') {
      const id = asText(data['toolCallId']);
      if (!toolUses.some((use) => use.id === id)) {
        toolUses.push({ name: copilotToolName(data), input: { ...(asRecord(data['arguments']) ?? {}) }, id });
      }
    } else if (event.type === 'tool.execution_complete') {
      const text = asText(asRecord(data['result'])?.['content']) || asText(asRecord(data['error'])?.['message']);
      toolResults.push({
        tool_use_id: asText(data['toolCallId']),
        isError: data['success'] !== true,
        text,
        // The outcome is the first text block, one line of JSON (§4.3); a second block (the fix
        // text of FM-10) follows it after a line break.
        outcome: parseOutcome(text) ?? parseOutcome(text.split(/\r?\n/u)[0] ?? ''),
        // No scenario of the subset sends a picture (E2E-3 is Claude Code's).
        images: [],
      });
    }
  }

  const final = transcript.find((event) => event.type === 'result');
  return {
    agent: 'copilot',
    exitCode,
    durationMs,
    timedOut,
    stderr,
    transcript,
    toolUses,
    toolResults,
    // The closing event carries no text: the reply and the usage are put on it here.
    result: final === undefined ? undefined : { ...final, result: reply, usage: usage ?? final['usage'] ?? null },
    sessionId,
    finalText: reply,
  };
}

/** GitHub Copilot's CLI, as the Copilot subset of the suite runs it. */
export const COPILOT: AgentRunner = {
  id: 'copilot',
  displayName: 'GitHub Copilot',
  stopHook: false,
  start: startCopilot,
  tool: (name) => `${name} (a tool of the MCP server ${MCP_SERVER_NAME})`,
};

/**
 * The preflight of the Copilot subset: the real Copilot CLI reads the file the installer writes.
 *
 * The scenarios build their entry from the golden file, so on their own they would prove the
 * values and not the file. This puts the golden `mcp-config.json` in a throw-away Copilot folder,
 * with an empty home folder beside it, and asks the CLI's own `copilot mcp get` what it read — no
 * model, no request, no server started. `--show-secrets`, because the CLI masks every value of
 * `env` otherwise; the golden file holds none. VS Code has no such command: the editor scenario
 * hands VS Code the golden entry itself, and what registers is the answer.
 */
export function copilotReadsTheInstalledEntry(): Assertion {
  const what = 'the real Copilot CLI reads the entry the installer writes (copilot mcp get)';
  const root = mkdtempSync(join(tmpdir(), 'baton-e2e-copilot-config-'));
  try {
    const copilotHome = join(root, 'copilot-home');
    const home = join(root, 'home');
    for (const folder of [copilotHome, home]) mkdirSync(folder, { recursive: true });
    copyFileSync(INSTALLED_CLI_ENTRY, join(copilotHome, 'mcp-config.json'));
    const copilot = resolveCopilot();
    const printed = execFileSync(
      copilot.command,
      ['mcp', 'get', MCP_SERVER_NAME, '--json', '--show-secrets'],
      {
        cwd: home,
        encoding: 'utf8',
        env: {
          ...cleanEnvironment(process.env),
          COPILOT_HOME: copilotHome,
          COPILOT_AUTO_UPDATE: 'false',
          USERPROFILE: home,
          HOME: home,
        },
        shell: copilot.shell,
        windowsHide: true,
        timeout: 60_000,
      },
    );
    const entry = asRecord(asRecord(JSON.parse(printed) as unknown)?.[MCP_SERVER_NAME]) ?? {};
    const env = envOf(entry);
    const args = entry['args'];
    const tools = entry['tools'];
    const ok =
      entry['type'] === 'local' &&
      entry['command'] === GOLDEN_COMMAND &&
      Array.isArray(args) &&
      args.length === 0 &&
      env['HANDOFF_AGENT'] === COPILOT_AGENT_ID &&
      env['HANDOFF_TOOL_TIMEOUT_MS'] === '1800000' &&
      Array.isArray(tools) &&
      tools.length === 1 &&
      tools[0] === '*' &&
      entry['timeout'] === 1_800_000 &&
      entry['enabled'] === true;
    return check('INST-08', what, 'protocol', ok, `copilot mcp get: ${JSON.stringify(entry)}`);
  } catch (cause) {
    return check('INST-08', what, 'protocol', false, String(cause));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
