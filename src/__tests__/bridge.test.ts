/**
 * The bridge is the seam every component test relies on: it must be replaceable, and
 * outside a webview it must swallow calls rather than throw.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';

import { bridge, inTauri, noopBridge, setBridge, type Bridge } from '../bridge';
import { fakeBridge } from './fake-bridge';

afterEach(() => setBridge(null));

describe('bridge', () => {
  it('is the no-op implementation outside a Tauri webview', async () => {
    expect(inTauri()).toBe(false);
    // Nothing to assert but the absence of a throw: a component rendered in a browser or
    // under jsdom must not fail because there is no core to talk to.
    await expect(bridge().resizeToContent(120)).resolves.toBeUndefined();
    await expect(bridge().setUiLanguage('it')).resolves.toBeUndefined();
    const unlisten = await bridge().onShowView(() => {});
    expect(() => unlisten()).not.toThrow();
  });

  it('hands out the installed replacement', async () => {
    const fake: Bridge = fakeBridge({
      resizeToContent: vi.fn().mockResolvedValue(undefined),
      setUiLanguage: vi.fn().mockResolvedValue(undefined),
    });
    setBridge(fake);

    await bridge().resizeToContent(240);
    await bridge().setUiLanguage('en');

    expect(fake.resizeToContent).toHaveBeenCalledWith(240);
    expect(fake.setUiLanguage).toHaveBeenCalledWith('en');
  });

  it('goes back to the automatic choice when the replacement is removed', () => {
    setBridge(noopBridge());
    setBridge(null);
    expect(bridge()).toBeDefined();
  });
});
