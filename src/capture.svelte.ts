/**
 * The screenshot waiting to be looked at, and the two ways of taking one (§7.8, CAP-01).
 *
 * The Rust side hides the panel, takes the pixels and holds them; this is the window's half
 * of the same flow — which of the two choices was pressed, and what the preview draws when
 * the capture comes back.
 *
 * Three things it is deliberately **not**:
 *
 * - it never decides *whether* to capture. CAP-01 gives that to the user, every time, and
 *   the popover of `ScreenshotButton.svelte` is where the choice is made;
 * - it holds no pixels. The bytes become a blob URL as they arrive and are released the
 *   moment the preview is left, so a screenshot the user decided against does not sit in
 *   the webview's memory (PRIN-04, PRIN-09);
 * - it does not send anything. Sending is the preview of T-049; until then a capture can
 *   be looked at and discarded, which is what §7.8 leaves this task with.
 */
import { bridge } from './bridge';
import type { CaptureChoice, CaptureOutcome } from './model';
import { showView } from './view-state.svelte';

/** What the preview area is showing. */
export type PreviewState =
  | { status: 'empty' }
  | { status: 'ready'; url: string; width: number; height: number }
  /** macOS has not granted the screen-recording permission (FM-17, CAP-04). */
  | { status: 'denied' }
  | { status: 'failed'; message: string };

let state = $state<PreviewState>({ status: 'empty' });

/** The preview's state. Reactive: reading it inside a component subscribes to it. */
export function preview(): PreviewState {
  return state;
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
 * One of the two choices of CAP-01, pressed.
 *
 * Nothing is shown yet: a full-screen capture hides the panel for a moment and a region
 * selection puts the overlays on screen, and in both cases what happens next arrives on
 * `onCaptureReady` — which is also the only path a failure takes, because the window that
 * finishes a region selection is destroyed before its call could answer.
 */
export async function startCapture(choice: CaptureChoice): Promise<void> {
  release();
  state = { status: 'empty' };
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
  if (outcome.status === 'denied') {
    state = { status: 'denied' };
  } else if (outcome.status === 'failed') {
    state = { status: 'failed', message: outcome.message };
  } else {
    try {
      const png = await bridge().capturePreview();
      state = {
        status: 'ready',
        url: URL.createObjectURL(new Blob([png], { type: 'image/png' })),
        width: outcome.width,
        height: outcome.height,
      };
    } catch (error) {
      state = { status: 'failed', message: messageOf(error) };
    }
  }
  showView('preview');
}

/** The user is done looking: the pixels go, on both sides, and the panel comes back. */
export async function discardPreview(): Promise<void> {
  release();
  state = { status: 'empty' };
  showView('overlay');
  try {
    await bridge().discardCapture();
  } catch {
    // The core keeps one capture at most and the next one replaces it; a refusal here
    // costs memory until then and nothing the user can see.
  }
}
