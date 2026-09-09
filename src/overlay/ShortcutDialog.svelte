<!--
  The one-time "choose another combination" dialog of FM-18 (OPEN-03, A-19).

  It appears only when the app tried to register the global shortcut at startup and the
  system refused it, which means another application already holds it. Baton never takes a
  combination that is taken, so what is left is to say so once and let the user pick another
  one — and "once" is a setting on the Rust side, because a dialog that returns at every
  launch is a dialog people learn to close without reading. `New request` in the tray menu
  works throughout.

  The recorder is the input: the user presses the combination and the modifiers and the key
  are read from the keyboard event, in the accelerator syntax the plugin parses
  (`Control+Alt+H`). A modifier on its own is not a combination, so it is shown and not
  accepted until a real key arrives. Whether the new one is free is the system's answer, not
  ours: `setShortcut` rejects with the reason and the dialog stays open.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';

  const {
    accelerator,
    ondone,
  }: {
    /** The combination that could not be registered, for the explanation. */
    accelerator: string;
    /** The dialog is finished with: chosen, or dismissed. */
    ondone: () => void;
  } = $props();

  let recorded = $state<string | null>(null);
  let problem = $state<string | null>(null);

  /** The keys that are only ever part of a combination. */
  const MODIFIER_KEYS: ReadonlySet<string> = new Set([
    'Control',
    'Alt',
    'Shift',
    'Meta',
    'AltGraph',
    'Dead',
  ]);

  /**
   * The accelerator a key press describes, or `null` while only modifiers are down.
   *
   * `event.code` rather than `event.key`: the plugin's syntax names physical keys (`KeyH`,
   * `Digit1`, `F5`), and `key` would give `˙` for the very combination OPEN-03 defaults to
   * on a Mac.
   */
  function acceleratorOf(event: KeyboardEvent): string | null {
    if (MODIFIER_KEYS.has(event.key)) {
      return null;
    }
    const parts: string[] = [];
    if (event.ctrlKey) {
      parts.push('Control');
    }
    if (event.altKey) {
      parts.push('Alt');
    }
    if (event.shiftKey) {
      parts.push('Shift');
    }
    if (event.metaKey) {
      parts.push('Super');
    }
    if (parts.length === 0) {
      // A bare key is not a global shortcut: it would swallow that key everywhere.
      return null;
    }
    parts.push(event.code);
    return parts.join('+');
  }

  function record(event: KeyboardEvent): void {
    event.preventDefault();
    const combination = acceleratorOf(event);
    if (combination !== null) {
      recorded = combination;
      problem = null;
    }
  }

  async function save(): Promise<void> {
    if (recorded === null) {
      return;
    }
    try {
      await bridge().setShortcut(recorded);
      ondone();
    } catch {
      problem = t('shortcut.failed');
    }
  }

  async function dismiss(): Promise<void> {
    await bridge().dismissShortcutQuestion();
    ondone();
  }
</script>

<section class="shortcut-dialog" role="group" aria-label={t('shortcut.title')}>
  <h2>{t('shortcut.title')}</h2>
  <p class="shortcut-explain">{t('shortcut.explain', { accelerator })}</p>

  <button type="button" class="shortcut-recorder" onkeydown={record}>
    {recorded ?? t('shortcut.record')}
  </button>

  {#if problem !== null}
    <p class="shortcut-problem" role="alert">{problem}</p>
  {/if}

  <div class="shortcut-actions" role="group" aria-label={t('overlay.actions')}>
    <button
      type="button"
      class="button"
      disabled={recorded === null}
      onclick={() => void save()}
    >
      {t('shortcut.save')}
    </button>
    <button type="button" class="button button-quiet" onclick={() => void dismiss()}>
      {t('shortcut.dismiss')}
    </button>
  </div>
</section>
