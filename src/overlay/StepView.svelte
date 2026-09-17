<!--
  One step at a time, with the counter, and never a progress bar (GUIDE-01, GUIDE-05,
  PRIN-07).

  The order on screen is the order the user needs it in: where they are, then what to watch
  out for (GUIDE-04), then what to do, then what to do it with, then where to paste what we
  never see (SEC-02). The **Open** button is the step's own starting point when it has one and
  the spec's otherwise, and it exists only for the four schemes of SPEC-07 — anything else is
  shown as the text it is (SPEC-08).

  The counter is a pill and not a bar, and it never becomes one: GUIDE-05 and PRIN-07 forbid a
  percentage and an estimate anywhere in this product, and a test over the whole frontend
  source fails the build if a progress element ever appears.

  In the narrow panel the step is a card on the surface colour; in the expanded view the whole
  column is already that surface, so `framed` drops the card and keeps the spacing. Nothing
  else differs between the two.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { HandoffView } from '../model';
  import Icon from './Icon.svelte';
  import SecretList from './SecretList.svelte';
  import StepText from './StepText.svelte';
  import ValueChip from './ValueChip.svelte';

  const { view, framed = true }: { view: HandoffView; framed?: boolean } = $props();

  const step = $derived(view.step);
</script>

{#if step !== null}
  <section class="step" class:step-card={framed} data-step={step.counter.index}>
    <div class="step-head">
      <span class="counter">
        {t(step.counter.key, { index: step.counter.index, total: step.counter.total })}
      </span>
      <span class="step-head-gap"></span>
      {#if step.url !== null && step.url.openable}
        <button
          type="button"
          class="button button-small"
          onclick={() => void bridge().openUrl(step.url?.href ?? '')}
        >
          {t('overlay.open')}
          <Icon name="external" size={13} />
        </button>
      {/if}
    </div>

    {#if step.warning !== null}
      <p class="warning" role="note">
        <Icon name="warning" size={15} />
        <span>{step.warning}</span>
      </p>
    {/if}

    <StepText text={step.text} />

    {#if step.url !== null && !step.url.openable}
      <p class="plain-url">{step.url.href}</p>
    {/if}

    {#if step.values.length > 0}
      <section class="values">
        <span class="section-label">{t('overlay.values')}</span>
        <div class="value-box">
          {#each step.values as chip (chip.name)}
            <ValueChip handoffId={view.tab.id} {chip} />
          {/each}
        </div>
      </section>
    {/if}

    <SecretList handoffId={view.tab.id} secrets={view.secrets} />

    {#if step.notes.length > 0}
      <section class="notes">
        <span class="section-label">{t('overlay.notes')}</span>
        <ul>
          {#each step.notes as note, index (index)}
            <li>{note.text}</li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if step.questions.length > 0 || step.replies.length > 0}
      <section class="replies">
        <!--
          The round trip of RESP-04 and TOOL-04, on the step it was about: what was asked,
          then what came back. The question is here and not only in the pending view because
          an answer without its question is half a conversation.
        -->
        {#each step.questions as question, index (index)}
          <p class="question">
            <span class="who">{t('overlay.question')}:</span>
            {question.text}
          </p>
        {/each}
        {#each step.replies as reply, index (index)}
          <p class="reply">
            <span class="who">{t('overlay.reply')}:</span>
            {reply.text}
          </p>
        {/each}
      </section>
    {/if}
  </section>
{/if}
