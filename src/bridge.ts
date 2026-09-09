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
  HandoffView,
  Notice,
  Redacted,
  SessionChoice,
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

  /** Runs `handler` whenever the tray menu asks for a view (Show, New request, Settings). */
  onShowView(handler: (view: ViewName) => void): Promise<Unlisten>;

  /** Runs `handler` when the window gains or loses the focus (WIN-03). */
  onWindowFocus(handler: (focused: boolean) => void): Promise<Unlisten>;

  /** What the window needs to know about its own behaviour (§7.16, R-10). */
  windowSettings(): Promise<WindowSettings>;

  /** Switches the R-10 fallback collapse on or off. The checkbox for it is T-041. */
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

  /** Runs the certain detector over typed text, before it can be sent (§7.10). */
  scanTypedText(text: string): Promise<Redacted>;

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
   * Brings the overlay to the front (MULTI-03).
   *
   * Called for the first handoff opened while none is active, and never for one that
   * arrives beside an active tab — that one gets a badge and waits.
   */
  showWindow(): Promise<void>;

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
export function tauriBridge(): Bridge {
  return {
    async resizeToContent(height) {
      await invoke('resize_to_content', { height });
    },
    async setUiLanguage(language) {
      await invoke('set_ui_language', { language });
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
    async scanTypedText(text) {
      return invoke<Redacted>('scan_typed_text', { text });
    },
    async sessionPicker() {
      return invoke<SessionChoice[]>('session_picker');
    },
    async answerSessionPicker(sessionRef) {
      await invoke('answer_session_picker', { sessionRef });
    },
    async showWindow() {
      await invoke('show_window');
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
    async scanTypedText(text) {
      // Outside the webview there is no detector; saying "nothing was found" here would be
      // a claim this side cannot make, so the text comes back as it went in and the sheet
      // shows it unchanged.
      return { text, kinds: [] };
    },
    async sessionPicker() {
      return [];
    },
    async answerSessionPicker() {},
    async showWindow() {},
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
