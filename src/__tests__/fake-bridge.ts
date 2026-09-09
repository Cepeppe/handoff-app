/**
 * A whole {@link Bridge} for a test, with the parts a case cares about replaced.
 *
 * The bridge is the only seam between the window and the core, so a component test that
 * wants to observe one call still has to satisfy the whole interface. Building it here
 * rather than in each test means a command added to the bridge does not break every case
 * that never mentions it — and the handful that should break, break where they are wrong.
 */
import { vi } from 'vitest';

import { noopBridge, type Bridge, type Unlisten } from '../bridge';
import type { HandoffView, Notice, TabView } from '../model';

/** The handlers a fake bridge collected, so a test can fire an event by hand. */
export interface FakeEvents {
  handoffChanged?: (id: string) => void;
  sessionsChanged?: () => void;
  notice?: (notice: Notice) => void;
}

/** A bridge whose every method is a spy, with `overrides` applied on top. */
export function fakeBridge(overrides: Partial<Bridge> = {}): Bridge {
  const base = noopBridge();
  const spied: Bridge = {
    resizeToContent: vi.fn(base.resizeToContent),
    setUiLanguage: vi.fn(base.setUiLanguage),
    setWideLayout: vi.fn(base.setWideLayout),
    generalSettings: vi.fn(base.generalSettings),
    setLanguageSetting: vi.fn(base.setLanguageSetting),
    setAutostart: vi.fn(base.setAutostart),
    onShowView: vi.fn(base.onShowView),
    onWindowFocus: vi.fn(base.onWindowFocus),
    windowSettings: vi.fn(base.windowSettings),
    setCollapseFallback: vi.fn(base.setCollapseFallback),
    listHandoffs: vi.fn(base.listHandoffs),
    getHandoffView: vi.fn(base.getHandoffView),
    act: vi.fn(base.act),
    copyValue: vi.fn(base.copyValue),
    copyRequestText: vi.fn(base.copyRequestText),
    copyHandoffId: vi.fn(base.copyHandoffId),
    revealValue: vi.fn(base.revealValue),
    openUrl: vi.fn(base.openUrl),
    openSecretFile: vi.fn(base.openSecretFile),
    scanTypedText: vi.fn(base.scanTypedText),
    sessionPicker: vi.fn(base.sessionPicker),
    answerSessionPicker: vi.fn(base.answerSessionPicker),
    sessions: vi.fn(base.sessions),
    createRequest: vi.fn(base.createRequest),
    openRequests: vi.fn(base.openRequests),
    shortcutStatus: vi.fn(base.shortcutStatus),
    setShortcut: vi.fn(base.setShortcut),
    dismissShortcutQuestion: vi.fn(base.dismissShortcutQuestion),
    showWindow: vi.fn(base.showWindow),
    onboarding: vi.fn(base.onboarding),
    finishOnboarding: vi.fn(base.finishOnboarding),
    agents: vi.fn(base.agents),
    scanAgents: vi.fn(base.scanAgents),
    consentPlan: vi.fn(base.consentPlan),
    installAgent: vi.fn(base.installAgent),
    uninstallAgent: vi.fn(base.uninstallAgent),
    repairToken: vi.fn(base.repairToken),
    pickProjectFolder: vi.fn(base.pickProjectFolder),
    openScreenRecordingSettings: vi.fn(base.openScreenRecordingSettings),
    onHandoffChanged: vi.fn(base.onHandoffChanged),
    onSessionsChanged: vi.fn(base.onSessionsChanged),
    onNotice: vi.fn(base.onNotice),
  };
  return { ...spied, ...overrides };
}

/** A bridge that serves `tabs` and `views`, and hands the event handlers back in `events`. */
export function servingBridge(
  tabs: TabView[],
  views: Record<string, HandoffView>,
  events: FakeEvents = {},
): Bridge {
  return fakeBridge({
    listHandoffs: vi.fn(async () => tabs),
    getHandoffView: vi.fn(async (id: string) => views[id] ?? null),
    onHandoffChanged: vi.fn(async (handler: (id: string) => void): Promise<Unlisten> => {
      events.handoffChanged = handler;
      return () => {};
    }),
    onSessionsChanged: vi.fn(async (handler: () => void): Promise<Unlisten> => {
      events.sessionsChanged = handler;
      return () => {};
    }),
    onNotice: vi.fn(async (handler: (notice: Notice) => void): Promise<Unlisten> => {
      events.notice = handler;
      return () => {};
    }),
  });
}
