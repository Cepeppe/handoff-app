/**
 * The real Cursor Agent CLI, run non-interactively against the real server and the built app
 * (T-070, §11.5, §13 M7, ADPT-06 item 2): the Cursor twin of `codex.ts` and `opencode.ts`. It
 * answers the same `AgentRun`, so a scenario reads the same whichever agent it ran against.
 *
 * Every switch below was measured by the Cursor canary of `handoff-mcp` (T-069);
 * `handoff-mcp/test/canary/agents/cursor/workspace.ts` says why each one is there. It is copied
 * here rather than imported, because nothing in this repository may reach into the other one
 * (§3.1 rule 3):
 *
 * - **Our server is declared in the run's own project**, `<project>/.cursor/mcp.json`, which the
 *   CLI reads from its workspace, and `--approve-mcps` approves it for this run alone: a server
 *   of a project file is refused until it is approved.
 * - **One permission rule, the narrowest there is**: `<project>/.cursor/cli.json` allows
 *   `Mcp(handoff:*)`, because print mode refuses a tool that is not annotated read-only.
 *   `--force` would allow every shell command and file write as well, and is never passed.
 * - **`--trust`**, so the run's folder is trusted without a prompt, and `--workspace` names it.
 * - **What a run leaves behind is deleted**: `agent -p` has no ephemeral mode, and every run adds
 *   a folder under `~/.cursor/projects` and a conversation under `~/.cursor/chats`.
 *
 * What it cannot isolate: the CLI has no switch that leaves the user's own `~/.cursor/mcp.json`
 * out of a run, so a `handoff` server declared there would load beside the run's and could reach
 * the Baton the owner uses every day. `main.ts` refuses to start the subset on such a machine
 * ([`cursorUserConfigProblem`]).
 *
 * The entry holds the values the installer writes (`src-tauri/src/install/cursor.rs`): the pinned
 * binary as the command with no arguments, and `HANDOFF_AGENT = "cursor"` — and no timeout of
 * any kind, because Cursor reads none and the installer writes none. A scenario's `toolTimeoutMs`
 * is therefore not passed on: the row's 60 000 ms, which is the CLI's own cut, puts the heartbeat
 * at the 50 s floor, exactly as it does for a person using Cursor. The one value the installer
 * does not write is `HANDOFF_HOME`, which isolates the run (`TASKS.md` §0.4 item 4). That the
 * real CLI reads the file the installer *does* write is the preflight at the bottom of this file.
 *
 * `agent -p --output-format stream-json` prints one JSON event per line: `system`/`init`, the
 * `user` prompt, `assistant` text, a `tool_call` event `started` and one `completed` for every
 * call, and a final `result`. A call travels as Cursor's own protocol message — `mcpToolCall`
 * with its `args` and its `result` (`success`, `error`, `rejected` or `permissionDenied`) — and
 * our tools are reported as `mcp__handoff__<tool>`, as Claude Code names them. There is no
 * `--max-turns`: the harness timeout is the bound. Every run spends one request of the account,
 * which the owner keeps on the Free plan (T-068): the subset runs by hand, and rarely.
 */
import { execFileSync, spawn } from 'node:child_process';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import {
  DEFAULT_TIMEOUT_MS,
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
 * The model a Cursor scenario runs on unless `HANDOFF_E2E_CURSOR_MODEL` says otherwise: Cursor's
 * own `auto`, the CLI's default and the one the Free plan offers without question (T-068).
 */
export const CURSOR_DEFAULT_MODEL = 'auto';

/** The `HANDOFF_AGENT` the installer writes for Cursor, and the capability table's key. */
export const CURSOR_AGENT_ID = 'cursor';

/** The one permission rule a run carries: every tool of our server, and nothing else. */
export const CURSOR_PERMISSION = `Mcp(${MCP_SERVER_NAME}:*)`;

/**
 * The limit Cursor gives a tool call whatever a scenario asks: the Agent CLI's cut, which is the
 * row's `tool_timeout_ms_default` (T-069). No entry can raise it.
 */
export const CURSOR_TOOL_TIMEOUT_MS = 60_000;

/**
 * The entry the installer writes into an empty machine, byte for byte: the golden file of the
 * Rust suite (`tests/install_golden.rs` fails if `install::cursor` writes anything else).
 */
export const INSTALLED_ENTRY = join(
  REPO_ROOT,
  'src-tauri',
  'tests',
  'fixtures',
  'install',
  'cursor-empty',
  'out',
  '.cursor',
  'mcp.json',
);

/** The folders of `~/.cursor` where every run leaves something: its project and its chats. */
export const CURSOR_STATE_FOLDERS = ['projects', 'chats'] as const;

/** Our entry, with the installer's values and the run's `HANDOFF_HOME`. */
export function cursorEntry(workspace: Workspace): Record<string, unknown> {
  return {
    command: serverBinary(),
    args: [],
    env: {
      HANDOFF_AGENT: CURSOR_AGENT_ID,
      HANDOFF_HOME: workspace.home,
    },
  };
}

/** Every file a run writes into its project, as `relative path → JSON text`. */
export function cursorProjectFiles(workspace: Workspace): Record<string, string> {
  const json = (value: unknown): string => `${JSON.stringify(value, null, 2)}\n`;
  return {
    '.cursor/mcp.json': json({ mcpServers: { [MCP_SERVER_NAME]: cursorEntry(workspace) } }),
    '.cursor/cli.json': json({ permissions: { allow: [CURSOR_PERMISSION], deny: [] } }),
  };
}

/** The `agent -p` command line, the prompt last. */
export function cursorArgs(options: AgentOptions, project: string): string[] {
  return [
    '-p',
    '--output-format',
    'stream-json',
    '--approve-mcps',
    '--trust',
    '--model',
    options.model ?? process.env['HANDOFF_E2E_CURSOR_MODEL'] ?? CURSOR_DEFAULT_MODEL,
    '--workspace',
    project,
    options.prompt,
  ];
}

/** The parent's environment without `CLAUDECODE`. */
function cleanEnvironment(parent: NodeJS.ProcessEnv): Record<string, string> {
  const child: Record<string, string> = {};
  for (const [name, value] of Object.entries(parent)) {
    if (value === undefined) continue;
    if (name === 'CLAUDECODE') continue;
    child[name] = value;
  }
  return child;
}

/**
 * The environment of the `agent` child. The CLI hands its servers a short list of its own —
 * `USERDOMAIN` and `USERNAME` among them, which the pipe name is derived from — and the entry
 * carries the run's `HANDOFF_HOME` to ours; it is set here as well, so that nothing else the CLI
 * starts can reach `~/.handoff/`.
 */
export function cursorEnvironment(
  workspace: Workspace,
  parent: NodeJS.ProcessEnv = process.env,
): Record<string, string> {
  const child = cleanEnvironment(parent);
  child['HANDOFF_HOME'] = workspace.home;
  return child;
}

/** How to start the Cursor Agent CLI: the program, what goes before our arguments, a shell or not. */
interface CursorCommand {
  readonly command: string;
  readonly prefix: readonly string[];
  readonly shell: boolean;
}

/**
 * A version folder of a standalone Cursor Agent install, `YYYY.MM.DD-<commit>`, as a number that
 * sorts the way the CLI's own launcher sorts them, or `undefined` for any other name.
 */
function versionRank(name: string): number | undefined {
  const match = /^(\d{4})\.(\d{1,2})\.(\d{1,2})-/u.exec(name);
  if (match === null) return undefined;
  const [, year = '', month = '', day = ''] = match;
  return Number(`${year}${month.padStart(2, '0')}${day.padStart(2, '0')}`);
}

function listFolder(folder: string): string[] {
  try {
    return readdirSync(folder);
  } catch {
    return [];
  }
}

/** The `cursor-agent` (else `agent`) programs on `PATH`, in the order the system would pick them. */
function agentsOnPath(): string[] {
  for (const name of ['cursor-agent', 'agent']) {
    try {
      const found = execFileSync(process.platform === 'win32' ? 'where' : 'which', [name], {
        encoding: 'utf8',
        windowsHide: true,
      })
        .split(/\r?\n/u)
        .map((line) => line.trim())
        .filter((line) => line !== '');
      if (found.length > 0) return found;
    } catch {
      // Not on PATH under this name: try the next one.
    }
  }
  return [];
}

/** `HANDOFF_E2E_CURSOR`, when it names a launcher. */
function launcherOverride(): string | undefined {
  const override = process.env['HANDOFF_E2E_CURSOR']?.trim();
  return override === undefined || override === '' ? undefined : override;
}

/** Whether there is a Cursor Agent CLI to run at all (`main.ts` stops with exit 2 when there is not). */
export function cursorOnPath(): boolean {
  const override = launcherOverride();
  return override !== undefined ? existsSync(override) : agentsOnPath().length > 0;
}

/**
 * Picks the way to start the Cursor Agent CLI. On Windows the install puts `agent.cmd` and
 * `cursor-agent.cmd` on `PATH`, which reach the CLI through `cmd.exe` and then PowerShell, whose
 * quoting would mangle the JSON a prompt carries: so a `.cmd` shim is bypassed for what the CLI's
 * own launcher starts — the `node.exe` and `index.js` of the newest folder under `versions` — and
 * a shell is used only when there is no such folder. The canary's rule (T-069).
 */
export function resolveCursorAgent(): CursorCommand {
  const windows = process.platform === 'win32';
  const override = launcherOverride();
  const found = override === undefined ? agentsOnPath() : [override];
  const usable = found.filter((path) => !windows || /\.(?:exe|cmd|bat)$/iu.test(path));
  const first = usable[0];
  if (first === undefined) return { command: 'cursor-agent', prefix: [], shell: windows };
  if (!/\.(?:cmd|bat)$/iu.test(first)) return { command: first, prefix: [], shell: false };

  const versions = join(dirname(first), 'versions');
  const newestFirst = listFolder(versions)
    .map((name) => ({ name, rank: versionRank(name) }))
    .filter((entry): entry is { name: string; rank: number } => entry.rank !== undefined)
    .sort((a, b) => b.rank - a.rank || (a.name < b.name ? 1 : -1));
  for (const { name } of newestFirst) {
    const node = join(versions, name, 'node.exe');
    const index = join(versions, name, 'index.js');
    if (existsSync(node) && existsSync(index)) {
      return { command: node, prefix: [index], shell: false };
    }
  }
  return { command: first, prefix: [], shell: true };
}

/** What `~/.cursor` holds in the folders a run writes into, before the run. */
function stateSnapshot(root: string): Map<string, readonly string[]> {
  return new Map(
    CURSOR_STATE_FOLDERS.map((folder) => [folder, listFolder(join(root, folder))] as const),
  );
}

/**
 * Removes what a run added under `~/.cursor/projects` and `~/.cursor/chats`, and nothing that was
 * there before it. Best effort: an entry that cannot be removed is a folder in a list, not a
 * failed scenario.
 */
function removeRunState(root: string, before: Map<string, readonly string[]>): void {
  for (const folder of CURSOR_STATE_FOLDERS) {
    const known = new Set(before.get(folder) ?? []);
    for (const entry of listFolder(join(root, folder))) {
      if (known.has(entry)) continue;
      try {
        rmSync(join(root, folder, entry), { recursive: true, force: true });
      } catch {
        // See above.
      }
    }
  }
}

/**
 * Starts one Cursor run and answers a promise of everything it produced — not awaited by the
 * caller straight away, exactly like `startAgent`.
 */
export function startCursor(workspace: Workspace, options: AgentOptions): Promise<AgentRun> {
  for (const [relative, text] of Object.entries(cursorProjectFiles(workspace))) {
    const file = join(workspace.project, relative);
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, text, 'utf8');
  }
  const cursor = resolveCursorAgent();
  const stateRoot = join(homedir(), '.cursor');
  const before = stateSnapshot(stateRoot);
  const started = Date.now();

  return new Promise<AgentRun>((resolve, reject) => {
    const child = spawn(cursor.command, [...cursor.prefix, ...cursorArgs(options, workspace.project)], {
      cwd: workspace.project,
      env: cursorEnvironment(workspace),
      shell: cursor.shell,
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
      removeRunState(stateRoot, before);
      reject(cause);
    });
    child.on('close', (code) => {
      clearTimeout(timer);
      removeRunState(stateRoot, before);
      const run = readCursorRun(stdout, stderr, code, timedOut, Date.now() - started);
      // The conversation is deleted with the run's state above, so this file is the only record
      // of the run: it is in the run's root, and `HANDOFF_E2E_KEEP=1` keeps it.
      writeFileSync(
        join(workspace.root, `agent-cursor-${String(run.sessionId ?? 'unknown')}.jsonl`),
        run.transcript.map((event) => JSON.stringify(event)).join('\n'),
        'utf8',
      );
      transcriptIds.push(run.sessionId ?? 'unknown');
      resolve(run);
    });
  });
}

function asRecord(value: unknown): Readonly<Record<string, unknown>> | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function asText(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

/**
 * The kind of a Cursor tool call and its body. The message prints as canonical protobuf JSON,
 * `{ "mcpToolCall": { … } }`; the in-memory shape `{ tool: { case, value } }` is read as well, so
 * a CLI that stops calling `toJSON` does not turn every call into nothing (the canary's rule).
 */
function cursorToolCall(
  toolCall: unknown,
): { readonly kind: string; readonly body: Readonly<Record<string, unknown>> } | undefined {
  const call = asRecord(toolCall);
  if (call === undefined) return undefined;
  const oneof = asRecord(call['tool']);
  if (oneof !== undefined && typeof oneof['case'] === 'string') {
    return { kind: oneof['case'], body: asRecord(oneof['value']) ?? {} };
  }
  const key = Object.keys(call).find((name) => name.endsWith('ToolCall'));
  return key === undefined ? undefined : { kind: key, body: asRecord(call[key]) ?? {} };
}

/**
 * A tool name as Claude Code spells it. Cursor names an MCP tool by its server and its own name —
 * `providerIdentifier` `handoff` and `toolName` `handoff_to_user`, or `handoff-handoff_to_user` in
 * one string — and a built-in tool keeps the name of its kind.
 */
export function cursorToolName(kind: string, body: Readonly<Record<string, unknown>>): string {
  if (kind !== 'mcpToolCall') return kind;
  const args = asRecord(body['args']) ?? {};
  const provider = (asText(args['providerIdentifier']) || asText(args['serverIdentifier'])).split(
    '::mcpScope:',
  )[0];
  const tool = asText(args['toolName']);
  if (provider === MCP_SERVER_NAME && tool !== '') return `mcp__${MCP_SERVER_NAME}__${tool}`;
  const name = asText(args['name']);
  const prefix = `${MCP_SERVER_NAME}-`;
  if (name.startsWith(prefix)) return `mcp__${MCP_SERVER_NAME}__${name.slice(prefix.length)}`;
  return name === '' ? kind : name;
}

/** Every string under a key named `text`, `error`, `reason` or `message`, in document order. */
function collectText(value: unknown, depth = 0): string[] {
  if (depth > 10) return [];
  if (Array.isArray(value)) return value.flatMap((entry: unknown) => collectText(entry, depth + 1));
  const record = asRecord(value);
  if (record === undefined) return [];
  const texts: string[] = [];
  for (const [key, entry] of Object.entries(record)) {
    if (typeof entry === 'string') {
      if (key === 'text' || key === 'error' || key === 'reason' || key === 'message') {
        texts.push(entry);
      }
    } else {
      texts.push(...collectText(entry, depth + 1));
    }
  }
  return texts;
}

/** The outcome of a completed call: whether it failed, and the text of what came back. */
function cursorToolResult(
  body: Readonly<Record<string, unknown>>,
): { readonly isError: boolean; readonly text: string } | undefined {
  const result = asRecord(body['result']);
  if (result === undefined) return undefined;
  const oneof = asRecord(result['result']);
  const tagged = oneof !== undefined && typeof oneof['case'] === 'string';
  const kind = tagged ? asText(oneof['case']) : (Object.keys(result)[0] ?? '');
  const value = tagged ? oneof['value'] : result[kind];
  return {
    isError: kind !== 'success' || asRecord(value)?.['isError'] === true,
    text: collectText(value).join('\n'),
  };
}

/** The text of an `assistant` event. */
function assistantText(event: TranscriptMessage): string {
  return (event.message?.content ?? [])
    .map((block) => (typeof block['text'] === 'string' ? block['text'] : ''))
    .join('');
}

/**
 * The CLI's stream-json output as the `AgentRun` every scenario reads.
 *
 * A line that is not a JSON object with a `type` is dropped here and shows up as a missing result
 * instead. A completed shell call can carry a snapshot of the shell's environment (`env`); it is
 * dropped before anything else sees it, and never reaches a report.
 */
export function readCursorRun(
  stdout: string,
  stderr: string,
  exitCode: number | null,
  timedOut: boolean,
  durationMs: number,
): AgentRun {
  const transcript: TranscriptMessage[] = [];
  for (const line of stdout.split(/\r?\n/u)) {
    if (line.trim() === '') continue;
    try {
      const event = asRecord(JSON.parse(line) as unknown);
      if (event === undefined || typeof event['type'] !== 'string') continue;
      const kept: Record<string, unknown> = { ...event };
      delete kept['env'];
      transcript.push(kept as TranscriptMessage);
    } catch {
      // Reported as a missing result by the scenario's first assertion.
    }
  }

  const toolUses: ToolUse[] = [];
  const toolResults: ToolResult[] = [];
  let sessionId: string | undefined;
  let lastText = '';
  for (const event of transcript) {
    const session = event['session_id'];
    if (sessionId === undefined && typeof session === 'string') sessionId = session;
    if (event.type === 'assistant') {
      const text = assistantText(event);
      if (text.trim() !== '') lastText = text;
      continue;
    }
    if (event.type !== 'tool_call') continue;
    const call = cursorToolCall(event['tool_call']);
    if (call === undefined) continue;
    const id = asText(event['call_id']);
    if (!toolUses.some((use) => use.id === id)) {
      toolUses.push({
        name: cursorToolName(call.kind, call.body),
        input: { ...(asRecord(asRecord(call.body['args'])?.['args']) ?? {}) },
        id,
      });
    }
    if (event['subtype'] !== 'completed') continue;
    const outcome = cursorToolResult(call.body);
    const text = outcome?.text ?? '';
    toolResults.push({
      tool_use_id: id,
      isError: outcome?.isError ?? true,
      text,
      // The outcome is the first text block, one line of JSON (§4.3); a second block (the fix
      // text of FM-10) follows it after a line break.
      outcome: parseOutcome(text) ?? parseOutcome(text.split(/\r?\n/u)[0] ?? ''),
      // No scenario of the subset sends a picture (E2E-3 is Claude Code's).
      images: [],
    });
  }

  const result = transcript.find((event) => event.type === 'result');
  const resultText = typeof result?.result === 'string' ? result.result : '';
  return {
    agent: 'cursor',
    exitCode,
    durationMs,
    timedOut,
    stderr,
    transcript,
    toolUses,
    toolResults,
    result,
    sessionId,
    finalText: resultText.trim() !== '' ? resultText : lastText,
  };
}

/** Cursor's Agent CLI, as the Cursor subset of the suite runs it. */
export const CURSOR: AgentRunner = {
  id: 'cursor',
  displayName: 'Cursor',
  stopHook: false,
  fixedToolTimeoutMs: CURSOR_TOOL_TIMEOUT_MS,
  start: startCursor,
  tool: (name) => `${name} (a tool of the MCP server ${MCP_SERVER_NAME})`,
};

/**
 * Why the Cursor subset cannot run on this machine, or `undefined` when it can.
 *
 * The CLI has no switch that leaves `~/.cursor/mcp.json` out of a run (T-069), so a `handoff`
 * server declared there — Baton registered for Cursor, which is what the installer does — would
 * load beside the run's, under the same name, and a scenario's handoff could open in the Baton
 * the owner uses every day. The subset refuses to start rather than guess which of the two the
 * model will call. The editor scenario is not affected — it gives its editor a home folder of its
 * own — but it runs inside the same subset.
 */
export function cursorUserConfigProblem(home: string = homedir()): string | undefined {
  const file = join(home, '.cursor', 'mcp.json');
  let text: string;
  try {
    text = readFileSync(file, 'utf8');
  } catch {
    return undefined;
  }
  try {
    const servers = asRecord(asRecord(JSON.parse(text) as unknown)?.['mcpServers']);
    if (servers?.[MCP_SERVER_NAME] === undefined) return undefined;
  } catch {
    return (
      `${file} is not JSON the harness can read, and the Cursor CLI reads it into every run: ` +
      'fix it, or move it aside for the length of the run.'
    );
  }
  return (
    `${file} declares a "${MCP_SERVER_NAME}" server, and the Cursor CLI reads it into every run ` +
    "beside the run's own: remove Baton's registration for Cursor (Settings → Agents → Remove) " +
    'for the length of the run, or run the subset where Cursor is not registered.'
  );
}

/**
 * The preflight of the Cursor subset: the real Cursor CLI reads the file the installer writes.
 *
 * The scenarios declare the server in the run's own project, because a run must not touch the
 * user's `~/.cursor/mcp.json`; on their own they would prove the values and not the file. This
 * puts the golden `mcp.json` in a throw-away project and asks the CLI's own `agent mcp list` what
 * it read — no model, no request, and nothing left under `~/.cursor/projects` — with the home
 * folder pointed at an empty one of the preflight's, so that the golden file is the only one it
 * can read (measured: in an empty project the same command answers "No MCP servers
 * configured"). The server stays unloaded, since a server of a project file waits for an
 * approval the preflight never gives, so the answer is about the file: a server named `handoff`.
 */
export function cursorReadsTheInstalledEntry(): Assertion {
  const what = 'the real Cursor CLI reads the entry the installer writes (agent mcp list)';
  const root = mkdtempSync(join(tmpdir(), 'baton-e2e-cursor-config-'));
  try {
    const project = join(root, 'project');
    const home = join(root, 'home');
    mkdirSync(join(project, '.cursor'), { recursive: true });
    mkdirSync(home, { recursive: true });
    copyFileSync(INSTALLED_ENTRY, join(project, '.cursor', 'mcp.json'));
    const cursor = resolveCursorAgent();
    const printed = execFileSync(cursor.command, [...cursor.prefix, 'mcp', 'list'], {
      cwd: project,
      encoding: 'utf8',
      env: { ...cleanEnvironment(process.env), USERPROFILE: home, HOME: home },
      shell: cursor.shell,
      windowsHide: true,
      timeout: 90_000,
    });
    const ok = new RegExp(`^${MCP_SERVER_NAME}: `, 'mu').test(printed);
    return check('INST-08', what, 'protocol', ok, `agent mcp list: ${JSON.stringify(printed.trim())}`);
  } catch (cause) {
    return check('INST-08', what, 'protocol', false, String(cause));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
