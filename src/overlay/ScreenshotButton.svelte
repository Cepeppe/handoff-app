<!--
  The Screenshot button and the two choices behind it (CAP-01).

  CAP-01 is a rule about what the button must *not* do: pressing it never starts a capture.
  Two options appear, **Full screen** and **Select region**, and the user picks every time —
  including the time after the time they picked the same one. The last choice is highlighted
  so the common case is one glance away, and highlighting is all it does: there is no
  default, no timer and no "press again to repeat".

  The reason is the one PRIN-04 states: a screenshot is the only thing this application does
  that reads the user's screen, so it happens because somebody said which part of it, now.

  The same component is used by the action bar and by the collapsed bar, so the two cannot
  drift into two different popovers.
-->
<script lang="ts">
  import { startCapture } from '../capture.svelte';
  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { CaptureChoice } from '../model';

  const { onchoose }: { onchoose?: () => void } = $props();

  let open = $state(false);
  let last = $state<CaptureChoice | null>(null);

  async function toggle(): Promise<void> {
    if (open) {
      open = false;
      return;
    }
    // Read every time it opens rather than once at mount: the highlight is a fact about
    // the last capture, and the panel outlives many of them.
    last = await bridge()
      .captureSettings()
      .then((settings) => settings.lastChoice)
      .catch(() => null);
    open = true;
  }

  function choose(choice: CaptureChoice): void {
    open = false;
    onchoose?.();
    void startCapture(choice);
  }
</script>

<div
  class="screenshot"
  onkeydown={(event) => {
    if (event.key === 'Escape' && open) {
      open = false;
    }
  }}
  role="presentation"
>
  <button
    type="button"
    class="button"
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={() => void toggle()}
  >
    {t('action.screenshot')}
  </button>

  {#if open}
    <div class="screenshot-menu" role="menu" aria-label={t('capture.choose')}>
      <button
        type="button"
        role="menuitem"
        class="screenshot-choice"
        aria-current={last === 'fullScreen' ? 'true' : undefined}
        onclick={() => choose('fullScreen')}
      >
        {t('capture.fullScreen')}
      </button>
      <button
        type="button"
        role="menuitem"
        class="screenshot-choice"
        aria-current={last === 'region' ? 'true' : undefined}
        onclick={() => choose('region')}
      >
        {t('capture.selectRegion')}
      </button>
    </div>
  {/if}
</div>
