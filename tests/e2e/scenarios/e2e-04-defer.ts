/**
 * E2E-4 — defer → agent resumes → done (TECHNICAL-DESIGN §11.5, F-05, RESP-05, TOOL-14).
 *
 * The user defers the step; the blocking call comes back `deferred` with the instruction to
 * come back to it; the agent resumes and the tab goes **back to `active`** — the correction
 * `DEVIATIONS.md` records under T-034, and the one thing an agent that defers and returns
 * must find. The user then finishes the round and the handoff waits for the verification.
 */
import { callsTo, statuses } from '../agent.ts';
import { check, type Assertion } from '../classify.ts';
import { Log } from '../db.ts';
import {
  catalogueText,
  handoffOf,
  NO_HOOK_PHRASE,
  openPrompt,
  plantedSpec,
  REPORT_LINE,
  VERIFY_TEXT,
  walkTheSteps,
} from '../specs.ts';
import { agentRegistered, tabNamesTheAgent, wellFormed, type Scenario } from '../scenario.ts';

/** The reason the user types into the Defer sheet. */
export const DEFER_REASON = 'I do not have the console open right now.';

export const deferScenario: Scenario = {
  id: 'e2e-04-defer',
  covers: 'E2E-4',
  title: 'a deferred handoff comes back to `active` when the agent resumes it',

  async run({ workspace, app, agent, forbidden, facts, say }) {
    const planted = plantedSpec('04', { verify: VERIFY_TEXT });
    forbidden.push(...planted.forbidden);

    const prompt = [
      openPrompt(planted.spec, agent),
      'If the call returns with status "deferred", the user postponed the work.',
      `Follow the instruction you were given: call ${agent.tool('handoff_to_user')} again with`,
      '{"resume": "<the handoff_id>"} and wait again.',
      `If the status becomes "awaiting_verification", call ${agent.tool('handoff_verify')} once with`,
      `{"handoff_id": "<the handoff_id>", "verify": {"ok": true, "detail": "${VERIFY_TEXT}"}}.`,
      REPORT_LINE,
    ].join(' ');

    say('starting the agent');
    const running = agent.start(workspace, { prompt, maxTurns: 12 });

    const handoff = await app.theHandoff();
    const id = handoff.tab.id;
    say(`the agent opened ${id}; deferring it`);
    await app.waitFor(`${id} is being guided`, (seen) => handoffOf(seen, id)?.state === 'active');
    await app.act(id, 'defer', DEFER_REASON);

    const deferred = await app.waitFor(
      `${id} is deferred`,
      (seen) => handoffOf(seen, id)?.state === 'deferred',
      60_000,
    );
    facts['deferral_count_when_deferred'] = handoffOf(deferred, id)?.deferralCount;
    facts['banner_when_deferred'] = handoffOf(deferred, id)?.banner?.key;

    say('waiting for the agent to resume');
    const resumed = await app.waitFor(
      `${id} is active again with a call attached`,
      (seen) => {
        const one = handoffOf(seen, id);
        return one?.state === 'active' && one.callAttached;
      },
      180_000,
    );
    const afterResume = handoffOf(resumed, id);

    await walkTheSteps(app, id, say);
    const waiting = await app.waitFor(
      `${id} is awaiting the verification`,
      (seen) => {
        const one = handoffOf(seen, id);
        return one !== undefined && one.state !== 'active';
      },
      120_000,
    );
    const stateAfterDone = handoffOf(waiting, id)?.state;

    say('waiting for the agent to close it');
    await app
      .waitFor(`${id} is closed`, (seen) => handoffOf(seen, id)?.closedAt != null, 120_000)
      .catch(() => waiting);
    const run = await running;
    facts['statuses'] = statuses(run);

    const log = new Log(workspace);
    const row = log.handoff(id);
    const handoffsInTheLog = log.handoffs().length;
    log.close();

    const deferredOutcome = outcomeOf(run, 'deferred');
    const deferredInstruction =
      typeof deferredOutcome?.['instruction'] === 'string' ? deferredOutcome['instruction'] : '';
    const bannerKey = facts['banner_when_deferred'];
    const bannerText = typeof bannerKey === 'string' ? catalogueText(bannerKey) : undefined;
    return [
      wellFormed(run, 'E2E-4'),
      await agentRegistered(run, app, 'E2E-4'),
      tabNamesTheAgent(handoff.tab.label, agent, 'E2E-4'),
      check(
        'E2E-4',
        'the blocking call comes back with status deferred (RESP-05)',
        'protocol',
        deferredOutcome !== undefined,
        `statuses: ${JSON.stringify(statuses(run))}`,
      ),
      check(
        'E2E-4',
        'the deferred outcome counts one deferral and carries the reason the user typed',
        'protocol',
        deferredOutcome?.['deferral_count'] === 1 && deferredOutcome['user_text'] === DEFER_REASON,
        `deferral_count: ${JSON.stringify(deferredOutcome?.['deferral_count'])}, ` +
          `user_text: ${JSON.stringify(deferredOutcome?.['user_text'])}`,
      ),
      check(
        'E2E-4',
        'the overlay records exactly one deferral (§7.4)',
        'protocol',
        facts['deferral_count_when_deferred'] === 1,
        `deferralCount: ${JSON.stringify(facts['deferral_count_when_deferred'])}`,
      ),
      check(
        'E2E-4',
        'the agent resumes with {"resume": id} rather than opening a second handoff (TOOL-08)',
        'model',
        callsTo(run, 'handoff_to_user').some((use) => use.input['resume'] === id) &&
          callsTo(run, 'handoff_to_user').filter((use) => 'spec' in use.input).length === 1,
        `calls: ${JSON.stringify(callsTo(run, 'handoff_to_user').map((use) => Object.keys(use.input)))}`,
      ),
      check(
        'E2E-4',
        'the resume takes the tab back to active with a call attached (§8.1)',
        'protocol',
        afterResume?.state === 'active' && afterResume.callAttached,
        `state: ${String(afterResume?.state)}, callAttached: ${String(afterResume?.callAttached)}`,
      ),
      check(
        'E2E-4',
        'the last step then moves it to awaiting_verification (RESP-09)',
        'protocol',
        stateAfterDone === 'awaiting_verification' ||
          statuses(run).includes('awaiting_verification'),
        `state after done: ${String(stateAfterDone)}; statuses: ${JSON.stringify(statuses(run))}`,
      ),
      check(
        'E2E-4',
        'the log kept one handoff through the whole thing (DD-13)',
        'protocol',
        handoffsInTheLog === 1 && row !== undefined,
        `handoffs in the log: ${String(handoffsInTheLog)}`,
      ),
      // The server picks the instruction's variant from the row's `stop_hook` (§4.7.4): the
      // no-hook one for Codex, which tells the agent that nothing will remind it (FM-03), and
      // the Stop-hook one for Claude Code, whose row promises the hook.
      check(
        'E2E-4',
        agent.id === 'codex'
          ? 'the deferred instruction is the no-hook one: nothing will remind the agent (FM-03)'
          : "the deferred instruction is the Stop-hook one the agent's row promises (§4.7.4)",
        'protocol',
        deferredOutcome === undefined ||
          (agent.id === 'codex') === deferredInstruction.includes(NO_HOOK_PHRASE),
        `instruction: ${JSON.stringify(deferredInstruction)}`,
      ),
      check(
        'E2E-4',
        'the banner the window draws while it is deferred promises no hook (ADPT-04)',
        'protocol',
        bannerText !== undefined && !bannerText.toLowerCase().includes('hook'),
        `${String(bannerKey)}: ${JSON.stringify(bannerText)}`,
      ),
    ] satisfies Assertion[];
  },
};

/** The first outcome of a given status the agent was handed. */
export function outcomeOf(
  run: { readonly toolResults: readonly { readonly outcome: Record<string, unknown> | undefined }[] },
  status: string,
): Record<string, unknown> | undefined {
  return run.toolResults
    .map((result) => result.outcome)
    .find((outcome) => outcome?.['status'] === status);
}
