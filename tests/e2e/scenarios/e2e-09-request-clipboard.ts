/**
 * E2E-9 for an agent with no end-of-turn hook: a request the user opens reaches Codex through
 * the clipboard, and the handoff adopts it (T-067; TECHNICAL-DESIGN §11.5, F-07, OPEN-03..08,
 * OPEN-04a, DD-13, ADPT-04).
 *
 * `e2e-09-request` proves the Claude Code shape of this flow, where the Stop hook hands the
 * sentence to the agent at the end of its turn (OPEN-06). Codex runs no hook (T-066), so the
 * clipboard the request sheet fills (OPEN-05) is the whole of the delivery, and the person who
 * pastes it is the whole of the transport. The scenario plays exactly that person:
 *
 * 1. the user types a request while no session is running: OPEN-04a queues it, the tab appears
 *    at once in "waiting for spec", and the sentence goes on the clipboard;
 * 2. the harness reads the clipboard back and starts Codex with it as its first message, which
 *    is what Ctrl+V into a new Codex session does;
 * 3. the handoff Codex opens takes the request's id (DD-13, OPEN-08), the request is the first
 *    registering session's (OPEN-04a), and no hook row exists, because no hook ran.
 *
 * Reading the clipboard is the point rather than a shortcut: the sentence the agent gets is the
 * one the app rendered, its id and its language included, and not one the harness composed to
 * look like it.
 */
import { execFileSync } from 'node:child_process';

import { callsTo } from '../agent.ts';
import { check, type Assertion } from '../classify.ts';
import { Log } from '../db.ts';
import { handoffOf, plantedSpec, REPORT_LINE, walkTheSteps } from '../specs.ts';
import { agentRegistered, tabNamesTheAgent, wellFormed, type Scenario } from '../scenario.ts';
import { REQUEST_TEXT } from './e2e-09-request.ts';

/** What a person pastes: the text on the system clipboard, as the operating system holds it. */
export function readClipboard(): string {
  if (process.platform === 'win32') {
    return execFileSync(
      'powershell.exe',
      [
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        '[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Get-Clipboard -Raw',
      ],
      { encoding: 'utf8', windowsHide: true, timeout: 30_000 },
    ).replace(/\r?\n$/u, '');
  }
  if (process.platform === 'darwin') {
    return execFileSync('pbpaste', [], { encoding: 'utf8', timeout: 30_000 });
  }
  throw new Error(`there is no clipboard reader for ${process.platform}`);
}

export const requestByClipboardScenario: Scenario = {
  id: 'e2e-09-request-clipboard',
  covers: 'E2E-9',
  title: 'a request reaches an agent with no hook through the clipboard and the handoff adopts it',
  timeoutMs: 420_000,

  async run({ workspace, app, agent, forbidden, facts, say }) {
    const planted = plantedSpec('09');
    forbidden.push(...planted.forbidden);

    say('the user types a request into the sheet, with no session running yet');
    const requestId = await app.openRequest(REQUEST_TEXT);
    facts['request_id'] = requestId;

    const waiting = await app.waitFor(
      'the request opened a tab waiting for its spec (OPEN-04)',
      (seen) => handoffOf(seen, requestId)?.state === 'awaiting_spec',
      30_000,
    );
    const waitingTab = handoffOf(waiting, requestId);

    // `open_request` answers after the sheet's clipboard write, so the sentence is there now.
    const pasted = readClipboard();
    facts['pasted_carries_the_request_id'] = pasted.includes(requestId);

    const prompt = [
      pasted,
      '',
      'You are the coding agent in an automated end-to-end test of a handoff system, and the',
      'line above is what the user pasted into this session.',
      `Answer it by calling ${agent.tool('handoff_to_user')} once with this exact argument,`,
      'copied verbatim, except that you must replace REQUEST_ID_HERE with the handoff id the',
      'pasted line gave you:',
      JSON.stringify({ spec: planted.spec, request_id: 'REQUEST_ID_HERE' }),
      'The call blocks while the user works; wait for it.',
      REPORT_LINE,
    ].join('\n');

    say(`pasting it into a new ${agent.displayName} session`);
    const running = agent.start(workspace, { prompt, maxTurns: 12 });

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
    const run = await running;
    const settled = await app.state();

    const log = new Log(workspace);
    const row = log.handoff(requestId);
    const requests = log.requests();
    const handoffsInTheLog = log.handoffs().length;
    const blocks = log.hookBlocks();
    log.close();
    // The row, not `state().requests`: that view is the *open* queue, and a request the handoff
    // adopted has left it. The row keeps the session OPEN-04a gave it.
    const queued = requests.find((request) => request.id === requestId);
    const session = settled.sessions.find((one) => one.agentId === agent.id);

    const opened = callsTo(run, 'handoff_to_user').filter((use) => 'spec' in use.input);
    return [
      wellFormed(run, 'E2E-9'),
      await agentRegistered(run, app, 'E2E-9'),
      tabNamesTheAgent(adoptedTab?.tab.label ?? '', agent, 'E2E-9'),
      check(
        'E2E-9',
        'the request opens a tab immediately, before any agent has seen it (OPEN-04)',
        'protocol',
        waitingTab?.state === 'awaiting_spec' && waitingTab.requestText === REQUEST_TEXT,
        `tab: ${JSON.stringify({ state: waitingTab?.state, requestText: waitingTab?.requestText })}`,
      ),
      check(
        'E2E-9',
        'what the sheet put on the clipboard carries the request id and the words (OPEN-05)',
        'protocol',
        pasted.includes(requestId) && pasted.includes(REQUEST_TEXT),
        `clipboard: ${JSON.stringify(pasted)}`,
      ),
      check(
        'E2E-9',
        'the queue records the clipboard as the way the request was delivered (OPEN-05)',
        'protocol',
        queued?.delivered_via === 'clipboard',
        `delivered_via: ${JSON.stringify(queued?.delivered_via)}`,
      ),
      check(
        'E2E-9',
        'no hook ran for an agent that has none, so none wrote a block (ADPT-04)',
        'protocol',
        blocks.length === 0,
        `hook_blocks: ${JSON.stringify(blocks)}`,
      ),
      check(
        'E2E-9',
        'the request went to the first session that registered (OPEN-04a)',
        'protocol',
        session !== undefined && queued?.session_ref === session.sessionRef,
        `request session: ${JSON.stringify(queued?.session_ref)}; ${agent.id} session: ${JSON.stringify(
          session?.sessionRef,
        )}`,
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
