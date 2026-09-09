/**
 * E2E-6 — verify false → replacement steps → verify true
 * (TECHNICAL-DESIGN §11.5, F-08, VER-08..10, RUN-09, VER-09).
 *
 * The correction round: the agent's own verification fails, it sends `replacement_steps`
 * that start from the actual error, the user walks the new steps, and the second report
 * closes the handoff as `verified`. What this proves that E2E-1 does not is that a handoff
 * survives a failure — one id, two rounds, the first round kept as history.
 *
 * §11.5 also asks for the "runbook update proposal state". That is written and reported
 * `pending`: the runbook writer and its update proposals are T-044, and `lib.rs` passes
 * `NoRunbookSink` until then.
 */
import { readdirSync } from 'node:fs';
import { join } from 'node:path';

import { callsTo, startAgent, statuses } from '../agent.ts';
import { check, pending, type Assertion } from '../classify.ts';
import { Log } from '../db.ts';
import { handoffOf, openPrompt, plantedSpec, REPORT_LINE, VERIFY_TEXT, walkTheSteps } from '../specs.ts';
import { serverRegistered, wellFormed, type Scenario } from '../scenario.ts';

/** What the agent reports the first time: a failure it can describe and correct. */
export const FAILURE_DETAIL =
  'The announcements table shows the banner as a draft, not as published.';

/** The step it is told to send as the correction. Recognisable in the overlay. */
export const REPLACEMENT_STEP = 'Open the draft banner and press Publish.';

export const correctionScenario: Scenario = {
  id: 'e2e-06-correction',
  covers: 'E2E-6',
  title: 'a failed verification opens a correction round that ends verified',
  timeoutMs: 480_000,

  async run({ workspace, app, forbidden, facts, say }) {
    const planted = plantedSpec('06', { verify: VERIFY_TEXT });
    forbidden.push(...planted.forbidden);

    const prompt = [
      openPrompt(planted.spec),
      'When the status is "awaiting_verification" for the FIRST time, report a failure:',
      'call mcp__handoff__handoff_verify once with',
      `{"handoff_id": "<the handoff_id>", "verify": {"ok": false, "detail": "${FAILURE_DETAIL}"}}.`,
      'Then correct the handoff: call mcp__handoff__handoff_to_user with',
      `{"handoff_id": "<the handoff_id>", "reply": "${FAILURE_DETAIL}", "replacement_steps": [{"text": "${REPLACEMENT_STEP}"}]}`,
      'and wait: the user walks the new step.',
      'When the status is "awaiting_verification" for the SECOND time, report a success:',
      `call mcp__handoff__handoff_verify with {"handoff_id": "<the handoff_id>", "verify": {"ok": true, "detail": "${VERIFY_TEXT}"}}.`,
      REPORT_LINE,
    ].join(' ');

    say('starting the agent');
    const agent = startAgent(workspace, { prompt, maxTurns: 18 });

    const handoff = await app.theHandoff();
    const id = handoff.tab.id;
    say(`the agent opened ${id}; walking round 1`);
    await walkTheSteps(app, id, say);

    say('waiting for the failed verification and the replacement steps');
    const corrected = await app.waitFor(
      `${id} is in a correction round`,
      (seen) => {
        const one = handoffOf(seen, id);
        return one !== undefined && one.round >= 2 && one.state === 'active';
      },
      240_000,
    );
    const inRoundTwo = handoffOf(corrected, id);
    facts['round_after_correction'] = inRoundTwo?.round;
    facts['step_text_after_correction'] = inRoundTwo?.step?.text;

    say('walking round 2');
    await walkTheSteps(app, id, say);
    const closed = await app
      .waitFor(`${id} is closed`, (seen) => handoffOf(seen, id)?.closedAt != null, 180_000)
      .catch(() => undefined);
    const run = await agent;
    facts['statuses'] = statuses(run);

    const log = new Log(workspace);
    const row = log.handoff(id);
    const rounds = log.rounds(id);
    const runbooks = readRunbooks(workspace.home);
    log.close();

    const reports = callsTo(run, 'handoff_verify');
    return [
      wellFormed(run, 'E2E-6'),
      serverRegistered(run, 'E2E-6'),
      check(
        'E2E-6',
        'the agent reports the verification twice: once false, once true (VER-08)',
        'model',
        reports.length === 2 && okOf(reports[0]) === false && okOf(reports[1]) === true,
        `verify reports: ${JSON.stringify(reports.map((use) => use.input['verify']))}`,
      ),
      check(
        'E2E-6',
        'the failed report is handed back as status failed (VER-08)',
        'protocol',
        statuses(run).includes('failed'),
        `statuses: ${JSON.stringify(statuses(run))}`,
      ),
      check(
        'E2E-6',
        'the replacement steps open a second round on the same handoff (VER-09)',
        'protocol',
        rounds.length === 2 && inRoundTwo?.round === 2,
        `rounds: ${JSON.stringify(rounds)}; view round: ${String(inRoundTwo?.round)}`,
      ),
      check(
        'E2E-6',
        'the user is shown the replacement step and not the original ones',
        'protocol',
        inRoundTwo?.step?.text === REPLACEMENT_STEP,
        `step text: ${JSON.stringify(inRoundTwo?.step?.text)}`,
      ),
      check(
        'E2E-6',
        'the first round is kept as history (VER-09)',
        'protocol',
        (inRoundTwo?.history.length ?? 0) >= 1,
        `history entries: ${String(inRoundTwo?.history.length ?? 0)}`,
      ),
      check(
        'E2E-6',
        'the second report closes the handoff as verified',
        'protocol',
        row?.final_state === 'verified' && handoffOf(closed ?? { handoffs: [] }, id)?.state === 'verified',
        `row: ${JSON.stringify(row)}`,
      ),
      pending(
        'E2E-6',
        'the runbook of the corrected handoff carries an update proposal (RUN-09)',
        'T-044',
        runbooks.length === 1,
        `runbooks/: ${JSON.stringify(runbooks)} — the writer and its proposals are T-044`,
      ),
    ] satisfies Assertion[];
  },
};

/** The `ok` of a `handoff_verify` call. */
function okOf(use: { readonly input: Record<string, unknown> } | undefined): unknown {
  const verify = use?.input['verify'];
  if (typeof verify !== 'object' || verify === null) return undefined;
  return (verify as Record<string, unknown>)['ok'];
}

function readRunbooks(home: string): string[] {
  try {
    return readdirSync(join(home, 'runbooks')).filter((name) => name.endsWith('.json'));
  } catch {
    return [];
  }
}
