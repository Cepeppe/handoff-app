/**
 * The WebDriver suite: `pnpm test:ui` (T-055, TECHNICAL-DESIGN §11.4).
 *
 * The window, driven the way a person drives it. `tauri-driver` starts a debug build of the
 * app through `msedgedriver`, and each scenario clicks, types and drags in the real WebView2
 * while a fake server session plays the agent on the real channel. Everything the component
 * suite stubs — the bridge, the IPC, the events, the window the frontend resizes — is the
 * real thing here; what is not real is the person and the agent.
 *
 * ```
 * pnpm test:ui                          # every scenario
 * pnpm test:ui -- collapse preview      # only these
 * pnpm test:ui -- --list                # what exists, without running anything
 * pnpm test:ui -- --setup               # tauri-driver, and this machine's msedgedriver
 * ```
 *
 * Every attempt gets a fresh application, and a scenario that fails is run once more from
 * scratch — the flaky-test guard. A pass on the second attempt is reported as **flaky**, so a
 * guard that is doing work is visible rather than silent. Every failed attempt leaves its
 * evidence in `tests/ui/results/`: a screenshot of the window, its DOM, the error, and what
 * the drivers and the application printed. CI keeps that folder as an artifact.
 *
 * Environment: `HANDOFF_UI_APP` points at another build, `HANDOFF_UI_KEEP=1` keeps each
 * attempt's temporary root, `HANDOFF_UI_RUST_LOG` changes what the application logs, and
 * `HANDOFF_UI_TAURI_DRIVER` / `HANDOFF_UI_MSEDGEDRIVER` point at drivers of your own.
 *
 * Exit codes: **0** every scenario passed, flaky ones included · **1** at least one failed
 * twice · **2** the suite could not run (not Windows, nothing built, no drivers, another Baton
 * running, an unknown scenario).
 */
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { REPO_ROOT } from '../e2e/paths.ts';
import {
  APP_BINARY,
  BUILD_COMMAND,
  batonIsRunning,
  cleanUp,
  makeWorkspace,
  START_FAILURE,
  startApp,
  type RunningApp,
} from './app.ts';
import { FakeServer } from './fake-server.ts';
import { Page, type UiScenario } from './scenario.ts';
import { SCENARIOS } from './scenarios/index.ts';
import { locateTools, setup, TAURI_DRIVER_VERSION, type Tools } from './tools.ts';
import { until } from './wait.ts';

/** Where the report and the evidence of failed attempts go. Git-ignored. */
export const RESULTS_DIR = join(REPO_ROOT, 'tests', 'ui', 'results');

/** How long one attempt may take, application start included. */
const ATTEMPT_TIMEOUT_MS = 180_000;

/** One run and, if it fails, one more. */
const ATTEMPTS = 2;

type Verdict = 'passed' | 'flaky' | 'failed' | 'not run';

interface AttemptReport {
  readonly attempt: number;
  readonly ok: boolean;
  readonly durationMs: number;
  readonly error?: string;
  /** The files of `tests/ui/results/` this attempt left, when it failed. */
  readonly evidence: readonly string[];
}

interface ScenarioReport {
  readonly id: string;
  readonly covers: string;
  readonly title: string;
  readonly verdict: Verdict;
  readonly attempts: readonly AttemptReport[];
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
 * Whether the binary carries the automation channel, which the setup writes its settings
 * through: the two names `scripts/check-no-automation.mjs` looks for, as UTF-8 or UTF-16.
 */
function carriesAutomation(path: string): boolean {
  const bytes = readFileSync(path);
  return bytes.includes(Buffer.from('handoff-e2e', 'utf8')) || bytes.includes(Buffer.from('handoff-e2e', 'utf16le'));
}

/** What stops the suite from running at all, in the order it is worth fixing. */
function preflight(): string[] {
  const problems: string[] = [];
  if (process.platform !== 'win32') {
    problems.push(
      'the WebDriver suite runs on Windows: tauri-driver has no driver for the macOS webview, ' +
        'and macOS is deferred (implementation decision 7).',
    );
    return problems;
  }
  if (!existsSync(APP_BINARY)) {
    problems.push(`${APP_BINARY} is missing. Build it: ${BUILD_COMMAND}`);
  } else if (!carriesAutomation(APP_BINARY)) {
    problems.push(`${APP_BINARY} was built without --features e2e. Build it: ${BUILD_COMMAND}`);
  }
  if (batonIsRunning()) {
    problems.push(
      'Baton is already running on this machine. Quit it from the tray first: a second launch ' +
        'hands itself over to the running instance, and the suite would be driving that one.',
    );
  }
  return problems;
}

/** Keeps what a failed attempt leaves: the error, a screenshot, the DOM, the output. */
async function keepEvidence(
  scenario: UiScenario,
  attempt: number,
  app: RunningApp | undefined,
  error: string,
): Promise<string[]> {
  const stem = `${scenario.id}-attempt${String(attempt)}`;
  const kept: string[] = [];
  const keep = (suffix: string, content: string | Buffer): void => {
    writeFileSync(join(RESULTS_DIR, `${stem}${suffix}`), content);
    kept.push(`${stem}${suffix}`);
  };
  keep('.txt', `${error}\n`);
  if (app !== undefined) {
    try {
      keep('.png', await app.session.screenshot());
    } catch (cause) {
      keep('.png.txt', `no screenshot: ${cause instanceof Error ? cause.message : String(cause)}\n`);
    }
    try {
      keep('.html', await app.session.source());
    } catch {
      // The window is gone, and the error above already says why.
    }
    keep('.log', app.log());
  }
  return kept;
}

/** One attempt at one scenario: a fresh application, the scenario, the evidence, cleanup. */
async function attempt(
  scenario: UiScenario,
  tools: Tools,
  number: number,
  facts: Record<string, unknown>,
): Promise<AttemptReport> {
  const started = Date.now();
  const workspace = makeWorkspace(scenario.id);
  const servers: FakeServer[] = [];
  let app: RunningApp | undefined;
  let page: Page | undefined;
  try {
    // The previous attempt's application must be gone first, for the reason `preflight` gives.
    await until('the previous Baton has left', () => !batonIsRunning(), 30_000);
    scenario.prepare?.(workspace);
    app = await startApp(workspace, tools, { onboarded: scenario.onboarded ?? true });
    const running = app;
    page = new Page(running.session);
    await within(
      scenario.id,
      ATTEMPT_TIMEOUT_MS,
      scenario.run({
        page,
        app: running,
        session: async (options = {}) => {
          const server = await FakeServer.connect(running.channelEndpoint, workspace.home, {
            imagesInResults: options.imagesInResults ?? true,
            projectDir: options.projectDir ?? join(workspace.root, 'project'),
          });
          servers.push(server);
          // The listener answers `hello` and hands the registration to the dispatch task, so
          // there is a moment in which the session has its reference and the registry does not
          // have the session yet. A request sheet opened in that moment reads "no active
          // session" and keeps it — it reads the list once, when it opens. The scenario starts
          // from the registry's answer instead.
          await until('the application has registered the session', async () =>
            (await running.automation.state()).sessions.some(
              (known) => known.sessionRef === server.sessionRef && known.connected,
            ),
          );
          return server;
        },
        facts,
        say: (what) => line(`             · ${what}`),
      }),
    );
    return { attempt: number, ok: true, durationMs: Date.now() - started, evidence: [] };
  } catch (cause) {
    const error = cause instanceof Error ? cause.message : String(cause);
    const evidence = await keepEvidence(scenario, number, app, error);
    return { attempt: number, ok: false, durationMs: Date.now() - started, error, evidence };
  } finally {
    // A panel the page had to open again is reported, attempt by attempt: the guard is doing
    // work, and a run where it does a lot of it is a machine worth looking at.
    if (page !== undefined && page.reopened > 0) {
      facts[`panelReopened (attempt ${String(number)})`] = page.reopened;
    }
    for (const server of servers) server.close();
    await app?.stop().catch(() => undefined);
    cleanUp(workspace);
  }
}

/** One scenario, with its one retry. */
async function runScenario(scenario: UiScenario, tools: Tools): Promise<ScenarioReport> {
  const facts: Record<string, unknown> = {};
  const attempts: AttemptReport[] = [];
  for (let number = 1; number <= ATTEMPTS; number += 1) {
    const result = await attempt(scenario, tools, number, facts);
    attempts.push(result);
    if (result.ok) break;
    const first = (result.error ?? '').split('\n')[0] ?? '';
    line(
      `             attempt ${String(number)} failed: ${first}` +
        (number < ATTEMPTS ? ' — once more, from a fresh application' : ''),
    );
  }
  const last = attempts[attempts.length - 1];
  const verdict: Verdict = last?.ok === true ? (attempts.length > 1 ? 'flaky' : 'passed') : 'failed';
  return { id: scenario.id, covers: scenario.covers, title: scenario.title, verdict, attempts, facts };
}

async function main(argv: readonly string[]): Promise<number> {
  if (argv.includes('--list')) {
    for (const scenario of SCENARIOS) line(`${scenario.id.padEnd(20)} ${scenario.covers.padEnd(38)} ${scenario.title}`);
    return 0;
  }

  if (argv.includes('--setup')) {
    try {
      const tools = await setup();
      line(`ui: tauri-driver ${TAURI_DRIVER_VERSION}: ${tools.tauriDriver}`);
      line(`ui: msedgedriver ${tools.webView2}: ${tools.edgeDriver}`);
      return 0;
    } catch (cause) {
      line(`ui: ${cause instanceof Error ? cause.message : String(cause)}`);
      return 2;
    }
  }

  const wanted = argv.filter((argument) => !argument.startsWith('--'));
  const unknown = wanted.filter((id) => !SCENARIOS.some((scenario) => scenario.id === id));
  if (unknown.length > 0) {
    line(`ui: no scenario named ${unknown.join(', ')}. Try --list.`);
    return 2;
  }

  const problems = preflight();
  const located = process.platform === 'win32' ? locateTools() : { tools: null, missing: [] };
  problems.push(...located.missing);
  if (problems.length > 0 || located.tools === null) {
    for (const problem of problems) line(`ui: ${problem}`);
    return 2;
  }
  const tools = located.tools;

  // Emptied rather than removed: on Windows a folder that is some shell's current directory
  // cannot be deleted, and its files can.
  mkdirSync(RESULTS_DIR, { recursive: true });
  for (const entry of readdirSync(RESULTS_DIR)) {
    rmSync(join(RESULTS_DIR, entry), { recursive: true, force: true });
  }

  const scenarios = wanted.length === 0 ? SCENARIOS : SCENARIOS.filter((scenario) => wanted.includes(scenario.id));
  line(
    `ui: ${String(scenarios.length)} scenarios · ${APP_BINARY} · WebView2 ${tools.webView2} · ` +
      `tauri-driver ${TAURI_DRIVER_VERSION}`,
  );
  const reports: ScenarioReport[] = [];
  for (const [index, scenario] of scenarios.entries()) {
    line(`  ...      ${scenario.id} — ${scenario.title}`);
    const report = await runScenario(scenario, tools);
    reports.push(report);
    const seconds = Math.round(report.attempts.reduce((sum, one) => sum + one.durationMs, 0) / 1000);
    line(`  ${report.verdict.toUpperCase().padEnd(8)} ${report.id} · ${String(seconds)}s`);

    // A scenario whose every attempt died before the window existed says nothing about the
    // window: the machine cannot start the application under the drivers, and every scenario
    // after it would spend the same minutes learning the same thing. The rest are reported as
    // not run, which is what they were, and the run fails.
    if (report.attempts.every((one) => !one.ok && (one.error ?? '').startsWith(START_FAILURE))) {
      line('ui: the drivers could not start the application in any attempt; the scenarios after this one are not run');
      for (const skipped of scenarios.slice(index + 1)) {
        reports.push({
          id: skipped.id,
          covers: skipped.covers,
          title: skipped.title,
          verdict: 'not run',
          attempts: [],
          facts: {},
        });
      }
      break;
    }
  }

  writeFileSync(
    join(RESULTS_DIR, 'last-run.json'),
    `${JSON.stringify(
      {
        generated_at: new Date().toISOString(),
        platform: `${process.platform}-${process.arch}`,
        webview2: tools.webView2,
        tauri_driver: TAURI_DRIVER_VERSION,
        app: APP_BINARY,
        scenarios: reports,
        failed: reports.filter((report) => report.verdict === 'failed').length,
        not_run: reports.filter((report) => report.verdict === 'not run').length,
        flaky: reports.filter((report) => report.verdict === 'flaky').length,
      },
      null,
      2,
    )}\n`,
    'utf8',
  );

  const failed = reports.filter((report) => report.verdict === 'failed');
  const notRun = reports.filter((report) => report.verdict === 'not run');
  const flaky = reports.filter((report) => report.verdict === 'flaky');
  const passed = reports.length - failed.length - notRun.length;
  line(
    `ui: ${String(passed)}/${String(reports.length)} passed` +
      (flaky.length > 0 ? `, ${String(flaky.length)} of them on the second attempt` : '') +
      (notRun.length > 0 ? `, ${String(notRun.length)} not run` : '') +
      ' · report in tests/ui/results/last-run.json',
  );
  for (const report of [...failed, ...flaky]) {
    for (const one of report.attempts.filter((each) => !each.ok)) {
      line(`  ${report.id} attempt ${String(one.attempt)}: ${(one.error ?? '').split('\n')[0] ?? ''}`);
      if (one.evidence.length > 0) line(`    evidence: ${one.evidence.join(', ')}`);
    }
  }
  return failed.length === 0 ? 0 : 1;
}

process.exitCode = await main(process.argv.slice(2));
