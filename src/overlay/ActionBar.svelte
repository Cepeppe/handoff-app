<!--
  The action bar of §7.6: Done / Ask / Screenshot / Note / Skip / Defer / Abandon, plus the
  two contextual actions of RESP-07 and SRV-23.

  Note and Ask are distinct buttons (RESP-02): one annotates the step here, the other
  interrupts the agent, and a single "comment" button would blur the only difference that
  matters. Abandon is always beside Defer (RESP-08).

  Every button is drawn from `actions`, which the view model computes from the state of §8.4:
  the store refuses an action a state does not offer, and §7.4 calls a button that produces
  that refusal a defect of the view.

  Screenshot is a component of its own, because pressing it opens the two choices of CAP-01
  rather than doing anything, and the collapsed bar offers exactly the same popover.

  Done keeps its label on the last step and changes what it sends: GUIDE-01 gives the user
  one button to advance with, and RESP-09 makes the last press the end of the round rather
  than a step forward. `confirm` and `done` are two transitions of §8.1, so which one this
  button is depends on where the cursor stands and on nothing else.
-->
<script lang="ts">
  import { t } from '../i18n';
  import type { ActionName, ActionsView } from '../model';
  import ScreenshotButton from './ScreenshotButton.svelte';

  const {
    actions,
    lastStep,
    onact,
  }: {
    actions: ActionsView;
    /** Whether the cursor is on the last step of the round (RESP-09). */
    lastStep: boolean;
    onact: (action: ActionName) => void;
  } = $props();
</script>

<div class="actions" role="group" aria-label={t('overlay.actions')}>
  {#if actions.done}
    <button
      type="button"
      class="button button-primary"
      onclick={() => onact(lastStep ? 'done' : 'confirm')}
    >
      {t('action.done')}
    </button>
  {/if}
  {#if actions.ask}
    <button type="button" class="button" onclick={() => onact('ask')}>{t('action.ask')}</button>
  {/if}
  {#if actions.note}
    <button type="button" class="button" onclick={() => onact('note')}>{t('action.note')}</button>
  {/if}
  {#if actions.skip}
    <button type="button" class="button" onclick={() => onact('skip')}>{t('action.skip')}</button>
  {/if}
  {#if actions.screenshot}
    <ScreenshotButton />
  {/if}
  {#if actions.defer}
    <button type="button" class="button" onclick={() => onact('defer')}>{t('action.defer')}</button>
  {/if}
  {#if actions.resume}
    <button type="button" class="button button-primary" onclick={() => onact('resume_from_overlay')}>
      {t('action.resume')}
    </button>
  {/if}
  {#if actions.abandon}
    <button type="button" class="button button-quiet" onclick={() => onact('abandon')}>
      {t('action.abandon')}
    </button>
  {/if}
  {#if actions.closeOrphan}
    <button type="button" class="button" onclick={() => onact('close_orphan')}>
      {t('action.closeOrphan')}
    </button>
  {/if}
</div>
