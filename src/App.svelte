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

  No component in this application carries a `<style>` block: all styling is in
  `styles.css`. The webview CSP is `default-src 'self'` with no `unsafe-inline`, so an
  injected inline stylesheet would be refused, and one file is also what the task asks for.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from './bridge';
  import { t } from './i18n';
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

  onMount(() => {
    const stopping: Array<() => void> = [];

    void bridge()
      .onShowView((next: ViewName) => showView(next))
      .then((unlisten) => stopping.push(unlisten));

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

<div class="app" bind:this={root}>
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

  <main class="content">
    <Current />
  </main>
</div>
