<!--
  Recording a global shortcut (OPEN-03, FM-18, A-19).

  The user presses the combination and the modifiers and the key are read from the keyboard
  event, in the accelerator syntax the plugin parses (`Control+Alt+H`). A modifier on its own
  is not a combination, so it is shown and not accepted until a real key arrives. Whether the
  new one is free is the system's answer and not ours: `setShortcut` rejects with the reason
  and the recorder stays open with the combination the user pressed still in it.

  It is a component of its own because there are two doors to the same act: the one-time
  dialog of FM-18, which opens when the combination could not be registered at startup, and
  the **Change** control of the General settings page, where nothing is wrong and the user
  simply wants another one. What differs between them is what is said around the recorder and
  what the second button means — dismiss the question for ever, or close the control — so
  both are the caller's, and the keys and the failure are here.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';

  const {
    oncancel,
    cancelLabel,
    onsaved,
  }: {
    /** The user is finished without choosing. What that means is the caller's. */
    oncancel: () => void;
    /** The label of that second button: dismissing a question is not cancelling a change. */
    cancelLabel: string;
    /** The system accepted the combination and it is now in force. */
    onsaved: (accelerator: string) => void;
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
      onsaved(recorded);
    } catch {
      problem = t('shortcut.failed');
    }
  }
</script>

<button type="button" class="shortcut-recorder" onkeydown={record}>
  {recorded ?? t('shortcut.record')}
</button>

{#if problem !== null}
  <p class="shortcut-problem" role="alert">{problem}</p>
{/if}

<div class="shortcut-actions" role="group" aria-label={t('overlay.actions')}>
  <button type="button" class="button" disabled={recorded === null} onclick={() => void save()}>
    {t('shortcut.save')}
  </button>
  <button type="button" class="button button-quiet" onclick={oncancel}>
    {cancelLabel}
  </button>
</div>
