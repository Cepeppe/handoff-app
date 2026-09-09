/**
 * The region-selection overlay of DD-29, from the drag to what it reports (CAP-02).
 *
 * The rectangle is measured in this window's CSS pixels and turned into virtual-desktop
 * pixels on the Rust side, so what these cases pin is the half that lives here: which
 * corners a drag produces whichever way it was dragged, that a click without a drag takes
 * nothing, and that Esc cancels. The other half — the scale factor, the negative origins,
 * the composite across two monitors — is `src-tauri/src/capture/geometry.rs`, where it can
 * be exercised against layouts this machine does not have.
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setBridge } from '../bridge';
import { DEFAULT_LANGUAGE, setLanguage } from '../i18n';
import SelectionOverlay from '../selection/SelectionOverlay.svelte';
import { fakeBridge } from './fake-bridge';

/** A bridge whose overlay covers monitor 4 at 150 %. */
function overlayBridge(overrides = {}) {
  return fakeBridge({
    selectionSetup: vi.fn(async () => ({ monitor: 4, scaleFactor: 1.5 })),
    ...overrides,
  });
}

/** Renders the overlay and waits for it to learn which monitor it is on. */
async function open(bridge = overlayBridge()) {
  setBridge(bridge);
  render(SelectionOverlay);
  await tick();
  await tick();
  return bridge;
}

function surface(): Element {
  const found = document.querySelector('.selection');
  expect(found).not.toBeNull();
  return found as Element;
}

/** One drag, press to release, in the window's CSS pixels. */
async function drag(from: [number, number], to: [number, number]): Promise<void> {
  await fireEvent.pointerDown(surface(), { clientX: from[0], clientY: from[1], button: 0 });
  await fireEvent.pointerMove(surface(), { clientX: to[0], clientY: to[1] });
  await fireEvent.pointerUp(surface(), { clientX: to[0], clientY: to[1] });
}

beforeEach(() => {
  setLanguage(DEFAULT_LANGUAGE);
});

afterEach(() => {
  cleanup();
  setBridge(null);
});

describe('the region selection overlay (DD-29)', () => {
  it('says what to do before anything is dragged', async () => {
    await open();
    expect(screen.getByText('Drag to select an area. Esc cancels.')).toBeDefined();
    expect(document.querySelector('.selection-rect')).toBeNull();
  });

  it('reports the rectangle on the monitor its window covers', async () => {
    const bridge = await open();
    await drag([100, 60], [300, 160]);

    expect(bridge.regionCaptured).toHaveBeenCalledWith({
      monitor: 4,
      rect: { x: 100, y: 60, width: 200, height: 100 },
    });
    expect(bridge.cancelRegionCapture).not.toHaveBeenCalled();
  });

  it('is the same rectangle dragged towards the top left', async () => {
    const bridge = await open();
    await drag([300, 160], [100, 60]);

    expect(bridge.regionCaptured).toHaveBeenCalledWith({
      monitor: 4,
      rect: { x: 100, y: 60, width: 200, height: 100 },
    });
  });

  it('shows the size in the pixels the image will have, not in CSS pixels', async () => {
    // At 150 % a 200 × 100 drag is a 300 × 150 image, and the number a user reads has to be
    // the one they will get.
    await open();
    await fireEvent.pointerDown(surface(), { clientX: 10, clientY: 10, button: 0 });
    await fireEvent.pointerMove(surface(), { clientX: 210, clientY: 110 });

    expect(screen.getByText('300 × 150')).toBeDefined();
  });

  it('takes nothing from a click that never became a drag', async () => {
    const bridge = await open();
    await drag([40, 40], [40, 40]);

    expect(bridge.regionCaptured).not.toHaveBeenCalled();
    expect(bridge.cancelRegionCapture).toHaveBeenCalledTimes(1);
  });

  it('cancels on Esc', async () => {
    const bridge = await open();
    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(bridge.cancelRegionCapture).toHaveBeenCalledTimes(1);
    expect(bridge.regionCaptured).not.toHaveBeenCalled();
  });

  it('cancels rather than cropping the wrong screen when it cannot learn which it is on', async () => {
    // Without the setup the drag would be attributed to monitor 0, which is a rectangle
    // somewhere on the desktop and not the one the user framed.
    const bridge = await open(
      overlayBridge({
        selectionSetup: vi.fn(async () => {
          throw new Error('not a selection overlay');
        }),
      }),
    );

    expect(bridge.cancelRegionCapture).toHaveBeenCalledTimes(1);
  });
});
