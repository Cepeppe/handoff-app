/**
 * The title bar's three window controls, and the shape of the window they decide (§7.6).
 *
 * The window has no decorations, so these three buttons *are* its system buttons, and two
 * promises hang on them:
 *
 * - **They are always the same three, in the same order, on every screen.** A control that
 *   would do nothing where it is shown is dimmed and left where it is, with a `title` that
 *   says why — never hidden and never `disabled`, because a disabled button is out of the tab
 *   order and shows no tooltip, and the tooltip is the whole explanation.
 * - **The window has one width, derived in one place.** `App.svelte` turns "which view, which
 *   form, collapsed or not" into a single `setWindowLayout` call, so the settings page, the
 *   expanded view and the collapsed bar can never disagree about how wide the window is.
 *
 * The Rust side is the one that applies a width, and it is not here: what is checked is the
 * word this side sends it.
 */
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import App from '../App.svelte';
import { setBridge, type Bridge, type Unlisten } from '../bridge';
import { DEFAULT_LANGUAGE, setLanguage, t } from '../i18n';
import type { ActionsView, HandoffView, TabView } from '../model';
import { resetCollapse } from '../overlay/collapse.svelte';
import { resetOverlay } from '../overlay/state.svelte';
import { resetView, showView } from '../view-state.svelte';
import { form, resetForm } from '../window-form.svelte';
import type { ViewName } from '../views';
import { fakeBridge, servingBridge } from './fake-bridge';

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

/** A bridge serving one guided handoff, with the focus handler in hand. */
function guidedBridge(handoff: HandoffView = view()): {
  bridge: Bridge;
  focus: (focused: boolean) => void;
} {
  let handler: ((focused: boolean) => void) | null = null;
  const bridge = servingBridge([handoff.tab], { [handoff.tab.id]: handoff });
  bridge.onWindowFocus = vi.fn(async (next: (focused: boolean) => void): Promise<Unlisten> => {
    handler = next;
    return () => {
      handler = null;
    };
  });
  return { bridge, focus: (focused) => handler?.(focused) };
}

/** The three controls, in the order the header draws them. */
function controls(): HTMLButtonElement[] {
  const group = screen.getByRole('group', { name: t('window.controls') });
  return [...group.querySelectorAll('button')];
}

/** Whether each of the three is available, in order: Minimize, Shrink, Expand/Restore. */
function available(): boolean[] {
  return controls().map((button) => button.getAttribute('aria-disabled') !== 'true');
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
});

describe('the window controls (§7.6)', () => {
  it('draws the same three, in the same order, on every screen', async () => {
    setBridge(fakeBridge());
    render(App);
    await tick();

    for (const name of ['overlay', 'request', 'settings', 'onboarding', 'preview'] as ViewName[]) {
      showView(name);
      await tick();
      expect(
        controls().map((button) => button.getAttribute('aria-label')),
        `the controls of ${name}`,
      ).toEqual([t('window.minimize'), t('window.collapse'), t('window.expand')]);
    }
  });

  it('offers all three over a guided step', async () => {
    const { bridge } = guidedBridge();
    setBridge(bridge);
    render(App);
    await waitFor(() =>
      expect(screen.getByText('Open the dashboard and add the endpoint.')).toBeDefined(),
    );

    expect(available()).toEqual([true, true, true]);
    expect(controls()[2]?.getAttribute('title')).toBe(t('window.expand'));
  });

  it('refuses the bar with no step to shrink to, and says why', async () => {
    // An empty overlay, a tab waiting for its spec and a final one have no "current step":
    // a bar showing nothing is not an improvement on a panel (WIN-03).
    setBridge(fakeBridge());
    render(App);
    await tick();

    expect(available()).toEqual([true, false, true]);
    expect(controls()[1]?.getAttribute('title')).toBe(t('window.collapseUnavailable'));
    // It is dimmed, not taken away, and it is not `disabled`: a disabled button shows no
    // tooltip, and the tooltip is the explanation.
    expect(controls()[1]?.hasAttribute('disabled')).toBe(false);
  });

  it('refuses the expanded view outside the overlay, and says why', async () => {
    setBridge(fakeBridge());
    render(App);

    for (const name of ['request', 'settings', 'onboarding', 'preview'] as ViewName[]) {
      showView(name);
      await tick();
      expect(available(), `the controls of ${name}`).toEqual([true, false, false]);
      expect(controls()[2]?.getAttribute('title')).toBe(t('window.expandUnavailable'));
    }
  });

  it('ignores a press on a control that is not available', async () => {
    const bridge = fakeBridge();
    setBridge(bridge);
    render(App);
    showView('settings');
    await tick();

    controls()[2]?.click();
    await tick();
    expect(form()).toBe('panel');
  });

  it('puts the window away through the same path as closing it (WIN-04)', async () => {
    const bridge = fakeBridge();
    setBridge(bridge);
    render(App);
    await tick();

    screen.getByRole('button', { name: t('window.minimize') }).click();
    await waitFor(() => expect(bridge.hideWindow).toHaveBeenCalled());
  });

  it('shrinks to the bar on command, not only when the focus goes elsewhere (WIN-03)', async () => {
    const { bridge } = guidedBridge();
    setBridge(bridge);
    render(App);
    await waitFor(() =>
      expect(screen.getByText('Open the dashboard and add the endpoint.')).toBeDefined(),
    );

    screen.getByRole('button', { name: t('window.collapse') }).click();
    await tick();
    expect(document.querySelector('.collapsed')).not.toBeNull();
  });

  it('turns Expand into Restore, and says so to a screen reader', async () => {
    const { bridge } = guidedBridge();
    setBridge(bridge);
    render(App);
    await waitFor(() =>
      expect(screen.getByText('Open the dashboard and add the endpoint.')).toBeDefined(),
    );

    screen.getByRole('button', { name: t('window.expand') }).click();
    await tick();

    const restore = screen.getByRole('button', { name: t('window.restore') });
    expect(restore.getAttribute('aria-pressed')).toBe('true');
    expect(document.querySelector('.view-expanded')).not.toBeNull();

    restore.click();
    await tick();
    expect(screen.getByRole('button', { name: t('window.expand') })).toBeDefined();
    expect(document.querySelector('.view-expanded')).toBeNull();
  });
});

describe('the width the window asks for (§7.6, WIN-02)', () => {
  it('opens at the width of the panel', async () => {
    const bridge = fakeBridge();
    setBridge(bridge);
    render(App);
    await waitFor(() => expect(bridge.setWindowLayout).toHaveBeenCalledWith('panel'));
  });

  it('is the settings page wherever the settings are opened from', async () => {
    const bridge = fakeBridge();
    setBridge(bridge);
    render(App);
    showView('settings');
    await waitFor(() => expect(bridge.setWindowLayout).toHaveBeenLastCalledWith('settings'));
  });

  it('is expanded only in the overlay, and comes back to it when the overlay does', async () => {
    const { bridge } = guidedBridge();
    setBridge(bridge);
    render(App);
    await waitFor(() =>
      expect(screen.getByText('Open the dashboard and add the endpoint.')).toBeDefined(),
    );

    screen.getByRole('button', { name: t('window.expand') }).click();
    await waitFor(() => expect(bridge.setWindowLayout).toHaveBeenLastCalledWith('expanded'));

    // The request sheet is not an expanded screen, whatever the form says.
    showView('request');
    await waitFor(() => expect(bridge.setWindowLayout).toHaveBeenLastCalledWith('panel'));

    // And coming back to the overlay comes back to the form, which lasts the session.
    resetView();
    await waitFor(() => expect(bridge.setWindowLayout).toHaveBeenLastCalledWith('expanded'));
  });

  it('keeps the expanded form across a trip to the bar and back', async () => {
    const { bridge, focus } = guidedBridge();
    setBridge(bridge);
    render(App);
    await waitFor(() =>
      expect(screen.getByText('Open the dashboard and add the endpoint.')).toBeDefined(),
    );

    screen.getByRole('button', { name: t('window.expand') }).click();
    await tick();
    expect(form()).toBe('expanded');

    focus(false);
    await tick();
    expect(document.querySelector('.collapsed')).not.toBeNull();
    // The bar is 360 wide whatever the window was.
    await waitFor(() => expect(bridge.setWindowLayout).toHaveBeenLastCalledWith('panel'));

    focus(true);
    await tick();
    expect(document.querySelector('.view-expanded')).not.toBeNull();
    await waitFor(() => expect(bridge.setWindowLayout).toHaveBeenLastCalledWith('expanded'));
  });

  it('is not changed by a handoff arriving', async () => {
    // A tab appearing beside the one being worked on raises a badge and waits (MULTI-03);
    // it does not resize the window somebody is reading.
    const { bridge } = guidedBridge();
    setBridge(bridge);
    render(App);
    await waitFor(() =>
      expect(screen.getByText('Open the dashboard and add the endpoint.')).toBeDefined(),
    );
    screen.getByRole('button', { name: t('window.expand') }).click();
    await waitFor(() => expect(bridge.setWindowLayout).toHaveBeenLastCalledWith('expanded'));

    vi.mocked(bridge.listHandoffs).mockResolvedValue([
      tab(),
      tab({ id: 'hf_0000000002', label: 'Codex · web' }),
    ]);
    await tick();

    expect(form()).toBe('expanded');
    expect(bridge.setWindowLayout).toHaveBeenLastCalledWith('expanded');
  });
});
