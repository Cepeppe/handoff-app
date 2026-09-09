<!--
  The History of §7.6 (VER-09).

  "Previous rounds collapsed, notes, questions, replies." Collapsed is the operative word:
  the current step is what the user is doing and everything before it is context, so it
  lives inside a `<details>` that starts closed and opens when asked.

  Two markers ride along, and they are what makes the history worth keeping. A round after
  the first is a **correction** — §8.1 opens one only from a failed verification (VER-08) —
  and a round whose report came back negative says so. Together they answer the question a
  person actually has when they open this: *why am I doing this a second time?*

  The verification detail keeps the "declared by agent" label it has everywhere else
  (VER-05, PRIN-08): the log preserves a declaration and never a proof.
-->
<script lang="ts">
  import { t } from '../i18n';
  import type { HistoryRoundView } from '../model';

  const { rounds }: { rounds: HistoryRoundView[] } = $props();
</script>

{#if rounds.length > 0}
  <details class="history">
    <summary>{t('overlay.history')}</summary>

    {#each rounds as round (round.no)}
      <section class="history-round" data-round={round.no} class:failed={round.failed}>
        <h2>
          {round.correction
            ? t('overlay.correction', { no: round.no - 1 })
            : t('overlay.round', { no: round.no })}
          {#if round.failed}
            <span class="tag tag-failed">{t('overlay.verificationFailed')}</span>
          {/if}
        </h2>

        <ol>
          {#each round.steps as text, index (index)}
            <li class:done={round.confirmed.includes(index + 1)}>
              {text}
              {#if round.skipped.includes(index + 1)}
                <span class="tag">{t('overlay.skipped')}</span>
              {/if}
            </li>
          {/each}
        </ol>

        {#each round.notes as note, index (index)}
          <p class="history-note">{note.text}</p>
        {/each}

        {#each round.questions as question, index (index)}
          <p class="history-question">
            <span class="who">{t('overlay.question')}:</span>
            {question.text}
          </p>
        {/each}

        {#each round.replies as reply, index (index)}
          <p class="history-reply">
            <span class="who">{t('overlay.reply')}:</span>
            {reply.text}
          </p>
        {/each}

        {#if round.verify !== null && round.verify.detail !== null}
          <p class="history-verify">
            {round.verify.detail}
            <span class="tag">{t('overlay.declaredByAgent')}</span>
            {#if round.verify.late}
              <span class="tag tag-late">{t('overlay.late')}</span>
            {/if}
          </p>
        {/if}
      </section>
    {/each}
  </details>
{/if}
