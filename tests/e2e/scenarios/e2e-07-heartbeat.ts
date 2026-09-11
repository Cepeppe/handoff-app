/**
 * E2E-7 — the heartbeat with a short tool timeout injected (§11.5, F-06, TOOL-05..07).
 *
 * The one scenario in which the harness does **nothing** for a while. The user is slow; the
 * agent's blocking call would otherwise sit past the runtime's own tool timeout and be cut
 * off with nothing to show for it, so the server returns `in_progress` first, the agent
 * resumes at once, and the handoff is never lost. That is PRIN-10 in one flow: without a
 * raised timeout, resume holds.
 *
 * **The 30 s of the §11.5 table is stale arithmetic and this scenario measures why.** §5.6
 * fires the heartbeat at `max(timeout − 60 s, 50 s)`, and `handoff-mcp`'s
 * `adapters/heartbeat.ts` has the floor as a constant: with `HANDOFF_TOOL_TIMEOUT_MS` at
 * 90 000 the subtraction gives 30 s, the floor raises it to **50 s**, and 30 s would only be
 * right for a timeout of 90 s if the floor did not exist. The assertion is therefore that
 * the heartbeat arrives between 40 s and 75 s, and the measured value is recorded — a
 * scenario that asserted 30 s would have been red for a correct implementation. `TASKS.md`
 * and `DEVIATIONS.md` carry the correction.
 *
 * An agent whose entry has no timeout field takes no injected timeout at all (T-070): Cursor's
 * runner passes none on, the row's own 60 000 ms — the CLI's cut — decides, and the floor lands
 * the heartbeat at the same 50 s. The report records the timeout the agent actually ran with.
 */
import { callsTo, statuses } from '../agent.ts';
import { sleep } from '../automation.ts';
import { check, type Assertion } from '../classify.ts';
import { Log } from '../db.ts';
import { handoffOf, openPrompt, plantedSpec, REPORT_LINE, walkTheSteps } from '../specs.ts';
import { agentRegistered, tabNamesTheAgent, wellFormed, type Scenario } from '../scenario.ts';
import { outcomeOf } from './e2e-04-defer.ts';

/** The tool timeout §11.5 names for this scenario. */
export const TOOL_TIMEOUT_MS = 90_000;

/** The floor of §5.6, which is what actually decides when the heartbeat fires. */
export const HEARTBEAT_FLOOR_MS = 50_000;

/** How long the harness leaves the user idle: past the floor, well inside the timeout. */
export const IDLE_MS = 62_000;

export const heartbeatScenario: Scenario = {
  id: 'e2e-07-heartbeat',
  covers: 'E2E-7',
  title: 'a slow user gets an `in_progress` heartbeat and the resume re-attaches the call',
  timeoutMs: 420_000,

  async run({ workspace, app, agent, forbidden, facts, say }) {
    const planted = plantedSpec('07');
    forbidden.push(...planted.forbidden);
    // The timeout the agent really runs with: the injected one, unless its entry cannot carry
    // one and the agent's own limit decides (Cursor, T-070).
    const toolTimeoutMs = agent.fixedToolTimeoutMs ?? TOOL_TIMEOUT_MS;

    const prompt = [
      openPrompt(planted.spec, agent),
      'If the status is "in_progress" the user is still working and this is not the end of it:',
      `call ${agent.tool('handoff_to_user')} again immediately with {"resume": "<the handoff_id>"}`,
      'and wait again. Repeat that for as long as you keep getting "in_progress".',
      'Stop only when the outcome has "final": true.',
      REPORT_LINE,
    ].join(' ');

    say(
      agent.fixedToolTimeoutMs === undefined
        ? `starting the agent with HANDOFF_TOOL_TIMEOUT_MS=${String(TOOL_TIMEOUT_MS)}`
        : `starting the agent, whose own limit of ${String(toolTimeoutMs)} ms no entry changes`,
    );
    const startedAt = Date.now();
    const running = agent.start(workspace, {
      prompt,
      maxTurns: 16,
      toolTimeoutMs: TOOL_TIMEOUT_MS,
    });

    const handoff = await app.theHandoff();
    const id = handoff.tab.id;
    const openedAt = Date.now();
    say(`the agent opened ${id}; leaving the user idle for ${String(IDLE_MS / 1000)} s`);

    // The user does nothing. The call must come back on its own and be re-attached.
    let detachedAt: number | undefined;
    const watcher = (async () => {
      const deadline = Date.now() + IDLE_MS;
      while (Date.now() < deadline) {
        const seen = await app.state();
        if (detachedAt === undefined && handoffOf(seen, id)?.callAttached === false) {
          detachedAt = Date.now();
        }
        await sleep(500);
      }
    })();
    await watcher;

    say('waiting for the resume to re-attach the call');
    const reattached = await app.waitFor(
      `${id} has a call attached again`,
      (seen) => handoffOf(seen, id)?.callAttached === true,
      120_000,
    );
    facts['heartbeat_after_ms'] = detachedAt === undefined ? null : detachedAt - openedAt;
    facts['heartbeat_floor_ms'] = HEARTBEAT_FLOOR_MS;
    facts['tool_timeout_ms'] = toolTimeoutMs;

    say('finishing the handoff');
    await walkTheSteps(app, id, say);
    await app
      .waitFor(`${id} is closed`, (seen) => handoffOf(seen, id)?.closedAt != null, 120_000)
      .catch(() => undefined);
    const run = await running;
    facts['statuses'] = statuses(run);
    facts['total_ms'] = Date.now() - startedAt;

    const log = new Log(workspace);
    const row = log.handoff(id);
    const handoffsInTheLog = log.handoffs().length;
    log.close();

    const beat = outcomeOf(run, 'in_progress');
    const elapsed = facts['heartbeat_after_ms'];
    const finals = statuses(run).filter((status) =>
      ['verified', 'failed', 'not_verified', 'confirmed_by_user', 'abandoned'].includes(status),
    );

    return [
      wellFormed(run, 'E2E-7'),
      await agentRegistered(run, app, 'E2E-7'),
      tabNamesTheAgent(handoff.tab.label, agent, 'E2E-7'),
      check(
        'E2E-7',
        'the blocked call comes back with status in_progress (TOOL-05)',
        'protocol',
        beat !== undefined,
        `statuses: ${JSON.stringify(statuses(run))}`,
      ),
      check(
        'E2E-7',
        'the heartbeat fires at the floor of §5.6, not at timeout − 60 s',
        'protocol',
        typeof elapsed === 'number' && elapsed > 40_000 && elapsed < 75_000,
        `measured ${String(elapsed)} ms after the handoff appeared; ` +
          `max(${String(toolTimeoutMs)} − 60000, ${String(HEARTBEAT_FLOOR_MS)}) = ${String(
            Math.max(toolTimeoutMs - 60_000, HEARTBEAT_FLOOR_MS),
          )} ms`,
      ),
      check(
        'E2E-7',
        'the in_progress outcome is not final and tells the agent to resume at once',
        'protocol',
        beat?.['final'] === false &&
          typeof beat['instruction'] === 'string' &&
          beat['instruction'].includes('resume'),
        `instruction: ${JSON.stringify(beat?.['instruction'])}`,
      ),
      check(
        'E2E-7',
        'the agent resumes and the call re-attaches (TOOL-06, TOOL-07)',
        'protocol',
        handoffOf(reattached, id)?.callAttached === true &&
          callsTo(run, 'handoff_to_user').some((use) => use.input['resume'] === id),
        `calls: ${JSON.stringify(callsTo(run, 'handoff_to_user').map((use) => Object.keys(use.input)))}`,
      ),
      check(
        'E2E-7',
        'the final outcome is produced exactly once, on one handoff',
        'protocol',
        finals.length === 1 && handoffsInTheLog === 1 && row?.final_state !== null,
        `final statuses: ${JSON.stringify(finals)}; handoffs: ${String(handoffsInTheLog)}`,
      ),
    ] satisfies Assertion[];
  },
};
