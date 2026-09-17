<!--
  The sheet behind Ask, Note, Defer and Abandon (RESP-02, RESP-04, RESP-05, RESP-08).

  Ask, Defer and Abandon **send** what the user typed, so both detectors run over it as it
  is written and the sheet shows what the agent will actually read (§7.10, PRIN-09): the
  redaction is something the user sees before pressing the button, never something that
  happened to their words afterwards. Note annotates the step locally (RESP-03) and is left
  exactly as it was written.

  The two levels are treated as DET-01 asks, and they say so differently on purpose. A
  **certain** match is already replaced in the preview, and its line carries the shield in the
  accent colour: something was done, and there is no way to put it back. A **suspected** one is
  marked in the preview and is sent as it was written, in the warning colour: the decision is
  the user's, and here that decision is the keyboard they are already holding.

  Enter with a modifier sends, Escape cancels; a plain Enter is a newline, because these are
  sentences and not a search box. The keycap at the end of the row says both, in the keys of
  the machine it is running on (`keys.ts`).

  It takes the action bar's place rather than floating over it: the window's height follows
  its content (WIN-02), so a sheet is one more thing the panel is tall enough for.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import { sendModifier } from '../keys';
  import type { ActionName, Redacted } from '../model';
  import Icon from './Icon.svelte';

  const {
    handoffId,
    action,
    optional = false,
    scanned = false,
    onsend,
    oncancel,
  }: {
    /** The handoff the sheet belongs to; its values are the exemption list (DET-03). */
    handoffId: string;
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
    redacted = scanned ? await bridge().scanTypedText(handoffId, next) : null;
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

  {#if redacted !== null && (redacted.kinds.length > 0 || redacted.reasons.length > 0)}
    {#if redacted.kinds.length > 0}
      <p class="sheet-redacted">
        <Icon name="shield" />
        <span>{t('sheet.redacted', { kinds: redacted.kinds.join(', ') })}</span>
      </p>
    {/if}
    {#if redacted.reasons.length > 0}
      <p class="sheet-suspected">
        <Icon name="warning" size={15} />
        <span>{t('sheet.suspected')}</span>
      </p>
    {/if}
    <div class="sheet-will-send">
      <span class="section-label">{t('sheet.willSend')}</span>
      <p class="sheet-preview" aria-label={t('sheet.willSend')}>{#each redacted.segments as segment,
          index (index)}{#if segment.suspected}<mark class="sheet-mark">{segment.text}</mark>{:else}{segment.text}{/if}{/each}</p>
    </div>
  {/if}

  <div class="sheet-actions">
    <button type="button" class="button button-primary" disabled={!ready} onclick={send}>
      <Icon name="send" size={14} />
      {action === 'note' ? t('sheet.save') : t('sheet.send')}
    </button>
    <button type="button" class="button button-quiet" onclick={oncancel}>
      {t('sheet.cancel')}
    </button>
    <span class="sheet-gap"></span>
    <span class="kbd sheet-hint">{t('sheet.hint', { modifier: sendModifier() })}</span>
  </div>
</section>
