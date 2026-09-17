<!--
  The collapsed bar of WIN-03.

  One line of the current step, and the three actions a person needs without looking at
  anything else: Done, Ask, Screenshot. The other four (Note, Skip, Defer, Abandon) are
  deliberately absent — WIN-03 names them as expanded-panel actions, and a bar with seven
  buttons is not a bar.

  Ask and Screenshot are icon-only here, because the bar is 56 pixels tall and 360 wide and
  the step line is what it is for. Icon-only is not label-less: each carries the same name in
  an `aria-label` and a `title`, so a screen reader and a hover read exactly what the panel
  prints under the icon.

  The step line is itself the button that brings the panel back, so the gesture is "press the
  line you were reading" rather than "find the small arrow"; the window controls at the right
  end repeat it explicitly — **Minimize to tray**, and **Open the panel** — for anyone who
  never tries it. The three action buttons stop the press from reaching the line, because
  pressing Done should do Done and not open the panel.

  It is drawn by `App.svelte` in place of the whole window, which is why it carries its own
  drag region: the header is not on screen while it is.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { askCaptureChoiceInPanel } from '../capture.svelte';
  import { t } from '../i18n';
  import type { ActionName, HandoffView } from '../model';
  import Icon from './Icon.svelte';
  import ScreenshotButton from './ScreenshotButton.svelte';
  import { expand } from './collapse.svelte';

  const {
    view,
    onact,
  }: {
    view: HandoffView;
    onact: (action: ActionName) => void;
  } = $props();

  /** What the one line says: the step being worked on, else the goal, else the tab label. */
  const line = $derived(view.step?.text ?? view.goal ?? view.tab.label);

  /** The counter above it, when there is a step to count (GUIDE-01). */
  const counter = $derived(view.step?.counter ?? null);

  function act(action: ActionName): void {
    // Ask opens a sheet, which needs the panel; Done stays here.
    if (action === 'ask') {
      expand();
    }
    onact(action);
  }
</script>

<div class="collapsed" role="group" aria-label={t('overlay.collapsedBar')} data-tauri-drag-region>
  <button type="button" class="collapsed-line" title={t('overlay.expand')} onclick={() => expand()}>
    {#if counter !== null}
      <span class="collapsed-counter">
        {t(counter.key, { index: counter.index, total: counter.total })}
      </span>
    {/if}
    <span class="collapsed-step">{line}</span>
  </button>

  <div class="collapsed-actions">
    {#if view.actions.done}
      <button
        type="button"
        class="button button-primary"
        onclick={() => act(view.step?.last === true ? 'done' : 'confirm')}
      >
        <Icon name="check" size={14} />
        {t('action.done')}
      </button>
    {/if}
    {#if view.actions.ask}
      <button
        type="button"
        class="icon-button"
        aria-label={t('action.ask')}
        title={t('action.ask')}
        onclick={() => act('ask')}
      >
        <Icon name="ask" />
      </button>
    {/if}
    {#if view.actions.screenshot}
      <!--
        The bar has no room above the button for the two choices of CAP-01, so the press
        opens the panel and the panel's own button shows them (`capture.svelte.ts`).
      -->
      <ScreenshotButton
        handoffId={view.tab.id}
        variant="icon"
        onopen={() => {
          askCaptureChoiceInPanel();
          expand();
        }}
      />
    {/if}
  </div>

  <div class="collapsed-separator"></div>

  <div class="collapsed-window" role="group" aria-label={t('window.controls')}>
    <button
      type="button"
      class="win-button win-button-narrow"
      aria-label={t('window.minimize')}
      title={t('window.minimize')}
      onclick={() => void bridge().hideWindow()}
    >
      <Icon name="minimize" />
    </button>
    <button
      type="button"
      class="win-button win-button-narrow"
      aria-label={t('overlay.expand')}
      title={t('overlay.expand')}
      onclick={() => expand()}
    >
      <Icon name="unfold" />
    </button>
  </div>
</div>
