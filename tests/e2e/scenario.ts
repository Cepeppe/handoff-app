/**
 * What an e2e scenario is, and the five things every one of them checks (T-043, §11.5).
 *
 * A scenario is one function. It is handed a running app with its automation channel, an
 * isolated workspace and a place to record what it measured; it starts the agent, plays the
 * user, and produces assertions. Everything around it — the temporary folders, the app, the
 * database, the retry, the report — belongs to `main.ts`, so a scenario reads as the story
 * of a run and nothing else.
 *
 * The shared assertions are here because they are the ones a scenario must not be able to
 * forget: that the agent ran at all, that the server registered, and that the log holds no
 * value the user typed and no secret the spec carried (§11.2, LOG-02).
 */
import type { AgentRun } from './agent.ts';
import type { Automation } from './automation.ts';
import type { Workspace } from './app.ts';
import { check, type Assertion } from './classify.ts';
import { leaks, Log } from './db.ts';

/** What a scenario is given. */
export interface ScenarioContext {
  /** The isolated folders of this run. */
  readonly workspace: Workspace;
  /** The automation channel of the running app. */
  readonly app: Automation;
  /** Values that must not appear anywhere in the log afterwards (§11.2). */
  readonly forbidden: string[];
  /** What was measured, for the report. Never a spec value. */
  readonly facts: Record<string, unknown>;
  /** One line on stderr, so a long scenario says where it is. */
  say(what: string): void;
}

/** One scenario of §11.5. */
export interface Scenario {
  /** The id of the §11.5 table, lower-cased and hyphenated: `e2e-01-verified`. */
  readonly id: string;
  /** The row of the §11.5 table this is. */
  readonly covers: string;
  /** One line, in the present tense, for the report. */
  readonly title: string;
  /** How long the whole scenario may take, app and agent included. */
  readonly timeoutMs?: number;
  /** Runs it and answers what it checked. */
  run(context: ScenarioContext): Promise<Assertion[]>;
}

/** The agent ran to a result at all: the first assertion of every scenario. */
export function wellFormed(run: AgentRun, id: string): Assertion {
  const reason = run.timedOut
    ? 'the run was killed at the harness timeout'
    : run.result === undefined
      ? 'stream-json carried no result message'
      : run.exitCode === 0
        ? ''
        : `claude exited ${String(run.exitCode)}`;
  return {
    id,
    what: 'the agent ran to a result',
    kind: 'protocol',
    ok: reason === '',
    ...(reason === ''
      ? {}
      : { detail: `${reason}; stderr: ${run.stderr.slice(0, 400)}; final: ${run.finalText.slice(0, 200)}` }),
  };
}

/** The server registered with the app before the first turn (A-01, SRV-20). */
export function serverRegistered(run: AgentRun, id: string): Assertion {
  const init = run.transcript.find(
    (message) => message.type === 'system' && message.subtype === 'init',
  );
  const entry = init?.mcp_servers?.find((server) => server.name === 'handoff');
  return check(
    id,
    'the agent reports the MCP server connected before the first turn',
    'protocol',
    entry?.status === 'connected',
    `mcp_servers: ${JSON.stringify(init?.mcp_servers ?? null)}`,
  );
}

/**
 * The zero-egress check of §11.7, run after **every** scenario (T-051).
 *
 * §11.7 asks for it with the firewall blocking the app; this is the half a suite can assert
 * without a firewall, and it is the stronger half in one respect — a firewall says what was
 * refused, this says what the app believes it did. The two writers of that belief are the
 * same code path: `net::egress` records a connection **before** it opens one, so a row here
 * would mean an attempt, and no row means no attempt was ever made.
 *
 * In this build the expected count is **zero on every scenario**: the update check that is
 * the one intended caller is deferred (`TASKS.md` §0.4 item 8, T-078). The variant that
 * expects exactly one row, with the check enabled, belongs to that task.
 */
export function zeroEgress(workspace: Workspace, id: string): Assertion {
  let log: Log;
  try {
    log = new Log(workspace);
  } catch (cause) {
    return check(id, 'the log can be read', 'protocol', false, String(cause));
  }
  try {
    const events = log.networkEvents();
    return check(
      id,
      'the app opened no network connection (NET-01, §11.7)',
      'protocol',
      events.length === 0,
      events.length === 0
        ? 'network_events is empty'
        : events.map((event) => `${event.domain} (${event.purpose})`).join(', '),
    );
  } finally {
    log.close();
  }
}

/**
 * The log-invariant check of §11.2, run after **every** scenario.
 *
 * It is here rather than in a scenario of its own because LOG-02 is a property of the log
 * and not of a flow: every scenario plants at least one sentinel and one fixture secret, and
 * every scenario has to come back clean.
 */
export function logInvariants(workspace: Workspace, forbidden: readonly string[], id: string): Assertion {
  if (forbidden.length === 0) {
    return check(id, 'the scenario planted a value to look for in the log', 'protocol', false,
      'a scenario with nothing forbidden cannot check LOG-02');
  }
  let log: Log;
  try {
    log = new Log(workspace);
  } catch (cause) {
    return check(id, 'the log can be read', 'protocol', false, String(cause));
  }
  try {
    const found = leaks(log.dump(), forbidden);
    return check(
      id,
      'no row of the log holds a spec value or a fixture secret (LOG-02)',
      'protocol',
      found.length === 0,
      found.length === 0
        ? `${String(forbidden.length)} value(s) checked against every text column`
        : found.map((leak) => `${leak.needle} in ${leak.where}`).join(', '),
    );
  } finally {
    log.close();
  }
}
