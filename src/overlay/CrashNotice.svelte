<!--
  The notice of §7.14: the previous run of Baton ended in a panic (TEL-01, TEL-02).

  One sentence and one action. **Nothing is uploaded, ever** — no telemetry code exists —
  so the only thing the app can offer is to open the folder the report is sitting in and
  let the user decide whether it goes anywhere. That is the whole of TEL-01, and the reason
  the button says "open the folder" and not "send".

  It asks from its own mount rather than from the launch checks of `App.svelte`, because
  unlike those two it switches no view and opens no window: it is a sentence in the overlay,
  and the component that draws it is the one that should ask for it. Asking more than once
  is harmless by construction — `crash_notice` records the report it reported (the
  `known_agents` rule of INST-05), so a remount, a reloaded webview and the launch after
  this one all answer "nothing happened".

  Dismissing it is a local `hidden`: the Rust side has already remembered.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { t } from '../i18n';

  const { onfailed }: { onfailed: (message: string) => void } = $props();

  let crashed = $state(false);
  let hidden = $state(false);

  onMount(() => {
    void bridge()
      .crashNotice()
      .then((notice) => (crashed = notice.crashed))
      // A crash notice that cannot be read is not worth a second notice.
      .catch(() => (crashed = false));
  });

  async function openFolder(): Promise<void> {
    try {
      await bridge().openCrashesFolder();
      hidden = true;
    } catch {
      onfailed(t('notice.openFailed'));
    }
  }
</script>

{#if crashed && !hidden}
  <section class="crash-notice" role="status" aria-label={t('crash.title')}>
    <p class="crash-text">{t('crash.text')}</p>
    <div class="crash-actions">
      <button type="button" class="button" onclick={() => void openFolder()}>
        {t('crash.openFolder')}
      </button>
      <button type="button" class="button button-quiet" onclick={() => (hidden = true)}>
        {t('crash.dismiss')}
      </button>
    </div>
  </section>
{/if}
