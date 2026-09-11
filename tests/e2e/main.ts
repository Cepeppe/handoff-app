/**
 * The e2e driver: `pnpm e2e` (T-043, TECHNICAL-DESIGN §11.5).
 *
 * It runs the scenarios of `scenarios/`, each in its own isolated app instance, classifies
 * every run, retries a model failure exactly once (§11.5), prints a report on stderr and
 * writes the whole thing to `tests/e2e/results/`, which is git-ignored because it names a
 * machine and a moment.
 *
 * ```
 * pnpm e2e                          # every scenario, against Claude Code
 * pnpm e2e -- e2e-01-verified       # only these
 * pnpm e2e -- --agent codex         # the Codex subset, against the Codex CLI (T-067)
 * pnpm e2e -- --agent opencode      # the OpenCode subset, against OpenCode (T-074)
 * pnpm e2e -- --agent cursor        # the Cursor subset, against Cursor's editor and CLI (T-070)
 * pnpm e2e -- --agent copilot       # the GitHub Copilot subset, against VS Code and the Copilot CLI (T-072)
 * pnpm e2e -- --list                # what exists, without running anything
 * ```
 *
 * Environment: `HANDOFF_E2E_MODEL` pins Claude Code's model (default `sonnet`),
 * `HANDOFF_E2E_CODEX_MODEL` Codex's (default `gpt-5.6-luna`), `HANDOFF_E2E_OPENCODE_MODEL`
 * OpenCode's (default a free OpenRouter model, see `opencode.ts`), `HANDOFF_E2E_CURSOR_MODEL`
 * Cursor's (default `auto`), `HANDOFF_E2E_COPILOT_MODEL` Copilot's (default `auto`),
 * `HANDOFF_E2E_KEEP=1` keeps
 * each run's temporary root so a failure can be read by hand, `HANDOFF_E2E_SERVER` points the
 * MCP entry at a server binary other than the pinned one.
 *
 * Exit codes: **0** every scenario passed · **1** at least one failed · **2** the harness
 * could not run (nothing built, no agent on PATH, an unknown scenario id or agent).
 *
 * These runs cost real usage and real minutes. Nothing here retries more than §11.5
 * allows, and an app is started and stopped per scenario rather than shared: a scenario that
 * inherited another one's database would be asserting on somebody else's handoffs.
 */
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { CLAUDE_CODE, DEFAULT_MODEL, transcriptIds, type AgentRunner } from './agent.ts';
import { cleanUp, makeWorkspace, startApp } from './app.ts';
import { classify, failures, label, reported, shouldRetry, type Assertion, type RunVerdict } from './classify.ts';
import { CODEX, CODEX_DEFAULT_MODEL, codexOnPath, codexReadsTheInstalledEntry } from './codex.ts';
import {
  COPILOT,
  COPILOT_DEFAULT_MODEL,
  copilotOnPath,
  copilotReadsTheInstalledEntry,
} from './copilot.ts';
import {
  CURSOR,
  CURSOR_DEFAULT_MODEL,
  cursorOnPath,
  cursorReadsTheInstalledEntry,
  cursorUserConfigProblem,
} from './cursor.ts';
import {
  OPENCODE,
  OPENCODE_DEFAULT_MODEL,
  opencodeOnPath,
  opencodeReadsTheInstalledEntry,
} from './opencode.ts';
import { missingPrerequisites, REPO_ROOT } from './paths.ts';
import { logInvariants, zeroEgress, type Scenario } from './scenario.ts';
import {
  CODEX_SCENARIOS,
  COPILOT_SCENARIOS,
  CURSOR_SCENARIOS,
  OPENCODE_SCENARIOS,
  SCENARIOS,
} from './scenarios/index.ts';

/** Where the reports are written. Git-ignored. */
export const RESULTS_DIR = join(REPO_ROOT, 'tests', 'e2e', 'results');

/** How long a scenario may take, app and agent included, unless it asks for more. */
export const DEFAULT_SCENARIO_TIMEOUT_MS = 360_000;

/**
 * The agents the suite runs against: what each one runs, on which model, and where its report
 * goes. One report file per agent, so a Codex run never overwrites the Claude Code report the
 * T-043 procedure records after every Claude Code update.
 */
const AGENTS: Readonly<
  Record<
    string,
    {
      readonly runner: AgentRunner;
      readonly scenarios: readonly Scenario[];
      readonly model: () => string;
      readonly results: string;
    }
  >
> = {
  'claude-code': {
    runner: CLAUDE_CODE,
    scenarios: SCENARIOS,
    model: () => process.env['HANDOFF_E2E_MODEL'] ?? DEFAULT_MODEL,
    results: 'last-run.json',
  },
  codex: {
    runner: CODEX,
    scenarios: CODEX_SCENARIOS,
    model: () => process.env['HANDOFF_E2E_CODEX_MODEL'] ?? CODEX_DEFAULT_MODEL,
    results: 'last-run-codex.json',
  },
  opencode: {
    runner: OPENCODE,
    scenarios: OPENCODE_SCENARIOS,
    model: () => process.env['HANDOFF_E2E_OPENCODE_MODEL'] ?? OPENCODE_DEFAULT_MODEL,
    results: 'last-run-opencode.json',
  },
  cursor: {
    runner: CURSOR,
    scenarios: CURSOR_SCENARIOS,
    model: () => process.env['HANDOFF_E2E_CURSOR_MODEL'] ?? CURSOR_DEFAULT_MODEL,
    results: 'last-run-cursor.json',
  },
  copilot: {
    runner: COPILOT,
    scenarios: COPILOT_SCENARIOS,
    model: () => process.env['HANDOFF_E2E_COPILOT_MODEL'] ?? COPILOT_DEFAULT_MODEL,
    results: 'last-run-copilot.json',
  },
};

/**
 * The one check a scenario cannot make, because every scenario must stay off the user's
 * configuration: that the agent itself reads what its installer writes (T-067, T-074, T-070,
 * T-072). Claude Code's is the golden-file suite's plus the smoke of T-042.
 */
const PREFLIGHTS: Readonly<Record<string, () => Assertion>> = {
  codex: codexReadsTheInstalledEntry,
  opencode: opencodeReadsTheInstalledEntry,
  cursor: cursorReadsTheInstalledEntry,
  copilot: copilotReadsTheInstalledEntry,
};

/** The program each agent is, for the prerequisite check. */
const ON_PATH: Readonly<Record<string, { readonly found: () => boolean; readonly why: string }>> = {
  codex: {
    found: codexOnPath,
    why: 'codex is not on PATH. The Codex subset drives the real Codex CLI, logged in.',
  },
  opencode: {
    found: opencodeOnPath,
    why: 'opencode is not on PATH. The OpenCode subset drives the real OpenCode, logged in.',
  },
  cursor: {
    found: cursorOnPath,
    why: "cursor-agent is not on PATH. The Cursor subset drives Cursor's real Agent CLI, logged in.",
  },
  copilot: {
    found: copilotOnPath,
    why: 'copilot is not on PATH. The GitHub Copilot subset drives the real Copilot CLI, signed in.',
  },
};

/**
 * What else must hold on this machine before an agent's subset may run, as a sentence for the
 * report, or nothing (T-070): the Cursor CLI reads the user's own `~/.cursor/mcp.json` into
 * every run, so Baton registered there would load beside the run's server.
 */
const MACHINE_CHECKS: Readonly<Record<string, () => string | undefined>> = {
  cursor: () => cursorUserConfigProblem(),
};

interface ScenarioReport {
  readonly id: string;
  readonly covers: string;
  readonly title: string;
  readonly verdict: RunVerdict;
  readonly attempts: number;
  readonly durationMs: number;
  readonly assertions: readonly Assertion[];
  readonly facts: Record<string, unknown>;
}

function line(text: string): void {
  process.stderr.write(`${text}\n`);
}

/** Rejects with `what` if `work` has not settled in time, so one hang is not the whole run. */
async function within<T>(what: string, ms: number, work: Promise<T>): Promise<T> {
  let timer: NodeJS.Timeout | undefined;
  const alarm = new Promise<never>((_resolve, reject) => {
    timer = setTimeout(() => reject(new Error(`${what} did not finish within ${String(ms)} ms`)), ms);
    timer.unref();
  });
  try {
    return await Promise.race([work, alarm]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/**
 * One attempt at one scenario: a fresh app, the scenario, the two invariants, cleanup.
 *
 * The invariants are the log's (§11.2: no spec value, no fixture secret) and the network's
 * (§11.7: no connection at all in this build). Both are properties of the product rather
 * than of a flow, so both run after every scenario instead of in the one that seemed
 * relevant, and both read the database after the app has stopped writing it.
 */
async function attempt(
  scenario: Scenario,
  agent: AgentRunner,
): Promise<{ assertions: Assertion[]; facts: Record<string, unknown> }> {
  const workspace = makeWorkspace(scenario.id.replace(/[^a-z0-9]/giu, ''));
  transcriptIds.length = 0;
  const forbidden: string[] = [];
  const facts: Record<string, unknown> = {};
  let assertions: Assertion[] = [];

  const app = await startApp(workspace);
  try {
    assertions = await within(
      scenario.id,
      scenario.timeoutMs ?? DEFAULT_SCENARIO_TIMEOUT_MS,
      scenario.run({
        workspace,
        app: app.automation,
        agent,
        forbidden,
        facts,
        say: (what) => line(`             · ${what}`),
      }),
    );
  } catch (cause) {
    assertions = [
      {
        id: scenario.covers,
        what: 'the scenario ran to its assertions',
        kind: 'protocol',
        ok: false,
        detail: `${cause instanceof Error ? cause.message : String(cause)}\n` +
          `--- app log (tail) ---\n${app.log().slice(-2500)}`,
      },
    ];
  } finally {
    // The app is stopped **before** the log invariants: the check reads the database, and a
    // WAL file with a live writer is exactly the thing a final read should not race.
    await app.stop().catch(() => undefined);
  }

  assertions.push(logInvariants(workspace, forbidden, scenario.covers));
  assertions.push(zeroEgress(workspace, scenario.covers));
  facts['transcript_ids'] = [...transcriptIds];
  facts['workspace'] = process.env['HANDOFF_E2E_KEEP'] === '1' ? workspace.root : undefined;
  cleanUp(workspace);
  return { assertions, facts };
}

/** One scenario, with §11.5's single retry for a model failure and nothing more. */
async function runScenario(scenario: Scenario, agent: AgentRunner): Promise<ScenarioReport> {
  const started = Date.now();
  let tries = 0;
  let assertions: Assertion[] = [];
  let facts: Record<string, unknown> = {};
  let verdict: RunVerdict;

  do {
    tries += 1;
    if (tries > 1) line('             model failure, retrying once (§11.5)');
    const attempted = await attempt(scenario, agent);
    assertions = attempted.assertions;
    facts = attempted.facts;
    verdict = classify(assertions);
  } while (shouldRetry(verdict, tries));

  return {
    id: scenario.id,
    covers: scenario.covers,
    title: scenario.title,
    verdict,
    attempts: tries,
    durationMs: Date.now() - started,
    assertions,
    facts,
  };
}

function report(scenario: ScenarioReport): void {
  const mark = scenario.verdict === 'passed' ? 'PASS' : scenario.verdict.toUpperCase();
  line(
    `  ${mark.padEnd(8)} ${scenario.id} · ${String(Math.round(scenario.durationMs / 1000))}s` +
      (scenario.attempts > 1 ? ` · ${String(scenario.attempts)} attempts` : ''),
  );
  for (const assertion of reported(scenario.assertions)) {
    line(`             [${label(assertion)}] ${assertion.id}: ${assertion.what}`);
    if (assertion.detail !== undefined) {
      for (const detail of assertion.detail.split('\n')) line(`               ${detail}`);
    }
  }
}

async function main(argv: readonly string[]): Promise<number> {
  const agentAt = argv.indexOf('--agent');
  const agentId = agentAt === -1 ? 'claude-code' : (argv[agentAt + 1] ?? '');
  const chosen = AGENTS[agentId];
  if (chosen === undefined) {
    line(`e2e: no agent named "${agentId}". The agents are ${Object.keys(AGENTS).join(', ')}.`);
    return 2;
  }
  const wanted = argv.filter(
    (argument, index) => !argument.startsWith('--') && !(agentAt !== -1 && index === agentAt + 1),
  );

  if (argv.includes('--list')) {
    for (const scenario of chosen.scenarios) line(`${scenario.id.padEnd(26)} ${scenario.covers}  ${scenario.title}`);
    return 0;
  }

  const missing = missingPrerequisites();
  const program = ON_PATH[chosen.runner.id];
  if (program !== undefined && !program.found()) missing.push(program.why);
  const machine = MACHINE_CHECKS[chosen.runner.id]?.();
  if (machine !== undefined) missing.push(machine);
  if (missing.length > 0) {
    for (const problem of missing) line(`e2e: ${problem}`);
    return 2;
  }

  const unknown = wanted.filter((id) => !chosen.scenarios.some((scenario) => scenario.id === id));
  if (unknown.length > 0) {
    line(`e2e: no scenario named ${unknown.join(', ')} for ${agentId}. Try --list.`);
    return 2;
  }

  const scenarios =
    wanted.length === 0
      ? chosen.scenarios
      : chosen.scenarios.filter((scenario) => wanted.includes(scenario.id));

  const preflight: Assertion[] = [];
  const preflightCheck = PREFLIGHTS[chosen.runner.id];
  if (preflightCheck !== undefined) {
    const reads = preflightCheck();
    preflight.push(reads);
    line(`  ${(reads.ok ? 'PASS' : 'PROTOCOL').padEnd(8)} preflight · ${reads.what}`);
    if (!reads.ok && reads.detail !== undefined) {
      for (const detail of reads.detail.split('\n')) line(`               ${detail}`);
    }
  }

  line(
    `e2e: ${String(scenarios.length)} scenarios against the real ${chosen.runner.displayName} and the built app`,
  );
  const reports: ScenarioReport[] = [];
  for (const scenario of scenarios) {
    line(`  ...      ${scenario.id} — ${scenario.title}`);
    reports.push(await runScenario(scenario, chosen.runner));
    report(reports[reports.length - 1] as ScenarioReport);
  }

  const failed = reports.filter((scenario) => scenario.verdict !== 'passed');
  const preflightFailed = preflight.filter((assertion) => !assertion.ok).length;
  const document = {
    generated_at: new Date().toISOString(),
    platform: `${process.platform}-${process.arch}`,
    agent: chosen.runner.id,
    model: chosen.model(),
    preflight,
    scenarios: reports,
    failed: failed.length + preflightFailed,
  };
  const results = join(RESULTS_DIR, chosen.results);
  mkdirSync(RESULTS_DIR, { recursive: true });
  writeFileSync(results, `${JSON.stringify(document, null, 2)}\n`, 'utf8');

  const pendingCount = reports.flatMap((scenario) =>
    scenario.assertions.filter((assertion) => !assertion.ok && assertion.pendingTask !== undefined),
  ).length;
  line(
    `e2e: ${String(reports.length - failed.length)}/${String(reports.length)} passed` +
      (preflightFailed > 0 ? `, the preflight failed` : '') +
      (pendingCount > 0 ? `, ${String(pendingCount)} pending a later task` : '') +
      ` · report in tests/e2e/results/${chosen.results}`,
  );
  for (const scenario of failed) {
    for (const assertion of failures(scenario.assertions)) {
      line(`  ${scenario.id}: [${assertion.kind}] ${assertion.what}`);
    }
  }
  return failed.length === 0 && preflightFailed === 0 ? 0 : 1;
}

process.exitCode = await main(process.argv.slice(2));
