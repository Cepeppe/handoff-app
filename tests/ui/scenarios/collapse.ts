/**
 * Collapse and expand (WIN-03).
 *
 * "When the user clicks elsewhere, the panel collapses to a bar showing the current step and
 * three buttons: Done, Ask, Screenshot. It re-expands on click. The other actions (Note,
 * Skip, Defer, Abandon) are available only in the expanded panel."
 *
 * Two things the redesign added to that sentence and this scenario now checks: the bar also
 * carries **Minimize to tray** and **Open the panel** (the window has no system buttons of its
 * own), and three of the four expanded-panel actions are one press deeper than Note, in the
 * **More** menu (RESP-08).
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

/**
 * What a button on the bar is called.
 *
 * Ask and Screenshot are icon-only there (360 × 56 has room for one label), so the name is in
 * the `aria-label`, which is what a screen reader and a hover both read.
 */
const NAME_OF = 'return [...document.querySelectorAll(arguments[0])].map((element) => (element.getAttribute("aria-label") || element.textContent).trim());';

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
    await page.find('.collapsed-step', STEPS[0] ?? '');
    // The counter is upper-cased by the stylesheet, so the text itself is the ordinary one.
    await page.findText('.collapsed-counter', 'Step 1 of 3');
    const bar = await page.execute<string[]>(NAME_OF, ['.collapsed-actions button']);
    expect(
      JSON.stringify(bar) === JSON.stringify(['Done', 'Ask', 'Screenshot']),
      'the bar carries Done, Ask and Screenshot, in that order',
      bar,
    );
    // WIN-04 and WIN-03 from the bar itself: put the window away, and open the panel.
    const windowControls = await page.execute<string[]>(NAME_OF, ['.collapsed-window button']);
    expect(
      JSON.stringify(windowControls) === JSON.stringify(['Minimize to tray', 'Open the panel']),
      'the bar carries Minimize to tray and Open the panel',
      windowControls,
    );
    const everything = await page.execute<string[]>(NAME_OF, ['.collapsed button']);
    for (const action of [...EXPANDED_ONLY, 'More']) {
      expect(!everything.includes(action), `${action} is not on the bar`, everything);
    }
    const shrunk = await until('the window shrank to the bar', async () => {
      const now = await page.size();
      return now.height < expanded.height ? now : null;
    });
    facts['heights'] = { expanded: expanded.height, collapsed: shrunk.height };

    // Done works from the bar, and the bar stays a bar.
    await page.click('.collapsed-actions button', 'Done');
    await page.find('.collapsed-step', STEPS[1] ?? '');

    // A click on the bar brings the whole panel back.
    await page.click('.collapsed-line');
    await page.findText('.step .counter', 'Step 2 of 3');
    await page.find('.header');

    // Note is back in the row that is always on screen; the other three are one press deeper,
    // in the More menu (RESP-08).
    const tools = await page.texts('.action-tools button');
    expect(tools.includes('Note'), 'Note is back in the expanded panel', tools);
    await page.click('.action-tools button', 'More');
    const menu = await page.texts('.more-menu [role="menuitem"]');
    for (const action of ['Skip', 'Defer', 'Abandon']) {
      expect(menu.includes(action), `${action} is in the More menu of the panel`, menu);
    }
    await page.click('.action-tools button', 'More');
    await until('the window took its height back', async () => (await page.size()).height > shrunk.height);

    // And coming back to the window by the focus alone does the same.
    await page.blur();
    await page.find('.collapsed');
    await page.focus();
    await page.findText('.step .counter', 'Step 2 of 3');
  },
};
