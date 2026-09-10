/**
 * The step view (GUIDE-01..03, DET-04, PRIN-07): what a person reads and presses on one step.
 *
 * - The counter says where they are, and there is no progress bar anywhere (PRIN-07).
 * - A step's `https://` address is a link and nothing else in its text is: an `http://` one
 *   and a `file://` one stay words (GUIDE-03, `src/overlay/linkify.ts`). The link is never
 *   clicked here, because a click opens the machine's browser.
 * - **Copy** puts the value on the clipboard, and for a secret-treated value it is the *true*
 *   value that arrives there while the window shows the mask (DET-04).
 * - The last **Done** ends the round, and the agent is told so.
 */
import { randomInt } from 'node:crypto';

import { clipboard, expect, spec, type UiScenario } from '../scenario.ts';
import { until } from '../wait.ts';

const ENDPOINT = 'https://api.example.test/hooks/payments';
const LINK = 'https://dashboard.example.test/webhooks';
const PLAIN_HTTP = 'http://legacy.example.test/webhooks';
const PLAIN_FILE = 'file:///C:/Windows/win.ini';

/** The mask of DET-04 (`ui_bridge::view::MASK`). */
const MASK = '\u2022'.repeat(6);

/**
 * A fake Stripe test key, made fresh for each run.
 *
 * Nothing with the shape of a credential is committed; the e2e suite generates the secrets it
 * plants the same way (`tests/e2e/specs.ts`).
 */
function fakeKey(): string {
  const alphabet = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
  let tail = '';
  for (let index = 0; index < 24; index += 1) tail += alphabet[randomInt(alphabet.length)];
  return `sk_test_${tail}`;
}

export const stepView: UiScenario = {
  id: 'step-view',
  covers: 'GUIDE-01, GUIDE-02, GUIDE-03, DET-04, PRIN-07',
  title: 'the counter, the copy chips (a masked one included) and the one clickable address',
  async run({ page, session, facts }) {
    const key = fakeKey();
    const server = await session();
    facts['handoff'] = await server.open(
      spec(
        'Register the payment webhook',
        [
          {
            text: `Open ${LINK} and press Add endpoint. Do not use ${PLAIN_HTTP} or ${PLAIN_FILE}.`,
            values: ['endpoint_url'],
          },
          { text: 'Paste the API key into the Authentication field.', values: ['api_key'] },
          'Save the endpoint.',
        ],
        { endpoint_url: ENDPOINT, api_key: key },
      ),
      [{ location: 'values.api_key', kind: 'api_key' }],
    );

    // GUIDE-01, PRIN-07: one step at a time, how many remain, and never a percentage.
    await page.findText('.step .counter', 'Step 1 of 3');
    expect((await page.count('progress, [role="progressbar"]')) === 0, 'no progress bar is drawn (PRIN-07)');

    // GUIDE-03: the one https address is the one link.
    const links = await page.attributes('.step-text a', 'href');
    expect(links.length === 1 && links[0] === LINK, 'the https address is the only link of the step text', links);
    const stepText = await page.text('.step-text');
    expect(
      stepText.includes(PLAIN_HTTP) && stepText.includes(PLAIN_FILE),
      'the http and file addresses are shown as words',
      stepText,
    );
    const anchors = await page.attributes('a', 'href');
    expect(
      anchors.every((href) => href === null || href.startsWith('https://')),
      'no anchor anywhere in the window points outside https',
      anchors,
    );

    // GUIDE-02: the value, as written, and Copy.
    await page.findText('[data-value="endpoint_url"] .chip-value', ENDPOINT);
    await page.click('[data-value="endpoint_url"] .chip-action', 'Copy');
    await until('the endpoint address is on the clipboard', () => clipboard() === ENDPOINT);

    await page.click('.actions button', 'Done');
    await page.findText('.step .counter', 'Step 2 of 3');

    // DET-04: the mask on screen, the true value on the clipboard.
    await page.find('[data-value="api_key"] .chip-items.chip-masked');
    await page.findText('[data-value="api_key"] .chip-value', MASK);
    expect(!(await page.bodyText()).includes(key), 'the secret-treated value is not on screen (DET-04)');
    await page.click('[data-value="api_key"] .chip-action', 'Copy');
    await until('the true API key is on the clipboard (DET-04)', () => clipboard() === key);

    await page.click('.actions button', 'Done');
    await page.findText('.step .counter', 'Step 3 of 3');
    await page.click('.actions button', 'Done');

    // RESP-09: the last Done is the end of the round, and it reaches the agent.
    const event = await server.event('confirmed_by_user');
    expect(event.outcome['final'] === true, 'the last Done produced a final outcome', event.outcome);
  },
};
