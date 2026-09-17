/**
 * The real OpenCode, run non-interactively against the real server and the built app (T-074,
 * §11.5, §13 M7, ADPT-06 item 4): the OpenCode twin of `agent.ts` and `codex.ts`. It answers the
 * same `AgentRun`, so a scenario reads the same whichever agent it ran against.
 *
 * Every switch below was measured by the OpenCode canary of `handoff-mcp` (T-074);
 * `handoff-mcp/test/canary/agents/opencode/workspace.ts` says why each one is there. It is
 * copied here rather than imported, because nothing in this repository may reach into the other
 * one (§3.1 rule 3):
 *
 * - **Our server is declared in `OPENCODE_CONFIG_CONTENT`**, OpenCode's inline configuration,
 *   so a run writes nothing into a file OpenCode reads.
 * - **`XDG_CONFIG_HOME` points at an empty folder of the run**, so the user's global
 *   configuration — their servers, plugins, agents and permissions — never loads. The login
 *   lives in OpenCode's data folder, which the variable does not move.
 * - **Project configuration and Claude Code's files are switched off**, sharing and self-update
 *   too, and every `OPENCODE_*` variable of the parent is dropped.
 * - **`PWD` is the run's project folder**: OpenCode starts its servers in `PWD` when the
 *   variable is set, not in its own working directory, and the suite is started from a shell.
 * - **The session is deleted afterwards** (`opencode session delete`): `opencode run` has no
 *   ephemeral mode, and a run must not leave a conversation in the user's history.
 *
 * The entry holds the values the installer writes (`src-tauri/src/install/opencode.rs`): a local
 * server whose command is the pinned binary alone, `HANDOFF_AGENT = "opencode"`, and a `timeout`
 * in milliseconds that mirrors `HANDOFF_TOOL_TIMEOUT_MS`. The one value the installer does not
 * write is `HANDOFF_HOME`, which isolates the run (implementation decision 4). That the real
 * OpenCode reads the entry the installer *does* write is the preflight at the bottom of this
 * file.
 *
 * OpenCode prints one JSON event per line (`--format json`): `step_start`, `tool_use`, `text`,
 * `step_finish` and, when the provider refuses, `error`. Every `tool_use` part is one tool use
 * and one tool result; OpenCode names our tools `handoff_<tool>`, and they are reported as
 * `mcp__handoff__<tool>`, as Claude Code names them. There is no `--max-turns`: the harness
 * timeout is the bound.
 */
import { execFileSync, spawn } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
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
  type ToolImage,
  type ToolResult,
  type ToolUse,
  type TranscriptMessage,
} from './agent.ts';
import type { Workspace } from './app.ts';
import { check, type Assertion } from './classify.ts';
import { REPO_ROOT, serverBinary } from './paths.ts';

/**
 * The model an OpenCode scenario runs on unless `HANDOFF_E2E_OPENCODE_MODEL` says otherwise: the
 * free OpenRouter model the canary was measured on, so a run spends nothing.
 */
export const OPENCODE_DEFAULT_MODEL = 'openrouter/thinkingmachines/inkling-small:free';

/** The session title, so that OpenCode does not name a session after its prompt. */
export const OPENCODE_SESSION_TITLE = 'baton e2e';

/** The switches every run sets, whatever the parent had. */
export const OPENCODE_ISOLATION_ENV: Readonly<Record<string, string>> = {
  OPENCODE_DISABLE_PROJECT_CONFIG: '1',
  OPENCODE_DISABLE_CLAUDE_CODE: '1',
  OPENCODE_DISABLE_SHARE: '1',
  OPENCODE_DISABLE_AUTOUPDATE: '1',
};

/**
 * The entry the installer writes into an empty machine, byte for byte: the golden file of the
 * Rust suite (`tests/install_golden.rs` fails if `install::opencode` writes anything else).
 */
export const INSTALLED_ENTRY = join(
  REPO_ROOT,
  'src-tauri',
  'tests',
  'fixtures',
  'install',
  'opencode-empty',
  'out',
  '.config',
  'opencode',
  'opencode.json',
);

/** Our entry, with the installer's values and the run's `HANDOFF_HOME`. */
export function opencodeEntry(workspace: Workspace, toolTimeoutMs: number): Record<string, unknown> {
  return {
    type: 'local',
    command: [serverBinary()],
    environment: {
      HANDOFF_AGENT: 'opencode',
      HANDOFF_HOME: workspace.home,
      HANDOFF_TOOL_TIMEOUT_MS: String(toolTimeoutMs),
    },
    timeout: toolTimeoutMs,
  };
}

/** The `opencode run` command line, the prompt last. */
export function opencodeArgs(options: AgentOptions): string[] {
  return [
    'run',
    '--pure',
    '--format',
    'json',
    '--title',
    OPENCODE_SESSION_TITLE,
    '-m',
    options.model ?? process.env['HANDOFF_E2E_OPENCODE_MODEL'] ?? OPENCODE_DEFAULT_MODEL,
    options.prompt,
  ];
}

/** The parent's environment without `CLAUDECODE` and without any `OPENCODE_*` variable. */
function cleanEnvironment(parent: NodeJS.ProcessEnv): Record<string, string> {
  const child: Record<string, string> = {};
  for (const [name, value] of Object.entries(parent)) {
    if (value === undefined) continue;
    if (name === 'CLAUDECODE') continue;
    if (name.toUpperCase().startsWith('OPENCODE_')) continue;
    child[name] = value;
  }
  return child;
}

/**
 * The environment of the `opencode` child. OpenCode hands its whole environment to the servers
 * it starts, which is how `USERDOMAIN` and `USERNAME` — the pipe name is derived from them —
 * reach ours, and why `CLAUDECODE` must not be in it.
 */
export function opencodeEnvironment(
  workspace: Workspace,
  toolTimeoutMs: number,
  parent: NodeJS.ProcessEnv = process.env,
): Record<string, string> {
  const child = cleanEnvironment(parent);
  child['PWD'] = workspace.project;
  child['XDG_CONFIG_HOME'] = join(workspace.root, 'opencode-config');
  child['OPENCODE_CONFIG_CONTENT'] = JSON.stringify({
    mcp: { [MCP_SERVER_NAME]: opencodeEntry(workspace, toolTimeoutMs) },
  });
  for (const [name, value] of Object.entries(OPENCODE_ISOLATION_ENV)) child[name] = value;
  child['HANDOFF_HOME'] = workspace.home;
  return child;
}

/** How to start `opencode`: the program, and whether a shell is needed. */
interface OpenCodeCommand {
  readonly command: string;
  readonly shell: boolean;
}

/** Where npm puts the native OpenCode executable, relative to the folder of its shims. */
const NPM_OPENCODE_EXECUTABLE: readonly string[] = ['node_modules', 'opencode-ai', 'bin', 'opencode.exe'];

/** Every `opencode` on `PATH`, in the order the system would pick them. */
function opencodesOnPath(): string[] {
  try {
    return execFileSync(process.platform === 'win32' ? 'where' : 'which', ['opencode'], {
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

/** Whether there is an `opencode` to run at all (`main.ts` stops with exit 2 when there is not). */
export function opencodeOnPath(): boolean {
  return opencodesOnPath().length > 0;
}

/**
 * Picks the way to start `opencode`. A native executable is started directly; an npm install on
 * Windows leaves a `.cmd` shim, which only runs through `cmd.exe`, whose quoting would mangle
 * the JSON a prompt carries, so the shim is bypassed for the executable it calls and a shell is
 * used only when that executable is not beside it. The same rule as the canary's.
 */
export function resolveOpenCode(): OpenCodeCommand {
  const windows = process.platform === 'win32';
  const usable = opencodesOnPath().filter((path) => !windows || /\.(?:exe|cmd|bat)$/iu.test(path));
  const first = usable[0];
  if (first === undefined) return { command: 'opencode', shell: false };
  const lower = first.toLowerCase();
  if (lower.endsWith('.cmd') || lower.endsWith('.bat')) {
    const executable = join(dirname(first), ...NPM_OPENCODE_EXECUTABLE);
    return existsSync(executable)
      ? { command: executable, shell: false }
      : { command: first, shell: true };
  }
  return { command: first, shell: false };
}

/** Removes a run's session from the user's OpenCode history. Best effort. */
function deleteSession(
  opencode: OpenCodeCommand,
  sessionId: string,
  cwd: string,
  env: Record<string, string>,
): void {
  try {
    execFileSync(opencode.command, ['session', 'delete', sessionId], {
      cwd,
      env,
      shell: opencode.shell,
      windowsHide: true,
      stdio: 'ignore',
      timeout: 60_000,
    });
  } catch {
    // A session left in a list is not a failed scenario.
  }
}

/**
 * Starts one OpenCode run and answers a promise of everything it produced — not awaited by the
 * caller straight away, exactly like `startAgent`.
 */
export function startOpenCode(workspace: Workspace, options: AgentOptions): Promise<AgentRun> {
  const opencode = resolveOpenCode();
  const env = opencodeEnvironment(workspace, options.toolTimeoutMs ?? DEFAULT_TOOL_TIMEOUT_MS);
  mkdirSync(env['XDG_CONFIG_HOME'] ?? join(workspace.root, 'opencode-config'), { recursive: true });
  const started = Date.now();

  return new Promise<AgentRun>((resolve, reject) => {
    const child = spawn(opencode.command, opencodeArgs(options), {
      cwd: workspace.project,
      env,
      shell: opencode.shell,
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
      const run = readOpenCodeRun(stdout, stderr, code, timedOut, Date.now() - started);
      // The session is deleted below, so this file is the only record of the run: it is in
      // the run's root, and `HANDOFF_E2E_KEEP=1` keeps it.
      writeFileSync(
        join(workspace.root, `agent-opencode-${String(run.sessionId ?? 'unknown')}.jsonl`),
        stdout,
        'utf8',
      );
      transcriptIds.push(run.sessionId ?? 'unknown');
      if (run.sessionId !== undefined) deleteSession(opencode, run.sessionId, workspace.project, env);
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

/** A tool name as Claude Code spells it: OpenCode's `handoff_<tool>` is `mcp__handoff__<tool>`. */
export function opencodeToolName(tool: string): string {
  const prefix = `${MCP_SERVER_NAME}_`;
  return tool.startsWith(prefix) ? `mcp__${MCP_SERVER_NAME}__${tool.slice(prefix.length)}` : tool;
}

/** The images OpenCode attached to a tool result, as data URLs, in the shape `agent.ts` reports. */
function attachedImages(attachments: unknown): ToolImage[] {
  if (!Array.isArray(attachments)) return [];
  const images: ToolImage[] = [];
  for (const entry of attachments as unknown[]) {
    const attachment = asRecord(entry);
    const mediaType = asText(attachment?.['mime']) || asText(attachment?.['mediaType']);
    const url = asText(attachment?.['url']);
    if (!mediaType.startsWith('image/') || !url.startsWith('data:')) continue;
    images.push({ mediaType, data: url.slice(url.indexOf(',') + 1) });
  }
  return images;
}

/**
 * OpenCode's `--format json` output as the `AgentRun` every scenario reads.
 *
 * A line that is not a JSON object with a `type` is dropped here and shows up as a missing
 * result instead. An `error` event is the provider refusing — a busy free model among the causes
 * — and the run ends there.
 */
export function readOpenCodeRun(
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
      if (event !== undefined && typeof event['type'] === 'string') {
        transcript.push(event as TranscriptMessage);
      }
    } catch {
      // Reported as a missing result by the scenario's first assertion.
    }
  }

  const toolUses: ToolUse[] = [];
  const toolResults: ToolResult[] = [];
  let sessionId: string | undefined;
  let reply = '';
  let steps = 0;
  let failed = false;
  let error = '';
  for (const event of transcript) {
    const session = event['sessionID'];
    if (sessionId === undefined && typeof session === 'string') sessionId = session;
    if (event.type === 'error') {
      failed = true;
      const details = asRecord(event['error']);
      error = asText(asRecord(details?.['data'])?.['message']) || asText(details?.['name']);
      continue;
    }
    const part = asRecord(event['part']);
    if (part === undefined) continue;
    if (event.type === 'step_finish') {
      steps += 1;
      continue;
    }
    if (event.type === 'text') {
      const text = asText(part['text']);
      if (text.trim() !== '') reply = text;
      continue;
    }
    if (event.type !== 'tool_use') continue;

    const state = asRecord(part['state']);
    const id = asText(part['callID']) || asText(part['id']);
    const completed = state?.['status'] === 'completed';
    toolUses.push({
      name: opencodeToolName(asText(part['tool'])),
      input: { ...(asRecord(state?.['input']) ?? {}) },
      id,
    });
    const text = completed ? asText(state['output']) : asText(state?.['error']);
    toolResults.push({
      tool_use_id: id,
      isError: !completed,
      text,
      // The outcome is the first text block, one line of JSON (§4.3); OpenCode joins the blocks
      // of a result into one output, so a second one (the fix text of FM-10) follows it.
      outcome: parseOutcome(text) ?? parseOutcome(text.split(/\r?\n/u)[0] ?? ''),
      images: attachedImages(state?.['attachments']),
    });
  }

  const result: TranscriptMessage | undefined =
    steps === 0 && !failed
      ? undefined
      : {
          type: 'result',
          subtype: failed ? 'error' : 'success',
          is_error: failed,
          result: failed && reply === '' ? error : reply,
          num_turns: steps,
        };
  return {
    agent: 'opencode',
    exitCode,
    durationMs,
    timedOut,
    stderr,
    transcript,
    toolUses,
    toolResults,
    result,
    sessionId,
    finalText: reply,
  };
}

/** OpenCode, as the OpenCode subset of the suite runs it. */
export const OPENCODE: AgentRunner = {
  id: 'opencode',
  displayName: 'OpenCode',
  stopHook: false,
  start: startOpenCode,
  tool: (name) => `${name} (a tool of the MCP server ${MCP_SERVER_NAME})`,
};

/**
 * The preflight of the OpenCode subset: the real OpenCode reads the entry the installer writes.
 *
 * The scenarios declare the server inline, because a run must not touch the user's
 * `opencode.json`; so on their own they would prove the values and not the file. This puts the
 * golden `opencode.json` in a throw-away configuration folder and asks `opencode debug config` —
 * no model, no login, only OpenCode's own parser — what it read: a local server, the fixed path
 * alone as its command, our two variables and thirty minutes in milliseconds.
 */
export function opencodeReadsTheInstalledEntry(): Assertion {
  const what = 'the real OpenCode reads the entry the installer writes (opencode debug config)';
  const root = mkdtempSync(join(tmpdir(), 'baton-e2e-opencode-config-'));
  try {
    mkdirSync(join(root, 'opencode'), { recursive: true });
    copyFileSync(INSTALLED_ENTRY, join(root, 'opencode', 'opencode.json'));
    const opencode = resolveOpenCode();
    const env = {
      ...cleanEnvironment(process.env),
      PWD: root,
      XDG_CONFIG_HOME: root,
      ...OPENCODE_ISOLATION_ENV,
    };
    const printed = execFileSync(opencode.command, ['debug', 'config', '--pure'], {
      cwd: root,
      encoding: 'utf8',
      env,
      shell: opencode.shell,
      windowsHide: true,
      timeout: 90_000,
    });
    const config = asRecord(JSON.parse(printed) as unknown);
    const entry = asRecord(asRecord(config?.['mcp'])?.[MCP_SERVER_NAME]);
    const command = entry?.['command'];
    const environment = asRecord(entry?.['environment']);
    const ok =
      entry?.['type'] === 'local' &&
      Array.isArray(command) &&
      command.length === 1 &&
      command[0] === '/apps/Baton/handoff-mcp' &&
      environment?.['HANDOFF_AGENT'] === 'opencode' &&
      environment['HANDOFF_TOOL_TIMEOUT_MS'] === '1800000' &&
      entry['timeout'] === 1_800_000;
    return check('INST-08', what, 'protocol', ok, `mcp.handoff: ${JSON.stringify(entry ?? null)}`);
  } catch (cause) {
    return check('INST-08', what, 'protocol', false, String(cause));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
