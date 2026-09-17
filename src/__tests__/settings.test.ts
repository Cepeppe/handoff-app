/**
 * Settings → General, and the two window rules that come with it (§7.16, APP-01, APP-02).
 *
 * Four promises, each with a case, and all four are about what the *machine* ends up in
 * rather than about a control changing colour:
 *
 * - **The language changes now** (APP-02: "changeable in settings"). Every label already on
 *   screen is in the new language on the next tick, with nothing reloaded and no restart.
 * - **The autostart box is the login entry** (APP-01). Ticking it writes; a platform that
 *   refuses puts the box back where the machine actually is.
 * - **`--hidden` opens nothing** (APP-01). A launch from the login entry switches the view
 *   but never brings the panel forward.
 * - **The panel is wider here and narrow everywhere else** (§7.6).
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import App from '../App.svelte';
import { setBridge } from '../bridge';
import { catalogue, DEFAULT_LANGUAGE, setLanguage, t } from '../i18n';
import type { GeneralSettings } from '../model';
import { fallbackIsOn, resetCollapse } from '../overlay/collapse.svelte';
import { resetOverlay } from '../overlay/state.svelte';
import GeneralSettings_ from '../settings/GeneralSettings.svelte';
import { resetView, showView, view } from '../view-state.svelte';
import SettingsView from '../views/SettingsView.svelte';
import { fakeBridge } from './fake-bridge';

/** What the core answers about a machine that has never touched these settings. */
function settings(overrides: Partial<GeneralSettings> = {}): GeneralSettings {
  return { language: null, autostart: false, startedHidden: false, ...overrides };
}

/**
 * Waits for the page's own reads to have finished.
 *
 * The controls are bound in both directions, so a click landing while the first read is
 * still in flight is a click the read then overwrites. That cannot happen to a person — the
 * reads settle in a microtask — but it happens to a test every time, and waiting for the
 * last line the load draws is what tells the two apart.
 *
 * The accelerator is drawn as keycaps beside that line rather than inside it, so what is
 * waited for is the sentence with its placeholder empty — "In force:" — which is exactly
 * what the page prints before the keys.
 */
async function loaded(): Promise<void> {
  await screen.findByText(t('settings.shortcutInForce', { accelerator: '' }).trim());
}

beforeEach(() => {
  resetView();
  resetOverlay();
  resetCollapse();
  setLanguage(DEFAULT_LANGUAGE);
});

afterEach(() => {
  cleanup();
  setBridge(null);
  setLanguage(DEFAULT_LANGUAGE);
});

describe('the language control (APP-02)', () => {
  it('changes every label on screen, with no restart', async () => {
    const setLanguageSetting = vi.fn(async () => {});
    const setUiLanguage = vi.fn(async () => {});
    setBridge(
      fakeBridge({
        generalSettings: vi.fn(async () => settings()),
        setLanguageSetting,
        setUiLanguage,
      }),
    );
    render(SettingsView);

    // The page is in English, and so is a label of the *other* section's nav entry: what
    // has to change is the whole window and not the control that was clicked.
    await screen.findByText(catalogue('en')['settings.startup']);
    expect(screen.getByText(catalogue('en')['install.agents'])).toBeTruthy();

    fireEvent.click(screen.getByLabelText(t('settings.languageIt')));
    await waitFor(() => expect(setLanguageSetting).toHaveBeenCalledWith('it'));
    await tick();

    expect(screen.getByText(catalogue('it')['settings.startup'])).toBeTruthy();
    expect(screen.getByText(catalogue('it')['install.agents'])).toBeTruthy();
    // The tray menu is the Rust side's, and it is told in the same breath (§7.16).
    expect(setUiLanguage).toHaveBeenCalledWith('it');
  });

  it('stores nothing for System, so the machine keeps deciding', async () => {
    const setLanguageSetting = vi.fn(async () => {});
    setBridge(
      fakeBridge({
        generalSettings: vi.fn(async () => settings({ language: 'it' })),
        setLanguageSetting,
      }),
    );
    render(GeneralSettings_);

    const system = await screen.findByLabelText<HTMLInputElement>(t('settings.languageSystem'));
    await waitFor(() =>
      expect(screen.getByLabelText<HTMLInputElement>(t('settings.languageIt')).checked).toBe(true),
    );

    fireEvent.click(system);
    await waitFor(() => expect(setLanguageSetting).toHaveBeenCalledWith(null));
  });

  it('opens on the language the core remembered', async () => {
    setBridge(fakeBridge({ generalSettings: vi.fn(async () => settings({ language: 'en' })) }));
    render(GeneralSettings_);

    await screen.findByLabelText<HTMLInputElement>(t('settings.languageEn'));
    await waitFor(() =>
      expect(screen.getByLabelText<HTMLInputElement>(t('settings.languageEn')).checked).toBe(true),
    );
  });
});

describe('the autostart control (APP-01)', () => {
  it('writes the login entry and keeps the sentence of APP-01', async () => {
    const setAutostart = vi.fn(async () => {});
    setBridge(
      fakeBridge({ generalSettings: vi.fn(async () => settings()), setAutostart }),
    );
    render(GeneralSettings_);
    await loaded();

    const box = screen.getByLabelText<HTMLInputElement>(t('onboarding.autostart'));
    expect(box.checked).toBe(false);
    // The same sentence onboarding shows, from the same key: one preference, one wording.
    expect(screen.getByText(t('onboarding.autostartText'))).toBeTruthy();

    await fireEvent.click(box);
    await waitFor(() => expect(setAutostart).toHaveBeenCalledWith(true));
    await waitFor(() =>
      expect(
        screen.getByLabelText<HTMLInputElement>(t('onboarding.autostart')).checked,
      ).toBe(true),
    );
  });

  it('puts the box back when the platform refuses', async () => {
    // A checkbox left ticked over a login entry that was never written is the one thing
    // this page must not do: it would tell the user Baton starts with them, and it would
    // not.
    setBridge(
      fakeBridge({
        generalSettings: vi.fn(async () => settings()),
        setAutostart: vi.fn(async () => {
          throw new Error('no login items');
        }),
      }),
    );
    render(GeneralSettings_);
    await loaded();

    await fireEvent.click(screen.getByLabelText<HTMLInputElement>(t('onboarding.autostart')));

    await screen.findByText(t('install.failed', { reason: 'Error: no login items' }));
    await waitFor(() =>
      expect(
        screen.getByLabelText<HTMLInputElement>(t('onboarding.autostart')).checked,
      ).toBe(false),
    );
  });

  it('shows the entry the machine has, not the answer once given', async () => {
    setBridge(fakeBridge({ generalSettings: vi.fn(async () => settings({ autostart: true })) }));
    render(GeneralSettings_);

    await screen.findByLabelText<HTMLInputElement>(t('onboarding.autostart'));
    await waitFor(() =>
      expect(screen.getByLabelText<HTMLInputElement>(t('onboarding.autostart')).checked).toBe(
        true,
      ),
    );
  });
});

describe('the shortcut control (OPEN-03)', () => {
  it('says which combination is in force and records another one', async () => {
    const setShortcut = vi.fn(async () => {});
    setBridge(
      fakeBridge({
        generalSettings: vi.fn(async () => settings()),
        shortcutStatus: vi.fn(async () => ({
          accelerator: 'Control+Alt+H',
          registered: true,
          askForAnother: false,
        })),
        setShortcut,
      }),
    );
    render(GeneralSettings_);

    await loaded();
    // The combination is printed as the keys a person presses, not as its wire spelling.
    expect([...document.querySelectorAll('.settings-shortcut .kbd')].map((cap) => cap.textContent))
      .toEqual(['Ctrl', 'Alt', 'H']);
    // Nothing is wrong here, so the recorder is behind a control rather than in the way.
    expect(screen.queryByText(t('shortcut.record'))).toBeNull();

    fireEvent.click(screen.getByText(t('settings.shortcutChange')));

    const recorder = await screen.findByText(t('shortcut.record'));
    await fireEvent.keyDown(recorder, { key: 'j', code: 'KeyJ', ctrlKey: true, altKey: true });
    fireEvent.click(screen.getByText(t('shortcut.save')));

    await waitFor(() => expect(setShortcut).toHaveBeenCalledWith('Control+Alt+KeyJ'));
  });

  it('says so when the system refused the combination (FM-18)', async () => {
    setBridge(
      fakeBridge({
        generalSettings: vi.fn(async () => settings()),
        shortcutStatus: vi.fn(async () => ({
          accelerator: 'Control+Alt+H',
          registered: false,
          askForAnother: false,
        })),
      }),
    );
    render(GeneralSettings_);

    expect((await screen.findByRole('alert')).textContent).toBe(t('settings.shortcutRefused'));
  });
});

describe('the collapse fallback (R-10)', () => {
  it('switches the timer on and re-reads what the window should do', async () => {
    const setCollapseFallback = vi.fn(async (_enabled: boolean) => {});
    let enabled = false;
    setBridge(
      fakeBridge({
        generalSettings: vi.fn(async () => settings()),
        setCollapseFallback: vi.fn(async (next: boolean) => {
          enabled = next;
          await setCollapseFallback(next);
        }),
        windowSettings: vi.fn(async () => ({
          collapseFallback: enabled,
          collapseFallbackMs: 3000,
        })),
      }),
    );
    render(GeneralSettings_);
    await loaded();

    const box = screen.getByLabelText<HTMLInputElement>(t('settings.collapseFallback'));
    expect(box.checked).toBe(false);

    await fireEvent.click(box);

    await waitFor(() => expect(setCollapseFallback).toHaveBeenCalledWith(true));
    // The re-read is the point: the window arms the timer from what it last read (T-037).
    await waitFor(() => expect(fallbackIsOn()).toBe(true));
  });
});

describe('the settings window (§7.6)', () => {
  it('widens the window while it is open and gives the width back when it closes', async () => {
    // The page asks for no width of its own any more: `App.svelte` derives the layout from
    // the view in one place, so what is checked is that switching to the settings and away
    // again is what widens and narrows the window (§7.6).
    const setWindowLayout = vi.fn(async () => {});
    setBridge(fakeBridge({ generalSettings: vi.fn(async () => settings()), setWindowLayout }));
    render(App);

    await waitFor(() => expect(setWindowLayout).toHaveBeenLastCalledWith('panel'));

    showView('settings');
    await waitFor(() => expect(setWindowLayout).toHaveBeenLastCalledWith('settings'));

    resetView();
    await waitFor(() => expect(setWindowLayout).toHaveBeenLastCalledWith('panel'));
  });

  it('lists General first and the six sections of §7.6 in the order it names them', async () => {
    setBridge(fakeBridge({ generalSettings: vi.fn(async () => settings()) }));
    render(SettingsView);

    await screen.findByText(t('settings.startup'));
    const nav = screen.getByRole('navigation', { name: t('view.settings') });
    expect(
      [...nav.querySelectorAll('.settings-nav-item')].map((button) => button.textContent?.trim()),
    ).toEqual([
      t('settings.general'),
      t('install.agents'),
      t('settings.network'),
      t('settings.log'),
      t('settings.runbooks'),
      t('settings.updates'),
    ]);
  });

  it('has a way back to the panel that does not go through the tray (§7.6)', async () => {
    setBridge(fakeBridge({ generalSettings: vi.fn(async () => settings()) }));
    showView('settings');
    render(App);

    await screen.findByText(t('settings.startup'));
    screen.getByRole('button', { name: t('settings.back') }).click();
    await waitFor(() => expect(view()).toBe('overlay'));
  });
});

describe('a launch from the login entry (APP-01, --hidden)', () => {
  it('switches the view without bringing the panel forward', async () => {
    const showWindow = vi.fn(async () => {});
    setBridge(
      fakeBridge({
        generalSettings: vi.fn(async () => settings({ startedHidden: true })),
        onboarding: vi.fn(async () => ({ needed: true, steps: [] })),
        showWindow,
      }),
    );
    render(App);

    await waitFor(() => expect(view()).toBe('onboarding'));
    expect(showWindow).not.toHaveBeenCalled();
  });

  it('brings it forward on an ordinary launch', async () => {
    const showWindow = vi.fn(async () => {});
    setBridge(
      fakeBridge({
        generalSettings: vi.fn(async () => settings()),
        onboarding: vi.fn(async () => ({ needed: true, steps: [] })),
        showWindow,
      }),
    );
    render(App);

    await waitFor(() => expect(showWindow).toHaveBeenCalledTimes(1));
    expect(view()).toBe('onboarding');
  });
});
