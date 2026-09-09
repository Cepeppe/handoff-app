<!--
  One step at a time, with the counter, and never a progress bar (GUIDE-01, GUIDE-05,
  PRIN-07).

  The order on screen is the order the user needs it in: what to watch out for first
  (GUIDE-04), then what to do, then what to do it with, then where to paste what we never
  see (SEC-02). The **Open** button is the step's own starting point when it has one and the
  spec's otherwise, and it exists only for the four schemes of SPEC-07 — anything else is
  shown as the text it is (SPEC-08).
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { HandoffView } from '../model';
  import SecretList from './SecretList.svelte';
  import StepText from './StepText.svelte';
  import ValueChip from './ValueChip.svelte';

  const { view }: { view: HandoffView } = $props();

  const step = $derived(view.step);
</script>

{#if step !== null}
  <section class="step" data-step={step.counter.index}>
    <p class="counter">
      {t(step.counter.key, { index: step.counter.index, total: step.counter.total })}
    </p>

    {#if step.warning !== null}
      <p class="warning" role="note">{step.warning}</p>
    {/if}

    <StepText text={step.text} />

    {#if step.url !== null}
      {#if step.url.openable}
        <button
          type="button"
          class="button"
          onclick={() => void bridge().openUrl(step.url?.href ?? '')}
        >
          {t('overlay.open')}
        </button>
      {:else}
        <p class="plain-url">{step.url.href}</p>
      {/if}
    {/if}

    {#if step.values.length > 0}
      <div class="chips">
        {#each step.values as chip (chip.name)}
          <ValueChip handoffId={view.tab.id} {chip} />
        {/each}
      </div>
    {/if}

    <SecretList handoffId={view.tab.id} secrets={view.secrets} />

    {#if step.notes.length > 0}
      <section class="notes">
        <h2>{t('overlay.notes')}</h2>
        <ul>
          {#each step.notes as note, index (index)}
            <li>{note.text}</li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if step.replies.length > 0}
      <section class="replies">
        <h2>{t('overlay.reply')}</h2>
        {#each step.replies as reply, index (index)}
          <p class="reply">{reply.text}</p>
        {/each}
      </section>
    {/if}
  </section>
{/if}
