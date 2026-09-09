<!--
  The Waiting-for-spec view of §7.6 (OPEN-04, OPEN-05).

  The tab appears the moment the user presses Enter in the request sheet, before any agent
  has said anything, and this is what stands in it until the spec arrives: what they typed,
  which session it went to, that nothing has come back yet, and the two things they can
  actually do about it — copy the sentence again, or give up.

  "Copy request again" exists because the first copy happened when the request was opened
  (OPEN-05) and a clipboard does not wait: by the time the user notices the agent never
  picked it up, what they copied is long gone. The sentence is rendered on the Rust side, in
  their language, from the same text the Stop hook delivers.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { ActionName, HandoffView } from '../model';

  const {
    view,
    onact,
  }: {
    view: HandoffView;
    onact: (action: ActionName) => void;
  } = $props();
</script>

<section class="waiting-for-spec" data-ui-state="waitingForSpec">
  {#if view.requestText !== null}
    <p class="request-text">{view.requestText}</p>
  {/if}

  {#if view.tab.agent !== null}
    <p class="session">{t('overlay.session')}: {view.tab.label}</p>
  {/if}

  <p class="not-answered">{t('overlay.notAnsweredYet')}</p>

  <div class="actions" role="group" aria-label={t('overlay.actions')}>
    <button
      type="button"
      class="button"
      onclick={() => void bridge().copyRequestText(view.tab.id)}
    >
      {t('action.copyRequest')}
    </button>
    {#if view.actions.abandon}
      <button type="button" class="button button-quiet" onclick={() => onact('abandon')}>
        {t('action.abandon')}
      </button>
    {/if}
  </div>
</section>
