<!--
  One value chip (GUIDE-02, DET-04).

  Copy is always offered, and it always copies the **true** value — that is the whole point
  of DET-04: a secret-treated value is hidden from the screen, not withheld from the user who
  has to paste it. An array is copyable as a whole and per item.

  A masked value arrives as `••••••` and never as itself: **Show** asks for it, and the ten
  seconds are counted in `state.svelte.ts`, which drops it when they are up.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { ValueChipView } from '../model';
  import { reveal, revealedValue } from './state.svelte';

  const { handoffId, chip }: { handoffId: string; chip: ValueChipView } = $props();

  const shown = $derived(revealedValue(`${handoffId}/${chip.name}`));
  const items = $derived(shown ?? chip.items);
</script>

<div class="chip" data-value={chip.name}>
  <div class="chip-head">
    <span class="chip-name">{chip.name}</span>
    {#if chip.masked}
      <span class="chip-kind" title={t('overlay.masked', { kind: chip.kind ?? '' })}
        >{chip.kind}</span
      >
      <button type="button" class="chip-action" onclick={() => void reveal(handoffId, chip.name)}>
        {shown === null ? t('overlay.show') : t('overlay.hide')}
      </button>
    {/if}
    <button
      type="button"
      class="chip-action"
      onclick={() => void bridge().copyValue(handoffId, chip.name)}
    >
      {t('overlay.copy')}
    </button>
  </div>

  <ul class="chip-items" class:chip-masked={chip.masked && shown === null}>
    {#each items as item, index (index)}
      <li>
        <span class="chip-value">{item}</span>
        {#if chip.list}
          <button
            type="button"
            class="chip-action"
            aria-label={`${t('overlay.copyItem')}: ${chip.name} ${index + 1}`}
            onclick={() => void bridge().copyValue(handoffId, chip.name, index)}
          >
            {t('overlay.copyItem')}
          </button>
        {/if}
      </li>
    {/each}
  </ul>
</div>
