<!--
  The sheet behind Ask, Note, Defer and Abandon (RESP-02, RESP-04, RESP-05, RESP-08).

  Ask, Defer and Abandon **send** what the user typed, so the certain detector runs over it
  as it is written and the sheet shows what the agent will actually read (§7.10, PRIN-09):
  the redaction is something the user sees before pressing the button, never something that
  happened to their words afterwards. Note annotates the step locally (RESP-03) and is left
  exactly as it was written.

  Enter with a modifier sends, Escape cancels; a plain Enter is a newline, because these are
  sentences and not a search box.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { ActionName, Redacted } from '../model';

  const {
    action,
    optional = false,
    scanned = false,
    onsend,
    oncancel,
  }: {
    action: ActionName;
    /** Whether an empty text is allowed (Defer and Abandon take a reason or nothing). */
    optional?: boolean;
    /** Whether what is typed will be sent, and therefore scanned (§7.10). */
    scanned?: boolean;
    onsend: (text: string) => void;
    oncancel: () => void;
  } = $props();

  let text = $state('');
  let redacted = $state<Redacted | null>(null);

  const ready = $derived(optional || text.trim().length > 0);
  const sending = $derived(redacted?.text ?? text);

  async function rescan(next: string): Promise<void> {
    text = next;
    redacted = scanned ? await bridge().scanTypedText(next) : null;
  }

  function send(): void {
    if (ready) {
      onsend(sending);
    }
  }
</script>

<section class="sheet" data-sheet={action}>
  <label class="sheet-label" for="sheet-text">{t(`sheet.${action}`)}</label>
  <textarea
    id="sheet-text"
    class="sheet-text"
    rows="3"
    value={text}
    oninput={(event) => void rescan(event.currentTarget.value)}
    onkeydown={(event) => {
      if (event.key === 'Escape') {
        oncancel();
      } else if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        send();
      }
    }}
  ></textarea>

  {#if redacted !== null && redacted.kinds.length > 0}
    <p class="sheet-redacted">{t('sheet.redacted', { kinds: redacted.kinds.join(', ') })}</p>
    <p class="sheet-preview" aria-label={t('sheet.willSend')}>{redacted.text}</p>
  {/if}

  <div class="sheet-actions">
    <button type="button" class="button" disabled={!ready} onclick={send}>
      {action === 'note' ? t('sheet.save') : t('sheet.send')}
    </button>
    <button type="button" class="button button-quiet" onclick={oncancel}>
      {t('sheet.cancel')}
    </button>
  </div>
</section>
