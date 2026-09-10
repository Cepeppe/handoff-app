/**
 * The request sheet (OPEN-04, OPEN-04a).
 *
 * "A session selector on top (pre-selected when only one session is active) and a field
 * 'What are you about to do?'. Enter sends, Esc cancels. The tab appears immediately in state
 * 'waiting for spec'." And, with no session at all, "the request window still opens and shows
 * the notice 'no active session'".
 *
 * The sheet is opened the way the tray's `New request` and the global shortcut open it: by
 * the event they both end in (`scenario.ts`). What is asserted afterwards is the sheet: where
 * the caret is without a click, what each key does, and what reached the queue — read back
 * through the automation channel, because "Esc queued nothing" is a fact about the core and
 * not about the screen.
 */
import { expect, type UiScenario } from '../scenario.ts';
import { until } from '../wait.ts';
import { KEY } from '../webdriver.ts';

const REQUEST = 'Rotate the webhook signing secret';

export const requestSheet: UiScenario = {
  id: 'request-sheet',
  covers: 'OPEN-04, OPEN-04a',
  title: 'the request sheet: Esc cancels, Enter sends, the only session is pre-selected',
  async run({ page, app, session }) {
    // OPEN-04a: no session yet. The sheet opens and says so; Esc leaves nothing behind.
    await page.showView('request');
    await page.find('.request-no-session');
    await until('the caret is in the field without a click', async () => (await page.activeId()) === 'request-what');
    await page.type('#request-what', 'Something I changed my mind about');
    await page.press('#request-what', KEY.escape);
    await page.untilView('overlay');
    expect((await app.automation.state()).requests.length === 0, 'Esc queued nothing');

    // OPEN-04: one session, and it is the one selected.
    const server = await session();
    await page.showView('request');
    await page.gone('.request-no-session');
    const offered = await page.values('#request-session option');
    expect(
      offered.length === 1 && offered[0] === server.sessionRef,
      'the selector offers the one registered session',
      offered,
    );
    expect(
      (await page.property<string>('#request-session', 'value')) === server.sessionRef,
      'the only session is pre-selected',
    );

    // Enter on an empty field sends nothing: the sheet stays, and so does the queue.
    await page.press('#request-what', KEY.enter);
    expect((await page.view()) === 'request', 'Enter on an empty field keeps the sheet open');
    expect((await app.automation.state()).requests.length === 0, 'Enter on an empty field queued nothing');

    // Enter sends: the tab is there at once, waiting for its spec.
    await page.type('#request-what', REQUEST);
    await page.press('#request-what', KEY.enter);
    await page.untilView('overlay');
    await page.findText('[data-ui-state="waitingForSpec"] .request-text', REQUEST);
    const queued = await until(
      'the request is in the queue',
      async () => (await app.automation.state()).requests.find((request) => request.text === REQUEST) ?? null,
    );
    expect(queued.sessionRef === server.sessionRef, 'the request is addressed to the chosen session', queued);

    // And Esc again, with a session: still only the one request.
    await page.showView('request');
    await page.type('#request-what', 'Something else I changed my mind about');
    await page.press('#request-what', KEY.escape);
    await page.untilView('overlay');
    expect((await app.automation.state()).requests.length === 1, 'Esc with a session queued nothing either');
  },
};
