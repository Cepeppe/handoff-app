<!--
  The single overlay window (§7.6, DD-10).

  One window, one view at a time: the request sheet, the settings and the onboarding are
  modes of this window rather than separate windows, so MULTI-04 ("never two windows")
  holds structurally and focus and always-on-top are managed in one place.

  Four things the skeleton owns and every later view inherits:

  - the header is the drag handle (WIN-02). The window has no decorations, so
    `data-tauri-drag-region` is the only way to move it, and it lives here rather than in
    a view so that dragging works whatever is being shown. Because there are no system
    buttons either, the header also carries the three window controls of §7.6 — Minimize to
    tray, Shrink to bar, Expand — which are the same three, in the same order, on every
    screen.
  - the height follows the content (WIN-02). The frontend is the only side that knows how
    tall the content is, so it measures and asks the core to resize.
  - **the width is derived here and nowhere else.** The collapsed bar is always the panel's
    width, the settings page is wider, and the expanded view is wider still; one `$effect`
    turns that into the single `setWindowLayout` call the Rust side answers. A view that
    asked for a width of its own would be a second opinion about the same window.
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

  import { bridge, type WindowLayout } from './bridge';
  import { captureFinished } from './capture.svelte';
  import { t } from './i18n';
  import type { ActionName } from './model';
  import CollapsedBar from './overlay/CollapsedBar.svelte';
  import ShortcutDialog from './overlay/ShortcutDialog.svelte';
  import WindowControls from './overlay/WindowControls.svelte';
  import {
    collapse,
    focusChanged,
    isCollapsed,
    loadWindowSettings,
    noteInteraction,
    stopIdleTimer,
  } from './overlay/collapse.svelte';
  import {
    allTabs,
    currentView,
    refreshCurrent,
    refreshTabs,
    showNotice,
  } from './overlay/state.svelte';
  import { showView, view } from './view-state.svelte';
  import { form, toggleForm } from './window-form.svelte';
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

  /** How many handoffs are being worked on, for the line beside the name in the header. */
  const openCount = $derived(allTabs().filter((tab) => tab.group === 'open').length);

  /** Whether the expanded view of §7.6 is a shape this screen has at all. */
  const canExpand = $derived(view() === 'overlay');

  const expanded = $derived(canExpand && form() === 'expanded');

  /**
   * The one derivation of the window's shape (§7.6, WIN-02).
   *
   * The order is the precedence: a collapsed window is a bar at the panel's width whatever
   * else is true, the settings page is wide wherever it is opened from, and the expanded
   * view exists only over the overlay. Everything else is the panel of WIN-02.
   */
  const layout = $derived.by((): WindowLayout => {
    if (collapsed) {
      return 'panel';
    }
    if (view() === 'settings') {
      return 'settings';
    }
    return expanded ? 'expanded' : 'panel';
  });

  // The width is the Rust side's to apply, like the height: it is the side that knows what
  // the monitor can show and where the window still has room to grow (`set_window_layout`).
  $effect(() => {
    void bridge().setWindowLayout(layout);
  });

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
   * A double-click on the empty part of the title bar expands and restores.
   *
   * It is the gesture every desktop has on a title bar, and it is the only affordance in the
   * window that is not also a visible control — the third window control does the same thing
   * for anyone who never tries it. The click is ignored on the buttons themselves, so
   * pressing Minimize twice quickly does not also widen the window.
   */
  function headerDoubleClick(event: MouseEvent): void {
    if (!canExpand) {
      return;
    }
    const target = event.target;
    if (target instanceof Element && target.closest('button') !== null) {
      return;
    }
    toggleForm();
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

    // §7.8: a capture ended. It arrives as an event and not as the answer to the command,
    // because a region selection is finished by an overlay window that is destroyed while
    // the pixels are being taken.
    void bridge()
      .onCaptureReady((outcome) => void captureFinished(outcome))
      .then((unlisten) => stopping.push(unlisten));

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
<div class="app" class:app-expanded={expanded} bind:this={root}>
  {#if collapsed && collapsible !== null}
    <CollapsedBar view={collapsible} onact={(action) => void act(action)} />
  {:else}
    <!--
      The double-click is a *second* way to press the third window control, which is on
      screen beside it and reachable with the keyboard: there is nothing here that a
      keyboard user cannot do, which is what the rule protects. Giving the strip a role
      would claim it is a control, and it is the drag handle.
    -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <header class="header" data-tauri-drag-region ondblclick={headerDoubleClick}>
      <img class="brand-mark" src="/baton.svg" alt="" width="18" height="18" />
      <span class="brand-name" data-tauri-drag-region>{t('app.name')}</span>
      {#if openCount > 0}
        <span class="brand-count" data-tauri-drag-region>
          · {t('overlay.inProgressCount', { count: openCount })}
        </span>
      {/if}
      <span class="header-gap" data-tauri-drag-region></span>
      <WindowControls
        canCollapse={collapsible !== null}
        {canExpand}
        {expanded}
        onminimize={() => void bridge().hideWindow()}
        oncollapse={collapse}
        ontoggleform={toggleForm}
      />
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
      <ShortcutDialog accelerator={shortcutProblem} ondone={() => (shortcutProblem = null)} />
    {/if}

    <!--
      One container for both shapes rather than two branches: the expanded view lays itself
      out — the handoff list is the tab strip moved to the left column — so what changes here
      is the padding around it and nothing else. Swapping the element would unmount the view
      on every Expand, and a sheet somebody was typing into would go with it.
    -->
    <main class="content" class:content-expanded={expanded}>
      <Current />
    </main>
  {/if}
</div>
