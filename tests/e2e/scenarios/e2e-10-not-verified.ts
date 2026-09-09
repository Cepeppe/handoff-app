/**
 * E2E-10 — the verification nobody reports (§11.5, F-08, VER-06, SRV-12, PRIN-08).
 *
 * The user does everything right and the agent walks away. PRIN-08 says a handoff is never
 * "verified" on trust, so the window of VER-06 runs out and the handoff is recorded
 * `not_verified` — a state that is honest rather than convenient — and the Stop hook stops
 * the agent once on the way to remind it (SRV-10..13).
 *
 * The window is thirty minutes in the product (§4.1 `VERIFYING_TIMEOUT_MS`), which no test
 * can wait for, so the harness injects twenty seconds through the automation channel's one
 * e2e-only setting (`e2e.verifying_timeout_ms`). It is not persisted: it lives in the store
 * actor for this process and dies with it.
 *
 * **Which of VER-06's two roads the run takes is not fixed, and §11.5's row assumes the
 * wrong one** (`DEVIATIONS.md`, T-045). The agent is told not to report and then finishes
 * its turn, so its session ends — and §8.3 closes an unreported verification at the
 * disconnect, before the injected window can run out. Measured here: `not_verified` at
 * about 9 s of a 20 s window, by the disconnect. Both roads are VER-06 and both are what
 * this scenario is about, so it asserts the state, the fact that the tab **says which**, and
 * that it happened nowhere near the thirty minutes of §4.1; `facts.not_verified_reason`
 * records the road that was taken.
 */
import { callsTo, startAgent, statuses } from '../agent.ts';
import { sleep } from '../automation.ts';
import { check, type Assertion } from '../classify.ts';
import { Log } from '../db.ts';
import { handoffOf, openPrompt, plantedSpec, REPORT_LINE, VERIFY_TEXT, walkTheSteps } from '../specs.ts';
import { serverRegistered, wellFormed, type Scenario } from '../scenario.ts';

/** The e2e-only setting `src-tauri/src/e2e/api.rs` forwards to the store. */
export const VERIFYING_TIMEOUT_KEY = 'e2e.verifying_timeout_ms';

/** The window this run gives the agent to report. */
export const INJECTED_WINDOW_MS = 20_000;

export const notVerifiedScenario: Scenario = {
  id: 'e2e-10-not-verified',
  covers: 'E2E-10',
  title: 'a verification that is never reported ends `not_verified` after its window',
  timeoutMs: 420_000,

  async run({ workspace, app, forbidden, facts, say }) {
    const planted = plantedSpec('10', { verify: VERIFY_TEXT });
    forbidden.push(...planted.forbidden);

    say(`injecting a ${String(INJECTED_WINDOW_MS / 1000)} s verification window`);
    const injected = await app.setSetting(VERIFYING_TIMEOUT_KEY, INJECTED_WINDOW_MS);
    facts['verifying_timeout_ms'] = injected;

    const prompt = [
      openPrompt(planted.spec),
      'When the call returns, read the status.',
      'This run is about what happens when a verification is never reported, so:',
      'do NOT call mcp__handoff__handoff_verify, not now and not later,',
      'even if something stops you and asks you to report it. If that happens, answer that',
      'you are deliberately not reporting it, and finish.',
      REPORT_LINE,
    ].join(' ');

    say('starting the agent, with the Stop hook installed');
    const agent = startAgent(workspace, { prompt, maxTurns: 12, stopHook: true });

    const handoff = await app.theHandoff();
    const id = handoff.tab.id;
    say(`the agent opened ${id}; walking the steps`);
    await walkTheSteps(app, id, say);

    const waitingForReport = await app.waitFor(
      `${id} is awaiting the verification`,
      (seen) => handoffOf(seen, id)?.state === 'awaiting_verification',
      120_000,
    );
    facts['banner_while_verifying'] = handoffOf(waitingForReport, id)?.banner?.key;
    const startedWaiting = Date.now();

    say(`waiting out the ${String(INJECTED_WINDOW_MS / 1000)} s window`);
    const timedOut = await app.waitFor(
      `${id} timed out into not_verified`,
      (seen) => handoffOf(seen, id)?.state === 'not_verified',
      120_000,
    );
    facts['window_measured_ms'] = Date.now() - startedWaiting;
    const finalView = handoffOf(timedOut, id);
    facts['not_verified_reason'] = finalView?.notVerifiedReason ?? undefined;

    // The agent may still be in the middle of its turn; give it its budget rather than
    // killing it, so the hook has fired and the transcript is complete.
    const run = await agent;
    facts['statuses'] = statuses(run);
    await sleep(500);

    const log = new Log(workspace);
    const row = log.handoff(id);
    const blocks = log.hookBlocks().filter((block) => block.item_key.includes(id));
    log.close();

    return [
      wellFormed(run, 'E2E-10'),
      serverRegistered(run, 'E2E-10'),
      check(
        'E2E-10',
        'the injected verification window reaches the store (DD-33)',
        'protocol',
        injected === INJECTED_WINDOW_MS,
        `settings answered ${JSON.stringify(injected)}`,
      ),
      check(
        'E2E-10',
        'the last step moves the handoff to awaiting_verification (RESP-09, VER-04)',
        'protocol',
        statuses(run).includes('awaiting_verification'),
        `statuses: ${JSON.stringify(statuses(run))}`,
      ),
      check(
        'E2E-10',
        'the agent does not report the verification, as the prompt asked',
        'model',
        callsTo(run, 'handoff_verify').length === 0,
        `handoff_verify calls: ${String(callsTo(run, 'handoff_verify').length)}`,
      ),
      check(
        'E2E-10',
        'the window runs out and the handoff is recorded not_verified (VER-06, PRIN-08)',
        'protocol',
        row?.state === 'not_verified' && row.final_state === 'not_verified',
        `row: ${JSON.stringify(row)}`,
      ),
      check(
        'E2E-10',
        'it closed inside the injected window and not on the thirty minutes of §4.1',
        'protocol',
        typeof facts['window_measured_ms'] === 'number' && facts['window_measured_ms'] < 90_000,
        `measured ${String(facts['window_measured_ms'])} ms`,
      ),
      check(
        'E2E-10',
        'the overlay shows it as final and closed',
        'protocol',
        finalView?.closedAt != null,
        `view: ${JSON.stringify({ state: finalView?.state, closedAt: finalView?.closedAt })}`,
      ),
      check(
        'E2E-10',
        'the tab says *why* it is not verified, and it is one of VER-06’s two roads (§8.4)',
        'protocol',
        finalView?.notVerifiedReason === 'overlay.notVerifiedTimeout' ||
          finalView?.notVerifiedReason === 'overlay.notVerifiedSessionGone',
        `notVerifiedReason: ${JSON.stringify(finalView?.notVerifiedReason)}`,
      ),
      check(
        'E2E-10',
        'the Stop hook stopped the agent exactly once about the unreported verification (SRV-12)',
        'protocol',
        blocks.length === 1,
        `hook_blocks for ${id}: ${JSON.stringify(blocks)}`,
      ),
    ] satisfies Assertion[];
  },
};
