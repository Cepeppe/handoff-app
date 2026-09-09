<!--
  The "which session is this?" picker of FM-22 and SRV-18.

  A Stop hook arrived, its ancestor chain matched more than one registered session, and the
  working directory did not separate them (§7.5). The hook was answered neutrally — nothing
  is ever guessed about which agent is which — and the question was kept for the next time
  the user is looking at the window, which is now.

  Answering binds the agent session id the hook carried, so the safety net of SRV-12 works
  from the next end of turn on. "I do not know" is a real answer: it drops the question
  without binding anything, and the next hook asks again.

  The question is re-read from the core on `sessionsChanged`; the event carries no payload,
  because the registry is the one source of truth about who is connected.
-->
<script lang="ts">
  import { t } from '../i18n';
  import type { SessionChoice } from '../model';

  const {
    choices,
    onanswer,
  }: {
    choices: SessionChoice[];
    /** The chosen `session_ref`, or `null` for "I do not know". */
    onanswer: (sessionRef: string | null) => void;
  } = $props();
</script>

{#if choices.length > 0}
  <section class="session-picker" role="group" aria-label={t('picker.title')}>
    <h2>{t('picker.title')}</h2>
    <p class="picker-explain">{t('picker.explain')}</p>

    <div class="picker-choices">
      {#each choices as choice (choice.sessionRef)}
        <button type="button" class="button" onclick={() => onanswer(choice.sessionRef)}>
          {choice.label}
        </button>
      {/each}
      <button type="button" class="button button-quiet" onclick={() => onanswer(null)}>
        {t('picker.dismiss')}
      </button>
    </div>
  </section>
{/if}
