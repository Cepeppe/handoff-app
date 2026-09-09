<!--
  The mandatory screenshot preview of §7.10 (PREV-01..05).

  What it draws today is the capture and nothing else: OCR is T-047, the detector and its
  boxes are T-048, and the two send buttons are T-049. PREV-01 already holds all the same —
  everything that reaches an agent goes through here, and nothing can reach one yet.

  The other half is FM-17. macOS raises its screen-recording prompt on the *first* attempt
  to read the screen, which in the middle of a handoff is the interruption CAP-04 exists to
  prevent; so the capture asks first, and when the answer is no this is where the
  explanation and the button to the settings pane are shown, in place of the image.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { discardPreview, preview } from '../capture.svelte';
  import { t } from '../i18n';

  const state = $derived(preview());
</script>

<section class="view" data-view="preview">
  <h1>{t('view.preview')}</h1>

  {#if state.status === 'ready'}
    <img class="preview-image" src={state.url} alt={t('preview.alt')} />
    <p class="preview-size">{t('preview.size', { width: state.width, height: state.height })}</p>
    <p class="preview-note">{t('preview.sendSoon')}</p>
  {:else if state.status === 'denied'}
    <p class="preview-problem" role="status">{t('onboarding.screenRecordingText')}</p>
    <button
      type="button"
      class="button"
      onclick={() => void bridge().openScreenRecordingSettings()}
    >
      {t('onboarding.screenRecordingOpen')}
    </button>
  {:else if state.status === 'failed'}
    <p class="preview-problem" role="status">{t('preview.failed', { message: state.message })}</p>
  {:else}
    <p class="preview-empty">{t('preview.empty')}</p>
  {/if}

  <div class="preview-actions">
    <button type="button" class="button button-quiet" onclick={() => void discardPreview()}>
      {t('preview.discard')}
    </button>
  </div>
</section>
