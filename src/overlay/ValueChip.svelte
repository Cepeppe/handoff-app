<!--
  One value of the step, as rows of the values box (GUIDE-02, DET-04).

  A row is a name, the value in the monospaced face a value deserves, and the buttons that act
  on it: the name column is fixed so a box of four values reads as a table rather than as four
  paragraphs, and the value wraps inside its own column so a long token never widens the
  window (WIN-02).

  Copy is always offered, and it always copies the **true** value — that is the whole point
  of DET-04: a secret-treated value is hidden from the screen, not withheld from the user who
  has to paste it. An array is copyable as a whole, on its name row, and per item, on each of
  its own (GUIDE-02).

  A masked value arrives as `••••••` and never as itself: **Show** asks for it, and the ten
  seconds are counted in `state.svelte.ts`, which drops it when they are up. The buttons are
  icon-only and carry the same names they had as words, in an `aria-label` and a `title`, so
  what a screen reader and a hover say is still "Copy", "Show", "Copy this one".
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { ValueChipView } from '../model';
  import Icon from './Icon.svelte';
  import { reveal, revealedValue } from './state.svelte';

  const { handoffId, chip }: { handoffId: string; chip: ValueChipView } = $props();

  const shown = $derived(revealedValue(`${handoffId}/${chip.name}`));
  const items = $derived(shown ?? chip.items);
  const hidden = $derived(chip.masked && shown === null);
</script>

<div class="value" data-value={chip.name}>
  <div class="value-row" class:value-masked={hidden}>
    <span class="value-name">
      {chip.name}
      {#if chip.masked}
        <span class="value-kind" title={t('overlay.masked', { kind: chip.kind ?? '' })}>
          {chip.kind}
        </span>
      {/if}
    </span>

    <!--
      A list has no single value to print on its name row: the items are the rows below, and
      this row is where the whole of it is copied from.
    -->
    <span class="value-text">{chip.list ? '' : (items[0] ?? '')}</span>

    <span class="value-actions">
      {#if chip.masked}
        <button
          type="button"
          class="icon-button"
          aria-label={shown === null ? t('overlay.show') : t('overlay.hide')}
          title={shown === null ? t('overlay.show') : t('overlay.hide')}
          onclick={() => void reveal(handoffId, chip.name)}
        >
          <Icon name="eye" size={15} />
        </button>
      {/if}
      <button
        type="button"
        class="icon-button"
        aria-label={t('overlay.copy')}
        title={t('overlay.copy')}
        onclick={() => void bridge().copyValue(handoffId, chip.name)}
      >
        <Icon name="copy" size={15} />
      </button>
    </span>
  </div>

  {#if chip.list}
    {#each items as item, index (index)}
      <div class="value-row value-item" class:value-masked={hidden}>
        <span class="value-name"></span>
        <span class="value-text">{item}</span>
        <span class="value-actions">
          <button
            type="button"
            class="icon-button"
            aria-label={`${t('overlay.copyItem')}: ${chip.name} ${index + 1}`}
            title={t('overlay.copyItem')}
            onclick={() => void bridge().copyValue(handoffId, chip.name, index)}
          >
            <Icon name="copy" size={15} />
          </button>
        </span>
      </div>
    {/each}
  {/if}
</div>
