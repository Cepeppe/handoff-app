/**
 * View switching (§7.6, DD-10): one window, one view at a time, changed from the dev
 * switcher or from outside the component tree by the tray menu.
 */
import { cleanup, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import App from '../App.svelte';
import { setBridge, type Bridge, type Unlisten } from '../bridge';
import { DEFAULT_LANGUAGE, setLanguage } from '../i18n';
import { resetOverlay } from '../overlay/state.svelte';
import { resetView } from '../view-state.svelte';
import { VIEW_NAMES, type ViewName } from '../views';
import { fakeBridge } from './fake-bridge';

/** A bridge that hands back the handler the application registered for the tray event. */
function recordingBridge(): { bridge: Bridge; show: (view: ViewName) => void } {
  let handler: ((view: ViewName) => void) | null = null;
  return {
    bridge: fakeBridge({
      async onShowView(next): Promise<Unlisten> {
        handler = next;
        return () => {
          handler = null;
        };
      },
    }),
    show: (view) => handler?.(view),
  };
}

beforeEach(() => {
  resetView();
  resetOverlay();
  setLanguage(DEFAULT_LANGUAGE);
});

afterEach(() => {
  cleanup();
  setBridge(null);
});

describe('App', () => {
  it('opens on the overlay', () => {
    render(App);
    // The overlay is the step view now, so what identifies it is the view marker and not a
    // heading: a handoff whose spec has arrived shows its goal there, and an empty overlay
    // shows the sentence of §7.6.
    expect(document.querySelector('[data-view="overlay"]')).not.toBeNull();
  });

  it('shows exactly one view at a time, and every view can be reached', async () => {
    render(App);

    for (const name of VIEW_NAMES) {
      const button = screen.getByRole('button', { name: labelOf(name) });
      button.click();
      await tick();

      expect(document.querySelectorAll('.view')).toHaveLength(1);
      expect(document.querySelector('.view')?.getAttribute('data-view')).toBe(name);
    }
  });

  it('follows the view the tray menu asks for', async () => {
    const recorder = recordingBridge();
    setBridge(recorder.bridge);
    render(App);
    await tick();

    recorder.show('request');
    await tick();
    expect(document.querySelector('.view')?.getAttribute('data-view')).toBe('request');

    recorder.show('preview');
    await tick();
    expect(document.querySelector('.view')?.getAttribute('data-view')).toBe('preview');
  });

  it('keeps a drag region in the header, the only way to move an undecorated window', () => {
    render(App);
    const header = document.querySelector('.header');
    expect(header?.hasAttribute('data-tauri-drag-region')).toBe(true);
  });

  it('renders the views in the resolved language', async () => {
    setLanguage('it');
    render(App);

    screen.getAllByRole('button', { name: 'Impostazioni' })[0]?.click();
    await tick();

    expect(screen.getByRole('heading', { name: 'Impostazioni' })).toBeDefined();
  });
});

/** The English label of a view, as the dev switcher prints it. */
function labelOf(view: ViewName): string {
  return {
    overlay: 'Overlay',
    request: 'New request',
    settings: 'Settings',
    onboarding: 'Welcome',
    preview: 'Preview',
  }[view];
}
