/**
 * E2E-1 — open → confirm all → done → verify true (TECHNICAL-DESIGN §11.5).
 *
 * The whole happy path of F-02 with a real agent on one end and the automation channel
 * playing the person on the other: the agent opens a handoff with a `verify`, the user
 * confirms every step, the last one moves the handoff to `awaiting_verification`, and the
 * agent's `handoff_verify` closes it as `verified` (VER-04, VER-05, PRIN-08).
 *
 * §11.5 asks for three things here, and all three are asserted: `verified` in the log, the
 * fields of the outcome the agent received, and the runbook file — which T-044's writer now
 * produces, so the check reads what is in it rather than only that it exists (RUN-01,
 * RUN-02, RUN-04).
 */
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import { callsTo, startAgent, statuses } from '../agent.ts';
import { check, type Assertion } from '../classify.ts';
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
    const runbookText = readRunbook(workspace.home, runbooks[0]);
    const runbook =
      runbookText === undefined
        ? undefined
        : (JSON.parse(runbookText) as Record<string, unknown>);
    const steps = (runbook?.['steps'] ?? []) as { text: string }[];
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
      check(
        'E2E-1',
        'a runbook was written for the verified handoff (RUN-01)',
        'protocol',
        runbooks.length === 1,
        `runbooks/: ${JSON.stringify(runbooks)}`,
      ),
      check(
        'E2E-1',
        'it is a v1 document, trusted as verified, with this one run folded in',
        'protocol',
        runbook?.['runbook_version'] === 1 &&
          runbook['trust'] === 'verified' &&
          runbook['runs'] === 1 &&
          (runbook['origin'] as Record<string, unknown> | undefined)?.['app'] === 'handoff-app',
        `runbook: ${JSON.stringify({
          runbook_version: runbook?.['runbook_version'],
          trust: runbook?.['trust'],
          runs: runbook?.['runs'],
          origin: runbook?.['origin'],
        })}`,
      ),
      check(
        'E2E-1',
        'it carries the two steps the user confirmed, the secret in them masked (RUN-02)',
        'protocol',
        steps.length === 2 &&
          steps[1]?.text.includes('[treated as secret: api_key]') === true,
        `steps: ${JSON.stringify(steps.map((step) => step.text))}`,
      ),
      check(
        'E2E-1',
        'it holds value names and no value at all, planted secrets included (RUN-04)',
        'protocol',
        runbookText !== undefined &&
          !planted.forbidden.some((secret) => runbookText.includes(secret)) &&
          !runbookText.includes(bannerOf(planted)) &&
          secretDescription(runbook) === '[treated as secret at ingress]',
        `values: ${JSON.stringify(runbook?.['values'])}`,
      ),
    ] satisfies Assertion[];
  },
};

/** The one runbook file of the run, as text, when the writer produced one. */
function readRunbook(home: string, name: string | undefined): string | undefined {
  if (name === undefined) return undefined;
  try {
    return readFileSync(join(home, 'runbooks', name), 'utf8');
  } catch {
    return undefined;
  }
}

/**
 * The banner value the spec planted, which no runbook may hold (RUN-04).
 *
 * It is an ordinary value and not a secret, so the log keeps it (§7.11) and the
 * log-invariant check of §11.2 deliberately does not look for it. A runbook is the other way
 * round: it holds names and never values, whether or not the detector matched them.
 */
function bannerOf(planted: { readonly spec: Record<string, unknown> }): string {
  const values = planted.spec['values'] as Record<string, string>;
  return values['banner_text'];
}

/** What the runbook says about the value the detector matched at ingress (§4.5.2). */
function secretDescription(runbook: Record<string, unknown> | undefined): unknown {
  const values = runbook?.['values'] as Record<string, { description?: unknown }> | undefined;
  return values?.['api_key']?.description;
}

/** The runbook folder of the run. */
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
