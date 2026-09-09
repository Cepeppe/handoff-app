<!--
  The collapsed bar of WIN-03.

  One line of the current step, and the three buttons a person needs without looking at
  anything else: Done, Ask, Screenshot. The other four (Note, Skip, Defer, Abandon) are
  deliberately absent — WIN-03 names them as expanded-panel actions, and a bar with seven
  buttons is not a bar.

  The whole strip is clickable and re-expands the panel, so the gesture that brings it back
  is "click it anywhere" rather than "find the small arrow". The three buttons stop the
  click from reaching that handler, because pressing Done should do Done and not open the
  panel; Ask needs the sheet, so it expands on purpose.

  It is drawn by `App.svelte` in place of the whole window, which is why it carries its own
  drag region: the header is not on screen while it is.
-->
<script lang="ts">
  import { t } from '../i18n';
  import type { ActionName, HandoffView } from '../model';
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

  function act(action: ActionName): void {
    // Ask opens a sheet, which needs the panel; Done stays here.
    if (action === 'ask') {
      expand();
    }
    onact(action);
  }
</script>

<div
  class="collapsed"
  role="group"
  aria-label={t('overlay.collapsedBar')}
  data-tauri-drag-region
>
  <button type="button" class="collapsed-line" title={t('overlay.expand')} onclick={() => expand()}>
    {line}
  </button>

  <div class="collapsed-actions">
    {#if view.actions.done}
      <button
        type="button"
        class="button button-primary"
        onclick={() => act(view.step?.last === true ? 'done' : 'confirm')}
      >
        {t('action.done')}
      </button>
    {/if}
    {#if view.actions.ask}
      <button type="button" class="button" onclick={() => act('ask')}>{t('action.ask')}</button>
    {/if}
    <button
      type="button"
      class="button"
      disabled={!view.actions.screenshot}
      title={view.actions.screenshot ? undefined : t('action.screenshotSoon')}
    >
      {t('action.screenshot')}
    </button>
  </div>
</div>
