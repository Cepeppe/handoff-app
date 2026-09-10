/**
 * Collapse and expand (WIN-03).
 *
 * "When the user clicks elsewhere, the panel collapses to a bar showing the current step and
 * three buttons: Done, Ask, Screenshot. It re-expands on click. The other actions (Note,
 * Skip, Defer, Abandon) are available only in the expanded panel."
 *
 * Every clause of that sentence is a check below, including the one a component test cannot
 * make: that the *window* shrinks to the bar, which is the Rust side answering the height the
 * frontend measured (WIN-02). The focus change itself is the event Tauri's report becomes
 * (`scenario.ts` says why the suite emits it), and a click on the bar is a real click.
 */
import { expect, spec, type UiScenario } from '../scenario.ts';
import { until } from '../wait.ts';

const STEPS = ['Open the billing page.', 'Download the latest invoice.', 'Forward it to accounting.'];
const EXPANDED_ONLY = ['Note', 'Skip', 'Defer', 'Abandon'];

export const collapse: UiScenario = {
  id: 'collapse',
  covers: 'WIN-03, WIN-02',
  title: 'the panel shrinks to the current step and three buttons, and comes back',
  async run({ page, session, facts }) {
    // This scenario is the one about the collapse, so the page must not undo it.
    page.keepPanelOpen = false;
    const server = await session();
    await server.open(spec('Send the invoice to accounting', STEPS));
    await page.findText('.step .counter', 'Step 1 of 3');
    const expanded = await page.size();

    // The user clicks elsewhere.
    await page.blur();
    await page.find('.collapsed');
    await page.gone('.header');
    await page.findText('.collapsed-line', STEPS[0] ?? '');
    const bar = await page.texts('.collapsed-actions button');
    expect(
      JSON.stringify(bar) === JSON.stringify(['Done', 'Ask', 'Screenshot']),
      'the bar carries Done, Ask and Screenshot, in that order',
      bar,
    );
    const everything = await page.texts('.collapsed button');
    for (const action of EXPANDED_ONLY) {
      expect(!everything.includes(action), `${action} is not on the bar`, everything);
    }
    const shrunk = await until('the window shrank to the bar', async () => {
      const now = await page.size();
      return now.height < expanded.height ? now : null;
    });
    facts['heights'] = { expanded: expanded.height, collapsed: shrunk.height };

    // Done works from the bar, and the bar stays a bar.
    await page.click('.collapsed-actions button', 'Done');
    await page.findText('.collapsed-line', STEPS[1] ?? '');

    // A click on the bar brings the whole panel back.
    await page.click('.collapsed-line');
    await page.findText('.step .counter', 'Step 2 of 3');
    await page.find('.header');
    const actions = await page.texts('.actions button');
    for (const action of EXPANDED_ONLY) {
      expect(actions.includes(action), `${action} is back in the expanded panel`, actions);
    }
    await until('the window took its height back', async () => (await page.size()).height > shrunk.height);

    // And coming back to the window by the focus alone does the same.
    await page.blur();
    await page.find('.collapsed');
    await page.focus();
    await page.findText('.step .counter', 'Step 2 of 3');
  },
};
