/**
 * E2E-1 — open → confirm all → done → verify true (TECHNICAL-DESIGN §11.5).
 *
 * The whole happy path of F-02 with a real agent on one end and the automation channel
 * playing the person on the other: the agent opens a handoff with a `verify`, the user
 * confirms every step, the last one moves the handoff to `awaiting_verification`, and the
 * agent's `handoff_verify` closes it as `verified` (VER-04, VER-05, PRIN-08).
 *
 * §11.5 asks for three things here. Two are asserted: `verified` in the log, and the fields
 * of the outcome the agent received. The third — "runbook file valid" — is written and
 * reported `pending`: the runbook writer is T-044 and `lib.rs` passes `NoRunbookSink` until
 * it exists, so there is nothing to be valid yet. Deleting the check would lose it; failing
 * on it would make the suite red for work nobody has done.
 */
import { readdirSync } from 'node:fs';
import { join } from 'node:path';

import { callsTo, startAgent, statuses } from '../agent.ts';
import { check, pending, type Assertion } from '../classify.ts';
import { Log } from '../db.ts';
import { openPrompt, plantedSpec, REPORT_LINE, VERIFY_TEXT, walkTheSteps } from '../specs.ts';
import { serverRegistered, wellFormed, type Scenario } from '../scenario.ts';

export const verifiedScenario: Scenario = {
  id: 'e2e-01-verified',
  covers: 'E2E-1',
  title: 'a handoff confirmed step by step and verified by the agent reaches `verified`',

  async run({ workspace, app, forbidden, facts, say }) {
    const planted = plantedSpec('01', { verify: VERIFY_TEXT });
    forbidden.push(...planted.forbidden);

    const prompt = [
      openPrompt(planted.spec),
      'When it returns, read the `status` field of the outcome.',
      'If the status is "awaiting_verification", call mcp__handoff__handoff_verify once with',
      `{"handoff_id": "<the handoff_id from the outcome>", "verify": {"ok": true, "detail": "${VERIFY_TEXT}"}}.`,
      REPORT_LINE,
    ].join(' ');

    say('starting the agent');
    const agent = startAgent(workspace, { prompt, maxTurns: 10 });

    const handoff = await app.theHandoff();
    const id = handoff.tab.id;
    facts['handoff_id_shape'] = /^hf_[0-9a-hjkmnp-tv-z]{10}$/u.test(id);
    say(`the agent opened ${id}`);

    await walkTheSteps(app, id, say);
    say('waiting for the agent to report the verification');
    const closed = await app.waitFor(
      `${id} reached a final state`,
      (seen) => seen.handoffs.some((one) => one.tab.id === id && one.closedAt !== null),
      180_000,
    );
    const finalView = closed.handoffs.find((one) => one.tab.id === id);
    const run = await agent;
    facts['statuses'] = statuses(run);
    facts['duration_ms'] = run.durationMs;

    const log = new Log(workspace);
    const row = log.handoff(id);
    const rounds = log.rounds(id);
    const runbooks = readRunbooks(workspace.home);
    log.close();

    const outcome = lastOutcome(run);
    return [
      wellFormed(run, 'E2E-1'),
      serverRegistered(run, 'E2E-1'),
      check(
        'E2E-1',
        'the agent calls handoff_to_user with the spec it was given',
        'model',
        callsTo(run, 'handoff_to_user').length >= 1,
        `tool calls: ${JSON.stringify(run.toolUses.map((use) => use.name))}`,
      ),
      check(
        'E2E-1',
        'the last step moves the handoff to awaiting_verification (RESP-09, VER-04)',
        'protocol',
        statuses(run).includes('awaiting_verification'),
        `statuses: ${JSON.stringify(statuses(run))}`,
      ),
      check(
        'E2E-1',
        'the agent reports the verification with handoff_verify',
        'model',
        callsTo(run, 'handoff_verify').length === 1,
        `handoff_verify calls: ${String(callsTo(run, 'handoff_verify').length)}`,
      ),
      check(
        'E2E-1',
        'the log records the handoff as verified (VER-05)',
        'protocol',
        row?.state === 'verified' && row.final_state === 'verified',
        `row: ${JSON.stringify(row)}`,
      ),
      check(
        'E2E-1',
        'the view the window would draw agrees with the log',
        'protocol',
        finalView?.state === 'verified',
        `view state: ${String(finalView?.state)} / ui ${String(finalView?.uiState)}`,
      ),
      check(
        'E2E-1',
        'the outcome the agent received is final and carries the handoff id',
        'protocol',
        outcome?.['status'] === 'verified' &&
          outcome['final'] === true &&
          outcome['handoff_id'] === id,
        `outcome: ${JSON.stringify({
          status: outcome?.['status'],
          final: outcome?.['final'],
          handoff_id: outcome?.['handoff_id'],
          app_reachable: outcome?.['app_reachable'],
        })}`,
      ),
      check(
        'E2E-1',
        'the happy path takes one round (VER-09)',
        'protocol',
        rounds.length === 1,
        `rounds: ${JSON.stringify(rounds)}`,
      ),
      pending(
        'E2E-1',
        'a runbook was written for the verified handoff (RUN-01)',
        'T-044',
        runbooks.length === 1,
        `runbooks/: ${JSON.stringify(runbooks)} — the writer is T-044; lib.rs passes NoRunbookSink`,
      ),
    ] satisfies Assertion[];
  },
};

/** The runbook folder of the run, which is empty until T-044 fills it. */
function readRunbooks(home: string): string[] {
  try {
    return readdirSync(join(home, 'runbooks')).filter((name) => name.endsWith('.json'));
  } catch {
    return [];
  }
}

/** The last outcome the agent was handed. */
export function lastOutcome(run: {
  readonly toolResults: readonly { readonly outcome: Record<string, unknown> | undefined }[];
}): Record<string, unknown> | undefined {
  for (let index = run.toolResults.length - 1; index >= 0; index -= 1) {
    const outcome = run.toolResults[index]?.outcome;
    if (outcome !== undefined) return outcome;
  }
  return undefined;
}
