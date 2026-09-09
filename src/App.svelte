<!--
  The single overlay window (§7.6, DD-10).

  One window, one view at a time: the request sheet, the settings and the onboarding are
  modes of this window rather than separate windows, so MULTI-04 ("never two windows")
  holds structurally and focus and always-on-top are managed in one place.

  Three things the skeleton owns and every later view inherits:

  - the header is the drag handle (WIN-02). The window has no decorations, so
    `data-tauri-drag-region` is the only way to move it, and it lives here rather than in
    a view so that dragging works whatever is being shown.
  - the height follows the content (WIN-02). The frontend is the only side that knows how
    tall the content is, so it measures and asks the core to resize; the width is fixed.
  - the tray menu switches views from outside the component tree, through the bridge.

  The collapsed bar of WIN-03 is here rather than inside the overlay, because it replaces
  the *window* and not a view: the header goes with it, and the bar carries its own drag
  region. It is drawn only over a handoff being guided — collapsing the settings page or the
  request sheet to "the current step" would be collapsing them to nothing.

  No component in this application carries a `<style>` block: all styling is in
  `styles.css`. The webview CSP is `default-src 'self'` with no `unsafe-inline`, so an
  injected inline stylesheet would be refused, and one file is also what the task asks for.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from './bridge';
  import { t } from './i18n';
  import type { ActionName } from './model';
  import CollapsedBar from './overlay/CollapsedBar.svelte';
  import ShortcutDialog from './overlay/ShortcutDialog.svelte';
  import {
    focusChanged,
    isCollapsed,
    loadWindowSettings,
    noteInteraction,
    stopIdleTimer,
  } from './overlay/collapse.svelte';
  import { currentView, refreshCurrent, refreshTabs, showNotice } from './overlay/state.svelte';
  import { showView, view } from './view-state.svelte';
  import { VIEW_NAMES, viewTitleKey, type ViewName } from './views';
  import OnboardingView from './views/OnboardingView.svelte';
  import OverlayView from './views/OverlayView.svelte';
  import PreviewView from './views/PreviewView.svelte';
  import RequestView from './views/RequestView.svelte';
  import SettingsView from './views/SettingsView.svelte';

  const VIEWS = {
    overlay: OverlayView,
    request: RequestView,
    settings: SettingsView,
    onboarding: OnboardingView,
    preview: PreviewView,
  } as const;

  /**
   * The dev-only view switcher. It exists so that the placeholder views can be reached
   * before the real routing does it for them; a release build has no way to show it.
   *
   * T-036 built the overlay and left this: the tray already reaches the request sheet and
   * the settings, but onboarding is opened by a first launch (T-040) and the preview by a
   * screenshot (T-049), so until then those two have no other door.
   */
  // TASK: T-049 — delete this once every view has a way in of its own.
  const showDevMenu = import.meta.env.DEV;

  let root = $state<HTMLElement | null>(null);
  const Current = $derived(VIEWS[view()]);

  /**
   * The combination whose registration failed at startup, while the FM-18 dialog is unanswered.
   *
   * Read once, when the window mounts: the shortcut is registered in `setup()` and the
   * answer is a setting, so there is nothing to listen to. `null` is the ordinary case.
   */
  let shortcutProblem = $state<string | null>(null);

  /**
   * The handoff the collapsed bar would show, when there is one to show (WIN-03).
   *
   * Only the overlay collapses, and only over a tab that is being guided: there is no
   * "current step" behind the settings page, and a final tab has nothing left to press.
   */
  const collapsible = $derived.by(() => {
    if (view() !== 'overlay') {
      return null;
    }
    const handoff = currentView();
    return handoff !== null && handoff.step !== null ? handoff : null;
  });

  const collapsed = $derived(isCollapsed() && collapsible !== null);

  /** One action from the bar, on the tab the bar is showing. */
  async function act(action: ActionName): Promise<void> {
    const handoff = collapsible;
    if (handoff === null) {
      return;
    }
    try {
      await bridge().act(handoff.tab.id, action);
    } catch {
      // The core pushed the sentence the user reads; the view redraws what it now says.
    }
    await refreshCurrent();
    await refreshTabs();
  }

  /**
   * The two launch checks of §7.2 that decide whether the window opens by itself.
   *
   * §7.16 keeps the panel hidden until there is something to show, and the Rust side no
   * longer shows it at startup: what is worth interrupting the user for is decided here,
   * where both answers are sentences a person reads.
   *
   * - **A first launch shows onboarding** (F-13). Nothing else runs then: the scan would
   *   announce as a discovery the very agent the consent screen is about to show, and
   *   `finishOnboarding` records what was found so it is never announced later either.
   * - **A moved bundle shows the repair offer** (FM-23). The agents still spawn the old
   *   path, so every session of this machine is in text mode with nothing on screen to say
   *   why; the Agents page names both paths and repairs them in one press.
   * - **A newly found agent gets one discreet notice** (INST-05), and only that: it is news,
   *   not a problem, and the window is not taken away from whatever it was showing.
   *
   * **A launch from the login entry opens nothing** (APP-01, `--hidden`). Baton "stays in the
   * background doing nothing until an agent asks for a handoff", and a panel that appears
   * while somebody is logging in is the opposite of that promise. The view is still switched,
   * so the first thing they see when they open Baton from the tray is what wanted them.
   */
  async function checkAtLaunch(): Promise<void> {
    const startedHidden = await bridge()
      .generalSettings()
      .then((settings) => settings.startedHidden)
      .catch(() => false);
    const raise = async (): Promise<void> => {
      if (!startedHidden) {
        await bridge().showWindow();
      }
    };

    if ((await bridge().onboarding()).needed) {
      await raise();
      showView('onboarding');
      return;
    }

    const report = await bridge().scanAgents();
    if (report.moved.length > 0) {
      await raise();
      showView('settings');
      return;
    }
    for (const agent of report.newAgents) {
      showNotice({ kind: 'info', text: t('install.newAgent', { agent: t(agent.nameKey) }) });
    }
  }

  onMount(() => {
    const stopping: Array<() => void> = [];

    void bridge()
      .onShowView((next: ViewName) => showView(next))
      .then((unlisten) => stopping.push(unlisten));

    // §7.2: the scan and the path check, once per launch.
    void checkAtLaunch();

    // WIN-03: clicking elsewhere collapses the panel, clicking back on it expands it. The
    // Rust side is where Tauri reports the focus, so the fact arrives as an event.
    void bridge()
      .onWindowFocus(focusChanged)
      .then((unlisten) => stopping.push(unlisten));

    // R-10: the fallback timer, off unless the user switched it on.
    void loadWindowSettings();
    stopping.push(stopIdleTimer);

    // FM-18: the global shortcut was taken by something else and the user has not been asked
    // yet. The tray's `New request` works meanwhile, so this is a dialog and not a blocker.
    void bridge()
      .shortcutStatus()
      .then((status) => {
        shortcutProblem = status.askForAnother ? status.accelerator : null;
      });

    // The window height follows the content, so it is remeasured whenever the content
    // changes rather than only after a view switch. jsdom has no `ResizeObserver`, and a
    // plain browser has no window to resize, so both simply skip it.
    if (root !== null && typeof ResizeObserver !== 'undefined') {
      const observed = root;
      const observer = new ResizeObserver(() => {
        void bridge().resizeToContent(Math.ceil(observed.getBoundingClientRect().height));
      });
      observer.observe(observed);
      stopping.push(() => observer.disconnect());
    }

    return () => {
      for (const stop of stopping) {
        stop();
      }
    };
  });
</script>

<!--
  `onpointerdown` and `onkeydown` are the "last interaction" of R-10 and nothing else: they
  restart a timer that is only armed when the setting is on, and they never swallow an
  event. The handlers sit on a plain container, so no element loses its own behaviour.
-->
<svelte:document onpointerdown={noteInteraction} onkeydown={noteInteraction} />

<!--
  The measured element is the outer one, so the height follows the content in **both**
  shapes: WIN-02 asks for a content-driven height and WIN-03 makes the collapsed bar one of
  the contents it has to follow. Measuring the panel alone would leave the window its full
  height with a one-line bar in it.
-->
<div class="app" bind:this={root}>
  {#if collapsed && collapsible !== null}
    <CollapsedBar view={collapsible} onact={(action) => void act(action)} />
  {:else}
    <header class="header" data-tauri-drag-region>
      <span class="title" data-tauri-drag-region>{t('app.name')}</span>
    </header>

    {#if showDevMenu}
      <nav class="dev-menu" aria-label={t('dev.views')}>
        {#each VIEW_NAMES as name (name)}
          <button
            type="button"
            class="dev-menu-item"
            aria-current={view() === name ? 'page' : undefined}
            onclick={() => showView(name)}
          >
            {t(viewTitleKey(name))}
          </button>
        {/each}
      </nav>
    {/if}

    {#if shortcutProblem !== null}
      <ShortcutDialog
        accelerator={shortcutProblem}
        ondone={() => (shortcutProblem = null)}
      />
    {/if}

    <main class="content">
      <Current />
    </main>
  {/if}
</div>
