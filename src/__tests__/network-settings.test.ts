/**
 * Settings → Network and Settings → Updates (§7.6, §7.13, NET-01, NET-02, UPD-01).
 *
 * Four promises, and none of them is about a control:
 *
 * - **The page says this build connects to nothing**, beside the list rather than instead of
 *   it. An empty list on its own is ambiguous — nothing happened, or nothing was recorded —
 *   and the difference is the whole point of NET-01.
 * - **It says the firewall test is the verification** (NET-02: the page is a
 *   self-declaration, and both are required).
 * - **A row is drawn as a person reads it**: the domain, the instant through `datetime.ts`,
 *   the bytes, and the purpose as a sentence. A purpose key this version does not know is
 *   shown as it is, because hiding a connection over a missing label would be the one failure
 *   this page cannot afford.
 * - **Updates is one line**, and it is the true one.
 */
import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setBridge } from '../bridge';
import { moment } from '../datetime';
import { DEFAULT_LANGUAGE, setLanguage, t } from '../i18n';
import type { NetworkEventView } from '../model';
import NetworkSettings from '../settings/NetworkSettings.svelte';
import UpdatesSettings from '../settings/UpdatesSettings.svelte';
import { fakeBridge } from './fake-bridge';

function event(overrides: Partial<NetworkEventView> = {}): NetworkEventView {
  return {
    at: '2026-09-10T09:00:00.000Z',
    domain: 'updates.example.test',
    bytesSent: 121,
    purpose: 'update-check',
    ...overrides,
  };
}

beforeEach(() => {
  setLanguage(DEFAULT_LANGUAGE);
});

afterEach(() => {
  cleanup();
  setBridge(null);
});

describe('Settings → Network (NET-01, NET-02)', () => {
  it('says this build connects to nothing, and that the firewall test is the check', async () => {
    const networkEvents = vi.fn(async () => []);
    setBridge(fakeBridge({ networkEvents }));

    render(NetworkSettings);

    await screen.findByText(t('net.empty'));
    expect(screen.getByText(t('net.zero'))).toBeTruthy();
    expect(screen.getByText(t('net.declaration'))).toBeTruthy();
    expect(networkEvents).toHaveBeenCalled();
  });

  it('draws a recorded connection with its domain, instant, bytes and purpose', async () => {
    setBridge(fakeBridge({ networkEvents: vi.fn(async () => [event()]) }));

    render(NetworkSettings);

    await screen.findByText('updates.example.test');
    const row = document.querySelector('[data-net-entry="updates.example.test"]');
    expect(row?.textContent).toContain(moment('2026-09-10T09:00:00.000Z'));
    expect(row?.textContent).toContain(t('net.bytes', { bytes: 121 }));
    expect(row?.textContent).toContain(t('net.purposeUpdateCheck'));
    // The stable key is what the row holds; it is not what the page shows.
    expect(row?.textContent).not.toContain('update-check');
  });

  it('shows a purpose it does not know rather than hiding the connection', async () => {
    setBridge(fakeBridge({ networkEvents: vi.fn(async () => [event({ purpose: 'something-new' })]) }));

    render(NetworkSettings);

    await screen.findByText('updates.example.test');
    const row = document.querySelector('[data-net-entry="updates.example.test"]');
    expect(row?.textContent).toContain('something-new');
  });

  it('says why it is empty when the core cannot be read at all', async () => {
    setBridge(
      fakeBridge({
        networkEvents: vi.fn(async () => {
          throw new Error('no database');
        }),
      }),
    );

    render(NetworkSettings);

    expect(await screen.findByRole('alert')).toBeTruthy();
    expect(screen.getByText(t('net.empty'))).toBeTruthy();
  });
});

describe('Settings → Updates (UPD-01)', () => {
  it('is the one true line, in the language of the window', async () => {
    render(UpdatesSettings);
    expect(screen.getByText(t('updates.none'))).toBeTruthy();

    cleanup();
    setLanguage('it');
    render(UpdatesSettings);
    expect(screen.getByText(t('updates.none'))).toBeTruthy();
  });
});
