/**
 * Settings → Log and Settings → Runbooks (§7.11, §7.12, LOG-04, RUN-01, RUN-09).
 *
 * Four promises, and every case is about what reaches the core rather than about a control
 * changing colour:
 *
 * - **Nothing destructive happens on one click.** Delete, on a row and on the whole log, asks
 *   first; the confirmation is what calls the core, and cancelling calls nothing at all.
 * - **The detail is the record and never the value.** What the page draws is what the log
 *   stored, mask included (LOG-02, DET-04): there is no **Show** here.
 * - **Export says where it wrote, and says nothing when it was cancelled** (LOG-04).
 * - **A runbook is deleted by its file name.** The page never handles a path: the Rust side
 *   resolves the name against the runbook folder, which is what stops the webview naming a
 *   file anywhere else.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setBridge } from '../bridge';
import { moment } from '../datetime';
import { DEFAULT_LANGUAGE, setLanguage, t } from '../i18n';
import type {
  LogDetailView,
  LogEntryView,
  PendingRunbookProposalView,
  RunbookEntryView,
} from '../model';
import LogSettings from '../settings/LogSettings.svelte';
import RunbooksSettings from '../settings/RunbooksSettings.svelte';
import { fakeBridge } from './fake-bridge';

function entry(overrides: Partial<LogEntryView> = {}): LogEntryView {
  return {
    id: 'hf_0000000001',
    createdAt: '2026-09-08T11:00:00.000Z',
    closedAt: '2026-09-08T12:00:00.000Z',
    agent: 'Claude Code',
    project: 'baton',
    goal: 'Register the webhook',
    stateKey: 'state.verified',
    rounds: 2,
    delivered: true,
    ...overrides,
  };
}

function detail(overrides: Partial<LogDetailView> = {}): LogDetailView {
  return {
    entry: entry(),
    requestText: null,
    lang: 'en',
    spec: {
      goal: 'Register the webhook',
      location: 'Stripe Dashboard',
      whyHuman: 'only a person can log in',
      url: null,
      values: [{ name: 'api_key', items: ['[treated as secret: api_key]'] }],
      secrets: [{ name: 'STRIPE_WEBHOOK_SECRET', file: '.env' }],
      steps: [{ index: 1, text: 'open the dashboard', warning: null }],
      verify: 'the webhook fires',
    },
    outcomeStatus: 'verified',
    outcomeInstruction: 'Recorded as verified; a runbook was saved.',
    outcomeUserText: null,
    secretTreated: [{ location: 'values.api_key', kind: 'api_key' }],
    rounds: [
      {
        no: 1,
        startedAt: '2026-09-08T11:00:00.000Z',
        endedAt: '2026-09-08T11:40:00.000Z',
        steps: ['open the dashboard'],
        verifyOk: false,
        verifyDetail: 'the signature did not validate',
        verifyReportedAt: '2026-09-08T11:40:00.000Z',
        verifyLate: true,
      },
    ],
    sends: [
      {
        at: '2026-09-08T11:20:00.000Z',
        kind: 'question',
        text: 'which button is it now?',
        imageSha256: null,
        imageW: null,
        imageH: null,
        redactionBoxes: 0,
        ocrEngine: null,
      },
    ],
    ...overrides,
  };
}

function runbook(overrides: Partial<RunbookEntryView> = {}): RunbookEntryView {
  return {
    id: 'rb_0123456789',
    fileName: 'stripe__webhook__rb_0123456789.json',
    location: 'Stripe Dashboard',
    goal: 'Register the webhook',
    trust: 'verified',
    lastVerifiedAt: '2026-09-09T10:00:00.000Z',
    lastRunFailedAt: null,
    runs: 3,
    steps: 4,
    ...overrides,
  };
}

beforeEach(() => {
  setLanguage(DEFAULT_LANGUAGE);
});

afterEach(() => {
  cleanup();
  setBridge(null);
  setLanguage(DEFAULT_LANGUAGE);
});

describe('the Log page (§7.11, LOG-04, LOG-05)', () => {
  it('lists what finished, and says that nothing is ever deleted for you', async () => {
    setBridge(fakeBridge({ logEntries: vi.fn(async () => [entry()]) }));
    render(LogSettings);

    await screen.findByText('Register the webhook');
    // LOG-05 is a promise, not the absence of a control: the page says it out loud.
    expect(screen.getByText(t('log.retention'))).toBeTruthy();
    const meta = screen.getByText(/Claude Code/u).textContent ?? '';
    expect(meta).toContain('baton');
    expect(meta).toContain(t('state.verified'));
    expect(meta).toContain(t('log.rounds', { count: 2 }));
  });

  it('draws an instant as a date and not as an RFC 3339 string (APP-02)', async () => {
    setBridge(fakeBridge({ logEntries: vi.fn(async () => [entry()]) }));
    render(LogSettings);

    const meta = (await screen.findByText(/Claude Code/u)).textContent ?? '';
    expect(meta).toContain(t('log.closed', { at: moment('2026-09-08T12:00:00.000Z') }));
    expect(meta).not.toContain('2026-09-08T12:00:00.000Z');
  });

  it('says so when an outcome was never collected (SRV-23)', async () => {
    setBridge(fakeBridge({ logEntries: vi.fn(async () => [entry({ delivered: false })]) }));
    render(LogSettings);

    await screen.findByText(t('log.notCollected'));
  });

  it('draws the stored spec, mask and all, with no way to reveal a value', async () => {
    const logDetail = vi.fn(async () => detail());
    setBridge(fakeBridge({ logEntries: vi.fn(async () => [entry()]), logDetail }));
    render(LogSettings);

    fireEvent.click(await screen.findByText(t('log.open')));
    await waitFor(() => expect(logDetail).toHaveBeenCalledWith('hf_0000000001'));

    await screen.findByText(t('log.masked'));
    expect(screen.getByText(/\[treated as secret: api_key\]/u)).toBeTruthy();
    // DET-04's reveal is the overlay's, on a live handoff. The record has no such button.
    expect(screen.queryByText(t('overlay.show'))).toBeNull();
    expect(screen.getByText('Recorded as verified; a runbook was saved.')).toBeTruthy();
  });

  it('labels a round report as declared by the agent, and marks a late one (VER-05, DD-16)', async () => {
    setBridge(fakeBridge({ logEntries: vi.fn(async () => [entry()]), logDetail: vi.fn(async () => detail()) }));
    render(LogSettings);

    fireEvent.click(await screen.findByText(t('log.open')));
    await screen.findByText('the signature did not validate');
    expect(screen.getByText(t('overlay.declaredByAgent'))).toBeTruthy();
    expect(screen.getByText(t('overlay.late'))).toBeTruthy();
  });

  it('deletes one entry only after the question is answered', async () => {
    const deleteLogEntry = vi.fn(async () => {});
    setBridge(fakeBridge({ logEntries: vi.fn(async () => [entry()]), deleteLogEntry }));
    render(LogSettings);

    fireEvent.click(await screen.findByText(t('log.delete')));
    await screen.findByText(t('log.deleteConfirm'));
    expect(deleteLogEntry).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText(t('sheet.cancel')));
    await waitFor(() => expect(screen.queryByText(t('log.deleteConfirm'))).toBeNull());
    expect(deleteLogEntry).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText(t('log.delete')));
    fireEvent.click(await screen.findByText(t('log.confirm')));
    await waitFor(() => expect(deleteLogEntry).toHaveBeenCalledWith('hf_0000000001'));
  });

  it('empties the whole log only after the question is answered', async () => {
    const deleteLog = vi.fn(async () => {});
    setBridge(fakeBridge({ logEntries: vi.fn(async () => [entry()]), deleteLog }));
    render(LogSettings);

    fireEvent.click(await screen.findByText(t('log.deleteAll')));
    await screen.findByText(t('log.deleteAllConfirm'));
    expect(deleteLog).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText(t('log.confirm')));
    await waitFor(() => expect(deleteLog).toHaveBeenCalledTimes(1));
  });

  it('says where the export went, and says nothing when it was cancelled', async () => {
    const exportLog = vi.fn(async () => 'C:/Users/alice/baton-log.json');
    setBridge(fakeBridge({ logEntries: vi.fn(async () => []), exportLog }));
    const view = render(LogSettings);

    fireEvent.click(await screen.findByText(t('log.export')));
    await screen.findByText(t('log.exported', { path: 'C:/Users/alice/baton-log.json' }));

    view.unmount();
    cleanup();
    setBridge(fakeBridge({ logEntries: vi.fn(async () => []), exportLog: vi.fn(async () => null) }));
    render(LogSettings);
    fireEvent.click(await screen.findByText(t('log.export')));
    await waitFor(() => expect(screen.queryByText(/Written to/u)).toBeNull());
  });

  it('reports a refused deletion instead of pretending it happened', async () => {
    const deleteLogEntry = vi.fn(async () => {
      throw new Error('the handoff is active and does not take that action');
    });
    setBridge(fakeBridge({ logEntries: vi.fn(async () => [entry()]), deleteLogEntry }));
    render(LogSettings);

    fireEvent.click(await screen.findByText(t('log.delete')));
    fireEvent.click(await screen.findByText(t('log.confirm')));
    await screen.findByRole('alert');
  });
});

describe('the Runbooks page (§7.12, RUN-01, RUN-09)', () => {
  it('lists the folder with its trust, its runs and its dates', async () => {
    setBridge(fakeBridge({ runbooks: vi.fn(async () => [runbook()]) }));
    render(RunbooksSettings);

    await screen.findByText('Register the webhook');
    expect(screen.getByText('Stripe Dashboard')).toBeTruthy();
    expect(screen.getByText(/Verified/u).textContent).toContain(t('runbooks.runs', { count: 3 }));
    expect(
      screen.getByText(t('runbooks.lastVerified', { at: moment('2026-09-09T10:00:00.000Z') })),
    ).toBeTruthy();
  });

  it('shows a failure mark beside a newer verification, which is not a defect (RUN-09)', async () => {
    setBridge(
      fakeBridge({
        runbooks: vi.fn(async () => [runbook({ lastRunFailedAt: '2026-09-08T09:00:00.000Z' })]),
      }),
    );
    render(RunbooksSettings);

    await screen.findByText(
      t('runbooks.lastRunFailed', { at: moment('2026-09-08T09:00:00.000Z') }),
    );
    expect(
      screen.getByText(t('runbooks.lastVerified', { at: moment('2026-09-09T10:00:00.000Z') })),
    ).toBeTruthy();
  });

  it('deletes by file name, and only after the question is answered', async () => {
    const deleteRunbook = vi.fn(async () => {});
    setBridge(fakeBridge({ runbooks: vi.fn(async () => [runbook()]), deleteRunbook }));
    render(RunbooksSettings);

    fireEvent.click(await screen.findByText(t('runbooks.delete')));
    await screen.findByText(
      t('runbooks.deleteConfirm', { name: 'stripe__webhook__rb_0123456789.json' }),
    );
    expect(deleteRunbook).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText(t('log.confirm')));
    await waitFor(() =>
      expect(deleteRunbook).toHaveBeenCalledWith('stripe__webhook__rb_0123456789.json'),
    );
  });

  it('lists the rewrites nobody has answered and performs the answer (§7.6, RUN-09)', async () => {
    const proposal: PendingRunbookProposalView = {
      handoffId: 'hf_0000000001',
      runbookId: 'rb_0123456789',
      fileName: 'stripe__webhook__rb_0123456789.json',
      goal: 'Register the webhook',
    };
    const resolveRunbookProposal = vi.fn(async () => {});
    setBridge(
      fakeBridge({
        runbooks: vi.fn(async () => [runbook()]),
        runbookProposals: vi.fn(async () => [proposal]),
        resolveRunbookProposal,
      }),
    );
    render(RunbooksSettings);

    await screen.findByText(
      t('overlay.runbookProposal', { name: 'stripe__webhook__rb_0123456789.json' }),
    );
    fireEvent.click(screen.getByText(t('overlay.runbookAccept')));
    await waitFor(() =>
      expect(resolveRunbookProposal).toHaveBeenCalledWith('hf_0000000001', true),
    );

    fireEvent.click(screen.getByText(t('overlay.runbookDecline')));
    await waitFor(() =>
      expect(resolveRunbookProposal).toHaveBeenLastCalledWith('hf_0000000001', false),
    );
  });

  it('asks nothing when there is no rewrite waiting', async () => {
    setBridge(fakeBridge({ runbooks: vi.fn(async () => [runbook()]) }));
    render(RunbooksSettings);

    await screen.findByText('Register the webhook');
    expect(screen.queryByText(t('overlay.runbookAccept'))).toBeNull();
  });

  it('opens the folder rather than editing anything (RUN-03)', async () => {
    const openRunbooksFolder = vi.fn(async () => {});
    setBridge(fakeBridge({ runbooks: vi.fn(async () => []), openRunbooksFolder }));
    render(RunbooksSettings);

    fireEvent.click(await screen.findByText(t('runbooks.openFolder')));
    await waitFor(() => expect(openRunbooksFolder).toHaveBeenCalledTimes(1));
    expect(screen.getByText(t('runbooks.empty'))).toBeTruthy();
  });
});
