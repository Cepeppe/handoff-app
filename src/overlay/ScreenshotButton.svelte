<!--
  The Screenshot button and the two choices behind it (CAP-01).

  CAP-01 is a rule about what the button must *not* do: pressing it never starts a capture.
  Two options appear, **Full screen** and **Select region**, and the user picks every time —
  including the time after the time they picked the same one. The last choice is highlighted
  so the common case is one glance away, and highlighting is all it does: there is no
  default, no timer and no "press again to repeat".

  The reason is the one PRIN-04 states: a screenshot is the only thing this application does
  that reads the user's screen, so it happens because somebody said which part of it, now.

  The same component is used by the panel's action bar, the expanded view's and the collapsed
  bar, so the three cannot drift into three different popovers. `variant` is only the shape of
  the trigger: the icon-over-label tool of the panel, the ordinary button of the expanded row,
  or the icon-only square of the bar, where an `aria-label` carries the name the label would.

  **The bar opens the panel first.** The window is 56 pixels tall there and the popover opens
  *above* its button, so it would be drawn outside the window and clipped away. Pressing
  Screenshot on the bar therefore expands the panel and opens the menu in it — the same two
  choices, in a window tall enough to show them.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { startCapture, takeCaptureChoiceRequest } from '../capture.svelte';
  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { CaptureChoice } from '../model';
  import Icon from './Icon.svelte';

  const {
    handoffId,
    variant = 'tool',
    onopen,
  }: {
    /**
     * The tab the capture belongs to.
     *
     * A screenshot is an interrupting action on a handoff (§7.4): the exemption list of
     * DET-03 is that handoff's spec and the outcome names its step, so the id travels with
     * the capture from this press. Reading the selected tab when the preview opens instead
     * would send the picture to whichever tab the user had switched to meanwhile.
     */
    handoffId: string;
    /** `tool` in the panel, `button` in the expanded row, `icon` on the collapsed bar. */
    variant?: 'tool' | 'button' | 'icon';
    /**
     * The press has to be answered somewhere else first (the collapsed bar).
     *
     * It runs *before* the menu opens, and the menu is then drawn wherever this component
     * ends up — which for the bar is the panel it has just opened.
     */
    onopen?: () => void;
  } = $props();

  let open = $state(false);
  let last = $state<CaptureChoice | null>(null);

  async function toggle(): Promise<void> {
    if (open) {
      open = false;
      return;
    }
    onopen?.();
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
    void startCapture(choice, handoffId);
  }

  onMount(() => {
    // The press happened on the collapsed bar, whose button is already gone with the bar:
    // this is the one that shows the choices (`capture.svelte.ts` says why). The icon-only
    // variant never claims the request — it is the bar's own button, and it is what asked.
    if (variant !== 'icon' && takeCaptureChoiceRequest()) {
      void toggle();
    }
  });
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
    class={variant === 'tool' ? 'tool' : variant === 'icon' ? 'icon-button' : 'button'}
    aria-haspopup="menu"
    aria-expanded={open}
    aria-label={variant === 'icon' ? t('action.screenshot') : undefined}
    title={t('action.screenshot')}
    onclick={() => void toggle()}
  >
    <Icon name="camera" size={variant === 'tool' ? 17 : 16} />
    {#if variant !== 'icon'}{t('action.screenshot')}{/if}
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
