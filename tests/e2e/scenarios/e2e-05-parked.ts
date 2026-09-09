/**
 * E2E-5 — defer twice → the agent stops → the Stop hook blocks it once
 * (TECHNICAL-DESIGN §11.5, F-05, F-10, SRV-10..13, RESP-06, RESP-07).
 *
 * This is the safety net, end to end and with nothing simulated: a real `Stop` hook entry in
 * a real `.claude/settings.json`, the real `handoff-mcp hook stop` subprocess, the real
 * ancestor-chain binding (DD-22, §7.5), and a real `hook_blocks` row. Two deferrals park the
 * handoff; the agent is about to finish its turn leaving it parked; the hook stops it once
 * and names it.
 *
 * **The §11.5 row ends "agent resumes" and that is not what a parked handoff asks for.** The
 * hook's own sentence for this item is *"was deferred twice and waits for the user in the
 * overlay: mention it in your final summary and **do not resume it**"* — §8.1 gives `parked`
 * to the user, RESP-07 tells the agent to cite it, and `hook::decide` says so in as many
 * words. An agent that resumed here would be doing the one thing three normative places
 * forbid. The scenario therefore asserts what the design asks for: the block happens, once,
 * and the agent carries the id into its answer. Measured first: the model obeyed the
 * instruction and did not resume, and the run was red against the table's wording.
 * `DEVIATIONS.md` records it and the T-043 task text is corrected.
 *
 * "Exactly one `hook_blocks` row" is the assertion that matters most: SRV-12 is *at most once
 * per handoff per session*, and the primary key of the table is what enforces it. A second
 * row would mean the agent can be stopped for ever on the same handoff.
 */
import { startAgent, statuses } from '../agent.ts';
import { check, type Assertion } from '../classify.ts';
import { Log } from '../db.ts';
import { handoffOf, openPrompt, plantedSpec, REPORT_LINE } from '../specs.ts';
import { serverRegistered, wellFormed, type Scenario } from '../scenario.ts';
import { outcomeOf } from './e2e-04-defer.ts';

export const parkedScenario: Scenario = {
  id: 'e2e-05-parked',
  covers: 'E2E-5',
  title: 'a handoff parked by two deferrals stops the agent once through the Stop hook',
  timeoutMs: 420_000,

  async run({ workspace, app, forbidden, facts, say }) {
    const planted = plantedSpec('05');
    forbidden.push(...planted.forbidden);

    const prompt = [
      openPrompt(planted.spec),
      'If the status is "deferred", call mcp__handoff__handoff_to_user again with',
      '{"resume": "<the handoff_id>"} and wait again.',
      'If the status is "parked", the user will pick it up themselves: do not resume it.',
      'If something then stops you and tells you to mention it, do exactly that.',
      REPORT_LINE,
    ].join(' ');

    say('starting the agent, with the Stop hook installed');
    const agent = startAgent(workspace, { prompt, maxTurns: 16, stopHook: true });

    const handoff = await app.theHandoff();
    const id = handoff.tab.id;

    say(`the agent opened ${id}; deferring it the first time`);
    await app.waitFor(`${id} is being guided`, (seen) => handoffOf(seen, id)?.state === 'active');
    await app.act(id, 'defer', 'Not now, I will come back to it.');
    await app.waitFor(`${id} is deferred`, (seen) => handoffOf(seen, id)?.state === 'deferred', 60_000);

    say('waiting for the agent to resume, then deferring it a second time');
    await app.waitFor(
      `${id} is active again`,
      (seen) => {
        const one = handoffOf(seen, id);
        return one?.state === 'active' && one.callAttached;
      },
      180_000,
    );
    await app.act(id, 'defer', 'Still not now.');
    const parked = await app.waitFor(
      `${id} is parked`,
      (seen) => handoffOf(seen, id)?.state === 'parked',
      60_000,
    );
    const parkedView = handoffOf(parked, id);
    facts['deferral_count_when_parked'] = parkedView?.deferralCount;
    facts['banner_when_parked'] = parkedView?.banner?.key;

    say('waiting for the Stop hook to stop the agent at the end of its turn');
    const run = await agent;
    facts['statuses'] = statuses(run);

    const log = new Log(workspace);
    const blocks = log.hookBlocks();
    const sessions = log.sessions();
    log.close();

    const parkedOutcome = outcomeOf(run, 'parked');
    const forThisHandoff = blocks.filter((block) => block.item_key.includes(id));
    const blockedText = blockReasonIn(run, id);

    return [
      wellFormed(run, 'E2E-5'),
      serverRegistered(run, 'E2E-5'),
      check(
        'E2E-5',
        'a second deferral parks the handoff (RESP-06, §8.1)',
        'protocol',
        parkedView?.state === 'parked' && parkedView.deferralCount === 2,
        `state: ${String(parkedView?.state)}, deferralCount: ${String(parkedView?.deferralCount)}`,
      ),
      check(
        'E2E-5',
        'the outcome tells the agent it is parked and not to resume it (RESP-07)',
        'protocol',
        parkedOutcome !== undefined &&
          typeof parkedOutcome['instruction'] === 'string' &&
          parkedOutcome['instruction'].includes('deferred this step twice'),
        `statuses: ${JSON.stringify(statuses(run))}`,
      ),
      check(
        'E2E-5',
        'the Stop hook bound the session and wrote exactly one block for this handoff (SRV-12)',
        'protocol',
        forThisHandoff.length === 1,
        `hook_blocks: ${JSON.stringify(blocks)}; sessions: ${JSON.stringify(sessions.map((s) => s.session_ref))}`,
      ),
      check(
        'E2E-5',
        'the transcript shows the agent was stopped and told which handoff (SRV-11)',
        'protocol',
        blockedText !== undefined,
        blockedText ?? `no message naming ${id} after the block; final: ${run.finalText.slice(0, 200)}`,
      ),
      check(
        'E2E-5',
        'the agent does exactly what the block asks: it names the handoff and does not resume it',
        'model',
        run.finalText.includes(id) && !resumedAfterParking(run, id),
        `final answer: ${run.finalText.slice(0, 300)}`,
      ),
      check(
        'E2E-5',
        'the handoff is still parked and waiting for the user (RESP-07, §8.1)',
        'protocol',
        (await app.state()).handoffs.find((one) => one.tab.id === id)?.state === 'parked',
        `state: ${JSON.stringify(handoffOf(await app.state(), id)?.state)}`,
      ),
    ] satisfies Assertion[];
  },
};

/** Whether the agent called `resume` on the handoff after it was parked (it must not). */
function resumedAfterParking(
  run: { readonly toolUses: readonly { readonly name: string; readonly input: Record<string, unknown> }[] },
  handoffId: string,
): boolean {
  const resumes = run.toolUses.filter((use) => use.input['resume'] === handoffId);
  // One resume is the first deferral's, which is correct. A second would be the parked one.
  return resumes.length > 1;
}

/**
 * The block reason, as the run shows it.
 *
 * A hook that blocks feeds its `reason` back into the agent's context, so it turns up as a
 * user-role message in the transcript. The handoff id is what makes it ours rather than any
 * other text mentioning a handoff.
 */
function blockReasonIn(
  run: { readonly transcript: readonly { type: string; message?: { content?: readonly Record<string, unknown>[] } }[] },
  handoffId: string,
): string | undefined {
  for (const message of run.transcript) {
    if (message.type !== 'user') continue;
    for (const block of message.message?.content ?? []) {
      const text = typeof block['text'] === 'string' ? block['text'] : '';
      if (text.includes(handoffId) && /resume|deferred|parked|pick it up/iu.test(text)) {
        return text.slice(0, 300);
      }
    }
  }
  return undefined;
}
