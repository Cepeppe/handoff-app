/**
 * The one place the frontend talks to the Rust side (§7.6).
 *
 * Everything the webview can ask of the core goes through this interface, for three
 * reasons that shape the rest of the frontend:
 *
 * - a component never imports `@tauri-apps/api` itself, so a component test needs no
 *   webview and no running app: it installs a fake with {@link setBridge};
 * - the payloads are typed here once, next to the Rust signatures they mirror, instead of
 *   being re-guessed at every `invoke` call site;
 * - outside a Tauri webview — `vite dev` in a plain browser, vitest under jsdom — the
 *   calls become no-ops instead of runtime errors, so the UI can be opened anywhere.
 *
 * Adding a command here means adding it to `ui_bridge` on the Rust side; adding a Tauri
 * *plugin* call means adding its permission to `src-tauri/capabilities/main.json`, which
 * grants the least privilege each task needs and nothing more. The clipboard and the opener
 * are deliberately **not** listed there: the window never calls those plugins, it calls
 * `copyValue`, `openUrl` and `openSecretFile`, and the Rust side decides what may be copied
 * and what may be opened.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import type { Language } from './i18n';
import type {
  ActionName,
  AgentStatus,
  CaptureOutcome,
  CaptureSettings,
  ConsentView,
  CrashNotice,
  GeneralSettings,
  HandoffView,
  LogDetailView,
  LogEntryView,
  Notice,
  OnboardingView,
  Redacted,
  PendingRunbookProposalView,
  RequestChoice,
  RunbookEntryView,
  ScanReport,
  Scope,
  Selection,
  SelectionSetup,
  SessionChoice,
  ShortcutStatus,
  TabView,
  WindowSettings,
} from './model';
import { isViewName, type ViewName } from './views';

/**
 * The event the tray menu emits to bring a view forward.
 *
 * The literal is duplicated in `src-tauri/src/ui_bridge/mod.rs` (`EVENT_SHOW_VIEW`) and a
 * Rust test reads this file to prove the two spellings still agree.
 */
export const EVENT_SHOW_VIEW = 'ui://show-view';

/**
 * The window gained or lost the focus (WIN-03).
 *
 * The panel collapses to the one-line bar when the user clicks elsewhere and expands when
 * they come back. Spelled in `ui_bridge/mod.rs` as `EVENT_WINDOW_FOCUS`; the same Rust test
 * reads this file to keep the two together.
 */
export const EVENT_WINDOW_FOCUS = 'ui://window-focus';

/** A tab changed; the window re-reads it. Spelled in `ui_bridge/events.rs` as well. */
export const EVENT_HANDOFF_CHANGED = 'handoff_changed';

/** The set of sessions changed; the window re-reads the tabs (SRV-21). */
export const EVENT_SESSIONS_CHANGED = 'sessions_changed';

/** One sentence for the user, already in their language. */
export const EVENT_NOTICE = 'notice';

/**
 * A capture is ready to be previewed, or could not be taken (§7.8, FM-17).
 *
 * Spelled in `ui_bridge/capture.rs` as `EVENT_CAPTURE_READY`; a Rust test reads this file to
 * keep the two together.
 */
export const EVENT_CAPTURE_READY = 'ui://capture-ready';

/** Stops delivering an event to the handler that returned it. */
export type Unlisten = () => void;

/** The commands and events of the Rust side, as the frontend sees them. */
export interface Bridge {
  /**
   * Asks the window to take the height the content measured, keeping the fixed width of
   * WIN-02. The Rust side clamps it to what the monitor can show.
   */
  resizeToContent(height: number): Promise<void>;

  /**
   * Tells the core which language the UI resolved (APP-02), so that the texts Rust owns —
   * the tray menu today, notifications and the crash notice later — match the window.
   */
  setUiLanguage(language: Language): Promise<void>;

  /**
   * Widens the panel while the settings page is open and narrows it back (§7.6).
   *
   * The width is the core's, like the height: only this side knows what the monitor can
   * show, and WIN-02 fixes the panel's own width for every other view.
   */
  setWideLayout(wide: boolean): Promise<void>;

  /** Settings -> General: the language, the login entry, the launch it came from (§7.16). */
  generalSettings(): Promise<GeneralSettings>;

  /**
   * Stores the language the user chose, `null` for **System** (APP-02).
   *
   * It does not switch the language: the caller resolves what to run in and reports it
   * through {@link setUiLanguage}, so the window and the tray come from one decision.
   */
  setLanguageSetting(language: Language | null): Promise<void>;

  /** Puts Baton in the operating system's login items, or takes it out (APP-01). */
  setAutostart(enabled: boolean): Promise<void>;

  /** Runs `handler` whenever the tray menu asks for a view (Show, New request, Settings). */
  onShowView(handler: (view: ViewName) => void): Promise<Unlisten>;

  /** Runs `handler` when the window gains or loses the focus (WIN-03). */
  onWindowFocus(handler: (focused: boolean) => void): Promise<Unlisten>;

  /** What the window needs to know about its own behaviour (§7.16, R-10). */
  windowSettings(): Promise<WindowSettings>;

  /** Switches the R-10 fallback collapse on or off (§7.16, the General settings page). */
  setCollapseFallback(enabled: boolean): Promise<void>;

  /** The tab strip, oldest handoff first (§7.6, MULTI-01). */
  listHandoffs(): Promise<TabView[]>;

  /** One whole tab, or `null` when the store no longer knows that id. */
  getHandoffView(id: string): Promise<HandoffView | null>;

  /**
   * One user action (RESP-01..09). `payload` is the text of a sheet, or the request id of a
   * relink; the actions that take none ignore it.
   */
  act(id: string, action: ActionName, payload?: string): Promise<void>;

  /**
   * Puts the **true** value on the clipboard, secret-treated or not (GUIDE-02, DET-04).
   * With an index it copies that item of a list, without one the whole value.
   */
  copyValue(id: string, key: string, index?: number): Promise<void>;

  /**
   * Puts the OPEN-05 sentence for a handoff waiting for its spec back on the clipboard.
   *
   * The "Copy request again" of §7.6: the first copy happened when the request was opened,
   * and by the time the user comes back their clipboard has moved on.
   */
  copyRequestText(id: string): Promise<void>;

  /** Puts a handoff's own id on the clipboard, to resume it in a new session (SRV-23). */
  copyHandoffId(id: string): Promise<void>;

  /** The true value, for the ten-second reveal of DET-04. One entry per item. */
  revealValue(id: string, key: string): Promise<string[]>;

  /** Opens a URL, if its scheme is one of the four SPEC-07 allows. */
  openUrl(url: string): Promise<void>;

  /** Opens the file a `secrets` entry names, with its default application (SEC-02). */
  openSecretFile(id: string, name: string): Promise<void>;

  /**
   * Runs both detectors over typed text, before it can be sent (§7.10, DET-01, DET-03).
   *
   * `id` is the handoff the sheet belongs to: its spec's values are the exemption list, so
   * a value the agent itself sent is not reported as a suspicion.
   */
  scanTypedText(id: string, text: string): Promise<Redacted>;

  /** The choice the two-option popover highlights, from the last capture (CAP-01). */
  captureSettings(): Promise<CaptureSettings>;

  /** Captures the monitor the cursor is on (CAP-02). The outcome arrives as an event. */
  captureFullScreen(): Promise<void>;

  /** Opens one transparent selection overlay per monitor (CAP-02, DD-29). */
  startRegionCapture(): Promise<void>;

  /** What the selection overlay this window is covers. Only a selection overlay may ask. */
  selectionSetup(): Promise<SelectionSetup>;

  /** The drag is over: close the overlays and crop what it framed. */
  regionCaptured(selection: Selection): Promise<void>;

  /** Esc, or a drag with no area: no capture, and the panel comes back. */
  cancelRegionCapture(): Promise<void>;

  /** The PNG of the capture waiting to be shown. Raw bytes, never base64. */
  capturePreview(): Promise<ArrayBuffer>;

  /** Drops the pixels the preview was showing (PRIN-04). */
  discardCapture(): Promise<void>;

  /** A capture ended, one way or another (§7.8, FM-17). */
  onCaptureReady(handler: (outcome: CaptureOutcome) => void): Promise<Unlisten>;

  /**
   * The "which session is this?" question, when a Stop hook left one (FM-22, SRV-18).
   *
   * Empty most of the time. It is re-read on `sessionsChanged`, which is how the registry
   * announces it: the question is a fact about the run and carries no payload of its own.
   */
  sessionPicker(): Promise<SessionChoice[]>;

  /** The user answered the picker, or said they do not know (`null`). */
  answerSessionPicker(sessionRef: string | null): Promise<void>;

  /**
   * The sessions the request sheet may address (OPEN-04).
   *
   * Only the connected ones, oldest first, so a single session pre-selects itself. An empty
   * answer is the "no active session" notice of OPEN-04a: the sheet still sends.
   */
  sessions(): Promise<SessionChoice[]>;

  /**
   * Queues what the user typed, opens its tab and copies the sentence (OPEN-04, OPEN-05).
   *
   * Returns the id of the request, which is the id of the tab: they are the same thing
   * (DD-13), and the agent quotes it back as `request_id`.
   */
  createRequest(text: string, sessionRef: string | null): Promise<string>;

  /** The requests a handoff could be answering instead of the one it is (FM-20). */
  openRequests(id: string): Promise<RequestChoice[]>;

  /** The global shortcut in force, and whether to ask for another (OPEN-03, FM-18). */
  shortcutStatus(): Promise<ShortcutStatus>;

  /** The user chose another combination; rejects with the reason when it is not free. */
  setShortcut(accelerator: string): Promise<void>;

  /** The user closed the FM-18 dialog without choosing: never ask again. */
  dismissShortcutQuestion(): Promise<void>;

  /**
   * Brings the overlay to the front (MULTI-03).
   *
   * Called for the first handoff opened while none is active, and never for one that
   * arrives beside an active tab — that one gets a badge and waits.
   */
  showWindow(): Promise<void>;

  /**
   * Whether the previous run ended in a crash (§7.14, TEL-02).
   *
   * It writes: the report it reports is recorded as told, so a remount or a reloaded
   * webview says nothing and the launch after this one says nothing either.
   */
  crashNotice(): Promise<CrashNotice>;

  /** Opens the crash folder so the user can send the report by hand (§7.14, TEL-01). */
  openCrashesFolder(): Promise<void>;

  /** Whether this launch shows onboarding, and what it consists of (§7.6, F-13). */
  onboarding(): Promise<OnboardingView>;

  /** Onboarding is over: remember it, and remember the autostart answer (APP-01). */
  finishOnboarding(autostart: boolean): Promise<void>;

  /** The state of every adapter in `scope`: the Agents page and "Find agents" (INST-05). */
  agents(scope: Scope): Promise<AgentStatus[]>;

  /**
   * The launch scan: which agents are new, and what has moved (INST-05, FM-23).
   *
   * It writes: an agent it reports as new is recorded, so it is announced once and never
   * again however many times this is called.
   */
  scanAgents(): Promise<ScanReport>;

  /** The plan for `agentId` in `scope`, as the consent screen shows it (INST-01). */
  consentPlan(agentId: string, scope: Scope): Promise<ConsentView>;

  /**
   * Writes the plan the user accepted (INST-01).
   *
   * `digest` is the one `consentPlan` handed over. The Rust side plans again and refuses when
   * the fingerprint no longer matches: the caller shows the new plan and asks again.
   */
  installAgent(agentId: string, scope: Scope, digest: string): Promise<void>;

  /** Removes exactly our entries from `scope` (INST-04). */
  uninstallAgent(agentId: string, scope: Scope): Promise<void>;

  /** Regenerates `~/.handoff/channel.token` (FM-10). */
  repairToken(): Promise<void>;

  /** The folder picker of the project scope; `null` when the user cancelled (INST-06). */
  pickProjectFolder(): Promise<string | null>;

  /** Opens the macOS screen-recording pane (CAP-04). Rejects off macOS. */
  openScreenRecordingSettings(): Promise<void>;

  /** Settings -> Log: the closed handoffs, most recently closed first (§7.11, LOG-04). */
  logEntries(): Promise<LogEntryView[]>;

  /** One whole entry, or `null` when the log no longer holds that id. */
  logDetail(id: string): Promise<LogDetailView | null>;

  /** Deletes one entry, with its rounds, events and sends (LOG-04). */
  deleteLogEntry(id: string): Promise<void>;

  /** Empties the log: every table but `settings` (LOG-04). */
  deleteLog(): Promise<void>;

  /**
   * Writes every table to a file the user chooses (LOG-04): it is their data.
   *
   * Answers the path it wrote, or `null` when they cancelled the save dialog.
   */
  exportLog(): Promise<string | null>;

  /** Settings -> Runbooks: what `~/.handoff/runbooks/` holds (§7.12). */
  runbooks(): Promise<RunbookEntryView[]>;

  /** The rewrites of RUN-09 nobody has answered yet, for the Runbooks page (§7.6). */
  runbookProposals(): Promise<PendingRunbookProposalView[]>;

  /** Opens the runbook folder with the operating system's file manager (RUN-03). */
  openRunbooksFolder(): Promise<void>;

  /**
   * Moves one runbook to the OS trash (RUN-01: the user can delete any runbook).
   *
   * The **file name** and never a path: the Rust side resolves it against the runbook
   * folder, which is what stops the webview naming a file anywhere else.
   */
  deleteRunbook(fileName: string): Promise<void>;

  /** The user answered the rewrite of RUN-09: `true` rewrites the file, `false` keeps both. */
  resolveRunbookProposal(id: string, accept: boolean): Promise<void>;

  /** Runs `handler` with the id of a handoff whose state changed. */
  onHandoffChanged(handler: (id: string) => void): Promise<Unlisten>;

  /** Runs `handler` when the set of sessions changed; it carries no payload by design. */
  onSessionsChanged(handler: () => void): Promise<Unlisten>;

  /** Runs `handler` with a sentence to show the user. */
  onNotice(handler: (notice: Notice) => void): Promise<Unlisten>;
}

/** True inside a Tauri webview, false in a plain browser and under vitest. */
export function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** The real bridge: `invoke` and `listen` against the running application. */
/**
 * A raw IPC answer as bytes.
 *
 * Tauri delivers a `tauri::ipc::Response` as an `ArrayBuffer`; a runtime that has not
 * negotiated that — a webview older than the one we ship, a mock — sends the same bytes as
 * an array of numbers. Both are accepted, because the alternative to accepting the second
 * is an image that silently does not appear.
 */
function bufferOf(answer: ArrayBuffer | number[]): ArrayBuffer {
  if (answer instanceof ArrayBuffer) {
    return answer;
  }
  const bytes = new Uint8Array(answer.length);
  bytes.set(answer);
  return bytes.buffer;
}

export function tauriBridge(): Bridge {
  return {
    async resizeToContent(height) {
      await invoke('resize_to_content', { height });
    },
    async setUiLanguage(language) {
      await invoke('set_ui_language', { language });
    },
    async setWideLayout(wide) {
      await invoke('set_wide_layout', { wide });
    },
    async generalSettings() {
      return invoke<GeneralSettings>('general_settings');
    },
    async setLanguageSetting(language) {
      await invoke('set_language', { language });
    },
    async setAutostart(enabled) {
      await invoke('set_autostart', { enabled });
    },
    async onShowView(handler) {
      return listen<string>(EVENT_SHOW_VIEW, (event) => {
        // An unknown name is dropped rather than switching the window to nothing: the
        // payload crosses a process boundary and the view list is the frontend's.
        if (isViewName(event.payload)) {
          handler(event.payload);
        }
      });
    },
    async onWindowFocus(handler) {
      return listen<boolean>(EVENT_WINDOW_FOCUS, (event) => handler(event.payload));
    },
    async windowSettings() {
      return invoke<WindowSettings>('window_settings');
    },
    async setCollapseFallback(enabled) {
      await invoke('set_collapse_fallback', { enabled });
    },
    async listHandoffs() {
      return invoke<TabView[]>('list_handoffs');
    },
    async getHandoffView(id) {
      return invoke<HandoffView | null>('get_handoff_view', { id });
    },
    async act(id, action, payload) {
      await invoke('act', { id, action, payload: payload ?? null });
    },
    async copyValue(id, key, index) {
      await invoke('copy_value', { id, key, index: index ?? null });
    },
    async copyRequestText(id) {
      await invoke('copy_request_text', { id });
    },
    async copyHandoffId(id) {
      await invoke('copy_handoff_id', { id });
    },
    async revealValue(id, key) {
      return invoke<string[]>('reveal_value', { id, key });
    },
    async openUrl(url) {
      await invoke('open_url', { url });
    },
    async openSecretFile(id, name) {
      await invoke('open_secret_file', { id, name });
    },
    async scanTypedText(id, text) {
      return invoke<Redacted>('scan_typed_text', { id, text });
    },
    async captureSettings() {
      return invoke<CaptureSettings>('capture_settings');
    },
    async captureFullScreen() {
      await invoke('capture_full_screen');
    },
    async startRegionCapture() {
      await invoke('start_region_capture');
    },
    async selectionSetup() {
      return invoke<SelectionSetup>('selection_setup');
    },
    async regionCaptured(selection) {
      await invoke('region_captured', { selection });
    },
    async cancelRegionCapture() {
      await invoke('cancel_region_capture');
    },
    async capturePreview() {
      // The Rust side answers with a raw IPC response, which arrives here as an
      // `ArrayBuffer`: a few megabytes of PNG as a JSON array of numbers would be an order
      // of magnitude larger and would be parsed twice.
      return bufferOf(await invoke<ArrayBuffer | number[]>('capture_preview'));
    },
    async discardCapture() {
      await invoke('discard_capture');
    },
    async onCaptureReady(handler) {
      return listen<CaptureOutcome>(EVENT_CAPTURE_READY, (event) => handler(event.payload));
    },
    async sessionPicker() {
      return invoke<SessionChoice[]>('session_picker');
    },
    async answerSessionPicker(sessionRef) {
      await invoke('answer_session_picker', { sessionRef });
    },
    async sessions() {
      return invoke<SessionChoice[]>('sessions');
    },
    async createRequest(text, sessionRef) {
      return invoke<string>('create_request', { text, sessionRef });
    },
    async openRequests(id) {
      return invoke<RequestChoice[]>('open_requests', { id });
    },
    async shortcutStatus() {
      return invoke<ShortcutStatus>('shortcut_status');
    },
    async setShortcut(accelerator) {
      await invoke('set_shortcut', { accelerator });
    },
    async dismissShortcutQuestion() {
      await invoke('dismiss_shortcut_question');
    },
    async showWindow() {
      await invoke('show_window');
    },
    async crashNotice() {
      return invoke<CrashNotice>('crash_notice');
    },
    async openCrashesFolder() {
      await invoke('open_crashes_folder');
    },
    async onboarding() {
      return invoke<OnboardingView>('onboarding');
    },
    async finishOnboarding(autostart) {
      await invoke('finish_onboarding', { autostart });
    },
    async agents(scope) {
      return invoke<AgentStatus[]>('agents', { scope });
    },
    async scanAgents() {
      return invoke<ScanReport>('scan_agents');
    },
    async consentPlan(agentId, scope) {
      return invoke<ConsentView>('consent_plan', { agentId, scope });
    },
    async installAgent(agentId, scope, digest) {
      await invoke('install_agent', { agentId, scope, digest });
    },
    async uninstallAgent(agentId, scope) {
      await invoke('uninstall_agent', { agentId, scope });
    },
    async repairToken() {
      await invoke('repair_token');
    },
    async pickProjectFolder() {
      return (await invoke<string | null>('pick_project_folder')) ?? null;
    },
    async openScreenRecordingSettings() {
      await invoke('open_screen_recording_settings');
    },
    async logEntries() {
      return invoke<LogEntryView[]>('log_entries');
    },
    async logDetail(id) {
      return (await invoke<LogDetailView | null>('log_detail', { id })) ?? null;
    },
    async deleteLogEntry(id) {
      await invoke('delete_log_entry', { id });
    },
    async deleteLog() {
      await invoke('delete_log');
    },
    async exportLog() {
      return (await invoke<string | null>('export_log')) ?? null;
    },
    async runbooks() {
      return invoke<RunbookEntryView[]>('runbooks');
    },
    async runbookProposals() {
      return invoke<PendingRunbookProposalView[]>('runbook_proposals');
    },
    async openRunbooksFolder() {
      await invoke('open_runbooks_folder');
    },
    async deleteRunbook(fileName) {
      await invoke('delete_runbook', { fileName });
    },
    async resolveRunbookProposal(id, accept) {
      await invoke('resolve_runbook_proposal', { id, accept });
    },
    async onHandoffChanged(handler) {
      return listen<string>(EVENT_HANDOFF_CHANGED, (event) => handler(event.payload));
    },
    async onSessionsChanged(handler) {
      return listen(EVENT_SESSIONS_CHANGED, () => handler());
    },
    async onNotice(handler) {
      return listen<Notice>(EVENT_NOTICE, (event) => handler(event.payload));
    },
  };
}

/** A bridge that accepts every call and does nothing, for a browser or a test. */
export function noopBridge(): Bridge {
  const unlisten = async (): Promise<Unlisten> => () => {};
  return {
    async resizeToContent() {},
    async setUiLanguage() {},
    async setWideLayout() {},
    async generalSettings() {
      // Outside the webview there is no settings table and no login items: the language
      // then comes from the system alone, which is the second step of the §7.16 rule.
      return { language: null, autostart: false, startedHidden: false };
    },
    async setLanguageSetting() {},
    async setAutostart() {},
    onShowView: unlisten,
    onWindowFocus: unlisten,
    async windowSettings() {
      // Outside the webview there is no window to collapse, so the blur rule is all there
      // would be; the fallback stays off, which is also its default (R-10).
      return { collapseFallback: false, collapseFallbackMs: 3000 };
    },
    async setCollapseFallback() {},
    async listHandoffs() {
      return [];
    },
    async getHandoffView() {
      return null;
    },
    async act() {},
    async copyValue() {},
    async copyRequestText() {},
    async copyHandoffId() {},
    async revealValue() {
      return [];
    },
    async openUrl() {},
    async openSecretFile() {},
    async scanTypedText(_id, text) {
      // Outside the webview there is no detector; saying "nothing was found" here would be
      // a claim this side cannot make, so the text comes back as it went in and the sheet
      // shows it unchanged.
      return { text, kinds: [], reasons: [], segments: [{ text, suspected: false }] };
    },
    async captureSettings() {
      // Nothing was ever captured here, so neither choice is the last one and the popover
      // highlights nothing (CAP-01).
      return { lastChoice: null };
    },
    async captureFullScreen() {},
    async startRegionCapture() {},
    async selectionSetup() {
      // A browser is one screen at its own scale, which is the honest answer for a preview
      // of the selection overlay opened outside Tauri.
      return { monitor: 1, scaleFactor: 1 };
    },
    async regionCaptured() {},
    async cancelRegionCapture() {},
    async capturePreview() {
      // Answering with an empty image would be a claim this side cannot make; the preview
      // shows its failure instead.
      throw new Error('no capture');
    },
    async discardCapture() {},
    onCaptureReady: unlisten,
    async sessionPicker() {
      return [];
    },
    async answerSessionPicker() {},
    async sessions() {
      return [];
    },
    async createRequest() {
      // Outside the webview nothing is queued, and answering with an id would be a claim
      // this side cannot make: the sheet reports the failure instead.
      throw new Error('no core');
    },
    async openRequests() {
      return [];
    },
    async shortcutStatus() {
      // A browser has no global shortcut, so there is nothing registered and nothing to ask
      // the user about (FM-18 only fires on a real registration failure).
      return { accelerator: 'Control+Alt+H', registered: false, askForAnother: false };
    },
    async setShortcut() {},
    async dismissShortcutQuestion() {},
    async showWindow() {},
    async crashNotice() {
      // Nothing crashed: a browser has no panic hook and no folder to open.
      return { crashed: false };
    },
    async openCrashesFolder() {},
    async onboarding() {
      // Outside the webview there is no settings table to have been through onboarding, and
      // an onboarding that cannot record its own completion would run at every reload.
      return { needed: false, steps: [] };
    },
    async finishOnboarding() {},
    async agents() {
      return [];
    },
    async scanAgents() {
      return { newAgents: [], moved: [] };
    },
    async consentPlan() {
      // Answering with an empty plan would say "nothing to change" about files this side has
      // never read; the caller shows its failure instead.
      throw new Error('no adapters');
    },
    async installAgent() {},
    async uninstallAgent() {},
    async repairToken() {},
    async pickProjectFolder() {
      return null;
    },
    async openScreenRecordingSettings() {},
    async logEntries() {
      return [];
    },
    async logDetail() {
      return null;
    },
    async deleteLogEntry() {},
    async deleteLog() {},
    async exportLog() {
      // Outside the webview there is no save dialog; "the user cancelled" is the honest
      // answer, and the page then says nothing rather than claiming a file was written.
      return null;
    },
    async runbooks() {
      return [];
    },
    async runbookProposals() {
      return [];
    },
    async openRunbooksFolder() {},
    async deleteRunbook() {},
    async resolveRunbookProposal() {},
    onHandoffChanged: unlisten,
    onSessionsChanged: unlisten,
    onNotice: unlisten,
  };
}

let current: Bridge | null = null;

/** The bridge in force: the real one inside Tauri, a no-op elsewhere, or an installed fake. */
export function bridge(): Bridge {
  if (current === null) {
    current = inTauri() ? tauriBridge() : noopBridge();
  }
  return current;
}

/** Installs a bridge (a fake in tests); `null` restores the automatic choice. */
export function setBridge(replacement: Bridge | null): void {
  current = replacement;
}
