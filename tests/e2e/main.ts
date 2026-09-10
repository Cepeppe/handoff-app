/**
 * The e2e driver: `pnpm e2e` (T-043, TECHNICAL-DESIGN §11.5).
 *
 * It runs the scenarios of `scenarios/`, each in its own isolated app instance, classifies
 * every run, retries a model failure exactly once (§11.5), prints a report on stderr and
 * writes the whole thing to `tests/e2e/results/last-run.json`, which is git-ignored because
 * it names a machine and a moment.
 *
 * ```
 * pnpm e2e                       # every scenario
 * pnpm e2e -- e2e-01-verified    # only these
 * pnpm e2e -- --list             # what exists, without running anything
 * ```
 *
 * Environment: `HANDOFF_E2E_MODEL` pins the model (default `sonnet`), `HANDOFF_E2E_KEEP=1`
 * keeps each run's temporary root so a failure can be read by hand, `HANDOFF_E2E_SERVER`
 * points the MCP entry at a server binary other than the pinned one.
 *
 * Exit codes: **0** every scenario passed · **1** at least one failed · **2** the harness
 * could not run (nothing built, no `claude`, an unknown scenario id).
 *
 * These runs cost real Claude usage and real minutes. Nothing here retries more than §11.5
 * allows, and an app is started and stopped per scenario rather than shared: a scenario that
 * inherited another one's database would be asserting on somebody else's handoffs.
 */
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { transcriptIds } from './agent.ts';
import { cleanUp, makeWorkspace, startApp } from './app.ts';
import { classify, failures, label, reported, shouldRetry, type Assertion, type RunVerdict } from './classify.ts';
import { missingPrerequisites, REPO_ROOT } from './paths.ts';
import { logInvariants, zeroEgress, type Scenario } from './scenario.ts';
import { SCENARIOS } from './scenarios/index.ts';

/** Where the report is written. Git-ignored. */
export const RESULTS_FILE = join(REPO_ROOT, 'tests', 'e2e', 'results', 'last-run.json');

/** How long a scenario may take, app and agent included, unless it asks for more. */
export const DEFAULT_SCENARIO_TIMEOUT_MS = 360_000;

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
async function attempt(scenario: Scenario): Promise<{ assertions: Assertion[]; facts: Record<string, unknown> }> {
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
async function runScenario(scenario: Scenario): Promise<ScenarioReport> {
  const started = Date.now();
  let tries = 0;
  let assertions: Assertion[] = [];
  let facts: Record<string, unknown> = {};
  let verdict: RunVerdict;

  do {
    tries += 1;
    if (tries > 1) line('             model failure, retrying once (§11.5)');
    const attempted = await attempt(scenario);
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
  const wanted = argv.filter((argument) => !argument.startsWith('--'));

  if (argv.includes('--list')) {
    for (const scenario of SCENARIOS) line(`${scenario.id.padEnd(26)} ${scenario.covers}  ${scenario.title}`);
    return 0;
  }

  const missing = missingPrerequisites();
  if (missing.length > 0) {
    for (const problem of missing) line(`e2e: ${problem}`);
    return 2;
  }

  const unknown = wanted.filter((id) => !SCENARIOS.some((scenario) => scenario.id === id));
  if (unknown.length > 0) {
    line(`e2e: no scenario named ${unknown.join(', ')}. Try --list.`);
    return 2;
  }

  const scenarios =
    wanted.length === 0 ? SCENARIOS : SCENARIOS.filter((scenario) => wanted.includes(scenario.id));

  line(`e2e: ${String(scenarios.length)} scenarios against the real Claude Code and the built app`);
  const reports: ScenarioReport[] = [];
  for (const scenario of scenarios) {
    line(`  ...      ${scenario.id} — ${scenario.title}`);
    reports.push(await runScenario(scenario));
    report(reports[reports.length - 1] as ScenarioReport);
  }

  const document = {
    generated_at: new Date().toISOString(),
    platform: `${process.platform}-${process.arch}`,
    model: process.env['HANDOFF_E2E_MODEL'] ?? 'sonnet',
    scenarios: reports,
    failed: reports.filter((scenario) => scenario.verdict !== 'passed').length,
  };
  mkdirSync(join(REPO_ROOT, 'tests', 'e2e', 'results'), { recursive: true });
  writeFileSync(RESULTS_FILE, `${JSON.stringify(document, null, 2)}\n`, 'utf8');

  const failed = reports.filter((scenario) => scenario.verdict !== 'passed');
  const pendingCount = reports.flatMap((scenario) =>
    scenario.assertions.filter((assertion) => !assertion.ok && assertion.pendingTask !== undefined),
  ).length;
  line(
    `e2e: ${String(reports.length - failed.length)}/${String(reports.length)} passed` +
      (pendingCount > 0 ? `, ${String(pendingCount)} pending a later task` : '') +
      ' · report in tests/e2e/results/last-run.json',
  );
  for (const scenario of failed) {
    for (const assertion of failures(scenario.assertions)) {
      line(`  ${scenario.id}: [${assertion.kind}] ${assertion.what}`);
    }
  }
  return failed.length === 0 ? 0 : 1;
}

process.exitCode = await main(process.argv.slice(2));
