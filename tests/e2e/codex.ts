/**
 * The real Codex CLI, run non-interactively against the real server and the built app
 * (T-067, §11.5, §13 M7, ADPT-06 item 1): the Codex twin of `agent.ts`. It answers the same
 * `AgentRun`, so a scenario reads the same whichever agent it ran against.
 *
 * Every flag below was measured by the Codex canary of `handoff-mcp` (T-066), and three of
 * them are never optional. `handoff-mcp/test/canary/agents/codex/workspace.ts` says why; it is
 * copied here rather than imported, because nothing in this repository may reach into the
 * other one (§3.1 rule 3):
 *
 * - **`--ignore-user-config`**: it skips `~/.codex/config.toml` and keeps the login. A
 *   `-c mcp_servers.…` override *merges* with the servers a user declared, so the override
 *   alone would not keep them out.
 * - **`--disable apps` and `--disable plugins`**: both are on by default, and they are how a
 *   Codex session reaches the user's connected accounts. The browser, computer-use,
 *   image-generation and sub-agent features go as well.
 * - **`--ephemeral`**, so a run leaves nothing in the user's session history.
 *
 * Our server is declared through `-c` overrides holding the values the installer writes
 * (`src-tauri/src/install/codex.rs`): the pinned binary with no arguments, `HANDOFF_AGENT =
 * "codex"`, the approval mode without which `codex exec` refuses `handoff_to_user`, and a
 * `tool_timeout_sec` in seconds that mirrors `HANDOFF_TOOL_TIMEOUT_MS`. The one value the
 * installer does not write is `HANDOFF_HOME`, which isolates the run (`TASKS.md` §0.4 item 4).
 * That the real Codex reads the entry the installer *does* write is the preflight at the
 * bottom of this file.
 *
 * Codex prints JSONL events rather than `stream-json` messages. Every `mcp_tool_call` item is
 * one tool use and one tool result, and a `result` message is made from the last agent
 * message once a turn has ended. There is no `--max-turns`: the harness timeout is the bound.
 */
import { execFileSync, spawn } from 'node:child_process';
import { copyFileSync, existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
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

/** The model a Codex scenario runs on unless `HANDOFF_E2E_CODEX_MODEL` says otherwise. */
export const CODEX_DEFAULT_MODEL = 'gpt-5.6-luna';

/** A scenario asks for tool calls and one closing line; it needs no deliberation. */
export const CODEX_REASONING_EFFORT = 'low';

/** The features every run turns off. `apps` and `plugins` are the isolation. */
export const CODEX_DISABLED_FEATURES: readonly string[] = [
  'apps',
  'plugins',
  'browser_use',
  'browser_use_external',
  'computer_use',
  'in_app_browser',
  'image_generation',
  'multi_agent',
];

/** `default_tools_approval_mode` of our entry, as the installer writes it. */
export const CODEX_APPROVAL_MODE = 'approve';

/**
 * The entry the installer writes, byte for byte: the golden file of the Rust suite
 * (`tests/install_golden.rs` fails if `install::codex` writes anything else into an empty
 * machine).
 */
export const INSTALLED_ENTRY = join(
  REPO_ROOT,
  'src-tauri',
  'tests',
  'fixtures',
  'install',
  'codex-empty',
  'out',
  '.codex',
  'config.toml',
);

/** A TOML basic string. JSON's escapes are TOML escapes too, so a Windows path survives. */
function tomlString(value: string): string {
  return JSON.stringify(value);
}

/** The `-c` overrides that declare our server, with the installer's values. */
export function codexServerOverrides(workspace: Workspace, toolTimeoutMs: number): string[] {
  const key = `mcp_servers.${MCP_SERVER_NAME}`;
  const env = {
    HANDOFF_AGENT: 'codex',
    HANDOFF_HOME: workspace.home,
    HANDOFF_TOOL_TIMEOUT_MS: String(toolTimeoutMs),
  };
  const table = Object.entries(env)
    .map(([name, value]) => `${name}=${tomlString(value)}`)
    .join(',');
  return [
    '-c',
    `${key}.command=${tomlString(serverBinary())}`,
    '-c',
    `${key}.args=[]`,
    '-c',
    `${key}.env={${table}}`,
    '-c',
    `${key}.default_tools_approval_mode=${tomlString(CODEX_APPROVAL_MODE)}`,
    '-c',
    `${key}.tool_timeout_sec=${String(Math.ceil(toolTimeoutMs / 1000))}`,
  ];
}

/**
 * The `codex exec` command line, the prompt last. `--json` prints one event per line,
 * `--skip-git-repo-check` lets it run in a temporary folder that is no repository, and
 * `-s read-only` is the sandbox for any command the model might try: a scenario asks for
 * tool calls and nothing else.
 */
export function codexArgs(options: AgentOptions, workspace: Workspace): string[] {
  return [
    'exec',
    '--json',
    '--ephemeral',
    '--ignore-user-config',
    '--skip-git-repo-check',
    '-C',
    workspace.project,
    '-m',
    options.model ?? process.env['HANDOFF_E2E_CODEX_MODEL'] ?? CODEX_DEFAULT_MODEL,
    '-c',
    `model_reasoning_effort=${tomlString(CODEX_REASONING_EFFORT)}`,
    '-s',
    'read-only',
    ...CODEX_DISABLED_FEATURES.flatMap((feature) => ['--disable', feature]),
    ...codexServerOverrides(workspace, options.toolTimeoutMs ?? DEFAULT_TOOL_TIMEOUT_MS),
    options.prompt,
  ];
}

/**
 * The environment of the `codex` child: the parent's without `CLAUDECODE` — the suite is
 * normally started from a Claude Code session — and with the run's `HANDOFF_HOME`. Codex
 * hands its MCP servers a whitelist of its own plus the entry's `env`, so the server gets
 * `HANDOFF_HOME` from the entry, not from here; `USERDOMAIN` and `USERNAME`, which the pipe
 * name is derived from, are on the whitelist (T-066).
 */
export function codexEnvironment(workspace: Workspace): Record<string, string> {
  const child: Record<string, string> = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (value === undefined) continue;
    if (name === 'CLAUDECODE') continue;
    child[name] = value;
  }
  child['HANDOFF_HOME'] = workspace.home;
  return child;
}

/** How to start `codex`: the program, what goes before our arguments, and whether a shell is needed. */
interface CodexCommand {
  readonly command: string;
  readonly prefix: readonly string[];
  readonly shell: boolean;
}

/** Where npm puts the launcher of `@openai/codex`, relative to the folder of its shims. */
const NPM_CODEX_LAUNCHER: readonly string[] = ['node_modules', '@openai', 'codex', 'bin', 'codex.js'];

/** Every `codex` on `PATH`, in the order the system would pick them. */
function codexesOnPath(): string[] {
  try {
    return execFileSync(process.platform === 'win32' ? 'where' : 'which', ['codex'], {
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

/** Whether there is a `codex` to run at all (`main.ts` stops with exit 2 when there is not). */
export function codexOnPath(): boolean {
  return codexesOnPath().length > 0;
}

/**
 * Picks the way to start `codex`.
 *
 * The native installer puts a real `codex.exe` on `PATH`, which is started directly. An npm
 * install leaves shims instead, and a `.cmd` only runs through `cmd.exe`, whose quoting would
 * mangle the JSON a prompt carries; so a shim is bypassed for the launcher it calls, run with
 * this Node, and a shell is used only when that launcher cannot be found. On Windows an
 * extensionless match is the shell-script shim and is skipped. The same rule as the canary's.
 */
export function resolveCodex(): CodexCommand {
  const windows = process.platform === 'win32';
  const usable = codexesOnPath().filter((path) => !windows || /\.(?:exe|cmd|bat|js)$/iu.test(path));
  const first = usable[0];
  if (first === undefined) return { command: 'codex', prefix: [], shell: false };
  const lower = first.toLowerCase();
  if (lower.endsWith('.js')) return { command: process.execPath, prefix: [first], shell: false };
  if (lower.endsWith('.cmd') || lower.endsWith('.bat')) {
    const launcher = join(dirname(first), ...NPM_CODEX_LAUNCHER);
    return existsSync(launcher)
      ? { command: process.execPath, prefix: [launcher], shell: false }
      : { command: first, prefix: [], shell: true };
  }
  return { command: first, prefix: [], shell: false };
}

/**
 * Starts one Codex run and answers a promise of everything it produced — not awaited by the
 * caller straight away, exactly like `startAgent`.
 */
export function startCodex(workspace: Workspace, options: AgentOptions): Promise<AgentRun> {
  const codex = resolveCodex();
  const args = [...codex.prefix, ...codexArgs(options, workspace)];
  const started = Date.now();

  return new Promise<AgentRun>((resolve, reject) => {
    const child = spawn(codex.command, args, {
      cwd: workspace.project,
      env: codexEnvironment(workspace),
      shell: codex.shell,
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
      const run = readCodexRun(stdout, stderr, code, timedOut, Date.now() - started);
      // Codex keeps no transcript of an `--ephemeral` run, so this file is the only record:
      // it is in the run's root, and `HANDOFF_E2E_KEEP=1` keeps it.
      writeFileSync(
        join(workspace.root, `agent-codex-${String(run.sessionId ?? 'unknown')}.jsonl`),
        stdout,
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

/** The text blocks of an MCP result's `content`. */
function textBlocks(content: unknown): string[] {
  if (!Array.isArray(content)) return [];
  return content
    .map((block: unknown) => asRecord(block))
    .filter((block) => block?.['type'] === 'text')
    .map((block) => asText(block?.['text']));
}

/** The image blocks of an MCP result's `content`, in the shape `agent.ts` reports them. */
function imageBlocks(content: unknown): ToolImage[] {
  if (!Array.isArray(content)) return [];
  const images: ToolImage[] = [];
  for (const entry of content as unknown[]) {
    const block = asRecord(entry);
    if (block?.['type'] !== 'image') continue;
    images.push({ mediaType: asText(block['mimeType']), data: asText(block['data']) });
  }
  return images;
}

/** A tool call's arguments: an object, or the JSON text of one. */
function argumentsOf(value: unknown): Record<string, unknown> {
  if (typeof value === 'string') {
    try {
      return { ...(asRecord(JSON.parse(value)) ?? {}) };
    } catch {
      return {};
    }
  }
  return { ...(asRecord(value) ?? {}) };
}

/**
 * Codex's `--json` output as the `AgentRun` every scenario reads.
 *
 * A line that is not a JSON object with a `type` is dropped here and shows up as a missing
 * result instead. An item of type `error` is a warning Codex prints beside a working run and
 * is not a failure; `turn.failed` and a top-level `error` event are.
 */
export function readCodexRun(
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
  let reply = '';
  let turns = 0;
  let failed = false;
  let threadId: string | undefined;
  for (const event of transcript) {
    if (event.type === 'thread.started') {
      const id = event['thread_id'];
      if (typeof id === 'string') threadId = id;
      continue;
    }
    if (event.type === 'turn.completed') {
      turns += 1;
      continue;
    }
    if (event.type === 'turn.failed' || event.type === 'error') {
      failed = true;
      continue;
    }
    if (event.type !== 'item.completed') continue;
    const item = asRecord(event['item']);
    if (item === undefined) continue;
    if (item['type'] === 'agent_message') {
      const text = asText(item['text']);
      if (text.trim() !== '') reply = text;
      continue;
    }
    if (item['type'] !== 'mcp_tool_call') continue;

    const id = asText(item['id']);
    const error = asRecord(item['error']);
    const result = asRecord(item['result']);
    toolUses.push({
      name: `mcp__${asText(item['server'])}__${asText(item['tool'])}`,
      input: argumentsOf(item['arguments']),
      id,
    });
    const blocks = textBlocks(result?.['content']);
    const text = error === undefined ? blocks.join('\n') : asText(error['message']);
    toolResults.push({
      tool_use_id: id,
      isError:
        item['status'] !== 'completed' || error !== undefined || result?.['is_error'] === true,
      text,
      // The outcome is the first text block (§4.3); a second one is the fix text of FM-10.
      outcome: parseOutcome(blocks[0] ?? text) ?? parseOutcome(text),
      images: imageBlocks(result?.['content']),
    });
  }

  const result: TranscriptMessage | undefined =
    turns === 0 && !failed
      ? undefined
      : {
          type: 'result',
          subtype: failed ? 'error' : 'success',
          is_error: failed,
          result: reply,
          num_turns: turns,
        };
  return {
    agent: 'codex',
    exitCode,
    durationMs,
    timedOut,
    stderr,
    transcript,
    toolUses,
    toolResults,
    result,
    sessionId: threadId,
    finalText: reply,
  };
}

/** Codex, as the Codex subset of the suite runs it. */
export const CODEX: AgentRunner = {
  id: 'codex',
  displayName: 'Codex CLI',
  start: startCodex,
  tool: (name) => `${name} (a tool of the MCP server ${MCP_SERVER_NAME})`,
};

/**
 * The preflight of the Codex subset: the real Codex reads the entry the installer writes.
 *
 * The scenarios declare the server through `-c` overrides, because a run must not touch the
 * user's `config.toml`; so on their own they would prove the values and not the file. This
 * hands the golden `config.toml` to `codex mcp get` in a throw-away `CODEX_HOME` — no model,
 * no login, no network, only Codex's own parser — and asks it back: the command, no
 * arguments, our two variables, thirty minutes in seconds, and the approval mode, which only
 * the human-readable form of the answer prints.
 */
export function codexReadsTheInstalledEntry(): Assertion {
  const what = 'the real Codex reads the entry the installer writes (codex mcp get)';
  const home = mkdtempSync(join(tmpdir(), 'baton-e2e-codex-home-'));
  try {
    copyFileSync(INSTALLED_ENTRY, join(home, 'config.toml'));
    const codex = resolveCodex();
    const get = (extra: readonly string[]): string =>
      execFileSync(codex.command, [...codex.prefix, 'mcp', 'get', MCP_SERVER_NAME, ...extra], {
        encoding: 'utf8',
        env: { ...codexEnvironmentOf(process.env), CODEX_HOME: home },
        shell: codex.shell,
        windowsHide: true,
        timeout: 60_000,
      });
    const json = JSON.parse(get(['--json'])) as {
      transport?: { command?: string; args?: unknown[]; env?: Record<string, string> };
      tool_timeout_sec?: number;
    };
    const plain = get([]);
    const env = json.transport?.env ?? {};
    const ok =
      json.transport?.command === '/apps/Baton/handoff-mcp' &&
      Array.isArray(json.transport.args) &&
      json.transport.args.length === 0 &&
      env['HANDOFF_AGENT'] === 'codex' &&
      env['HANDOFF_TOOL_TIMEOUT_MS'] === '1800000' &&
      json.tool_timeout_sec === 1800 &&
      /default_tools_approval_mode: approve/u.test(plain);
    return check(
      'INST-08',
      what,
      'protocol',
      ok,
      `json: ${JSON.stringify(json)}; text: ${JSON.stringify(plain.trim())}`,
    );
  } catch (cause) {
    return check('INST-08', what, 'protocol', false, String(cause));
  } finally {
    rmSync(home, { recursive: true, force: true });
  }
}

/** The parent's environment without `CLAUDECODE`, for a `codex` that is not a scenario's. */
function codexEnvironmentOf(parent: NodeJS.ProcessEnv): Record<string, string> {
  const child: Record<string, string> = {};
  for (const [name, value] of Object.entries(parent)) {
    if (value !== undefined && name !== 'CLAUDECODE') child[name] = value;
  }
  return child;
}
