<!--
  One entry of the waiting group: a parked handoff, or an outcome nobody collected (§7.6,
  SRV-23, RESP-07).

  It is one component because the same row is drawn twice — under the **Waiting** chip of the
  narrow panel, and under the **Waiting** label of the expanded view's list — and the three
  things SRV-23 lets a person do from the list must be the same three in both.

  The entry itself selects the tab; the buttons act on *this* handoff whether or not it is
  selected, which is the point of the requirement: closing an orphan from the list must not
  require opening it first. An orphan's title is in italics, as it is in the tab strip, because
  "nobody collected this" is a fact about the outcome and not about the work.
-->
<script lang="ts">
  import { t } from '../i18n';
  import type { ActionName, TabView } from '../model';
  import { select, selectedId } from './state.svelte';

  const {
    entry,
    onact,
    oncopyid,
  }: {
    entry: TabView;
    onact: (id: string, action: ActionName) => void;
    oncopyid: (id: string) => void;
  } = $props();
</script>

<div class="waiting-entry" data-waiting={entry.id}>
  <button
    type="button"
    class="waiting-title"
    class:waiting-orphan={entry.orphan}
    aria-current={selectedId() === entry.id ? 'true' : undefined}
    title={entry.goal ?? entry.label}
    onclick={() => void select(entry.id)}
  >
    <span class="waiting-goal">{entry.goal ?? entry.label}</span>
    <span class="waiting-meta">{entry.label}</span>
  </button>

  <div class="waiting-actions" role="group" aria-label={entry.label}>
    {#if entry.actions.resume}
      <button
        type="button"
        class="chip-button"
        onclick={() => onact(entry.id, 'resume_from_overlay')}
      >
        {t('action.resume')}
      </button>
    {/if}
    {#if entry.actions.closeOrphan}
      <button type="button" class="chip-button" onclick={() => onact(entry.id, 'close_orphan')}>
        {t('action.closeOrphan')}
      </button>
    {/if}
    <button type="button" class="chip-button chip-button-quiet" onclick={() => oncopyid(entry.id)}>
      {t('action.copyId')}
    </button>
  </div>
</div>
