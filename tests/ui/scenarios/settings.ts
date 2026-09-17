/**
 * The settings pages (§7.6, APP-02, LOG-04).
 *
 * The settings window is opened the way the tray opens it, and each of its six pages is
 * reached by its own button. Three things are checked beyond "the page is there", because each
 * is a promise a component test cannot keep with a fake bridge:
 *
 * - the **Log** page lists the handoff this scenario has just closed, read from the real
 *   database (LOG-04);
 * - the **language** changes every label on screen at once, with no restart (APP-02);
 * - the panel is **wider** while the settings are open and gives the width back when they
 *   close (§7.6), which is the Rust side resizing the real window.
 */
import { expect, spec, type UiScenario } from '../scenario.ts';
import { until } from '../wait.ts';

/** The six pages, in the order §7.6 names them, with their labels in English. */
const SECTIONS = [
  ['General', 'general'],
  ['Agents', 'agents'],
  ['Network', 'network'],
  ['Log', 'log'],
  ['Runbooks', 'runbooks'],
  ['Updates', 'updates'],
] as const;

const GOAL = 'Confirm the invoice was sent';

export const settings: UiScenario = {
  id: 'settings',
  covers: 'APP-02, LOG-04, WIN-02',
  title: 'the six settings pages, a closed handoff in the log, the language, the wider panel',
  async run({ page, session, facts }) {
    // Something for the Log page to list: a handoff taken to its end.
    const server = await session();
    await server.open(spec(GOAL, ['Confirm the invoice was sent.']));
    await page.findText('.step .counter', 'Step 1 of 1');
    await page.click('.action-primary', 'Done');
    await server.event('confirmed_by_user');
    const narrow = (await page.size()).width;

    await page.showView('settings');
    const wide = await until('the panel widened for the settings', async () => {
      const now = (await page.size()).width;
      return now > narrow ? now : null;
    });
    facts['widths'] = { overlay: narrow, settings: wide };

    const labels = await page.texts('.settings-nav-item');
    expect(
      JSON.stringify(labels) === JSON.stringify(SECTIONS.map(([label]) => label)),
      'the six pages are listed in the order §7.6 names them',
      labels,
    );
    for (const [label, name] of SECTIONS) {
      await page.click('.settings-nav-item', label);
      await page.find(`[data-settings="${name}"]`);
      expect(
        (await page.attributeOf('.settings-nav-item', label, 'aria-current')) === 'page',
        `${label} is marked as the page shown`,
      );
    }

    // LOG-04: the closed handoff, read from the database the store wrote.
    await page.click('.settings-nav-item', 'Log');
    await page.findText('[data-settings="log"] .log-goal', GOAL);

    // APP-02: the language changes at once, and back. The radios are painted as a segmented
    // control, so the pill beside each one is what a person presses; the input is still the
    // control, and it is still what is clicked here.
    await page.click('.settings-nav-item', 'General');
    await page.click('input[name="language"][value="it"] + .seg-pill');
    await page.findText('.settings-nav-item', 'Generale');
    await page.click('input[name="language"][value="en"] + .seg-pill');
    await page.findText('.settings-nav-item', 'General');

    // §7.6: the way back to the panel that does not go through the tray.
    await page.click('.settings-back');
    await page.untilView('overlay');
    await page.showView('settings');
    await page.find('[data-settings="general"]');

    // §7.6: leaving the settings gives the width back.
    await page.showView('overlay');
    await until('the panel narrowed again', async () => (await page.size()).width === narrow);
  },
};
