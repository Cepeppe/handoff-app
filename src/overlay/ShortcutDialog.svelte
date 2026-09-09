<!--
  The one-time "choose another combination" dialog of FM-18 (OPEN-03, A-19).

  It appears only when the app tried to register the global shortcut at startup and the
  system refused it, which means another application already holds it. Baton never takes a
  combination that is taken, so what is left is to say so once and let the user pick another
  one — and "once" is a setting on the Rust side, because a dialog that returns at every
  launch is a dialog people learn to close without reading. `New request` in the tray menu
  works throughout.

  The recording itself is `ShortcutRecorder`, shared with the **Change** control of the
  General settings page: what this screen adds is the explanation, and a second button that
  puts the question away for good rather than merely closing a control.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';

  import ShortcutRecorder from './ShortcutRecorder.svelte';

  const {
    accelerator,
    ondone,
  }: {
    /** The combination that could not be registered, for the explanation. */
    accelerator: string;
    /** The dialog is finished with: chosen, or dismissed. */
    ondone: () => void;
  } = $props();

  async function dismiss(): Promise<void> {
    await bridge().dismissShortcutQuestion();
    ondone();
  }
</script>

<section class="shortcut-dialog" role="group" aria-label={t('shortcut.title')}>
  <h2>{t('shortcut.title')}</h2>
  <p class="shortcut-explain">{t('shortcut.explain', { accelerator })}</p>

  <ShortcutRecorder
    onsaved={ondone}
    oncancel={() => void dismiss()}
    cancelLabel={t('shortcut.dismiss')}
  />
</section>
