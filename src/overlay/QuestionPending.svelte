<!--
  The Question-pending view of §7.6 (RESP-04, TOOL-04).

  What the user asked, and the fact that the agent has not answered yet. The banner of §8.4
  says the second part ("Sent to the agent, waiting for the reply"); what this adds is the
  first, because a person who asked something twenty minutes ago no longer remembers the
  words they used, and a screen that only says "waiting" tells them nothing they can act on.

  The reply itself is **not** drawn here: TOOL-04 puts it on the step it referred to, which
  is `StepView`. That is the whole point of the round trip — the answer belongs beside the
  thing it is about, not in a message list.
-->
<script lang="ts">
  import { t } from '../i18n';
  import type { PendingView } from '../model';

  const { pending }: { pending: PendingView } = $props();
</script>

<section class="pending" data-ui-state="questionSent" data-pending={pending.kind}>
  <p class="pending-what">
    {pending.kind === 'question'
      ? t('overlay.pendingQuestion', { step: pending.step })
      : t('overlay.pendingScreenshot', { step: pending.step })}
  </p>

  {#if pending.text !== null}
    <blockquote class="pending-text">{pending.text}</blockquote>
  {/if}
</section>
