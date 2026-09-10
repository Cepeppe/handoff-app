/**
 * The screenshot waiting to be looked at, and the two ways of taking one (§7.8, CAP-01).
 *
 * The Rust side hides the panel, takes the pixels and holds them; this is the window's half
 * of the same flow — which of the two choices was pressed, what the preview draws when the
 * capture comes back, and what the detectors then found in it (§7.10).
 *
 * Three things it is deliberately **not**:
 *
 * - it never decides *whether* to capture. CAP-01 gives that to the user, every time, and
 *   the popover of `ScreenshotButton.svelte` is where the choice is made;
 * - it holds no pixels. The bytes become a blob URL as they arrive and are released the
 *   moment the preview is left, so a screenshot the user decided against does not sit in
 *   the webview's memory (PRIN-04, PRIN-09);
 * - it does not decide what is redacted. The boxes, the crop and the burn-in are the Rust
 *   side's (`ui_bridge::preview`), because a webview is the one part of this application
 *   that must never be believed about what may leave the machine. What is here is the
 *   *last answer* to those commands, so the window can draw it.
 *
 * # Why a capture carries a handoff id
 *
 * A screenshot is an interrupting action **on a handoff** (§7.4): the exemption list of
 * DET-03 is that handoff's spec, the outcome names its step, and the send goes to its
 * agent. The button knows which tab it is on, so the id travels with the capture from the
 * press rather than being looked up in the overlay's selection when the preview opens —
 * which would send the screenshot to whichever tab the user had switched to meanwhile.
 */
import { bridge } from './bridge';
import type { CaptureChoice, CaptureOutcome, PreviewAnalysis, PreviewDraw } from './model';
import { showView } from './view-state.svelte';

/** What the preview area is showing. */
export type PreviewState =
  | { status: 'empty' }
  | { status: 'ready'; handoffId: string; url: string; width: number; height: number }
  /** macOS has not granted the screen-recording permission (FM-17, CAP-04). */
  | { status: 'denied' }
  | { status: 'failed'; message: string };

let state = $state<PreviewState>({ status: 'empty' });
let analysis = $state<PreviewAnalysis | null>(null);
let analysing = $state(false);

/** The handoff the capture in flight belongs to, from the press of the button. */
let forHandoff: string | null = null;

/** The preview's state. Reactive: reading it inside a component subscribes to it. */
export function preview(): PreviewState {
  return state;
}

/**
 * What the detectors found, or `null` while they are still looking (OCR-04).
 *
 * The preview draws the image as soon as there is one and enables the two send buttons when
 * this stops being `null`, which is the whole of OCR-04: the cost of a native engine is
 * absorbed with the picture already on screen.
 */
export function detection(): PreviewAnalysis | null {
  return analysis;
}

/** Whether the detectors are still running (OCR-04's "analyzing"). */
export function isAnalysing(): boolean {
  return analysing;
}

/** Lets go of the blob URL of whatever was on screen, if anything was. */
function release(): void {
  if (state.status === 'ready') {
    URL.revokeObjectURL(state.url);
  }
}

/** What to show a person about a failure that arrived as an exception. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * One of the two choices of CAP-01, pressed on the tab `handoffId`.
 *
 * Nothing is shown yet: a full-screen capture hides the panel for a moment and a region
 * selection puts the overlays on screen, and in both cases what happens next arrives on
 * `onCaptureReady` — which is also the only path a failure takes, because the window that
 * finishes a region selection is destroyed before its call could answer.
 */
export async function startCapture(choice: CaptureChoice, handoffId: string): Promise<void> {
  release();
  state = { status: 'empty' };
  analysis = null;
  forHandoff = handoffId;
  try {
    if (choice === 'fullScreen') {
      await bridge().captureFullScreen();
    } else {
      await bridge().startRegionCapture();
    }
  } catch (error) {
    state = { status: 'failed', message: messageOf(error) };
    showView('preview');
  }
}

/** A capture ended: fetch its pixels if there are any, and open the preview (PREV-01). */
export async function captureFinished(outcome: CaptureOutcome): Promise<void> {
  release();
  analysis = null;
  if (outcome.status === 'denied') {
    state = { status: 'denied' };
  } else if (outcome.status === 'failed') {
    state = { status: 'failed', message: outcome.message };
  } else {
    try {
      const png = await bridge().capturePreview();
      state = {
        status: 'ready',
        handoffId: forHandoff ?? '',
        url: URL.createObjectURL(new Blob([png], { type: 'image/png' })),
        width: outcome.width,
        height: outcome.height,
      };
    } catch (error) {
      state = { status: 'failed', message: messageOf(error) };
    }
  }
  showView('preview');
  if (state.status === 'ready') {
    // Deliberately not awaited. OCR-04: the preview appears immediately with an "analyzing"
    // state and the send buttons enable when detection finishes — so the caller of this is
    // done as soon as there is a picture, and the several seconds a native engine can take
    // are spent with the user already looking at it.
    void analyse(state.handoffId);
  }
}

/**
 * Runs the OCR and the detectors, with the picture already on screen (OCR-04).
 *
 * A failure here is not a failure of the capture: the image is still what the user is
 * looking at and the two tools that need no engine — crop, and a box drawn by hand — still
 * work, so it is reported as an analysis that read nothing rather than as a broken preview
 * (FM-16, PRIN-10).
 */
async function analyse(handoffId: string): Promise<void> {
  analysing = true;
  try {
    analysis = await bridge().analyzeCapture(handoffId);
  } catch (error) {
    analysis = {
      handoffId,
      width: state.status === 'ready' ? state.width : 0,
      height: state.status === 'ready' ? state.height : 0,
      boxes: [],
      crop: null,
      redactions: 0,
      text: '',
      ocrEngine: null,
      unread: messageOf(error),
      imagesInResults: true,
      large: false,
    };
  } finally {
    analysing = false;
  }
}

/** Applies what the last edit answered, so the window redraws from one source. */
export function applyDraw(draw: PreviewDraw): void {
  if (analysis === null) return;
  analysis = { ...analysis, ...draw };
}

/** The user is done looking: the pixels go, on both sides, and the panel comes back. */
export async function discardPreview(): Promise<void> {
  release();
  state = { status: 'empty' };
  analysis = null;
  forHandoff = null;
  showView('overlay');
  try {
    await bridge().discardCapture();
  } catch {
    // The core keeps one capture at most and the next one replaces it; a refusal here
    // costs memory until then and nothing the user can see.
  }
}
