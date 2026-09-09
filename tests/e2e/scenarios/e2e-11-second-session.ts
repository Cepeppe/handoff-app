/**
 * E2E-11 — a second session resumes what the first one opened
 * (TECHNICAL-DESIGN §11.5, F-11, TOOL-07, TOOL-08, DD-12).
 *
 * Two real agent processes, one after the other, against one app. The first opens a handoff
 * and leaves it deferred; the second resumes it by id alone, is told whose session opened it
 * (`resumed_from`), guides it to its end, and — asking a second time — is handed the same
 * final outcome with `already_delivered: true` rather than a second copy of the work.
 *
 * Between the two runs the handoff is **detached**: its opener's session is gone and no call
 * is attached, which is the one case §8.4's "Detached" row is really for (T-042's deviation).
 */
import { callsTo, startAgent, statuses } from '../agent.ts';
import { check, type Assertion } from '../classify.ts';
import { Log } from '../db.ts';
import { handoffOf, openPrompt, plantedSpec, REPORT_LINE, walkTheSteps } from '../specs.ts';
import { serverRegistered, wellFormed, type Scenario } from '../scenario.ts';

export const secondSessionScenario: Scenario = {
  id: 'e2e-11-second-session',
  covers: 'E2E-11',
  title: 'a second session resumes a handoff and a second resume says already_delivered',
  timeoutMs: 480_000,

  async run({ workspace, app, forbidden, facts, say }) {
    const planted = plantedSpec('11');
    forbidden.push(...planted.forbidden);

    // ---- session A: open it, meet the deferral, leave.
    const promptA = [
      openPrompt(planted.spec),
      'If the status is "deferred", do NOT resume it: another session will take it over.',
      REPORT_LINE,
    ].join(' ');

    say('session A: starting the agent');
    const agentA = startAgent(workspace, { prompt: promptA, maxTurns: 8 });

    const handoff = await app.theHandoff();
    const id = handoff.tab.id;
    say(`session A opened ${id}; deferring it so A leaves it behind`);
    await app.waitFor(`${id} is being guided`, (seen) => handoffOf(seen, id)?.state === 'active');
    await app.act(id, 'defer', 'Another session will pick this up.');
    await app.waitFor(`${id} is deferred`, (seen) => handoffOf(seen, id)?.state === 'deferred', 60_000);

    const runA = await agentA;
    facts['statuses_a'] = statuses(runA);
    say('session A has left');

    const detached = await app.waitFor(
      `${id} shows as detached once A's server has gone (SRV-22)`,
      (seen) => {
        const one = handoffOf(seen, id);
        return one !== undefined && !one.callAttached && one.uiState === 'detached';
      },
      90_000,
    );
    facts['ui_state_between_sessions'] = handoffOf(detached, id)?.uiState;
    facts['banner_between_sessions'] = handoffOf(detached, id)?.banner?.key;

    // ---- session B: resume by id, finish it, then resume once more.
    const promptB = [
      'You are the coding agent in an automated end-to-end test of a handoff system.',
      'Another session started a handoff and left it behind. Take it over:',
      `call the tool mcp__handoff__handoff_to_user with {"resume": "${id}"} and wait.`,
      'The call blocks while a person works through the steps.',
      'Keep calling {"resume": "' + id + '"} for as long as the outcome you get back is not final.',
      'When you finally receive an outcome with "final": true, call',
      `mcp__handoff__handoff_to_user one last time with {"resume": "${id}"}.`,
      'Then print exactly one line and nothing else, in this form:',
      'STATUS=<the status of the last outcome> ALREADY=<the already_delivered field of the last outcome>.',
    ].join(' ');

    say('session B: starting a second agent to resume it');
    const agentB = startAgent(workspace, { prompt: promptB, maxTurns: 14 });

    const attached = await app.waitFor(
      `${id} has a call attached from session B`,
      (seen) => handoffOf(seen, id)?.callAttached === true,
      180_000,
    );
    facts['ui_state_after_resume'] = handoffOf(attached, id)?.uiState;

    await walkTheSteps(app, id, say);
    await app
      .waitFor(`${id} is closed`, (seen) => handoffOf(seen, id)?.closedAt != null, 120_000)
      .catch(() => undefined);
    const runB = await agentB;
    facts['statuses_b'] = statuses(runB);

    const log = new Log(workspace);
    const row = log.handoff(id);
    const sessions = log.sessions();
    const handoffsInTheLog = log.handoffs().length;
    log.close();

    const outcomesB = runB.toolResults
      .map((result) => result.outcome)
      .filter((outcome): outcome is Record<string, unknown> => outcome !== undefined);
    const withResumedFrom = outcomesB.find((outcome) => outcome['resumed_from'] !== null &&
      outcome['resumed_from'] !== undefined);
    const alreadyDelivered = outcomesB.filter((outcome) => outcome['already_delivered'] === true);
    const finalOutcomes = outcomesB.filter((outcome) => outcome['final'] === true);

    return [
      wellFormed(runA, 'E2E-11'),
      serverRegistered(runA, 'E2E-11'),
      wellFormed(runB, 'E2E-11'),
      check(
        'E2E-11',
        'the two runs register as two sessions of one installation (§8.3)',
        'protocol',
        sessions.length >= 2,
        `sessions: ${JSON.stringify(sessions.map((session) => session.session_ref))}`,
      ),
      check(
        'E2E-11',
        'between the two, the tab reports itself detached (SRV-22, §8.4)',
        'protocol',
        facts['ui_state_between_sessions'] === 'detached',
        `uiState: ${JSON.stringify(facts['ui_state_between_sessions'])}, ` +
          `banner: ${JSON.stringify(facts['banner_between_sessions'])}`,
      ),
      check(
        'E2E-11',
        'session B resumes by id alone, without a spec (TOOL-08)',
        'model',
        callsTo(runB, 'handoff_to_user').every((use) => !('spec' in use.input)) &&
          callsTo(runB, 'handoff_to_user').some((use) => use.input['resume'] === id),
        `calls: ${JSON.stringify(callsTo(runB, 'handoff_to_user').map((use) => Object.keys(use.input)))}`,
      ),
      check(
        'E2E-11',
        'the outcome tells session B which session opened it (resumed_from)',
        'protocol',
        withResumedFrom !== undefined,
        `resumed_from values: ${JSON.stringify(outcomesB.map((outcome) => outcome['resumed_from']))}`,
      ),
      check(
        'E2E-11',
        'the resume clears the detached row while the call is attached (T-042)',
        'protocol',
        facts['ui_state_after_resume'] !== 'detached',
        `uiState after the resume: ${JSON.stringify(facts['ui_state_after_resume'])}`,
      ),
      check(
        'E2E-11',
        'a second resume returns the same final outcome with already_delivered (TOOL-07)',
        'protocol',
        alreadyDelivered.length >= 1 &&
          alreadyDelivered.every((outcome) => outcome['final'] === true),
        `already_delivered outcomes: ${String(alreadyDelivered.length)} of ${String(
          finalOutcomes.length,
        )} final ones; statuses: ${JSON.stringify(statuses(runB))}`,
      ),
      check(
        'E2E-11',
        'both sessions worked on one handoff and one row (DD-13)',
        'protocol',
        handoffsInTheLog === 1 && row?.final_state !== null,
        `handoffs: ${String(handoffsInTheLog)}; row: ${JSON.stringify(row)}`,
      ),
    ] satisfies Assertion[];
  },
};
