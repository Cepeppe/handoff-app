/**
 * The preview: what is drawn, what may be lifted, and what the two buttons send (T-049).
 *
 * The requirements these cases exist for are the ones a screenshot of the UI cannot show:
 *
 * - **PREV-01** — there is no way to send without passing through here, and what the user
 *   is looking at is what will leave: the boxes drawn over the picture are the rectangles
 *   the burn-in fills, and the text under the pane is the text the agent will read.
 * - **PREV-04** — two buttons, side by side, **no default**, and no **Send image** at all
 *   for a session whose capability row refuses images (FM-05).
 * - **OCR-04** — the picture is on screen while the detectors are still running and both
 *   buttons are disabled until they answer.
 * - **DET-01** — a flagged box is one click away from being lifted; a locked one is not a
 *   control at all, and the Rust side refuses it a second time (`ui_bridge::preview`).
 * - **FM-16 / OCR-01** — a capture no engine could read is *said*, not hidden: nothing was
 *   redacted automatically and the user is told so before they press anything.
 *
 * The command layer is faked, so what is asserted here is the window's half: which command
 * is called with what, and what the user is shown. That the burn actually covers the
 * glyphs is `src-tauri/tests/redaction_corpus.rs`, and that the whole chain works against a
 * real agent is E2E-3.
 */
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setBridge } from '../bridge';
import { captureFinished, detection, discardPreview, startCapture } from '../capture.svelte';
import { DEFAULT_LANGUAGE, setLanguage } from '../i18n';
import type { PreviewAnalysis, PreviewBox, PreviewDraw } from '../model';
import PreviewView from '../views/PreviewView.svelte';
import { view } from '../view-state.svelte';
import { fakeBridge } from './fake-bridge';

const HANDOFF = 'hf_0123456789';

/** The two boxes every case starts from: one certain, one suspected. */
function boxes(): PreviewBox[] {
  return [
    { id: 0, x: 10, y: 10, width: 300, height: 30, level: 'locked', cause: 'api_key', unlocked: false },
    { id: 1, x: 10, y: 50, width: 300, height: 30, level: 'flagged', cause: 'entropy', unlocked: false },
  ];
}

function analysis(overrides: Partial<PreviewAnalysis> = {}): PreviewAnalysis {
  return {
    handoffId: HANDOFF,
    width: 1200,
    height: 660,
    boxes: boxes(),
    crop: null,
    redactions: 2,
    text: 'Secret key   sk_live_51H8xQ2eZvKYlo2C0Sd8h4kL',
    ocrEngine: 'windows',
    unread: null,
    imagesInResults: true,
    large: false,
    ...overrides,
  };
}

/** A bridge that answers a capture and an analysis, with `overrides` on top. */
function previewBridge(
  answer: PreviewAnalysis | (() => Promise<PreviewAnalysis>) = analysis(),
  overrides: Partial<ReturnType<typeof fakeBridge>> = {},
): ReturnType<typeof fakeBridge> {
  return fakeBridge({
    capturePreview: vi.fn(async () => new ArrayBuffer(8)),
    analyzeCapture: vi.fn(async () =>
      typeof answer === 'function' ? await answer() : answer,
    ),
    previewText: vi.fn(async (text: string) => ({ text, kinds: [], suspected: 0 })),
    ...overrides,
  });
}

/** A pointer drag over the canvas, in fractions of the drawn image. */
function drag(fromX: number, fromY: number, toX: number, toY: number): void {
  const canvas = document.querySelector('.preview-canvas');
  if (canvas === null) throw new Error('there is no canvas to drag on');
  // jsdom gives every element a zero-sized rectangle; the component divides by it, so the
  // one the drag is measured against is stubbed to the size the picture is drawn at.
  canvas.getBoundingClientRect = (): DOMRect =>
    ({ left: 0, top: 0, width: 300, height: 165 }) as DOMRect;
  const at = (x: number, y: number): PointerEvent =>
    new PointerEvent('pointerdown', { clientX: x * 300, clientY: y * 165, bubbles: true });
  canvas.dispatchEvent(at(fromX, fromY));
  canvas.dispatchEvent(
    new PointerEvent('pointermove', {
      clientX: toX * 300,
      clientY: toY * 165,
      bubbles: true,
    }),
  );
  canvas.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }));
}

/**
 * Opens the preview the way the button does: a capture started on a tab, then its pixels.
 *
 * The press is what ties the capture to a handoff (`capture.svelte.ts`), so a helper that
 * skipped it would test a preview that belongs to nobody.
 */
async function open(bridge: ReturnType<typeof fakeBridge>): Promise<void> {
  setBridge(bridge);
  await startCapture('fullScreen', HANDOFF);
  await captureFinished({ status: 'ready', width: 1200, height: 660, monitor: 1 });
  render(PreviewView);
  await waitFor(() => expect(detection()).not.toBeNull());
}

beforeEach(() => {
  setLanguage(DEFAULT_LANGUAGE);
  const created: string[] = [];
  URL.createObjectURL = vi.fn((): string => {
    const url = `blob:test/${created.length}`;
    created.push(url);
    return url;
  }) as unknown as typeof URL.createObjectURL;
  URL.revokeObjectURL = vi.fn() as unknown as typeof URL.revokeObjectURL;
});

afterEach(async () => {
  cleanup();
  await discardPreview();
  setBridge(null);
});

describe('the preview (PREV-01, PREV-04)', () => {
  it('draws the capture, its boxes and the two send buttons', async () => {
    const bridge = previewBridge();
    await open(bridge);

    expect(screen.getByRole('img', { name: 'The captured screen' })).toBeDefined();
    expect(screen.getByText('1200 × 660 pixels')).toBeDefined();
    expect(bridge.analyzeCapture).toHaveBeenCalledWith(HANDOFF);
    expect(document.querySelectorAll('.preview-box-locked')).toHaveLength(1);
    expect(document.querySelectorAll('.preview-box-flagged')).toHaveLength(1);

    const image = screen.getByRole('button', { name: 'Send image' });
    const text = screen.getByRole('button', { name: 'Send text' });
    // PREV-04: both there, neither the default. "No default" is a fact about the markup —
    // nothing is `button-primary` and nothing is autofocused.
    expect(image.className).not.toContain('button-primary');
    expect(text.className).not.toContain('button-primary');
    expect(document.activeElement).not.toBe(image);
    expect(document.activeElement).not.toBe(text);
  });

  it('keeps both buttons disabled while the detectors are still running (OCR-04)', async () => {
    // The picture is on screen and the buttons are dead: the whole of OCR-04 is that the
    // cost of a native engine is absorbed with something already drawn.
    let answer = (_: PreviewAnalysis): void => {};
    const pending = new Promise<PreviewAnalysis>((resolve) => {
      answer = resolve;
    });
    setBridge(
      previewBridge(analysis(), {
        analyzeCapture: vi.fn(async () => pending),
      }),
    );
    await startCapture('fullScreen', HANDOFF);
    await captureFinished({ status: 'ready', width: 1200, height: 660, monitor: 1 });
    render(PreviewView);

    expect(screen.getByRole('img', { name: 'The captured screen' })).toBeDefined();
    expect(screen.getByRole('status').textContent).toContain('Reading the capture');
    expect(screen.getByRole('button', { name: 'Send image' })).toHaveProperty('disabled', true);
    expect(screen.getByRole('button', { name: 'Send text' })).toHaveProperty('disabled', true);

    answer(analysis());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Send image' })).toHaveProperty('disabled', false),
    );
  });

  it('does not offer Send image to a session that cannot read one (PREV-04, FM-05)', async () => {
    await open(previewBridge(analysis({ imagesInResults: false })));

    expect(screen.queryByRole('button', { name: 'Send image' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Send text' })).toBeDefined();
  });

  it('recommends text for a capture larger than what will be sent (FM-30)', async () => {
    await open(previewBridge(analysis({ large: true })));
    expect(screen.getByText(/Sending it as text/u)).toBeDefined();
  });
});

describe('editing what is hidden (PREV-02, DET-01)', () => {
  it('lifts a flagged box in one click and puts it back in another', async () => {
    const lifted: PreviewDraw = {
      boxes: [boxes()[0], { ...boxes()[1], unlocked: true }],
      crop: null,
      redactions: 1,
    };
    const bridge = previewBridge(analysis(), {
      editPreview: vi.fn(async () => lifted),
    });
    await open(bridge);

    screen.getByRole('button', { name: 'Show this again' }).click();
    await waitFor(() => expect(bridge.editPreview).toHaveBeenCalledWith({ kind: 'unlock', id: 1 }));
    await waitFor(() =>
      expect(document.querySelectorAll('.preview-box-lifted')).toHaveLength(1),
    );
    expect(screen.getByRole('status').textContent).toContain('Regions hidden: 1');

    screen.getByRole('button', { name: 'Hide this again' }).click();
    await waitFor(() => expect(bridge.editPreview).toHaveBeenCalledWith({ kind: 'relock', id: 1 }));
  });

  it('offers no control at all on a certain match (DET-01)', async () => {
    await open(previewBridge());
    // The locked box is a `<span>`: there is nothing to press, which is the requirement.
    const locked = document.querySelector('.preview-box-locked');
    expect(locked?.tagName).toBe('SPAN');
    expect(screen.getAllByRole('button', { name: 'Show this again' })).toHaveLength(1);
  });

  it('sends the rectangle the user dragged as a box to hide (PREV-02)', async () => {
    const bridge = previewBridge(analysis(), {
      editPreview: vi.fn(async () => ({ boxes: boxes(), crop: null, redactions: 3 })),
    });
    await open(bridge);

    screen.getByRole('button', { name: 'Hide an area' }).click();
    drag(0.1, 0.2, 0.5, 0.4);

    await waitFor(() =>
      expect(bridge.editPreview).toHaveBeenCalledWith({
        kind: 'addBox',
        // The drag is measured in fractions of the drawn image and lands in the capture's
        // own pixels, which is the one coordinate system the burn-in works in.
        rect: { x: 120, y: 132, width: 480, height: 132 },
      }),
    );
  });

  it('crops to the rectangle the user dragged, and never to a click (PREV-02)', async () => {
    const bridge = previewBridge(analysis(), {
      editPreview: vi.fn(async () => ({
        boxes: boxes(),
        crop: { x: 0, y: 0, width: 600, height: 330 },
        redactions: 2,
      })),
    });
    await open(bridge);

    // A press with no movement is not a rectangle: it would crop the picture to nothing.
    screen.getByRole('button', { name: 'Crop' }).click();
    drag(0.5, 0.5, 0.5, 0.5);
    expect(bridge.editPreview).not.toHaveBeenCalled();

    screen.getByRole('button', { name: 'Crop' }).click();
    drag(0, 0, 0.5, 0.5);
    await waitFor(() =>
      expect(bridge.editPreview).toHaveBeenCalledWith({
        kind: 'crop',
        rect: { x: 0, y: 0, width: 600, height: 330 },
      }),
    );
  });

  it('undoes a crop once there is one', async () => {
    const bridge = previewBridge(analysis({ crop: { x: 0, y: 0, width: 600, height: 300 } }), {
      editPreview: vi.fn(async () => ({ boxes: boxes(), crop: null, redactions: 2 })),
    });
    await open(bridge);

    screen.getByRole('button', { name: 'Undo the crop' }).click();
    await waitFor(() => expect(bridge.editPreview).toHaveBeenCalledWith({ kind: 'uncrop' }));
  });

  it('says a capture nobody could read was not redacted for them (FM-16, OCR-01)', async () => {
    await open(
      previewBridge(
        analysis({
          boxes: [],
          redactions: 0,
          text: '',
          ocrEngine: null,
          unread: 'no OCR engine could read the capture (ocrs: unavailable)',
        }),
      ),
    );

    const said = screen.getAllByRole('status').map((element) => element.textContent ?? '');
    expect(said.join(' ')).toContain('nothing was hidden automatically');
    // The tools that need no engine are still there.
    expect(screen.getByRole('button', { name: 'Hide an area' })).toBeDefined();
    expect(screen.getByRole('button', { name: 'Crop' })).toBeDefined();
  });
});

describe('sending (PREV-01, PREV-03, PREV-05, CTX-01)', () => {
  it('sends the image with the comment, and leaves the preview behind it', async () => {
    const bridge = previewBridge(analysis(), {
      sendScreenshot: vi.fn(async () => ({
        mode: 'image' as const,
        width: 1200,
        height: 660,
        redactions: 2,
        bytes: 40_000,
      })),
    });
    await open(bridge);

    const comment = screen.getByLabelText('Anything to say about it (optional)');
    (comment as HTMLTextAreaElement).value = 'the button is not where the step says';
    comment.dispatchEvent(new Event('input', { bubbles: true }));

    screen.getByRole('button', { name: 'Send image' }).click();
    await waitFor(() =>
      expect(bridge.sendScreenshot).toHaveBeenCalledWith(
        HANDOFF,
        'image',
        null,
        'the button is not where the step says',
      ),
    );
    // PRIN-04: the pixels go on both sides and the panel comes back.
    await waitFor(() => expect(view()).toBe('overlay'));
    expect(bridge.discardCapture).toHaveBeenCalled();
  });

  it('sends the text pane as the user edited it (PREV-03)', async () => {
    const bridge = previewBridge(analysis(), {
      sendScreenshot: vi.fn(async () => ({
        mode: 'text' as const,
        width: 1200,
        height: 660,
        redactions: 1,
        bytes: null,
      })),
    });
    await open(bridge);

    const pane = screen.getByLabelText('The text read from the capture, as it will be sent');
    expect((pane as HTMLTextAreaElement).value).toContain('Secret key');
    (pane as HTMLTextAreaElement).value = 'Secret key is on this page';
    pane.dispatchEvent(new Event('input', { bubbles: true }));

    screen.getByRole('button', { name: 'Send text' }).click();
    await waitFor(() =>
      expect(bridge.sendScreenshot).toHaveBeenCalledWith(
        HANDOFF,
        'text',
        'Secret key is on this page',
        null,
      ),
    );
  });

  it('runs both detectors over the comment before it can be sent (§7.10, DET-01)', async () => {
    // The comment is typed text, so it is treated the way a sheet treats it: a certain
    // match is replaced and a suspected one is marked and sent as it was written.
    const bridge = previewBridge(analysis(), {
      scanTypedText: vi.fn(async (_id: string, text: string) => ({
        text: text.replace('sk_live_51H8xQ2eZvKYlo2C0Sd8h4kL', '[REDACTED:api_key]'),
        kinds: ['api_key'],
        reasons: ['label'],
        segments: [
          { text: 'the key ', suspected: false },
          { text: '[REDACTED:api_key]', suspected: true },
        ],
      })),
      sendScreenshot: vi.fn(async () => ({
        mode: 'image' as const,
        width: 1200,
        height: 660,
        redactions: 2,
        bytes: 1,
      })),
    });
    await open(bridge);

    const comment = screen.getByLabelText('Anything to say about it (optional)');
    (comment as HTMLTextAreaElement).value = 'the key sk_live_51H8xQ2eZvKYlo2C0Sd8h4kL';
    comment.dispatchEvent(new Event('input', { bubbles: true }));

    await waitFor(() =>
      expect(bridge.scanTypedText).toHaveBeenCalledWith(
        HANDOFF,
        'the key sk_live_51H8xQ2eZvKYlo2C0Sd8h4kL',
      ),
    );
    await waitFor(() =>
      expect(screen.getByText('A secret was taken out before sending: api_key')).toBeDefined(),
    );
    expect(document.querySelector('.sheet-mark')).not.toBeNull();

    // And what is sent is what was shown, not what was typed.
    screen.getByRole('button', { name: 'Send image' }).click();
    await waitFor(() =>
      expect(bridge.sendScreenshot).toHaveBeenCalledWith(
        HANDOFF,
        'image',
        null,
        'the key [REDACTED:api_key]',
      ),
    );
  });

  it('shows what the text pane will lose before the button is pressed (§7.10)', async () => {
    const bridge = previewBridge(analysis(), {
      previewText: vi.fn(async () => ({
        text: 'Secret key   [REDACTED:api_key]',
        kinds: ['api_key'],
        suspected: 1,
      })),
    });
    await open(bridge);

    await waitFor(() =>
      expect(screen.getByText('Taken out of the text: api_key; suspected: 1.')).toBeDefined(),
    );
    // PREV-01: the sent form itself, not only a count of what went.
    expect(
      document.querySelector('.sheet-preview')?.textContent,
    ).toBe('Secret key   [REDACTED:api_key]');
  });

  it('keeps the preview and says why when the send is refused', async () => {
    const bridge = previewBridge(analysis(), {
      sendScreenshot: vi.fn(async () => {
        throw new Error('the handoff is not active');
      }),
    });
    await open(bridge);

    screen.getByRole('button', { name: 'Send image' }).click();
    await waitFor(() =>
      expect(screen.getByRole('alert').textContent).toContain('the handoff is not active'),
    );
    expect(view()).toBe('preview');
    expect(bridge.discardCapture).not.toHaveBeenCalled();
  });
});
