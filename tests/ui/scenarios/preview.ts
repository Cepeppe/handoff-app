/**
 * The mandatory preview (PREV-01..04, CAP-01, FM-05).
 *
 * A capture is taken the way a person takes one — **Screenshot**, then **Full screen** from
 * the two choices of CAP-01 — with the fixture backend of the e2e build standing in for the
 * screen (`capture::fake`, `e2e.capture_fixture`): the page is one of the synthetic corpus of
 * §11.7, so no real screen is ever captured or kept. Then the preview is used with the mouse:
 *
 * - both send buttons are there, side by side, and neither is the default (PREV-04);
 * - a flagged box is lifted with one click and put back with another (PREV-02, DET-01);
 * - a box drawn by hand appears, and is a locked box: a `<span>` with no role, because a
 *   certain or hand-made redaction offers the user no control (the T-049 note under T-055);
 * - a drag shorter than four pixels is a click and adds nothing;
 * - a crop is drawn and undone;
 * - what leaves is what the preview sent, and it reaches the agent (PREV-01).
 *
 * The boxes are positioned in percentages of the drawn image, so a drag is given in fractions
 * of the canvas and never in the capture's own pixels.
 *
 * The second scenario is the same capture for a session whose capability row says its agent
 * cannot read an image: **Send image** is not drawn at all, and the text is what leaves
 * (PREV-04, FM-05).
 */
import { screenshotFixture } from '../../e2e/paths.ts';
import { expect, spec, type Page, type UiScenario } from '../scenario.ts';
import type { RunningApp } from '../app.ts';
import { until } from '../wait.ts';

/**
 * The page captured: a settings page carrying suspected strings, so the preview has flagged
 * boxes to lift. It is read by the operating system's engine on every Windows machine the
 * suite runs on, and the detectors' answer to it is pinned by the corpus gates of §11.7.
 */
export const FIXTURE = screenshotFixture('suspected-08-mixed-page.png');

/** The settings keys the fixture capture backend reads (`capture::fake`). */
const FIXTURE_KEY = 'e2e.capture_fixture';
const FIXTURE_SCALE_KEY = 'e2e.capture_fixture_scale';

/** How long the OCR and the detectors may take over one capture in a debug build. */
const ANALYSIS_MS = 60_000;

/** Takes a capture of `fixture` from the action bar and waits until the preview has read it. */
async function capture(page: Page, app: RunningApp, fixture: string): Promise<void> {
  await app.automation.setSetting(FIXTURE_KEY, fixture);
  await app.automation.setSetting(FIXTURE_SCALE_KEY, 1);
  // CAP-01: the button captures nothing by itself; it offers two choices.
  await page.click('.actions .screenshot > button', 'Screenshot');
  await page.find('.screenshot-menu [role="menuitem"]', 'Select region');
  await page.click('.screenshot-menu [role="menuitem"]', 'Full screen');
  await page.untilView('preview', 20_000);
  await until('the capture is drawn', () =>
    page.execute<boolean>(
      'const image = document.querySelector(".preview-image"); return image !== null && image.complete && image.naturalWidth > 0;',
    ),
  );
  // OCR-04: the picture is on screen first, and the detectors answer after.
  await until(
    'the detectors have read the capture',
    async () => (await page.texts('.preview-note')).some((note) => note.startsWith('Regions hidden')),
    ANALYSIS_MS,
  );
}

/** The `aria-pressed` of a tool button, which is how a tool says it is armed. */
function armed(page: Page, tool: string): Promise<string | null> {
  return page.attributeOf('.preview-tools button', tool, 'aria-pressed');
}

/** Clicks the first flagged box and waits until it says it is `lifted` (or not). */
async function toggleFlagged(page: Page, lifted: boolean): Promise<void> {
  await page.click('button.preview-box-flagged');
  await until(`the flagged box is ${lifted ? 'lifted' : 'back'}`, async () => {
    const pressed = await page.attributes('button.preview-box-flagged', 'aria-pressed');
    return pressed[0] === String(lifted);
  });
}

export const preview: UiScenario = {
  id: 'preview',
  covers: 'PREV-01, PREV-02, PREV-04, CAP-01',
  title: 'the preview: both send buttons, unlock, a box by hand, a drag too short to count, crop',
  async run({ page, app, session, facts }) {
    const server = await session({ imagesInResults: true });
    await server.open(spec('Check the webhook settings page', ['Open the webhook settings.', 'Tell the agent what it shows.']));
    await page.findText('.step .counter', 'Step 1 of 2');
    await capture(page, app, FIXTURE);

    // PREV-04: both buttons, and no default between them.
    const sending = await page.texts('.preview-actions button');
    expect(
      sending.includes('Send image') && sending.includes('Send text'),
      'Send image and Send text are both offered',
      sending,
    );
    expect((await page.count('.preview-actions .button-primary')) === 0, 'neither send button is the default');
    await until('Send image can be pressed', () => page.enabled('.preview-actions button', 'Send image'));

    facts['flagged'] = await page.count('button.preview-box-flagged');
    facts['locked'] = await page.count('.preview-box-locked');
    expect((facts['flagged'] as number) > 0, 'the capture has a flagged box to lift', facts);

    // PREV-02: a false positive costs one click, and a second click puts it back.
    await toggleFlagged(page, true);
    await toggleFlagged(page, false);

    // PREV-02: a box drawn by hand.
    const before = await page.count('.preview-box');
    await page.click('.preview-tools button', 'Hide an area');
    await until('the tool is armed', async () => (await armed(page, 'Hide an area')) === 'true');
    await page.drag('.preview-canvas', { x: 0.04, y: 0.82 }, { x: 0.3, y: 0.96 });
    await until('the drawn box is on the picture', async () => (await page.count('.preview-box')) === before + 1);
    expect((await armed(page, 'Hide an area')) === 'false', 'the tool clears itself when the drag ends');
    const locked = await page.execute<{ tag: string; role: string | null; focusable: boolean }[]>(
      'return [...document.querySelectorAll(".preview-box-locked")].map((box) => ({ tag: box.tagName, role: box.getAttribute("role"), focusable: box.tabIndex >= 0 }));',
    );
    expect(
      locked.length > 0 && locked.every((box) => box.tag === 'SPAN' && box.role === null && !box.focusable),
      'a locked box is a plain span that offers no control',
      locked,
    );

    // A drag shorter than four pixels is a click, not a box. The four pixels are the
    // capture's, and the canvas draws the capture at a fraction of its size, so the drag is
    // sized from the scale on screen: three of the capture's pixels, which on this panel is a
    // click or a one-pixel move. The flagged box is toggled afterwards as a sequence point:
    // its answer comes back after the answer to any edit the short drag could have sent, so
    // the count read after it is final.
    const scale = await page.execute<number>(
      'const image = document.querySelector(".preview-image"); return image.getBoundingClientRect().width / image.naturalWidth;',
    );
    const short = Math.floor(3 * scale);
    facts['shortDragCssPixels'] = short;
    await page.click('.preview-tools button', 'Hide an area');
    await page.dragPixels('.preview-canvas', { x: 0.5, y: 0.5 }, { x: short, y: short });
    expect((await armed(page, 'Hide an area')) === 'false', 'the short drag still disarmed the tool');
    await toggleFlagged(page, true);
    expect((await page.count('.preview-box')) === before + 1, 'the short drag added no box');
    await toggleFlagged(page, false);

    // PREV-02: crop, and undo the crop.
    await page.click('.preview-tools button', 'Crop');
    await page.drag('.preview-canvas', { x: 0.02, y: 0.02 }, { x: 0.7, y: 0.7 });
    await page.find('.preview-crop');
    await page.click('.preview-tools button', 'Undo the crop');
    await page.gone('.preview-crop');

    // PREV-01: what leaves is the burned picture, and it reaches the agent.
    await page.click('.preview-actions button', 'Send image');
    const event = await server.event('screenshot');
    const shot = event.outcome['screenshot'] as { mode?: string; image_attached?: boolean } | undefined;
    expect(shot?.mode === 'image' && shot.image_attached === true, 'the outcome says an image was sent', shot);
    expect(
      typeof event.image === 'string' && event.image.length > 0,
      'the burned PNG crossed the channel beside the outcome',
    );
    await page.backOnTheHandoff();
  },
};

export const previewTextOnly: UiScenario = {
  id: 'preview-text-only',
  covers: 'PREV-03, PREV-04, FM-05',
  title: 'a session that cannot read images is offered Send text alone',
  async run({ page, app, session }) {
    const server = await session({ imagesInResults: false });
    await server.open(spec('Check the webhook settings page', ['Open the webhook settings.', 'Tell the agent what it shows.']));
    await page.findText('.step .counter', 'Step 1 of 2');
    await capture(page, app, FIXTURE);

    // PREV-04, FM-05: not drawn at all, rather than drawn and refused.
    await page.gone('.preview-actions button', 'Send image');
    await until('Send text can be pressed', () => page.enabled('.preview-actions button', 'Send text'));
    // PREV-03: the pane holds what was read, and it is the user's to edit.
    const pane = await page.property<string>('#preview-text', 'value');
    expect(pane.trim().length > 0, 'the text pane holds what the engine read');

    await page.click('.preview-actions button', 'Send text');
    const event = await server.event('screenshot');
    const shot = event.outcome['screenshot'] as { mode?: string; image_attached?: boolean } | undefined;
    expect(shot?.mode === 'text' && shot.image_attached === false, 'the outcome says text was sent', shot);
    expect(event.image === undefined, 'no picture crossed the channel');
    await page.backOnTheHandoff();
  },
};
