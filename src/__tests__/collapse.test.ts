/**
 * The collapsed bar of WIN-03 and the fallback of R-10.
 *
 * These are `App.svelte` tests and not overlay tests, because the collapse replaces the
 * *window*: the header goes with it, and what comes back is one line and three buttons. The
 * focus change arrives from the Rust side as an event, so a fake bridge is enough to play
 * the whole gesture — click elsewhere, click back — without a window manager.
 */
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import App from '../App.svelte';
import { setBridge, type Bridge, type Unlisten } from '../bridge';
import { DEFAULT_LANGUAGE, setLanguage } from '../i18n';
import type { ActionsView, HandoffView, TabView } from '../model';
import {
  fallbackDelay,
  fallbackIsOn,
  isCollapsed,
  resetCollapse,
} from '../overlay/collapse.svelte';
import { resetOverlay } from '../overlay/state.svelte';
import { resetView } from '../view-state.svelte';
import { expandForm, resetForm } from '../window-form.svelte';
import { servingBridge } from './fake-bridge';

const ID = 'hf_0000000001';

const ACTIONS: ActionsView = {
  done: true,
  ask: true,
  note: true,
  skip: true,
  defer: true,
  abandon: true,
  screenshot: true,
  resume: false,
  closeOrphan: false,
};

function tab(overrides: Partial<TabView> = {}): TabView {
  return {
    id: ID,
    label: 'Claude Code · baton',
    agent: 'Claude Code',
    project: 'baton',
    state: 'active',
    uiState: 'guiding',
    group: 'open',
    goal: 'Register the webhook',
    orphan: false,
    actions: ACTIONS,
    createdAt: '2026-09-09T09:00:00.000Z',
    ...overrides,
  };
}

function view(overrides: Partial<HandoffView> = {}): HandoffView {
  return {
    tab: tab(),
    state: 'active',
    uiState: 'guiding',
    banner: null,
    goal: 'Register the webhook',
    location: null,
    url: null,
    lang: 'en',
    step: {
      counter: { key: 'counter.step', index: 1, total: 2, round: 1 },
      text: 'Open the dashboard and add the endpoint.',
      warning: null,
      url: null,
      values: [],
      confirmed: false,
      skipped: false,
      notes: [],
      questions: [],
      replies: [],
      last: false,
    },
    secrets: [],
    notes: [],
    pending: null,
    history: [],
    verify: null,
    verifyResult: null,
    actions: ACTIONS,
    requestText: null,
    linkedRequest: null,
    notVerifiedReason: null,
    runbookProposal: null,
    resumedFrom: null,
    callAttached: true,
    undelivered: 0,
    createdAt: '2026-09-09T09:00:00.000Z',
    closedAt: null,
    ...overrides,
  };
}

/** A bridge serving one guided handoff, handing back the focus handler the app registered. */
function focusBridge(
  handoff: HandoffView = view(),
): { bridge: Bridge; focus: (focused: boolean) => void } {
  let handler: ((focused: boolean) => void) | null = null;
  const bridge = servingBridge([handoff.tab], { [handoff.tab.id]: handoff });
  bridge.onWindowFocus = vi.fn(
    async (next: (focused: boolean) => void): Promise<Unlisten> => {
      handler = next;
      return () => {
        handler = null;
      };
    },
  );
  return { bridge, focus: (focused) => handler?.(focused) };
}

/** Renders the application over `bridge` and waits for the first paint of the step. */
async function open(bridge: Bridge): Promise<void> {
  setBridge(bridge);
  render(App);
  await waitFor(() =>
    expect(screen.getByText('Open the dashboard and add the endpoint.')).toBeDefined(),
  );
}

beforeEach(() => {
  resetView();
  resetOverlay();
  resetCollapse();
  resetForm();
  setLanguage(DEFAULT_LANGUAGE);
});

afterEach(() => {
  cleanup();
  setBridge(null);
  resetOverlay();
  resetCollapse();
  resetForm();
  vi.useRealTimers();
});

describe('the collapsed bar (WIN-03)', () => {
  it('shrinks to one line with Done, Ask and Screenshot when the window loses the focus', async () => {
    const { bridge, focus } = focusBridge();
    await open(bridge);
    expect(document.querySelector('.collapsed')).toBeNull();

    focus(false);
    await tick();

    const bar = document.querySelector('.collapsed');
    expect(bar).not.toBeNull();
    expect(bar?.textContent).toContain('Open the dashboard and add the endpoint.');
    expect(bar?.textContent).toContain('Step 1 of 2');

    // Ask and Screenshot are icon-only on the bar, so what is read is the accessible name
    // and not the text: an icon with no label is a button nobody can name.
    const names = Array.from(bar?.querySelectorAll('button') ?? []).map((button) =>
      (button.getAttribute('aria-label') ?? button.textContent ?? '').replace(/\s+/g, ' ').trim(),
    );
    expect(names).toEqual([
      'Step 1 of 2 Open the dashboard and add the endpoint.',
      'Done',
      'Ask',
      'Screenshot',
      'Minimize to tray',
      'Open the panel',
    ]);
    // WIN-03 keeps the other four in the expanded panel only.
    for (const absent of ['Note', 'Skip', 'Defer', 'Abandon', 'More']) {
      expect(names).not.toContain(absent);
    }
    // The header goes with the panel: the bar is the whole window.
    expect(document.querySelector('.header')).toBeNull();
  });

  it('puts the window away from the bar, through the same path as closing it (WIN-04)', async () => {
    const { bridge, focus } = focusBridge();
    await open(bridge);
    focus(false);
    await tick();

    screen.getByRole('button', { name: 'Minimize to tray' }).click();
    await waitFor(() => expect(bridge.hideWindow).toHaveBeenCalled());
    // Putting the window away is not collapsing it: the bar is what comes back.
    expect(document.querySelector('.collapsed')).not.toBeNull();
  });

  it('keeps the bar at the width of the panel, whatever form the window was in (§7.6)', async () => {
    const { bridge, focus } = focusBridge();
    expandForm();
    await open(bridge);
    await waitFor(() => expect(bridge.setWindowLayout).toHaveBeenLastCalledWith('expanded'));

    focus(false);
    await tick();
    await waitFor(() => expect(bridge.setWindowLayout).toHaveBeenLastCalledWith('panel'));

    // And coming back to the window comes back to the form it was in.
    focus(true);
    await tick();
    await waitFor(() => expect(bridge.setWindowLayout).toHaveBeenLastCalledWith('expanded'));
  });

  it('re-expands when the window is clicked back into the focus', async () => {
    const { bridge, focus } = focusBridge();
    await open(bridge);
    focus(false);
    await tick();
    expect(isCollapsed()).toBe(true);

    focus(true);
    await tick();
    expect(document.querySelector('.collapsed')).toBeNull();
    expect(document.querySelector('.header')).not.toBeNull();
  });

  it('re-expands when the bar itself is clicked', async () => {
    const { bridge, focus } = focusBridge();
    await open(bridge);
    focus(false);
    await tick();

    // The step line is itself the way back; the control at the right end repeats it.
    (document.querySelector('.collapsed-line') as HTMLButtonElement).click();
    await tick();
    expect(document.querySelector('.collapsed')).toBeNull();

    focus(false);
    await tick();
    screen.getByRole('button', { name: 'Open the panel' }).click();
    await tick();
    expect(document.querySelector('.collapsed')).toBeNull();
  });

  it('sends the step action from the bar without expanding first', async () => {
    const { bridge, focus } = focusBridge();
    await open(bridge);
    focus(false);
    await tick();

    const bar = document.querySelector('.collapsed');
    Array.from(bar?.querySelectorAll('button') ?? [])
      .find((button) => button.textContent?.trim() === 'Done')
      ?.click();

    await waitFor(() => expect(bridge.act).toHaveBeenCalledWith(ID, 'confirm'));
    expect(document.querySelector('.collapsed')).not.toBeNull();
  });

  it('opens the panel for Ask, because the question needs the sheet', async () => {
    const { bridge, focus } = focusBridge();
    await open(bridge);
    focus(false);
    await tick();

    screen.getByRole('button', { name: 'Ask' }).click();
    await tick();

    expect(document.querySelector('.collapsed')).toBeNull();
  });

  it('does not collapse a window with no step to show', async () => {
    // The settings page and a final tab have no "current step"; a bar showing nothing is
    // not an improvement on a panel.
    const { bridge, focus } = focusBridge(
      view({ state: 'verified', uiState: 'final', step: null, goal: 'Register the webhook' }),
    );
    setBridge(bridge);
    render(App);
    await waitFor(() => expect(screen.getByText('Register the webhook')).toBeDefined());

    focus(false);
    await tick();
    expect(document.querySelector('.collapsed')).toBeNull();
  });
});

describe('the fallback collapse (R-10)', () => {
  it('is off unless the setting says otherwise', async () => {
    vi.useFakeTimers();
    const { bridge } = focusBridge();
    setBridge(bridge);
    render(App);
    await vi.advanceTimersByTimeAsync(1);
    expect(fallbackIsOn()).toBe(false);

    // Well past the three seconds; nothing collapses, because nothing armed a timer.
    await vi.advanceTimersByTimeAsync(10_000);
    expect(isCollapsed()).toBe(false);
  });

  it('collapses three seconds after the last interaction when it is switched on', async () => {
    vi.useFakeTimers();
    const { bridge } = focusBridge();
    bridge.windowSettings = vi.fn(async () => ({
      collapseFallback: true,
      collapseFallbackMs: 3_000,
    }));
    setBridge(bridge);
    render(App);
    await vi.advanceTimersByTimeAsync(1);
    expect(fallbackIsOn()).toBe(true);
    expect(fallbackDelay()).toBe(3_000);

    await vi.advanceTimersByTimeAsync(2_500);
    expect(isCollapsed()).toBe(false);

    // Any interaction restarts it: a user reading the panel is not idle.
    document.dispatchEvent(new Event('pointerdown', { bubbles: true }));
    await vi.advanceTimersByTimeAsync(2_500);
    expect(isCollapsed()).toBe(false);

    await vi.advanceTimersByTimeAsync(1_000);
    expect(isCollapsed()).toBe(true);
  });
});
