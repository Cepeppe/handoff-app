/**
 * The real Claude Code, run non-interactively against the real server (T-043, §11.5).
 *
 * §11.5 prints the command line; four of its flags are not in that text and every one of
 * them cost a run somewhere in this plan (`handoff-mcp/test/canary/README.md`, and the
 * handoff entries of T-023, T-039 and T-042):
 *
 * - **`--allowedTools`** with the three tool names spelled out. Without it a non-interactive
 *   run ends with "Claude requested permissions to use `mcp__handoff__handoff_to_user`, but
 *   you haven't granted it yet", having called nothing.
 * - **`--strict-mcp-config`**, never optional: without it the child loads the account's own
 *   claude.ai connectors.
 * - **`--verbose`**, without which `--output-format stream-json` is refused outright, and
 *   `--output-format json` prints only the final object with no transcript to assert on.
 * - **`--model`**, which pins the run so two of them are comparable and keeps a scenario off
 *   the largest model, which it neither needs nor benefits from.
 *
 * And two rules about what a prompt may say. It must not forbid "any other tool": an MCP
 * tool is not in the model's context directly — it reaches it through its own tool search —
 * so forbidding everything else makes the wanted tool unreachable. And it must carry the
 * spec verbatim: what a scenario tests is the system, not the model's drafting.
 */
import { execFileSync, spawn } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { serverBinary } from './paths.ts';
import type { Workspace } from './app.ts';

/**
 * The `session_id` of every agent run of the current scenario, in order.
 *
 * The acceptance of T-043 asks for the transcript ids to be recorded, and they are the only
 * handle on a run once its temporary workspace is gone: `~/.claude/projects/<slug>/<id>.jsonl`
 * is the child's own transcript, and it is the one place a hook error is written down.
 * `main.ts` empties it between scenarios and puts what it found in the report.
 */
export const transcriptIds: string[] = [];

/** The name the MCP entry is registered under; the tools are `mcp__handoff__*`. */
export const MCP_SERVER_NAME = 'handoff';

/** The three tools, named one by one rather than with a wildcard. */
export const ALLOWED_TOOLS = [
  `mcp__${MCP_SERVER_NAME}__handoff_to_user`,
  `mcp__${MCP_SERVER_NAME}__handoff_verify`,
  `mcp__${MCP_SERVER_NAME}__handoff_runbooks`,
].join(',');

/** The model a scenario runs on unless `HANDOFF_E2E_MODEL` says otherwise. */
export const DEFAULT_MODEL = 'sonnet';

/**
 * The tool timeout a scenario gets unless it is testing the heartbeat.
 *
 * Ten minutes, which puts the heartbeat at `max(600 s − 60 s, 50 s)` = nine minutes — longer
 * than any scenario here, so a blocking call comes back because the user finished and not
 * because a timer fired. The installer writes thirty minutes (T-026); a suite that waited
 * that long for a stuck call would be unusable.
 */
export const DEFAULT_TOOL_TIMEOUT_MS = 600_000;

/** How long one `claude -p` may take before it is killed and the run called a failure. */
export const DEFAULT_TIMEOUT_MS = 300_000;

/** One `stream-json` message. Only the fields a scenario reads are named. */
export interface TranscriptMessage {
  readonly type: string;
  readonly subtype?: string;
  readonly session_id?: string;
  readonly message?: {
    readonly role?: string;
    readonly content?: readonly Record<string, unknown>[];
  };
  readonly mcp_servers?: readonly { readonly name: string; readonly status: string }[];
  readonly tools?: readonly string[];
  readonly num_turns?: number;
  readonly total_cost_usd?: number;
  readonly is_error?: boolean;
  readonly result?: string;
  readonly [field: string]: unknown;
}

/** A `tools/call` as the transcript shows it. */
export interface ToolUse {
  readonly name: string;
  readonly input: Record<string, unknown>;
  readonly id: string;
}

/** An image block of a tool result: what §4.3 maps to `content[1]` (A-07). */
export interface ToolImage {
  /** `image/png` for everything this system sends. */
  readonly mediaType: string;
  /** The base64 payload, exactly as the agent received it. */
  readonly data: string;
}

/** A tool result as the transcript shows it. */
export interface ToolResult {
  readonly tool_use_id: string;
  readonly isError: boolean;
  readonly text: string;
  /** The outcome JSON, when the text was one. */
  readonly outcome: Record<string, unknown> | undefined;
  /**
   * The image blocks beside the text, in order (§4.3, A-07).
   *
   * E2E-3 is the one scenario that reads them, and it reads them twice over: that one is
   * *there* (the agent was handed a picture at all) and that its bytes are the bytes the
   * app burned, which the `sends` row's hash is what it is compared against.
   */
  readonly images: readonly ToolImage[];
}

/** Everything one agent run produced. */
export interface AgentRun {
  /** Which agent produced it: the capability-table key its server resolves (§5.6). */
  readonly agent: 'claude-code' | 'codex';
  readonly exitCode: number | null;
  readonly durationMs: number;
  readonly timedOut: boolean;
  readonly stderr: string;
  readonly transcript: readonly TranscriptMessage[];
  readonly toolUses: readonly ToolUse[];
  readonly toolResults: readonly ToolResult[];
  /** The `result` message, absent when the run did not get that far. */
  readonly result: TranscriptMessage | undefined;
  /** The agent's own session id, from the `init` line (§7.5 binds hooks by it). */
  readonly sessionId: string | undefined;
  /** The final assistant text, which is what a prompt's `STATUS=…` line lands in. */
  readonly finalText: string;
}

/** What a scenario asks of one agent run. */
export interface AgentOptions {
  readonly prompt: string;
  readonly maxTurns?: number;
  /** Off by default: only the scenarios about the safety net install the Stop hook. */
  readonly stopHook?: boolean;
  /** `HANDOFF_TOOL_TIMEOUT_MS`, which drives the heartbeat of §5.6 (E2E-7). */
  readonly toolTimeoutMs?: number;
  /** `Bash`, for the file gates that hold the agent still between two steps. */
  readonly alsoAllow?: readonly string[];
  readonly timeoutMs?: number;
  readonly model?: string;
}

/**
 * Writes `.claude.json` and `.claude/settings.json` into the run's project.
 *
 * The MCP entry runs the **pinned server binary** (§3.5) with the isolated `HANDOFF_HOME`,
 * and the hook entry quotes the path: a hook command runs through `bash` on Windows, which
 * reads every backslash of an unquoted `C:\Users\…` as an escape and dies with `exitCode
 * 127` on every turn (T-039's deviation).
 */
export function writeAgentConfig(workspace: Workspace, options: AgentOptions): string {
  const server = serverBinary();
  const timeout = options.toolTimeoutMs ?? DEFAULT_TOOL_TIMEOUT_MS;
  const mcpConfig = {
    mcpServers: {
      [MCP_SERVER_NAME]: {
        command: server,
        args: ['serve'],
        env: {
          HANDOFF_AGENT: 'claude-code',
          HANDOFF_HOME: workspace.home,
          HANDOFF_TOOL_TIMEOUT_MS: String(timeout),
        },
        timeout,
      },
    },
  };
  const configFile = join(workspace.project, 'mcp.json');
  writeFileSync(configFile, `${JSON.stringify(mcpConfig, null, 2)}\n`, 'utf8');

  const hookCommand = `"${server}" hook stop`;
  const settings =
    options.stopHook === true
      ? {
          hooks: {
            Stop: [{ matcher: '*', hooks: [{ type: 'command', command: hookCommand }] }],
            SubagentStop: [{ matcher: '*', hooks: [{ type: 'command', command: hookCommand }] }],
          },
        }
      : {};
  mkdirSync(join(workspace.project, '.claude'), { recursive: true });
  writeFileSync(
    join(workspace.project, '.claude', 'settings.json'),
    `${JSON.stringify(settings, null, 2)}\n`,
    'utf8',
  );
  return configFile;
}

/** The environment the `claude` child gets. */
export function agentEnvironment(workspace: Workspace): Record<string, string> {
  const child: Record<string, string> = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (value === undefined) continue;
    // A nested Claude Code refuses to run, and this suite is normally started from one.
    if (name === 'CLAUDECODE') continue;
    child[name] = value;
  }
  // The hook is spawned by the agent and inherits from here, not from the MCP entry's `env`.
  child['HANDOFF_HOME'] = workspace.home;
  return child;
}

/**
 * Where `claude` is, and whether it needs a shell.
 *
 * The native installer puts a real `claude.exe` on `PATH`, which `spawn` starts directly; an
 * npm install leaves a `claude.cmd`, which on Windows only a shell can run. One of the
 * prompts is a JSON document full of double quotes, so passing everything through `cmd.exe`
 * when it is not necessary is not a theoretical concern.
 */
export function resolveClaude(): { command: string; shell: boolean } {
  const finder = process.platform === 'win32' ? 'where' : 'which';
  try {
    const found = execFileSync(finder, ['claude'], { encoding: 'utf8', windowsHide: true })
      .split(/\r?\n/u)
      .map((line) => line.trim())
      .filter((line) => line !== '');
    const first = found[0];
    if (first !== undefined) {
      const lower = first.toLowerCase();
      return { command: first, shell: lower.endsWith('.cmd') || lower.endsWith('.bat') };
    }
  } catch {
    // Not on PATH: fall back and let the spawn report it.
  }
  return { command: 'claude', shell: process.platform === 'win32' };
}

/** The command line of §11.5, with the four flags that text does not name. */
export function claudeArgs(options: AgentOptions, configFile: string, project: string): string[] {
  const allowed = [ALLOWED_TOOLS, ...(options.alsoAllow ?? [])].join(',');
  return [
    '-p',
    options.prompt,
    '--mcp-config',
    configFile,
    '--strict-mcp-config',
    '--settings',
    join(project, '.claude', 'settings.json'),
    '--allowedTools',
    allowed,
    '--output-format',
    'stream-json',
    '--verbose',
    '--max-turns',
    String(options.maxTurns ?? 14),
    '--model',
    options.model ?? process.env['HANDOFF_E2E_MODEL'] ?? DEFAULT_MODEL,
  ];
}

/**
 * Starts one agent run and answers a promise of everything it produced.
 *
 * It is deliberately **not** awaited by the caller straight away: a scenario starts the
 * agent, plays the user through the automation channel while the agent's call is blocked,
 * and awaits this at the end. That is the whole shape of an end-to-end run.
 */
export function startAgent(workspace: Workspace, options: AgentOptions): Promise<AgentRun> {
  const configFile = writeAgentConfig(workspace, options);
  const args = claudeArgs(options, configFile, workspace.project);
  const claude = resolveClaude();
  const started = Date.now();

  return new Promise<AgentRun>((resolve, reject) => {
    const child = spawn(claude.command, args, {
      cwd: workspace.project,
      env: agentEnvironment(workspace),
      shell: claude.shell,
      windowsHide: true,
      // `claude -p` waits three seconds for stdin unless it is redirected.
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
      const run = readRun(stdout, stderr, code, timedOut, Date.now() - started);
      // One file per run, numbered: E2E-11 starts two agents in one workspace and the
      // second would otherwise overwrite the first. The name carries the session id, which
      // is also how the child's own transcript is found under `~/.claude/projects/` when a
      // hook misbehaves and only that file says why (the T-039 handoff entry).
      writeFileSync(
        join(workspace.root, `agent-${String(run.sessionId ?? 'unknown')}.jsonl`),
        stdout,
        'utf8',
      );
      transcriptIds.push(run.sessionId ?? 'unknown');
      resolve(run);
    });
  });
}

/** Turns the raw `stream-json` output into the shape a scenario asserts on. */
export function readRun(
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
      transcript.push(JSON.parse(line) as TranscriptMessage);
    } catch {
      // `claude` prints nothing but NDJSON on stdout in this mode; a line that is not JSON
      // is a fact the "the run is well formed" assertion reports.
    }
  }
  const result = transcript.find((message) => message.type === 'result');
  return {
    agent: 'claude-code',
    exitCode,
    durationMs,
    timedOut,
    stderr,
    transcript,
    toolUses: toolUses(transcript),
    toolResults: toolResults(transcript),
    result,
    sessionId: transcript.find((message) => message.subtype === 'init')?.session_id,
    finalText: typeof result?.result === 'string' ? result.result : '',
  };
}

function stringField(block: Record<string, unknown>, name: string): string {
  const value = block[name];
  return typeof value === 'string' ? value : '';
}

function toolUses(transcript: readonly TranscriptMessage[]): ToolUse[] {
  const uses: ToolUse[] = [];
  for (const message of transcript) {
    if (message.type !== 'assistant') continue;
    for (const block of message.message?.content ?? []) {
      if (block['type'] !== 'tool_use') continue;
      const input = block['input'];
      uses.push({
        name: stringField(block, 'name'),
        input: typeof input === 'object' && input !== null ? (input as Record<string, unknown>) : {},
        id: stringField(block, 'id'),
      });
    }
  }
  return uses;
}

function textOf(content: unknown): string {
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return '';
  return content
    .map((block: unknown) =>
      typeof block === 'object' && block !== null && 'text' in block ? String(block.text) : '',
    )
    .join('\n');
}

function toolResults(transcript: readonly TranscriptMessage[]): ToolResult[] {
  const results: ToolResult[] = [];
  for (const message of transcript) {
    if (message.type !== 'user') continue;
    for (const block of message.message?.content ?? []) {
      if (block['type'] !== 'tool_result') continue;
      const text = textOf(block['content']);
      results.push({
        tool_use_id: stringField(block, 'tool_use_id'),
        isError: block['is_error'] === true,
        text,
        outcome: parseOutcome(text),
        images: imagesOf(block['content']),
      });
    }
  }
  return results;
}

/** The image blocks of a tool result's content, in the order the agent was given them. */
function imagesOf(content: unknown): ToolImage[] {
  if (!Array.isArray(content)) return [];
  const images: ToolImage[] = [];
  for (const block of content as unknown[]) {
    if (typeof block !== 'object' || block === null) continue;
    const entry = block as Record<string, unknown>;
    if (entry['type'] !== 'image') continue;
    const source = entry['source'];
    if (typeof source !== 'object' || source === null) continue;
    const fields = source as Record<string, unknown>;
    images.push({
      mediaType: typeof fields['media_type'] === 'string' ? fields['media_type'] : '',
      data: typeof fields['data'] === 'string' ? fields['data'] : '',
    });
  }
  return images;
}

/** The outcome JSON inside a tool result, when the result was one. */
export function parseOutcome(text: string): Record<string, unknown> | undefined {
  const trimmed = text.trim();
  if (!trimmed.startsWith('{')) return undefined;
  try {
    const parsed: unknown = JSON.parse(trimmed);
    return typeof parsed === 'object' && parsed !== null
      ? (parsed as Record<string, unknown>)
      : undefined;
  } catch {
    return undefined;
  }
}

/** Every outcome the agent was handed, in order. */
export function outcomes(run: AgentRun): Record<string, unknown>[] {
  return run.toolResults
    .map((result) => result.outcome)
    .filter((outcome): outcome is Record<string, unknown> => outcome !== undefined);
}

/** The `status` of every outcome the agent was handed, in order. */
export function statuses(run: AgentRun): string[] {
  return outcomes(run)
    .map((outcome) => outcome['status'])
    .filter((status): status is string => typeof status === 'string');
}

/** The calls the agent made to one of our tools, in order. */
export function callsTo(run: AgentRun, tool: string): ToolUse[] {
  return run.toolUses.filter((use) => use.name === `mcp__${MCP_SERVER_NAME}__${tool}`);
}

/**
 * The agent a scenario runs against (T-067): how it is started, and how a prompt names our
 * tools to it.
 *
 * Claude Code is the default and the only agent of E2E-3, 5, 6, 10 and 11; Codex runs the
 * subset of `scenarios/index.ts` through `codex.ts`. Both answer the same `AgentRun`, with a
 * tool use named `mcp__handoff__<tool>` whichever agent made it, so an assertion reads the
 * same against either.
 */
export interface AgentRunner {
  /** The capability-table key the server resolves for this agent (§5.6). */
  readonly id: 'claude-code' | 'codex';
  /** The name its capability row carries, which is what the tab of its session shows. */
  readonly displayName: string;
  /** Starts one run. Not awaited straight away: the harness plays the user meanwhile. */
  start(workspace: Workspace, options: AgentOptions): Promise<AgentRun>;
  /** How a prompt names one of our tools to this agent. */
  tool(name: string): string;
}

/** Claude Code, exactly as every scenario of T-043 has run it. */
export const CLAUDE_CODE: AgentRunner = {
  id: 'claude-code',
  displayName: 'Claude Code',
  start: startAgent,
  tool: (name) => `mcp__${MCP_SERVER_NAME}__${name}`,
};
