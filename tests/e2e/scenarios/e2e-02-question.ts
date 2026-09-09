/**
 * E2E-2 — ask → reply → done (TECHNICAL-DESIGN §11.5, F-04, RESP-04, TOOL-04).
 *
 * The user interrupts the guidance with a question on the current step; the blocking call
 * comes back with `status: "question"` and the user's words; the agent answers with the
 * continue shape; the reply appears on the step the user is standing on, and the round the
 * question happened in is still the same round (VER-09).
 */
import { callsTo, startAgent, statuses } from '../agent.ts';
import type { HandoffState } from '../automation.ts';
import { check, type Assertion } from '../classify.ts';
import { Log } from '../db.ts';
import { handoffOf, openPrompt, plantedSpec, REPORT_LINE, walkTheSteps } from '../specs.ts';
import { serverRegistered, wellFormed, type Scenario } from '../scenario.ts';

/** What the user types into the Ask sheet. Distinctive, so the reply can be traced to it. */
export const QUESTION = 'Which announcement channel should this banner go to?';

/** What the agent is told to answer. Asserted verbatim on the step. */
export const ANSWER = 'Use the status page channel.';

export const questionScenario: Scenario = {
  id: 'e2e-02-question',
  covers: 'E2E-2',
  title: 'a question from the overlay comes back to the agent and its reply lands on the step',

  async run({ workspace, app, forbidden, facts, say }) {
    const planted = plantedSpec('02');
    forbidden.push(...planted.forbidden);

    const prompt = [
      openPrompt(planted.spec),
      'When the call returns with status "question", the user asked something on the current step.',
      `Answer it by calling mcp__handoff__handoff_to_user again with {"handoff_id": "<the handoff_id>", "reply": "${ANSWER}"}.`,
      'Send that reply text exactly, with no additions. Then keep waiting: the call blocks again.',
      'Repeat until the status you get back has "final": true.',
      REPORT_LINE,
    ].join(' ');

    say('starting the agent');
    const agent = startAgent(workspace, { prompt, maxTurns: 12 });

    const handoff = await app.theHandoff();
    const id = handoff.tab.id;
    say(`the agent opened ${id}; asking a question on step 1`);

    await app.waitFor(
      `${id} is being guided`,
      (seen) => handoffOf(seen, id)?.state === 'active',
    );
    await app.act(id, 'ask', QUESTION);

    say('waiting for the agent to answer');
    const answered = await app.waitFor(
      'the reply reaches the step',
      (seen) => repliesOf(handoffOf(seen, id)).length > 0,
      180_000,
    );
    const replies = repliesOf(handoffOf(answered, id));
    facts['replies'] = replies;

    await walkTheSteps(app, id, say);
    const closed = await app.waitFor(
      `${id} reached a final state`,
      (seen) => handoffOf(seen, id)?.closedAt != null,
      120_000,
    );
    const run = await agent;
    facts['statuses'] = statuses(run);

    const log = new Log(workspace);
    const row = log.handoff(id);
    const rounds = log.rounds(id);
    log.close();

    const questionOutcome = questionOutcomeOf(run);
    return [
      wellFormed(run, 'E2E-2'),
      serverRegistered(run, 'E2E-2'),
      check(
        'E2E-2',
        'the blocking call comes back with status question (RESP-04, TOOL-04)',
        'protocol',
        questionOutcome !== undefined,
        `statuses: ${JSON.stringify(statuses(run))}`,
      ),
      check(
        'E2E-2',
        'the outcome carries the words the user typed, on the step they were on',
        'protocol',
        questionOutcome?.['user_text'] === QUESTION && stepIndexOf(questionOutcome) === 1,
        `user_text: ${JSON.stringify(questionOutcome?.['user_text'])}, current_step: ${JSON.stringify(
          questionOutcome?.['current_step'],
        )}`,
      ),
      check(
        'E2E-2',
        'the agent answers with the continue shape (handoff_id + reply)',
        'model',
        callsTo(run, 'handoff_to_user').some(
          (use) => typeof use.input['reply'] === 'string' && use.input['handoff_id'] === id,
        ),
        `calls: ${JSON.stringify(callsTo(run, 'handoff_to_user').map((use) => Object.keys(use.input)))}`,
      ),
      check(
        'E2E-2',
        "the agent's reply is visible on the step in the overlay (§7.6)",
        'protocol',
        replies.some((reply) => reply.includes(ANSWER)),
        `replies on the step: ${JSON.stringify(replies)}`,
      ),
      check(
        'E2E-2',
        'the handoff ends confirmed by the user, since the spec asked for no verification (PRIN-08)',
        'protocol',
        row?.final_state === 'confirmed_by_user' &&
          handoffOf(closed, id)?.state === 'confirmed_by_user',
        `row: ${JSON.stringify(row)}`,
      ),
      check(
        'E2E-2',
        'a question does not open a round (VER-09)',
        'protocol',
        rounds.length === 1,
        `rounds: ${JSON.stringify(rounds)}`,
      ),
    ] satisfies Assertion[];
  },
};

/** The replies drawn on the current step. */
function repliesOf(handoff: HandoffState | undefined): string[] {
  const replies = handoff?.step?.replies ?? [];
  return replies.map((reply) => reply.text);
}

/** The 1-based step an outcome was produced on (`current_step.index`). */
function stepIndexOf(outcome: Record<string, unknown>): number | undefined {
  const step = outcome['current_step'];
  if (typeof step !== 'object' || step === null) return undefined;
  const index = (step as Record<string, unknown>)['index'];
  return typeof index === 'number' ? index : undefined;
}

/** The `question` outcome the agent was handed, if any. */
function questionOutcomeOf(run: {
  readonly toolResults: readonly { readonly outcome: Record<string, unknown> | undefined }[];
}): Record<string, unknown> | undefined {
  return run.toolResults
    .map((result) => result.outcome)
    .find((outcome) => outcome?.['status'] === 'question');
}
