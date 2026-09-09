/**
 * The overlay against a mocked bridge (§7.6, GUIDE-01..06, RESP-01..05, SEC-02, DET-04).
 *
 * Everything here is a component rendered under jsdom with a fake core behind it: what is
 * checked is what the window *asks the core to do* — which command, with which arguments —
 * because that is the whole contract between the two sides. What the core then does is the
 * view model's tests and the store's.
 */
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setBridge } from '../bridge';
import { DEFAULT_LANGUAGE, setLanguage } from '../i18n';
import type { ActionsView, HandoffView, TabView } from '../model';
import { REVEAL_MS, resetOverlay } from '../overlay/state.svelte';
import OverlayView from '../views/OverlayView.svelte';
import { fakeBridge, servingBridge, type FakeEvents } from './fake-bridge';

const ID = 'hf_0000000001';

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
    location: 'Dashboard → Webhooks',
    url: null,
    lang: 'en',
    step: {
      counter: { key: 'counter.step', index: 1, total: 2, round: 1 },
      text: 'Open the dashboard and add the endpoint.',
      warning: null,
      url: { href: 'https://dashboard.example.test/webhooks', openable: true },
      values: [
        {
          name: 'endpoint_url',
          masked: false,
          kind: null,
          list: false,
          items: ['https://api.example.test/hook'],
        },
        {
          name: 'events',
          masked: false,
          kind: null,
          list: true,
          items: ['payment.succeeded', 'payment.failed'],
        },
        { name: 'api_key', masked: true, kind: 'api_key', list: false, items: ['••••••'] },
      ],
      confirmed: false,
      skipped: false,
      notes: [],
      replies: [],
      last: false,
    },
    secrets: [{ name: 'STRIPE_SIGNING_SECRET', file: '.env.local' }],
    notes: [],
    pending: null,
    history: [],
    verify: 'the webhook fires',
    verifyResult: null,
    actions: ACTIONS,
    requestText: null,
    linkedRequest: null,
    resumedFrom: null,
    callAttached: true,
    undelivered: 0,
    createdAt: '2026-09-09T09:00:00.000Z',
    closedAt: null,
    ...overrides,
  };
}

/** Renders the overlay over a core that serves one handoff, and waits for the first paint. */
async function open(handoff: HandoffView = view(), events: FakeEvents = {}) {
  const bridge = servingBridge([handoff.tab], { [handoff.tab.id]: handoff }, events);
  setBridge(bridge);
  render(OverlayView);
  await waitFor(() => expect(screen.getByRole('heading', { level: 1 })).toBeDefined());
  return bridge;
}

beforeEach(() => {
  resetOverlay();
  setLanguage(DEFAULT_LANGUAGE);
});

afterEach(() => {
  cleanup();
  setBridge(null);
  resetOverlay();
});

describe('the overlay', () => {
  it('says so when there is nothing to do', async () => {
    setBridge(fakeBridge());
    render(OverlayView);
    await waitFor(() => expect(screen.getByText('Nothing to do yet.')).toBeDefined());
  });

  it('shows one step at a time, with the counter in words', async () => {
    await open();
    expect(screen.getByText('Step 1 of 2')).toBeDefined();
    expect(screen.getByText('Open the dashboard and add the endpoint.')).toBeDefined();
    // GUIDE-05, PRIN-07: there is no bar and no estimate anywhere.
    expect(document.querySelector('progress')).toBeNull();
  });

  it('copies a value as a whole and an array item by item (GUIDE-02)', async () => {
    const bridge = await open();

    screen.getAllByRole('button', { name: 'Copy' })[0]?.click();
    await tick();
    expect(bridge.copyValue).toHaveBeenCalledWith(ID, 'endpoint_url');

    screen.getByRole('button', { name: 'Copy this one: events 2' }).click();
    await tick();
    expect(bridge.copyValue).toHaveBeenCalledWith(ID, 'events', 1);
  });

  it('shows a secret-treated value masked, and reveals it only when asked (DET-04)', async () => {
    const bridge = await open();
    const chip = document.querySelector('[data-value="api_key"]');
    expect(chip?.textContent).toContain('••••••');
    expect(chip?.textContent).not.toContain('sk_live');

    vi.mocked(bridge.revealValue).mockResolvedValue(['sk_live_0123456789abcdef']);
    screen.getByRole('button', { name: 'Show' }).click();
    await waitFor(() =>
      expect(
        document.querySelector('[data-value="api_key"]')?.textContent,
      ).toContain('sk_live_0123456789abcdef'),
    );
    expect(bridge.revealValue).toHaveBeenCalledWith(ID, 'api_key');

    // Copy still copies the true value: the mask hides it from the screen, it does not
    // withhold it from the user who has to paste it.
    screen.getAllByRole('button', { name: 'Copy' })[2]?.click();
    await tick();
    expect(bridge.copyValue).toHaveBeenCalledWith(ID, 'api_key');
  });

  it('offers Open for an allowed scheme and plain text for anything else (SPEC-07)', async () => {
    const bridge = await open();
    screen.getByRole('button', { name: 'Open' }).click();
    await tick();
    expect(bridge.openUrl).toHaveBeenCalledWith('https://dashboard.example.test/webhooks');

    cleanup();
    resetOverlay();
    const refused = view();
    refused.step = { ...refused.step!, url: { href: 'file:///etc/passwd', openable: false } };
    await open(refused);

    expect(screen.queryByRole('button', { name: 'Open' })).toBeNull();
    expect(screen.getByText('file:///etc/passwd')).toBeDefined();
    expect(document.querySelector('a[href^="file:"]')).toBeNull();
  });

  it('auto-links an https url in the step text and opens it through the core', async () => {
    const linked = view();
    linked.step = {
      ...linked.step!,
      text: 'Open https://dashboard.example.test/keys and copy the value.',
      url: null,
    };
    const bridge = await open(linked);

    const anchor = screen.getByRole('link', { name: 'https://dashboard.example.test/keys' });
    anchor.click();
    await tick();
    expect(bridge.openUrl).toHaveBeenCalledWith('https://dashboard.example.test/keys');
  });

  it('opens the file of a secrets entry by its name, never by a path (SEC-02)', async () => {
    const bridge = await open();
    screen.getByRole('button', { name: 'Open file: .env.local' }).click();
    await tick();
    expect(bridge.openSecretFile).toHaveBeenCalledWith(ID, 'STRIPE_SIGNING_SECRET');
  });

  it('keeps Ask and Note as two different buttons and two different actions (RESP-02)', async () => {
    const bridge = await open();

    screen.getByRole('button', { name: 'Note' }).click();
    await tick();
    await type('a note on this step');
    screen.getByRole('button', { name: 'Save' }).click();
    await waitFor(() => expect(bridge.act).toHaveBeenCalledWith(ID, 'note', 'a note on this step'));

    screen.getByRole('button', { name: 'Ask' }).click();
    await tick();
    await type('which button is it?');
    screen.getByRole('button', { name: 'Send' }).click();
    await waitFor(() =>
      expect(bridge.act).toHaveBeenCalledWith(ID, 'ask', 'which button is it?'),
    );
  });

  it('sends the redacted text and shows the user what will be sent (§7.10)', async () => {
    const bridge = await open();
    vi.mocked(bridge.scanTypedText).mockResolvedValue({
      text: 'I pasted [REDACTED:api_key]',
      kinds: ['api_key'],
    });

    screen.getByRole('button', { name: 'Ask' }).click();
    await tick();
    await type('I pasted AKIAIOSFODNN7EXAMPLE');

    await waitFor(() => expect(screen.getByText('I pasted [REDACTED:api_key]')).toBeDefined());
    expect(screen.getByText('A secret was taken out before sending: api_key')).toBeDefined();

    screen.getByRole('button', { name: 'Send' }).click();
    await waitFor(() =>
      expect(bridge.act).toHaveBeenCalledWith(ID, 'ask', 'I pasted [REDACTED:api_key]'),
    );
  });

  it('does not scan a note, which stays on this machine (RESP-03)', async () => {
    const bridge = await open();
    screen.getByRole('button', { name: 'Note' }).click();
    await tick();
    await type('the button moved');
    expect(bridge.scanTypedText).not.toHaveBeenCalled();
  });

  it('advances with Done and ends the round with it on the last step (GUIDE-01, RESP-09)', async () => {
    // One button, two transitions of §8.1: `confirm` walks to the next step, `done` on the
    // last one closes the round and moves the tab to verifying. Sending `confirm` there
    // would leave the cursor where it is and the handoff open for ever.
    const bridge = await open();
    screen.getByRole('button', { name: 'Done' }).click();
    await waitFor(() => expect(bridge.act).toHaveBeenCalledWith(ID, 'confirm', undefined));

    cleanup();
    resetOverlay();
    const last = view();
    last.step = { ...last.step!, counter: { ...last.step!.counter, index: 2 }, last: true };
    const onLast = await open(last);
    screen.getByRole('button', { name: 'Done' }).click();
    await waitFor(() => expect(onLast.act).toHaveBeenCalledWith(ID, 'done', undefined));
  });

  it('skips and defers straight through, with no sheet for the first', async () => {
    const bridge = await open();

    screen.getByRole('button', { name: 'Skip' }).click();
    await waitFor(() => expect(bridge.act).toHaveBeenCalledWith(ID, 'skip', undefined));

    screen.getByRole('button', { name: 'Defer' }).click();
    await tick();
    // RESP-05: a reason is welcome and never required.
    screen.getByRole('button', { name: 'Send' }).click();
    await waitFor(() => expect(bridge.act).toHaveBeenCalledWith(ID, 'defer', ''));
  });

  it('draws Screenshot disabled, with a tooltip that says why', async () => {
    await open();
    const button = screen.getByRole('button', { name: 'Screenshot' });
    expect(button.hasAttribute('disabled')).toBe(true);
    expect(button.getAttribute('title')).toBe('Screenshots are not built yet.');
  });

  it('draws only the buttons the state offers (§8.4)', async () => {
    const parked = view({
      state: 'parked',
      uiState: 'parked',
      banner: { key: 'banner.parked', arg: null },
      actions: { ...ACTIONS, done: false, ask: false, note: false, skip: false, defer: false, resume: true },
    });
    await open(parked);

    expect(screen.getByText('Parked; resume when you want')).toBeDefined();
    expect(screen.getByRole('button', { name: 'Resume' })).toBeDefined();
    expect(screen.queryByRole('button', { name: 'Done' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Skip' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Abandon' })).toBeDefined();
  });

  it('quotes the spec in the verifying banner and marks a report as the agent declared it', async () => {
    await open(
      view({
        state: 'verified',
        uiState: 'final',
        banner: { key: 'state.verified', arg: null },
        step: null,
        verifyResult: {
          ok: true,
          detail: 'the webhook answered 200',
          reportedAt: '2026-09-09T09:45:00.000Z',
          late: false,
        },
      }),
    );

    expect(screen.getByText('Verified', { exact: false })).toBeDefined();
    expect(screen.getByText('(declared by agent)')).toBeDefined();
    expect(screen.getByText('the webhook answered 200')).toBeDefined();
  });

  it('raises a badge on the tab that changed while the user was on another one', async () => {
    const other = view({
      tab: tab({ id: 'hf_0000000002', label: 'Codex · api' }),
    });
    const events: FakeEvents = {};
    const bridge = servingBridge(
      [tab(), other.tab],
      { [ID]: view(), 'hf_0000000002': other },
      events,
    );
    setBridge(bridge);
    render(OverlayView);
    await waitFor(() => expect(screen.getByRole('heading', { level: 1 })).toBeDefined());

    // MULTI-03: the first tab is the one the user is on; a change on the other one only
    // raises a badge and never steals the view.
    events.handoffChanged?.('hf_0000000002');
    await waitFor(() => expect(screen.getByLabelText('1 new')).toBeDefined());
    expect(
      screen.getByRole('button', { name: /Claude Code/ }).getAttribute('aria-current'),
    ).toBe('true');

    screen.getByRole('button', { name: /Codex/ }).click();
    await waitFor(() => expect(screen.queryByLabelText('1 new')).toBeNull());
  });

  it('brings the window to the front for the first handoff and not for the next (MULTI-03)', async () => {
    const events: FakeEvents = {};
    let strip: TabView[] = [];
    const bridge = fakeBridge({
      listHandoffs: vi.fn(async () => strip),
      getHandoffView: vi.fn(async (id: string) =>
        id === ID ? view() : view({ tab: tab({ id: 'hf_0000000002' }) }),
      ),
      onHandoffChanged: vi.fn(async (handler: (id: string) => void) => {
        events.handoffChanged = handler;
        return () => {};
      }),
    });
    setBridge(bridge);
    render(OverlayView);
    await waitFor(() => expect(bridge.listHandoffs).toHaveBeenCalled());
    // The first read is the strip a previous run left behind, whatever is in it.
    expect(bridge.showWindow).not.toHaveBeenCalled();

    strip = [tab()];
    events.handoffChanged?.(ID);
    await waitFor(() => expect(bridge.showWindow).toHaveBeenCalledTimes(1));

    // A second handoff arriving beside an active tab gets a badge and nothing else.
    strip = [tab(), tab({ id: 'hf_0000000002' })];
    events.handoffChanged?.('hf_0000000002');
    await waitFor(() => expect(screen.getByLabelText('1 new')).toBeDefined());
    expect(bridge.showWindow).toHaveBeenCalledTimes(1);
  });

  it('hides a revealed value again after ten seconds (DET-04)', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const bridge = await open();
      vi.mocked(bridge.revealValue).mockResolvedValue(['sk_live_0123456789abcdef']);

      screen.getByRole('button', { name: 'Show' }).click();
      await waitFor(() =>
        expect(document.querySelector('[data-value="api_key"]')?.textContent).toContain(
          'sk_live_0123456789abcdef',
        ),
      );

      await vi.advanceTimersByTimeAsync(REVEAL_MS + 100);
      await tick();
      expect(document.querySelector('[data-value="api_key"]')?.textContent).not.toContain(
        'sk_live',
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it('shows the sentence the core pushed, in the language the core rendered it in', async () => {
    const events: FakeEvents = {};
    await open(view(), events);
    events.notice?.({ kind: 'warning', text: 'That is no longer available.' });
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain('no longer'));
  });

  it('collapses the previous rounds and keeps their notes and replies (VER-09)', async () => {
    await open(
      view({
        history: [
          {
            no: 1,
            steps: ['open the dashboard', 'save'],
            confirmed: [1],
            skipped: [2],
            notes: [{ step: 1, text: 'was already there', at: '2026-09-09T09:10:00.000Z' }],
            replies: [
              { round: 1, step: 2, text: 'the endpoint was wrong', at: '2026-09-09T09:41:00.000Z' },
            ],
            verify: {
              ok: false,
              detail: '404',
              reportedAt: '2026-09-09T09:40:00.000Z',
              late: false,
            },
          },
        ],
      }),
    );

    const history = document.querySelector('details.history');
    expect(history).not.toBeNull();
    expect(history?.hasAttribute('open')).toBe(false);
    expect(history?.textContent).toContain('was already there');
    expect(history?.textContent).toContain('the endpoint was wrong');
  });

  it('renders in Italian when that is the resolved language', async () => {
    setLanguage('it');
    await open();
    expect(screen.getByText('Passo 1 di 2')).toBeDefined();
    expect(screen.getByRole('button', { name: 'Fatto' })).toBeDefined();
  });
});

/** Types `text` into the open sheet, the way a person would. */
async function type(text: string): Promise<void> {
  const box = screen.getByRole('textbox') as HTMLTextAreaElement;
  box.value = text;
  box.dispatchEvent(new Event('input', { bubbles: true }));
  await tick();
  await tick();
}
