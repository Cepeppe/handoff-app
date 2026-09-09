/**
 * The request sheet, the FM-18 shortcut dialog and the FM-20 **Change** control (§7.7).
 *
 * Everything here is rendered under jsdom with a fake core behind it: what is checked is
 * what the window *asks the core to do* — which command, with which arguments — because
 * that is the whole contract between the two sides. What the core then does with a request
 * is the queue's tests and the store's.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setBridge } from '../bridge';
import { DEFAULT_LANGUAGE, setLanguage, t } from '../i18n';
import type { ActionsView, HandoffView, SessionChoice, TabView } from '../model';
import ShortcutDialog from '../overlay/ShortcutDialog.svelte';
import { refreshTabs, resetOverlay, selectedId } from '../overlay/state.svelte';
import { resetView, view } from '../view-state.svelte';
import OverlayView from '../views/OverlayView.svelte';
import RequestView from '../views/RequestView.svelte';
import { fakeBridge, servingBridge } from './fake-bridge';

const ONE: SessionChoice = { sessionRef: 'ses_00000001', label: 'Claude Code · baton' };
const TWO: SessionChoice = { sessionRef: 'ses_00000002', label: 'Codex · shop' };

const ACTIONS: ActionsView = {
  done: true,
  ask: true,
  note: true,
  skip: true,
  defer: true,
  abandon: true,
  screenshot: false,
  resume: false,
  closeOrphan: false,
};

beforeEach(() => {
  resetView();
  resetOverlay();
  setLanguage(DEFAULT_LANGUAGE);
});

afterEach(() => {
  cleanup();
  setBridge(null);
});

/** The one field of OPEN-04. */
function field(): HTMLInputElement {
  return screen.getByLabelText(t('request.what'));
}

/** One turn of the event loop, which is every microtask a send has left to run. */
function settled(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

describe('the request sheet (OPEN-04, OPEN-04a, OPEN-05)', () => {
  it('pre-selects the session when there is only one', async () => {
    setBridge(fakeBridge({ sessions: vi.fn(async () => [ONE]) }));
    render(RequestView);

    const selector = await screen.findByLabelText<HTMLSelectElement>(t('request.session'));
    expect(selector.value).toBe(ONE.sessionRef);
    expect(screen.queryByText(t('request.noSession'))).toBeNull();
  });

  it('offers every session when there is more than one', async () => {
    setBridge(fakeBridge({ sessions: vi.fn(async () => [ONE, TWO]) }));
    render(RequestView);

    const selector = await screen.findByLabelText<HTMLSelectElement>(t('request.session'));
    expect([...selector.options].map((option) => option.textContent?.trim())).toEqual([
      ONE.label,
      TWO.label,
    ]);
  });

  it('opens with no session at all and says so (OPEN-04a)', async () => {
    setBridge(fakeBridge({ sessions: vi.fn(async () => []) }));
    render(RequestView);

    expect(await screen.findByText(t('request.noSession'))).toBeTruthy();
    expect(screen.queryByLabelText(t('request.session'))).toBeNull();
    // The field is still there: the request is queued for the first session that starts.
    expect(field()).toBeTruthy();
  });

  it('sends on Enter, with the text and the chosen session', async () => {
    const createRequest = vi.fn(async () => 'hf_9p2r4k7m3t');
    setBridge(fakeBridge({ sessions: vi.fn(async () => [TWO]), createRequest }));
    render(RequestView);
    await screen.findByLabelText(t('request.session'));

    await fireEvent.input(field(), { target: { value: '  create the API key  ' } });
    await fireEvent.keyDown(field(), { key: 'Enter' });

    await waitFor(() => {
      expect(createRequest).toHaveBeenCalledWith('create the API key', TWO.sessionRef);
    });
  });

  it('queues the request for nobody when no session is registered (OPEN-04a)', async () => {
    const createRequest = vi.fn(async () => 'hf_9p2r4k7m3t');
    setBridge(fakeBridge({ sessions: vi.fn(async () => []), createRequest }));
    render(RequestView);
    await screen.findByText(t('request.noSession'));

    await fireEvent.input(field(), { target: { value: 'book the domain' } });
    await fireEvent.keyDown(field(), { key: 'Enter' });

    await waitFor(() => {
      expect(createRequest).toHaveBeenCalledWith('book the domain', null);
    });
  });

  it('refuses an empty request, on Enter and on the button', async () => {
    const createRequest = vi.fn(async () => 'hf_9p2r4k7m3t');
    setBridge(fakeBridge({ sessions: vi.fn(async () => [ONE]), createRequest }));
    render(RequestView);
    await screen.findByLabelText(t('request.session'));

    await fireEvent.input(field(), { target: { value: '   ' } });
    await fireEvent.keyDown(field(), { key: 'Enter' });
    await screen.findByText(t('request.empty'));

    const send = screen.getByRole('button', { name: t('request.send') });
    expect(send.hasAttribute('disabled')).toBe(true);
    await fireEvent.click(send);

    expect(createRequest).not.toHaveBeenCalled();
  });

  it('cancels on Esc, sending nothing', async () => {
    const createRequest = vi.fn(async () => 'hf_9p2r4k7m3t');
    setBridge(fakeBridge({ sessions: vi.fn(async () => [ONE]), createRequest }));
    render(RequestView);
    await screen.findByLabelText(t('request.session'));

    await fireEvent.input(field(), { target: { value: 'create the API key' } });
    await fireEvent.keyDown(field(), { key: 'Escape' });

    expect(createRequest).not.toHaveBeenCalled();
    // Esc puts the window back on the overlay: the sheet is a mode of it (DD-10).
    expect(view()).toBe('overlay');
  });

  it('does not ask for the front, so the terminal it raised keeps it (OPEN-05)', async () => {
    // Measured against the running app: `create_request` brought the session's terminal
    // forward and MULTI-03 — "a handoff arrived while none was active" — put the overlay
    // back over it a tick later, so the user pasted into the wrong window. The tab the
    // sheet creates is not a handoff arriving: they typed it here a moment ago.
    const created: TabView = {
      id: 'hf_9p2r4k7m3t',
      label: 'Claude Code · baton',
      agent: 'Claude Code',
      project: 'baton',
      state: 'awaiting_spec',
      uiState: 'waitingForSpec',
      group: 'open',
      goal: null,
      orphan: false,
      actions: { ...ACTIONS, done: false, ask: false, skip: false, defer: false },
      createdAt: '2026-09-09T09:00:00.000Z',
    };
    let strip: TabView[] = [];
    const showWindow = vi.fn(async () => {});
    setBridge(
      fakeBridge({
        sessions: vi.fn(async () => [ONE]),
        createRequest: vi.fn(async () => {
          strip = [created];
          return created.id;
        }),
        listHandoffs: vi.fn(async () => strip),
        showWindow,
      }),
    );
    // The window has read the strip once already: only a later read can raise it.
    await refreshTabs();

    render(RequestView);
    await screen.findByLabelText(t('request.session'));
    await fireEvent.input(field(), { target: { value: 'create the API key' } });
    await fireEvent.keyDown(field(), { key: 'Enter' });

    // A macrotask, not a `waitFor`: everything the send awaits is an already-resolved
    // promise, so one turn of the event loop runs all of it — and an assertion about a call
    // that must *not* happen has to stand after the last moment it could have.
    await settled();
    expect(selectedId()).toBe(created.id);
    expect(showWindow).not.toHaveBeenCalled();
  });

  it('says so when the request could not be opened', async () => {
    setBridge(
      fakeBridge({
        sessions: vi.fn(async () => [ONE]),
        createRequest: vi.fn(async () => {
          throw new Error('the log is not available');
        }),
      }),
    );
    render(RequestView);
    await screen.findByLabelText(t('request.session'));

    await fireEvent.input(field(), { target: { value: 'create the API key' } });
    await fireEvent.keyDown(field(), { key: 'Enter' });

    expect(await screen.findByText(t('request.failed'))).toBeTruthy();
  });
});

describe('the FM-18 shortcut dialog (OPEN-03)', () => {
  it('records a combination and offers it to the core', async () => {
    const setShortcut = vi.fn(async () => {});
    setBridge(fakeBridge({ setShortcut }));
    const done = vi.fn();
    render(ShortcutDialog, { props: { accelerator: 'Control+Alt+H', ondone: done } });

    const recorder = screen.getByRole('button', { name: t('shortcut.record') });
    // A modifier on its own is not a combination.
    await fireEvent.keyDown(recorder, { key: 'Control', code: 'ControlLeft', ctrlKey: true });
    expect(screen.getByRole('button', { name: t('shortcut.record') })).toBeTruthy();

    await fireEvent.keyDown(recorder, { key: 'j', code: 'KeyJ', ctrlKey: true, altKey: true });
    await tick();
    const save = screen.getByRole('button', { name: t('shortcut.save') });
    expect(save.hasAttribute('disabled')).toBe(false);

    await fireEvent.click(save);
    await waitFor(() => expect(setShortcut).toHaveBeenCalledWith('Control+Alt+KeyJ'));
    expect(done).toHaveBeenCalled();
  });

  it('never offers a bare key, which would be swallowed everywhere', async () => {
    setBridge(fakeBridge());
    render(ShortcutDialog, { props: { accelerator: 'Control+Alt+H', ondone: vi.fn() } });

    const recorder = screen.getByRole('button', { name: t('shortcut.record') });
    await fireEvent.keyDown(recorder, { key: 'h', code: 'KeyH' });
    await tick();

    expect(screen.getByRole('button', { name: t('shortcut.save') }).hasAttribute('disabled')).toBe(
      true,
    );
  });

  it('stays open and says so when the combination is not free either', async () => {
    setBridge(
      fakeBridge({
        setShortcut: vi.fn(async () => {
          throw new Error('HotKey already registered');
        }),
      }),
    );
    const done = vi.fn();
    render(ShortcutDialog, { props: { accelerator: 'Control+Alt+H', ondone: done } });

    const recorder = screen.getByRole('button', { name: t('shortcut.record') });
    await fireEvent.keyDown(recorder, { key: 'j', code: 'KeyJ', ctrlKey: true, altKey: true });
    await tick();
    await fireEvent.click(screen.getByRole('button', { name: t('shortcut.save') }));

    expect(await screen.findByText(t('shortcut.failed'))).toBeTruthy();
    expect(done).not.toHaveBeenCalled();
  });

  it('is asked once: dismissing it tells the core never to ask again', async () => {
    const dismissShortcutQuestion = vi.fn(async () => {});
    setBridge(fakeBridge({ dismissShortcutQuestion }));
    const done = vi.fn();
    render(ShortcutDialog, { props: { accelerator: 'Control+Alt+H', ondone: done } });

    await fireEvent.click(screen.getByRole('button', { name: t('shortcut.dismiss') }));

    await waitFor(() => expect(dismissShortcutQuestion).toHaveBeenCalled());
    expect(done).toHaveBeenCalled();
  });
});

describe('the Change control of FM-20', () => {
  const ID = 'hf_0000000001';

  function tab(): TabView {
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
    };
  }

  function linked(): HandoffView {
    return {
      tab: tab(),
      state: 'active',
      uiState: 'guiding',
      banner: null,
      goal: 'Register the webhook',
      location: null,
      url: null,
      lang: 'en',
      step: null,
      secrets: [],
      notes: [],
      pending: null,
      history: [],
      verify: null,
      verifyResult: null,
      actions: ACTIONS,
      requestText: 'create the API key',
      linkedRequest: { id: 'hf_9p2r4k7m3t', text: 'create the API key' },
      runbookProposal: null,
      resumedFrom: null,
      callAttached: true,
      undelivered: 0,
      createdAt: '2026-09-09T09:00:00.000Z',
      closedAt: null,
    };
  }

  it('lists the other open requests and relinks in one click', async () => {
    const act = vi.fn(async () => {});
    const openRequests = vi.fn(async () => [
      { id: 'hf_9p2r4k7m3t', text: 'create the API key', createdAt: '2026-09-09T09:00:00.000Z' },
      { id: 'hf_1a2b3c4d5e', text: 'add the webhook', createdAt: '2026-09-09T09:02:00.000Z' },
    ]);
    const serving = servingBridge([tab()], { [ID]: linked() });
    setBridge({ ...serving, act, openRequests });
    render(OverlayView);

    await fireEvent.click(await screen.findByRole('button', { name: t('overlay.changeRequest') }));
    await waitFor(() => expect(openRequests).toHaveBeenCalledWith(ID));

    await fireEvent.click(await screen.findByRole('button', { name: 'add the webhook' }));
    await waitFor(() => expect(act).toHaveBeenCalledWith(ID, 'relink', 'hf_1a2b3c4d5e'));
  });

  it('says when a session has no other request to move it to', async () => {
    const serving = servingBridge([tab()], { [ID]: linked() });
    setBridge({ ...serving, openRequests: vi.fn(async () => []) });
    render(OverlayView);

    await fireEvent.click(await screen.findByRole('button', { name: t('overlay.changeRequest') }));
    expect(await screen.findByText(t('overlay.noOtherRequest'))).toBeTruthy();
  });
});
