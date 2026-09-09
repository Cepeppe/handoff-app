/**
 * The Screenshot button, the two choices behind it and the preview they end in (§7.8).
 *
 * CAP-01 is the requirement these cases exist for, and it is a *negative* one: pressing the
 * button must not capture anything. That is invisible to a screenshot of the UI and easy to
 * lose to a "repeat the last choice" convenience, so the first two cases assert what does
 * **not** happen — no capture on opening the popover, and the highlighted choice sitting
 * there unfired.
 *
 * The rest is the preview of PREV-01 in the three states this task can reach: the capture,
 * the macOS permission of FM-17, and a platform that refused.
 */
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setBridge } from '../bridge';
import { captureFinished, discardPreview, preview, startCapture } from '../capture.svelte';
import { DEFAULT_LANGUAGE, setLanguage } from '../i18n';
import ScreenshotButton from '../overlay/ScreenshotButton.svelte';
import PreviewView from '../views/PreviewView.svelte';
import { view } from '../view-state.svelte';
import { fakeBridge } from './fake-bridge';

/** jsdom has no object URLs; the preview only ever hands the result to an `<img>`. */
function stubObjectUrls(): string[] {
  const created: string[] = [];
  URL.createObjectURL = vi.fn((): string => {
    const url = `blob:test/${created.length}`;
    created.push(url);
    return url;
  }) as unknown as typeof URL.createObjectURL;
  URL.revokeObjectURL = vi.fn() as unknown as typeof URL.revokeObjectURL;
  return created;
}

beforeEach(() => {
  setLanguage(DEFAULT_LANGUAGE);
  stubObjectUrls();
});

afterEach(async () => {
  cleanup();
  await discardPreview();
  setBridge(null);
});

describe('the Screenshot button (CAP-01)', () => {
  it('asks which capture instead of taking one', async () => {
    const bridge = fakeBridge();
    setBridge(bridge);
    render(ScreenshotButton);

    screen.getByRole('button', { name: 'Screenshot' }).click();
    await waitFor(() => expect(screen.getByRole('menu')).toBeDefined());

    expect(screen.getByRole('menuitem', { name: 'Full screen' })).toBeDefined();
    expect(screen.getByRole('menuitem', { name: 'Select region' })).toBeDefined();
    expect(bridge.captureFullScreen).not.toHaveBeenCalled();
    expect(bridge.startRegionCapture).not.toHaveBeenCalled();
  });

  it('highlights the last choice and still does not fire it', async () => {
    // The whole of CAP-01's "highlighted but does not fire": the mark is on the entry the
    // user picked last time, and opening the popover a second time changes nothing.
    const bridge = fakeBridge({
      captureSettings: vi.fn(async () => ({ lastChoice: 'region' as const })),
    });
    setBridge(bridge);
    render(ScreenshotButton);

    screen.getByRole('button', { name: 'Screenshot' }).click();
    await waitFor(() => expect(screen.getByRole('menu')).toBeDefined());

    const region = screen.getByRole('menuitem', { name: 'Select region' });
    const full = screen.getByRole('menuitem', { name: 'Full screen' });
    expect(region.getAttribute('aria-current')).toBe('true');
    expect(full.hasAttribute('aria-current')).toBe(false);
    expect(bridge.startRegionCapture).not.toHaveBeenCalled();
    expect(bridge.captureFullScreen).not.toHaveBeenCalled();
  });

  it('takes the capture the user picked, and closes the popover', async () => {
    const bridge = fakeBridge();
    setBridge(bridge);
    render(ScreenshotButton);

    screen.getByRole('button', { name: 'Screenshot' }).click();
    await waitFor(() => expect(screen.getByRole('menu')).toBeDefined());
    screen.getByRole('menuitem', { name: 'Select region' }).click();

    await waitFor(() => expect(bridge.startRegionCapture).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole('menu')).toBeNull();
    expect(bridge.captureFullScreen).not.toHaveBeenCalled();
  });

  it('closes without capturing when the popover is dismissed', async () => {
    const bridge = fakeBridge();
    setBridge(bridge);
    render(ScreenshotButton);

    const button = screen.getByRole('button', { name: 'Screenshot' });
    button.click();
    await waitFor(() => expect(screen.getByRole('menu')).toBeDefined());
    button.click();
    await tick();

    expect(screen.queryByRole('menu')).toBeNull();
    expect(bridge.startRegionCapture).not.toHaveBeenCalled();
  });
});

describe('the preview (PREV-01)', () => {
  it('draws the capture and says how many pixels it has', async () => {
    setBridge(fakeBridge({ capturePreview: vi.fn(async () => new ArrayBuffer(8)) }));
    await captureFinished({ status: 'ready', width: 1280, height: 720, monitor: 1 });
    render(PreviewView);

    const image = screen.getByRole('img', { name: 'The captured screen' });
    expect(image.getAttribute('src')).toBe('blob:test/0');
    expect(screen.getByText('1280 × 720 pixels')).toBeDefined();
    expect(view()).toBe('preview');
  });

  it('shows the macOS explanation and the way to the settings pane instead (FM-17)', async () => {
    // The permission is asked for *before* the platform is touched, so what reaches the
    // user is this screen and never the system prompt CAP-04 rules out mid-handoff.
    const bridge = fakeBridge();
    setBridge(bridge);
    await captureFinished({ status: 'denied' });
    render(PreviewView);

    expect(screen.getByRole('status').textContent).toContain('record the screen');
    screen.getByRole('button', { name: 'Open the settings pane' }).click();
    await waitFor(() =>
      expect(bridge.openScreenRecordingSettings).toHaveBeenCalledTimes(1),
    );
    expect(bridge.capturePreview).not.toHaveBeenCalled();
  });

  it('says what the platform refused rather than showing an empty frame', async () => {
    setBridge(fakeBridge());
    await captureFinished({ status: 'failed', message: 'no monitor answered' });
    render(PreviewView);

    expect(screen.getByRole('status').textContent).toContain('no monitor answered');
    expect(screen.queryByRole('img')).toBeNull();
  });

  it('reports a capture whose pixels could not be fetched', async () => {
    setBridge(
      fakeBridge({
        capturePreview: vi.fn(async () => {
          throw new Error('there is no capture to show');
        }),
      }),
    );
    await captureFinished({ status: 'ready', width: 10, height: 10, monitor: 1 });
    render(PreviewView);

    expect(screen.getByRole('status').textContent).toContain('there is no capture to show');
  });

  it('lets go of the pixels on both sides when the preview is left (PRIN-04)', async () => {
    const bridge = fakeBridge({ capturePreview: vi.fn(async () => new ArrayBuffer(8)) });
    setBridge(bridge);
    await captureFinished({ status: 'ready', width: 4, height: 4, monitor: 1 });
    render(PreviewView);

    screen.getByRole('button', { name: 'Discard' }).click();
    await waitFor(() => expect(bridge.discardCapture).toHaveBeenCalledTimes(1));
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:test/0');
    expect(preview().status).toBe('empty');
    expect(view()).toBe('overlay');
  });

  it('reports a capture the core refused to start', async () => {
    setBridge(
      fakeBridge({
        captureFullScreen: vi.fn(async () => {
          throw new Error('no window to hide');
        }),
      }),
    );
    await startCapture('fullScreen');

    expect(preview()).toEqual({ status: 'failed', message: 'no window to hide' });
    expect(view()).toBe('preview');
  });
});
