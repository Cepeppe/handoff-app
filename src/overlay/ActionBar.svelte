<!--
  The action bar of §7.6: what the user presses on the step they are looking at.

  Its shape says what each button is for. **One primary button** the width of the bar is the
  thing to do next — Done while a handoff is being guided, Resume on a parked one, Close it
  on an outcome nobody collected — and under it a row of equal tools: Ask, Screenshot, Note
  and More. Skip, Defer and Abandon live inside More (`MoreMenu.svelte` says why), and
  Resume and Close it never do: in the states that offer them they *are* the main action.

  Note and Ask stay two different buttons (RESP-02): one annotates the step here, the other
  interrupts the agent, and a single "comment" button would blur the only difference that
  matters.

  Every button is drawn from `actions`, which the view model computes from the state of §8.4:
  the store refuses an action a state does not offer, and §7.4 calls a button that produces
  that refusal a defect of the view. Nothing is hover-only.

  Screenshot is a component of its own, because pressing it opens the two choices of CAP-01
  rather than doing anything, and the collapsed bar offers exactly the same popover.

  Done keeps its label on the last step and changes what it sends: GUIDE-01 gives the user
  one button to advance with, and RESP-09 makes the last press the end of the round rather
  than a step forward. `confirm` and `done` are two transitions of §8.1, so which one this
  button is depends on where the cursor stands and on nothing else.

  The same component draws the panel's bar and the expanded view's, because they offer the
  same things: `expanded` turns the column into a row and the tools into ordinary buttons.
-->
<script lang="ts">
  import { t } from '../i18n';
  import type { ActionName, ActionsView } from '../model';
  import Icon from './Icon.svelte';
  import MoreMenu from './MoreMenu.svelte';
  import ScreenshotButton from './ScreenshotButton.svelte';

  const {
    handoffId,
    actions,
    lastStep,
    expanded = false,
    onact,
  }: {
    /** The tab these buttons act on; the Screenshot flow carries it to the preview. */
    handoffId: string;
    actions: ActionsView;
    /** Whether the cursor is on the last step of the round (RESP-09). */
    lastStep: boolean;
    /** Whether the bar is the expanded view's row rather than the panel's column. */
    expanded?: boolean;
    onact: (action: ActionName) => void;
  } = $props();

  /** How many tools the row holds, so the panel's grid gives each of them the same width. */
  const toolCount = $derived(
    [actions.ask, actions.screenshot, actions.note, actions.skip || actions.defer || actions.abandon]
      .filter(Boolean).length,
  );
</script>

<div
  class="action-bar"
  class:action-bar-wide={expanded}
  role="group"
  aria-label={t('overlay.actions')}
  data-tools={toolCount}
>
  {#if actions.done}
    <button
      type="button"
      class="button button-primary action-primary"
      onclick={() => onact(lastStep ? 'done' : 'confirm')}
    >
      <Icon name="check" size={expanded ? 15 : 16} />
      {t('action.done')}
    </button>
  {/if}
  {#if actions.resume}
    <button
      type="button"
      class="button button-primary action-primary"
      onclick={() => onact('resume_from_overlay')}
    >
      <Icon name="check" size={expanded ? 15 : 16} />
      {t('action.resume')}
    </button>
  {/if}
  {#if actions.closeOrphan}
    <button
      type="button"
      class="button button-primary action-primary"
      onclick={() => onact('close_orphan')}
    >
      <Icon name="check" size={expanded ? 15 : 16} />
      {t('action.closeOrphan')}
    </button>
  {/if}

  <div class="action-tools">
    {#if actions.ask}
      <button type="button" class={expanded ? 'button' : 'tool'} onclick={() => onact('ask')}>
        <Icon name="ask" size={expanded ? 15 : 17} />
        {t('action.ask')}
      </button>
    {/if}
    {#if actions.screenshot}
      <ScreenshotButton {handoffId} variant={expanded ? 'button' : 'tool'} />
    {/if}
    {#if actions.note}
      <button type="button" class={expanded ? 'button' : 'tool'} onclick={() => onact('note')}>
        <Icon name="note" size={expanded ? 15 : 17} />
        {t('action.note')}
      </button>
    {/if}
    <!--
      The flexible space of the expanded row: More sits at the far end, away from Done, which
      is the same distance the panel puts between them by making it the last column.
    -->
    {#if expanded}
      <span class="action-gap"></span>
    {/if}
    <MoreMenu {actions} variant={expanded ? 'button' : 'tool'} {onact} />
  </div>
</div>
