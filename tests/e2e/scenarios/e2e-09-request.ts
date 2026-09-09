/**
 * E2E-9 — a request the user opens, delivered by the Stop hook, adopted by the handoff
 * (TECHNICAL-DESIGN §11.5, F-07, OPEN-03..08, DD-13).
 *
 * The other direction: the work starts with the person, not with the agent. The user types a
 * sentence into the request sheet — here, through `open_request` on the automation channel —
 * a tab appears at once in "waiting for spec", the sentence goes on the clipboard (OPEN-05),
 * and when the agent's turn ends the Stop hook hands it the same sentence with the id
 * (OPEN-06). The agent then opens a handoff quoting `request_id`, and **the handoff takes
 * that id**: one row from the request to the outcome (DD-13, OPEN-08).
 *
 * The agent is held still with a `Bash` gate while the harness types the request, because
 * the delivery has to happen at the end of a turn and a turn that ended before the request
 * existed would have nothing to deliver.
 */
import { callsTo, startAgent } from '../agent.ts';
import { check, type Assertion } from '../classify.ts';
import { Log } from '../db.ts';
import { handoffOf, plantedSpec, REPORT_LINE, walkTheSteps } from '../specs.ts';
import { serverRegistered, wellFormed, type Scenario } from '../scenario.ts';

/** What the user types into the request sheet. Distinctive enough to trace end to end. */
export const REQUEST_TEXT =
  'Please set up the maintenance banner in the admin console for me.';

export const requestScenario: Scenario = {
  id: 'e2e-09-request',
  covers: 'E2E-9',
  title: 'a user-opened request is delivered by the hook and adopted by the handoff',
  timeoutMs: 420_000,

  async run({ workspace, app, forbidden, facts, say }) {
    const planted = plantedSpec('09');
    forbidden.push(...planted.forbidden);

    const prompt = [
      'You are the coding agent in an automated end-to-end test of a handoff system.',
      'First, run this exact Bash command and wait for it to finish: sleep 20; echo waited.',
      'Then say "ready" and finish your turn.',
      'If something stops you and says the user has asked for something, do exactly what it asks:',
      'call the tool mcp__handoff__handoff_to_user once with this exact argument, copied verbatim,',
      'except that you must replace REQUEST_ID_HERE with the handoff id the message gave you:',
      JSON.stringify({ spec: planted.spec, request_id: 'REQUEST_ID_HERE' }),
      'The call blocks while the user works; wait for it.',
      REPORT_LINE,
    ].join(' ');

    say('starting the agent, with the Stop hook installed and Bash allowed');
    const agent = startAgent(workspace, {
      prompt,
      maxTurns: 16,
      stopHook: true,
      alsoAllow: ['Bash'],
    });

    say('waiting for the session to register');
    const registered = await app.waitFor(
      'a session registered',
      (seen) => seen.sessions.some((session) => session.connected),
      120_000,
    );
    const sessionRef = registered.sessions.find((session) => session.connected)?.sessionRef;

    say('the user types a request into the sheet');
    const requestId = await app.openRequest(REQUEST_TEXT, sessionRef);
    facts['request_id'] = requestId;

    const waiting = await app.waitFor(
      'the request opened a tab waiting for its spec (OPEN-04)',
      (seen) => handoffOf(seen, requestId)?.state === 'awaiting_spec',
      30_000,
    );
    const waitingTab = handoffOf(waiting, requestId);
    facts['request_text_on_the_tab'] = waitingTab?.requestText;

    say('waiting for the hook to deliver it and the agent to send a spec');
    const adopted = await app.waitFor(
      `${requestId} received its spec`,
      (seen) => handoffOf(seen, requestId)?.state === 'active',
      240_000,
    );
    const adoptedTab = handoffOf(adopted, requestId);

    await walkTheSteps(app, requestId, say);
    await app
      .waitFor(`${requestId} is closed`, (seen) => handoffOf(seen, requestId)?.closedAt != null, 120_000)
      .catch(() => undefined);
    const run = await agent;

    const log = new Log(workspace);
    const row = log.handoff(requestId);
    const requests = log.requests();
    const handoffsInTheLog = log.handoffs().length;
    const blocks = log.hookBlocks();
    log.close();
    const queued = requests.find((request) => request.id === requestId);

    const opened = callsTo(run, 'handoff_to_user').filter((use) => 'spec' in use.input);
    return [
      wellFormed(run, 'E2E-9'),
      serverRegistered(run, 'E2E-9'),
      check(
        'E2E-9',
        'the request opens a tab immediately, before any agent has seen it (OPEN-04)',
        'protocol',
        waitingTab?.state === 'awaiting_spec' && waitingTab.requestText === REQUEST_TEXT,
        `tab: ${JSON.stringify({ state: waitingTab?.state, requestText: waitingTab?.requestText })}`,
      ),
      check(
        'E2E-9',
        'the sentence reaches the agent as a clipboard delivery or a hook block (OPEN-05, OPEN-06)',
        'protocol',
        queued?.delivered_via !== null || blocks.length > 0,
        `delivered_via: ${JSON.stringify(queued?.delivered_via)}; hook_blocks: ${JSON.stringify(blocks)}`,
      ),
      check(
        'E2E-9',
        'the agent opens the handoff quoting the request id',
        'model',
        opened.some((use) => use.input['request_id'] === requestId),
        `request_id sent: ${JSON.stringify(opened.map((use) => use.input['request_id']))}`,
      ),
      check(
        'E2E-9',
        'the handoff takes the id of the request rather than minting a new one (DD-13, OPEN-08)',
        'protocol',
        adoptedTab?.tab.id === requestId && handoffsInTheLog === 1,
        `handoffs in the log: ${String(handoffsInTheLog)}; ids: ${JSON.stringify(
          adopted.handoffs.map((one) => one.tab.id),
        )}`,
      ),
      check(
        'E2E-9',
        'the queue entry is closed by the handoff that answered it (§7.7)',
        'protocol',
        queued?.linked_handoff_id === requestId,
        `queue: ${JSON.stringify(queued)}`,
      ),
      check(
        'E2E-9',
        "the user's own words stay on the tab beside the spec (OPEN-04)",
        'protocol',
        adoptedTab?.requestText === REQUEST_TEXT && row?.request_text === REQUEST_TEXT,
        `view: ${JSON.stringify(adoptedTab?.requestText)}; row: ${JSON.stringify(row?.request_text)}`,
      ),
    ] satisfies Assertion[];
  },
};
