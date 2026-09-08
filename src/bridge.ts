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
 * grants the least privilege each task needs and nothing more.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import type { Language } from './i18n';
import { isViewName, type ViewName } from './views';

/**
 * The event the tray menu emits to bring a view forward.
 *
 * The literal is duplicated in `src-tauri/src/ui_bridge/mod.rs` (`EVENT_SHOW_VIEW`) and a
 * Rust test reads this file to prove the two spellings still agree.
 */
export const EVENT_SHOW_VIEW = 'ui://show-view';

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
  };
}

/** A bridge that accepts every call and does nothing, for a browser or a test. */
export function noopBridge(): Bridge {
  return {
    async resizeToContent() {},
    async setUiLanguage() {},
    async onShowView() {
      return () => {};
    },
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
