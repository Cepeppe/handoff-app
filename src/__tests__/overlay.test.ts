/**
 * The overlay against a mocked bridge (§7.6, GUIDE-01..06, RESP-01..05, SEC-02, DET-04).
 *
 * Everything here is a component rendered under jsdom with a fake core behind it: what is
 * checked is what the window *asks the core to do* — which command, with which arguments —
 * because that is the whole contract between the two sides. What the core then does is the
 * view model's tests and the store's.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setBridge } from '../bridge';
import { DEFAULT_LANGUAGE, setLanguage, t } from '../i18n';
import type { ActionsView, HandoffView, TabView, UiState } from '../model';
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
      questions: [],
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
      reasons: [],
      segments: [{ text: 'I pasted [REDACTED:api_key]', suspected: false }],
    });

    screen.getByRole('button', { name: 'Ask' }).click();
    await tick();
    await type('I pasted AKIAIOSFODNN7EXAMPLE');

    await waitFor(() => expect(screen.getByText('I pasted [REDACTED:api_key]')).toBeDefined());
    expect(screen.getByText('A secret was taken out before sending: api_key')).toBeDefined();
    expect(bridge.scanTypedText).toHaveBeenCalledWith(ID, 'I pasted AKIAIOSFODNN7EXAMPLE');

    screen.getByRole('button', { name: 'Send' }).click();
    await waitFor(() =>
      expect(bridge.act).toHaveBeenCalledWith(ID, 'ask', 'I pasted [REDACTED:api_key]'),
    );
  });

  it('marks a suspected passage and sends it as it was written (DET-01)', async () => {
    // The two levels are not treated alike: a certain match is already gone from the
    // preview, a suspected one is marked and the user's own sentence is what leaves.
    const bridge = await open();
    vi.mocked(bridge.scanTypedText).mockResolvedValue({
      text: 'the password is hunter2-tango',
      kinds: [],
      reasons: ['label'],
      segments: [
        { text: 'the password is ', suspected: false },
        { text: 'hunter2-tango', suspected: true },
      ],
    });

    screen.getByRole('button', { name: 'Ask' }).click();
    await tick();
    await type('the password is hunter2-tango');

    await waitFor(() =>
      expect(
        screen.getByText(
          'Marked below: this may contain a secret. It is sent as written — edit it if it should not be.',
        ),
      ).toBeDefined(),
    );
    expect(screen.getByText('hunter2-tango').tagName).toBe('MARK');
    expect(screen.queryByText(/A secret was taken out/)).toBeNull();

    screen.getByRole('button', { name: 'Send' }).click();
    await waitFor(() =>
      expect(bridge.act).toHaveBeenCalledWith(ID, 'ask', 'the password is hunter2-tango'),
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

  it('offers Screenshot, and pressing it asks which capture rather than taking one', async () => {
    // CAP-01: the button never starts a capture by itself. The choices appear and the
    // user picks; the popover test in `capture.test.ts` covers the highlight.
    const bridge = await open();

    screen.getByRole('button', { name: 'Screenshot' }).click();
    await waitFor(() => expect(screen.getByRole('menu')).toBeDefined());
    expect(screen.getByRole('menuitem', { name: 'Full screen' })).toBeDefined();
    expect(screen.getByRole('menuitem', { name: 'Select region' })).toBeDefined();
    expect(bridge.captureFullScreen).not.toHaveBeenCalled();
    expect(bridge.startRegionCapture).not.toHaveBeenCalled();
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

    // VER-05 read as one sentence across the two places §8.4 puts it: the state label in
    // the banner, the label and the detail with the report. Each is said once.
    expect(screen.getByText('Verified')).toBeDefined();
    expect(screen.getByText('declared by agent')).toBeDefined();
    expect(screen.getByText('the webhook answered 200')).toBeDefined();
  });

  it('quotes the spec under "the agent should now check" while the report is owed (VER-04)', async () => {
    await open(
      view({
        state: 'awaiting_verification',
        uiState: 'verifying',
        banner: { key: 'banner.verifying', arg: 'the webhook fires' },
        step: null,
        verify: 'the webhook fires',
        actions: { ...ACTIONS, done: false, ask: false, note: false, skip: false, defer: false },
      }),
    );

    expect(screen.getByText('The agent should now check:')).toBeDefined();
    expect(screen.getByText('the webhook fires')).toBeDefined();
    // Nothing has been declared yet, so nothing claims it has.
    expect(screen.queryByText('declared by agent')).toBeNull();
  });

  it('says when the agent could not check, and when a report arrived late (DD-16)', async () => {
    await open(
      view({
        state: 'not_verified',
        uiState: 'final',
        banner: { key: 'state.notVerified', arg: null },
        step: null,
        verifyResult: {
          ok: null,
          detail: 'the dashboard was unreachable',
          reportedAt: '2026-09-12T09:45:00.000Z',
          late: true,
        },
      }),
    );

    expect(screen.getByText('The agent could not check it.')).toBeDefined();
    expect(screen.getByText('arrived late')).toBeDefined();
    expect(screen.getByText('the dashboard was unreachable')).toBeDefined();
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
            questions: [
              { round: 1, step: 2, text: 'is this the right page?', at: '2026-09-09T09:35:00.000Z' },
            ],
            replies: [
              { round: 1, step: 2, text: 'the endpoint was wrong', at: '2026-09-09T09:41:00.000Z' },
            ],
            verify: {
              ok: false,
              detail: '404',
              reportedAt: '2026-09-09T09:40:00.000Z',
              late: false,
            },
            correction: false,
            failed: true,
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

  it('shows the questions and the replies of a closed round in the history (VER-09)', async () => {
    await open(
      view({
        history: [
          {
            no: 2,
            steps: ['try the other endpoint'],
            confirmed: [1],
            skipped: [],
            notes: [],
            questions: [
              { round: 2, step: 1, text: 'which endpoint?', at: '2026-09-09T09:50:00.000Z' },
            ],
            replies: [{ round: 2, step: 1, text: 'the v2 one', at: '2026-09-09T09:51:00.000Z' }],
            verify: null,
            correction: true,
            failed: false,
          },
        ],
      }),
    );

    const round = document.querySelector('[data-round="2"]');
    expect(round?.textContent).toContain('Correction 1');
    expect(round?.textContent).toContain('which endpoint?');
    expect(round?.textContent).toContain('the v2 one');
  });

  it('renders in Italian when that is the resolved language', async () => {
    setLanguage('it');
    await open();
    expect(screen.getByText('Passo 1 di 2')).toBeDefined();
    expect(screen.getByRole('button', { name: 'Fatto' })).toBeDefined();
  });
});

describe('every row of §8.4', () => {
  /**
   * The acceptance of T-037: each row is reachable and says its own sentence.
   *
   * The rows and their texts are the core's (`ui_bridge/view.rs` resolves them, precedence
   * included); what this walks is the other half — that the window draws each one and does
   * not fall back to a neighbour's screen.
   */
  const ROWS: ReadonlyArray<{
    uiState: UiState;
    state: string;
    banner: { key: string; arg: string | null } | null;
    says: string;
    overrides?: Partial<HandoffView>;
  }> = [
    {
      uiState: 'waitingForSpec',
      state: 'awaiting_spec',
      banner: { key: 'banner.awaitingSpec', arg: null },
      says: "Waiting for the agent's spec",
      overrides: { step: null, requestText: 'create the key' },
    },
    {
      uiState: 'guiding',
      state: 'active',
      banner: null,
      says: 'Open the dashboard and add the endpoint.',
    },
    {
      uiState: 'agentAway',
      state: 'active',
      banner: { key: 'banner.agentAway', arg: null },
      says: 'The agent will pick up on its next resume',
    },
    {
      uiState: 'questionSent',
      state: 'active',
      banner: { key: 'banner.questionSent', arg: null },
      says: 'Sent to the agent, waiting for the reply',
      overrides: { pending: { kind: 'question', step: 1, text: 'which button?' } },
    },
    {
      uiState: 'deferred',
      state: 'deferred',
      banner: { key: 'banner.deferred', arg: null },
      says: 'Deferred; the agent will come back',
    },
    {
      uiState: 'parked',
      state: 'parked',
      banner: { key: 'banner.parked', arg: null },
      says: 'Parked; resume when you want',
    },
    {
      uiState: 'verifying',
      state: 'awaiting_verification',
      banner: { key: 'banner.verifying', arg: 'the webhook fires' },
      says: 'The agent should now check:',
      overrides: { step: null },
    },
    {
      uiState: 'final',
      state: 'verified',
      banner: { key: 'state.verified', arg: null },
      says: 'Verified',
      overrides: { step: null },
    },
    {
      uiState: 'detached',
      state: 'active',
      banner: { key: 'banner.detached', arg: null },
      says: 'Session detached; the outcome will be delivered on the next resume',
    },
  ];

  for (const row of ROWS) {
    it(`draws the ${row.uiState} row`, async () => {
      const handoff = view({
        tab: tab({ state: row.state, uiState: row.uiState }),
        state: row.state,
        uiState: row.uiState,
        banner: row.banner,
        ...row.overrides,
      });
      const bridge = servingBridge([handoff.tab], { [handoff.tab.id]: handoff });
      setBridge(bridge);
      render(OverlayView);
      await waitFor(() => expect(screen.getByText(row.says)).toBeDefined());
    });
  }
});

describe('the waiting group of the tab strip (§7.6, SRV-23, RESP-07)', () => {
  /** A parked handoff and an orphan outcome, which is what the group is for. */
  function waitingTabs(): [HandoffView, HandoffView] {
    const parked = view({
      tab: tab({
        id: 'hf_0000000002',
        label: 'Claude Code · api',
        state: 'parked',
        uiState: 'parked',
        group: 'waiting',
        actions: { ...ACTIONS, resume: true },
      }),
      state: 'parked',
      uiState: 'parked',
    });
    const orphan = view({
      tab: tab({
        id: 'hf_0000000003',
        label: 'Codex · web',
        state: 'not_verified',
        uiState: 'final',
        group: 'waiting',
        orphan: true,
        actions: { ...ACTIONS, closeOrphan: true },
      }),
      state: 'not_verified',
      uiState: 'final',
    });
    return [parked, orphan];
  }

  async function openWaiting() {
    const [parked, orphan] = waitingTabs();
    const bridge = servingBridge(
      [tab(), parked.tab, orphan.tab],
      { [ID]: view(), [parked.tab.id]: parked, [orphan.tab.id]: orphan },
    );
    setBridge(bridge);
    render(OverlayView);
    await waitFor(() => expect(document.querySelector('.waiting-group')).not.toBeNull());
    return bridge;
  }

  it('lists parked handoffs and orphan outcomes apart from the open ones', async () => {
    await openWaiting();
    const group = document.querySelector('.waiting-group');
    expect(group?.textContent).toContain('Waiting (2)');
    expect(group?.textContent).toContain('Claude Code · api');
    expect(group?.textContent).toContain('Codex · web');
  });

  it('resumes a parked handoff from the list, without selecting it first (RESP-07)', async () => {
    const bridge = await openWaiting();
    const entry = document.querySelector('[data-waiting="hf_0000000002"]');
    const resume = Array.from(entry?.querySelectorAll('button') ?? []).find(
      (button) => button.textContent?.trim() === 'Resume',
    );
    resume?.click();
    await waitFor(() =>
      expect(bridge.act).toHaveBeenCalledWith('hf_0000000002', 'resume_from_overlay', undefined),
    );
  });

  it('closes an orphan by hand and copies its id (SRV-23)', async () => {
    const bridge = await openWaiting();
    const entry = document.querySelector('[data-waiting="hf_0000000003"]');
    const buttons = Array.from(entry?.querySelectorAll('button') ?? []);

    buttons.find((button) => button.textContent?.trim() === 'Close it')?.click();
    await waitFor(() =>
      expect(bridge.act).toHaveBeenCalledWith('hf_0000000003', 'close_orphan', undefined),
    );

    buttons.find((button) => button.textContent?.trim() === 'Copy the id')?.click();
    await waitFor(() => expect(bridge.copyHandoffId).toHaveBeenCalledWith('hf_0000000003'));
  });

  it('offers neither Resume nor Close it where the state does not have them', async () => {
    await openWaiting();
    const parked = document.querySelector('[data-waiting="hf_0000000002"]');
    const orphan = document.querySelector('[data-waiting="hf_0000000003"]');
    const labels = (entry: Element | null) =>
      Array.from(entry?.querySelectorAll('button') ?? []).map((button) =>
        button.textContent?.trim(),
      );

    expect(labels(parked)).not.toContain('Close it');
    expect(labels(orphan)).not.toContain('Resume');
  });
});

describe('the waiting-for-spec view (§7.6, OPEN-04, OPEN-05)', () => {
  function awaitingSpec(): HandoffView {
    return view({
      tab: tab({ state: 'awaiting_spec', uiState: 'waitingForSpec', goal: null }),
      state: 'awaiting_spec',
      uiState: 'waitingForSpec',
      banner: { key: 'banner.awaitingSpec', arg: null },
      goal: null,
      location: null,
      step: null,
      verify: null,
      requestText: 'I am about to create the API key on Stripe',
      actions: {
        ...ACTIONS,
        done: false,
        ask: false,
        note: false,
        skip: false,
        defer: false,
      },
    });
  }

  /**
   * A tab waiting for its spec has no goal yet, so there is no heading to wait for: the
   * view marker of §7.6 is what says it has been drawn.
   */
  async function openAwaiting() {
    const handoff = awaitingSpec();
    const bridge = servingBridge([handoff.tab], { [handoff.tab.id]: handoff });
    setBridge(bridge);
    render(OverlayView);
    await waitFor(() =>
      expect(document.querySelector('[data-ui-state="waitingForSpec"]')).not.toBeNull(),
    );
    return bridge;
  }

  it('shows what the user typed, the session, and that nothing has come back', async () => {
    await openAwaiting();
    expect(screen.getByText('I am about to create the API key on Stripe')).toBeDefined();
    expect(screen.getByText('Session: Claude Code · baton')).toBeDefined();
    expect(screen.getByText('The agent has not answered yet.')).toBeDefined();
    // There is no step to walk and no round to end.
    expect(screen.queryByRole('button', { name: 'Done' })).toBeNull();
  });

  it('puts the request sentence back on the clipboard (OPEN-05)', async () => {
    const bridge = await openAwaiting();
    screen.getByRole('button', { name: 'Copy request again' }).click();
    await waitFor(() => expect(bridge.copyRequestText).toHaveBeenCalledWith(ID));
  });

  it('lets the user give up on a request no agent picked up', async () => {
    const bridge = await openAwaiting();
    screen.getByRole('button', { name: 'Abandon' }).click();
    await tick();
    screen.getByRole('button', { name: 'Send' }).click();
    await waitFor(() => expect(bridge.act).toHaveBeenCalledWith(ID, 'abandon', ''));
  });
});

describe('the question-pending view (§7.6, RESP-04)', () => {
  it('shows the words the user asked, not only that they asked something', async () => {
    await open(
      view({
        uiState: 'questionSent',
        banner: { key: 'banner.questionSent', arg: null },
        pending: { kind: 'question', step: 1, text: 'is this the right page?' },
      }),
    );

    expect(screen.getByText('Sent to the agent, waiting for the reply')).toBeDefined();
    expect(screen.getByText('You asked something on step 1.')).toBeDefined();
    expect(screen.getByText('is this the right page?')).toBeDefined();
  });

  it('says a screenshot is pending without inventing a summary for it', async () => {
    await open(
      view({
        uiState: 'questionSent',
        banner: { key: 'banner.questionSent', arg: null },
        pending: { kind: 'screenshot', step: 2, text: null },
      }),
    );
    expect(screen.getByText('You sent a screenshot from step 2.')).toBeDefined();
    expect(document.querySelector('.pending-text')).toBeNull();
  });
});

describe('the session picker (FM-22, SRV-18)', () => {
  const CHOICES = [
    { sessionRef: 'ses_00000001', label: 'Claude Code · baton' },
    { sessionRef: 'ses_00000002', label: 'Claude Code · api' },
  ];

  it('stays out of the way while there is no question to ask', async () => {
    await open();
    expect(document.querySelector('.session-picker')).toBeNull();
  });

  it('asks which session a hook belonged to, and binds the answer', async () => {
    const bridge = servingBridge([tab()], { [ID]: view() });
    vi.mocked(bridge.sessionPicker).mockResolvedValue(CHOICES);
    setBridge(bridge);
    render(OverlayView);

    await waitFor(() => expect(screen.getByText('Which session is this?')).toBeDefined());
    screen.getByRole('button', { name: 'Claude Code · api' }).click();
    await waitFor(() =>
      expect(bridge.answerSessionPicker).toHaveBeenCalledWith('ses_00000002'),
    );
  });

  it('takes "I do not know" as an answer and binds nothing', async () => {
    const bridge = servingBridge([tab()], { [ID]: view() });
    vi.mocked(bridge.sessionPicker).mockResolvedValue(CHOICES);
    setBridge(bridge);
    render(OverlayView);

    await waitFor(() => expect(screen.getByText('Which session is this?')).toBeDefined());
    screen.getByRole('button', { name: 'I do not know' }).click();
    await waitFor(() => expect(bridge.answerSessionPicker).toHaveBeenCalledWith(null));
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

describe('the runbook rewrite of RUN-09 (§7.12 row 3)', () => {
  it('asks on the handoff that produced it, naming the file', async () => {
    await open(
      view({
        runbookProposal: {
          runbookId: 'rb_0123456789',
          fileName: 'stripe__webhook__rb_0123456789.json',
          goal: 'Register the webhook',
        },
      }),
    );

    await screen.findByText(
      t('overlay.runbookProposal', { name: 'stripe__webhook__rb_0123456789.json' }),
    );
  });

  it('rewrites the file on Accept and keeps both on Decline', async () => {
    const resolveRunbookProposal = vi.fn(async () => {});
    const handoff = view({
      runbookProposal: {
        runbookId: 'rb_0123456789',
        fileName: 'stripe__webhook__rb_0123456789.json',
        goal: 'Register the webhook',
      },
    });
    setBridge(
      fakeBridge({
        listHandoffs: vi.fn(async () => [handoff.tab]),
        getHandoffView: vi.fn(async () => handoff),
        resolveRunbookProposal,
      }),
    );
    render(OverlayView);

    fireEvent.click(await screen.findByText(t('overlay.runbookAccept')));
    await waitFor(() => expect(resolveRunbookProposal).toHaveBeenCalledWith(ID, true));

    fireEvent.click(screen.getByText(t('overlay.runbookDecline')));
    await waitFor(() => expect(resolveRunbookProposal).toHaveBeenLastCalledWith(ID, false));
  });

  it('is drawn on no handoff that has none', async () => {
    await open();
    expect(screen.queryByText(t('overlay.runbookAccept'))).toBeNull();
  });
});

describe('why a handoff is not verified (VER-06, §8.4)', () => {
  it('says that the window ran out when nothing was reported', async () => {
    await open(
      view({
        state: 'not_verified',
        uiState: 'final',
        tab: tab({ state: 'not_verified', uiState: 'final' }),
        step: null,
        verifyResult: null,
        notVerifiedReason: 'overlay.notVerifiedTimeout',
        banner: { key: 'state.notVerified', arg: null },
      }),
    );

    await screen.findByText(t('overlay.notVerifiedTimeout'));
  });

  it('says that the session ended instead, when that is what happened', async () => {
    await open(
      view({
        state: 'not_verified',
        uiState: 'final',
        tab: tab({ state: 'not_verified', uiState: 'final' }),
        step: null,
        verifyResult: null,
        notVerifiedReason: 'overlay.notVerifiedSessionGone',
        banner: { key: 'state.notVerified', arg: null },
      }),
    );

    await screen.findByText(t('overlay.notVerifiedSessionGone'));
  });

  it('marks a report that arrived after the handoff was closed as late (DD-16, FM-26)', async () => {
    // "Verified" and "verified three days later" are different facts, and the badge is the
    // only place the second one is said.
    await open(
      view({
        state: 'verified',
        uiState: 'final',
        tab: tab({ state: 'verified', uiState: 'final' }),
        step: null,
        notVerifiedReason: null,
        verifyResult: {
          ok: true,
          detail: 'the webhook fired',
          reportedAt: '2026-09-12T10:00:00.000Z',
          late: true,
        },
        banner: { key: 'state.verified', arg: null },
      }),
    );

    await screen.findByText(t('overlay.late'));
    expect(screen.getByText(t('overlay.declaredByAgent'))).toBeTruthy();
  });

  it("shows the agent's own detail instead when it reported one (VER-05)", async () => {
    // The third road to `not_verified`: `ok: null` with a detail. The core sends no reason
    // of its own there, and the label says the words are the agent's.
    await open(
      view({
        state: 'not_verified',
        uiState: 'final',
        tab: tab({ state: 'not_verified', uiState: 'final' }),
        step: null,
        notVerifiedReason: null,
        verifyResult: {
          ok: null,
          detail: 'no test event could be sent',
          reportedAt: '2026-09-09T10:00:00.000Z',
          late: false,
        },
        banner: { key: 'state.notVerified', arg: null },
      }),
    );

    await screen.findByText('no test event could be sent');
    expect(screen.getByText(t('overlay.verifyUnknown'))).toBeTruthy();
    expect(screen.getByText(t('overlay.declaredByAgent'))).toBeTruthy();
    expect(screen.queryByText(t('overlay.notVerifiedTimeout'))).toBeNull();
  });
});

describe('a correction round, as F-08 walks it (VER-08, VER-09)', () => {
  it('counts the steps of the correction and keeps the first round collapsed', async () => {
    // F-08: the report fails, the agent sends replacement steps, and round 2 opens as
    // "Correction · 1 of 2" with the history collapsed. The counter is the *step* counter of
    // §7.6 with its correction wording, and the round that failed is in the history with its
    // own marker.
    await open(
      view({
        step: {
          counter: { key: 'counter.correction', index: 1, total: 2, round: 2 },
          text: 'Delete the endpoint and add it again.',
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
        history: [
          {
            no: 1,
            steps: ['Open the dashboard and add the endpoint.'],
            confirmed: [1],
            skipped: [],
            notes: [],
            questions: [],
            replies: [],
            verify: {
              ok: false,
              detail: 'test event rejected: invalid signature',
              reportedAt: '2026-09-09T09:30:00.000Z',
              late: false,
            },
            correction: false,
            failed: true,
          },
        ],
      }),
    );

    await screen.findByText(t('counter.correction', { index: 1, total: 2 }));

    // "Collapsed" is the word §7.6 uses and `<details>` is what it means: the round is in
    // the document and closed until the user asks for it.
    const history = document.querySelector('details.history');
    expect(history).toBeTruthy();
    expect((history as HTMLDetailsElement).open).toBe(false);
    expect(screen.getByText(t('overlay.verificationFailed'))).toBeTruthy();
    expect(screen.getByText('test event rejected: invalid signature')).toBeTruthy();
  });
});
