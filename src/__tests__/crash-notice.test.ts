/**
 * The crash notice of §7.14 (TEL-01, TEL-02).
 *
 * Three promises, and the first is the one that matters: **nothing is uploaded**. The
 * component offers to open a folder and offers nothing else, so what is checked here is
 * the command it calls (`open_crashes_folder` through the bridge) and the commands it never
 * calls, plus the two ways the notice goes away.
 */
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setBridge } from '../bridge';
import { DEFAULT_LANGUAGE, setLanguage, t } from '../i18n';
import CrashNotice from '../overlay/CrashNotice.svelte';
import { fakeBridge } from './fake-bridge';

beforeEach(() => {
  setLanguage(DEFAULT_LANGUAGE);
});

afterEach(() => {
  cleanup();
  setBridge(null);
});

/** The notice as the overlay renders it, with `crashed` decided by the fake core. */
function renderNotice(crashed: boolean, openCrashesFolder = vi.fn(async () => {})) {
  const failures: string[] = [];
  const bridge = fakeBridge({
    crashNotice: vi.fn(async () => ({ crashed })),
    openCrashesFolder,
  });
  setBridge(bridge);
  render(CrashNotice, { props: { onfailed: (text: string) => failures.push(text) } });
  return { bridge, failures };
}

describe('the crash notice', () => {
  it('says nothing on an ordinary launch', async () => {
    const { bridge } = renderNotice(false);

    await waitFor(() => expect(bridge.crashNotice).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('says the previous run crashed, and offers the folder and nothing else', async () => {
    renderNotice(true);

    const notice = await screen.findByRole('status');
    expect(notice.textContent).toContain(t('crash.text'));
    // TEL-01: the sentence promises the report has gone nowhere, and the only action is to
    // open the folder. A "send" button here would be the requirement broken in one word.
    expect(screen.getByRole('button', { name: t('crash.openFolder') })).not.toBeNull();
    expect(
      screen.getAllByRole('button').map((button) => button.textContent?.trim()),
    ).toEqual([t('crash.openFolder'), t('crash.dismiss')]);
  });

  it('opens the folder through the core, and then has nothing left to say', async () => {
    const open = vi.fn(async () => {});
    renderNotice(true, open);
    (await screen.findByRole('button', { name: t('crash.openFolder') })).click();

    await waitFor(() => expect(open).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.queryByRole('status')).toBeNull());
  });

  it('reports a folder that would not open, and keeps the notice', async () => {
    const open = vi.fn(async () => {
      throw new Error('no file manager');
    });
    const { failures } = renderNotice(true, open);
    (await screen.findByRole('button', { name: t('crash.openFolder') })).click();

    await waitFor(() => expect(failures).toEqual([t('notice.openFailed')]));
    expect(screen.queryByRole('status')).not.toBeNull();
  });

  it('can be put away without opening anything', async () => {
    const open = vi.fn(async () => {});
    renderNotice(true, open);
    (await screen.findByRole('button', { name: t('crash.dismiss') })).click();

    await waitFor(() => expect(screen.queryByRole('status')).toBeNull());
    expect(open).not.toHaveBeenCalled();
  });

  it('draws nothing when the core cannot answer', async () => {
    const bridge = fakeBridge({
      crashNotice: vi.fn(async () => {
        throw new Error('no database');
      }),
    });
    setBridge(bridge);
    render(CrashNotice, { props: { onfailed: () => {} } });

    await waitFor(() => expect(bridge.crashNotice).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole('status')).toBeNull();
  });
});
