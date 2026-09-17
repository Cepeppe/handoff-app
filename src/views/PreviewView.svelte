<!--
  The mandatory preview (§7.6 Preview row, §7.10, PREV-01..05, PRIN-09).

  Everything that reaches an agent from a screen goes through here. There is no "send
  without preview" (PREV-01), the two send buttons sit side by side with **no default**
  (PREV-04), and **Send image** is not drawn at all for a session whose capability row says
  the agent cannot read one (FM-05).

  What is drawn over the picture is what will be burned into it: a **locked** box is a
  certain match and is not the user's to lift, a **flagged** one is a suspicion and costs
  one click (DET-01, PREV-02). The boxes are absolutely positioned in percentages of the
  image, so the drawing is the same picture at whatever width the panel is; the burn-in
  works in the capture's own pixels on the Rust side and the two never have to agree on a
  scale.

  Three states this screen has that are not the happy one, and each is a requirement:

  - **analyzing** (OCR-04): the image is on screen at once and the send buttons are disabled
    until the detectors answer. A native engine can take seconds, and the first capture of a
    session pays the bundled engine's model load on top (the T-048 note);
  - **unread** (FM-16, OCR-01): no engine could read the capture, so nothing was found and
    nothing was hidden. The user is told exactly that, and keeps crop and a hand-drawn box;
  - **denied** (FM-17, CAP-04): macOS has not granted screen recording. The explanation and
    the button to the settings pane are here rather than as a system prompt mid-handoff.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import {
    applyDraw,
    detection,
    discardPreview,
    isAnalysing,
    preview,
  } from '../capture.svelte';
  import { t } from '../i18n';
  import type { PixelRect, PreviewBox, PreviewText, Redacted, ScreenshotMode } from '../model';

  /** What a drag has to cover before it is a rectangle and not a click. */
  const MIN_DRAG_PX = 4;

  const shown = $derived(preview());
  const found = $derived(detection());
  const busy = $derived(isAnalysing());

  /** What the user is drawing with the next drag, if anything. */
  let tool = $state<'addBox' | 'crop' | null>(null);
  let dragFrom = $state<{ x: number; y: number } | null>(null);
  let dragTo = $state<{ x: number; y: number } | null>(null);

  /** The editable OCR text (PREV-03), and what it would send (§7.10). */
  let pane = $state('');
  let paneRedacted = $state<PreviewText | null>(null);
  let comment = $state('');
  let commentScanned = $state<Redacted | null>(null);
  let sending = $state(false);
  let failure = $state<string | null>(null);
  let loadedFor = $state<string | null>(null);

  // The pane starts from what the engine read and is the user's from then on: PREV-03 says
  // they send what they send. It is filled once per analysis, never on every repaint.
  $effect(() => {
    const analysis = found;
    if (analysis === null || loadedFor === analysis.handoffId + analysis.text) return;
    loadedFor = analysis.handoffId + analysis.text;
    pane = analysis.text;
    void rescan(analysis.text);
  });

  const ready = $derived(shown.status === 'ready' && found !== null && !busy && !sending);
  const canSendText = $derived(ready && pane.trim().length > 0);

  /** The drag rectangle, in the image's own pixels, or nothing. */
  const dragRect = $derived.by((): PixelRect | null => {
    if (dragFrom === null || dragTo === null || shown.status !== 'ready') return null;
    const x = Math.round(Math.min(dragFrom.x, dragTo.x) * shown.width);
    const y = Math.round(Math.min(dragFrom.y, dragTo.y) * shown.height);
    const width = Math.round(Math.abs(dragTo.x - dragFrom.x) * shown.width);
    const height = Math.round(Math.abs(dragTo.y - dragFrom.y) * shown.height);
    if (width < MIN_DRAG_PX || height < MIN_DRAG_PX) return null;
    return { x, y, width, height };
  });

  /** Where a box sits, as percentages of the drawn image. */
  function place(box: PreviewBox | PixelRect): string {
    if (shown.status !== 'ready') return '';
    const left = (box.x / shown.width) * 100;
    const top = (box.y / shown.height) * 100;
    const width = (box.width / shown.width) * 100;
    const height = (box.height / shown.height) * 100;
    return `left:${String(left)}%;top:${String(top)}%;width:${String(width)}%;height:${String(height)}%`;
  }

  /**
   * The comment is typed text, so it runs through both detectors before it can be sent
   * (§7.10) — the same command the Ask, Defer and Abandon sheets call, over the same
   * exemption list (DET-03).
   *
   * And it is treated the way a **sheet** treats it and not the way the text pane does: a
   * certain match is replaced, a suspected one is marked and sent as it was written, because
   * the sentence was composed in this window a moment ago and DET-01 gives that decision to
   * the person who wrote it.
   */
  async function rescanComment(next: string): Promise<void> {
    comment = next;
    if (shown.status !== 'ready') return;
    try {
      commentScanned = await bridge().scanTypedText(shown.handoffId, next);
    } catch {
      commentScanned = null;
    }
  }

  async function rescan(next: string): Promise<void> {
    pane = next;
    try {
      paneRedacted = await bridge().previewText(next);
    } catch {
      // Nothing to say: the pane is the user's text and the Rust side redacts it again on
      // the way out, so a failed preview of the redaction costs a warning and never a leak.
      paneRedacted = null;
    }
  }

  async function edit(
    what:
      | { kind: 'unlock'; id: number }
      | { kind: 'relock'; id: number }
      | { kind: 'addBox'; rect: PixelRect }
      | { kind: 'crop'; rect: PixelRect }
      | { kind: 'uncrop' },
  ): Promise<void> {
    try {
      applyDraw(await bridge().editPreview(what));
      failure = null;
    } catch (error) {
      failure = error instanceof Error ? error.message : String(error);
    }
  }

  function toggle(box: PreviewBox): void {
    if (box.level !== 'flagged') return;
    void edit(box.unlocked ? { kind: 'relock', id: box.id } : { kind: 'unlock', id: box.id });
  }

  function pointOf(event: PointerEvent): { x: number; y: number } {
    const bounds = (event.currentTarget as HTMLElement).getBoundingClientRect();
    return {
      x: (event.clientX - bounds.left) / bounds.width,
      y: (event.clientY - bounds.top) / bounds.height,
    };
  }

  function dragStarted(event: PointerEvent): void {
    if (tool === null) return;
    dragFrom = pointOf(event);
    dragTo = dragFrom;
  }

  function dragMoved(event: PointerEvent): void {
    if (tool === null || dragFrom === null) return;
    dragTo = pointOf(event);
  }

  function dragEnded(): void {
    const rect = dragRect;
    const drawing = tool;
    dragFrom = null;
    dragTo = null;
    tool = null;
    if (rect === null || drawing === null) return;
    void edit(drawing === 'addBox' ? { kind: 'addBox', rect } : { kind: 'crop', rect });
  }

  /** The comment as the agent will read it: what the sheet showed, or nothing. */
  function sentComment(): string | null {
    const written = commentScanned?.text ?? comment;
    return written.trim() === '' ? null : written;
  }

  async function send(mode: ScreenshotMode): Promise<void> {
    if (shown.status !== 'ready') return;
    sending = true;
    failure = null;
    try {
      await bridge().sendScreenshot(
        shown.handoffId,
        mode,
        mode === 'text' ? pane : null,
        sentComment(),
      );
      // The send is the end of this capture on both sides; `discardPreview` puts the panel
      // back and drops the pixels the webview was holding (PRIN-04).
      comment = '';
      commentScanned = null;
      await discardPreview();
    } catch (error) {
      failure = error instanceof Error ? error.message : String(error);
    } finally {
      sending = false;
    }
  }
</script>

<section class="view" data-view="preview">
  <h1>{t('view.preview')}</h1>

  {#if shown.status === 'ready'}
    <div
      class="preview-canvas"
      data-tool={tool}
      role="presentation"
      onpointerdown={dragStarted}
      onpointermove={dragMoved}
      onpointerup={dragEnded}
      onpointerleave={dragEnded}
    >
      <img class="preview-image" src={shown.url} alt={t('preview.alt')} />
      {#if found !== null}
        {#each found.boxes as box (box.id)}
          {#if box.level === 'flagged'}
            <button
              type="button"
              class="preview-box preview-box-flagged"
              class:preview-box-lifted={box.unlocked}
              style={place(box)}
              title={t('preview.suspected')}
              aria-pressed={box.unlocked}
              aria-label={box.unlocked ? t('preview.relock') : t('preview.unlock')}
              onclick={() => toggle(box)}
            ></button>
          {:else}
            <span class="preview-box preview-box-locked" style={place(box)} aria-hidden="true"
            ></span>
          {/if}
        {/each}
        {#if found.crop !== null}
          <span class="preview-crop" style={place(found.crop)} aria-hidden="true"></span>
        {/if}
      {/if}
      {#if dragRect !== null}
        <span class="preview-drag" style={place(dragRect)} aria-hidden="true"></span>
      {/if}
    </div>

    <p class="preview-size">{t('preview.size', { width: shown.width, height: shown.height })}</p>

    {#if busy || found === null}
      <p class="preview-note" role="status">{t('preview.analyzing')}</p>
    {:else}
      <p class="preview-note" role="status">
        {t('preview.redactions', { count: found.redactions })}
        {#if found.ocrEngine !== null}{t('preview.engine', { engine: found.ocrEngine })}{/if}
      </p>
      {#if found.unread !== null}
        <p class="preview-problem" role="status">{t('preview.unread')}</p>
      {/if}
      {#if found.large}
        <p class="preview-note">{t('preview.largeHint')}</p>
      {/if}
    {/if}

    <div class="preview-tools" role="group" aria-label={t('preview.tools')}>
      <button
        type="button"
        class="button"
        aria-pressed={tool === 'addBox'}
        onclick={() => (tool = tool === 'addBox' ? null : 'addBox')}
      >
        {t('preview.addBox')}
      </button>
      <button
        type="button"
        class="button"
        aria-pressed={tool === 'crop'}
        onclick={() => (tool = tool === 'crop' ? null : 'crop')}
      >
        {t('preview.crop')}
      </button>
      {#if found?.crop != null}
        <button type="button" class="button button-quiet" onclick={() => void edit({ kind: 'uncrop' })}>
          {t('preview.uncrop')}
        </button>
      {/if}
    </div>

    <label class="preview-label" for="preview-text">{t('preview.textPane')}</label>
    <textarea
      id="preview-text"
      class="sheet-text"
      rows="4"
      value={pane}
      oninput={(event) => void rescan(event.currentTarget.value)}
    ></textarea>
    {#if paneRedacted !== null && (paneRedacted.kinds.length > 0 || paneRedacted.suspected > 0)}
      <p class="sheet-redacted">
        {t('preview.textRedacted', {
          kinds: paneRedacted.kinds.join(', '),
          suspected: paneRedacted.suspected,
        })}
      </p>
      <!--
        The pane is the user's text and the masks are not typed into it, so PREV-01's "what
        you see is what is sent" needs the sent form shown beside it — the same shape
        `TextSheet.svelte` uses for the three sheets.
      -->
      <p class="sheet-preview" aria-label={t('sheet.willSend')}>{paneRedacted.text}</p>
    {/if}

    <label class="preview-label" for="preview-comment">{t('preview.comment')}</label>
    <textarea
      id="preview-comment"
      class="sheet-text"
      rows="2"
      value={comment}
      oninput={(event) => void rescanComment(event.currentTarget.value)}
    ></textarea>
    {#if commentScanned !== null && (commentScanned.kinds.length > 0 || commentScanned.reasons.length > 0)}
      {#if commentScanned.kinds.length > 0}
        <p class="sheet-redacted">
          {t('sheet.redacted', { kinds: commentScanned.kinds.join(', ') })}
        </p>
      {/if}
      {#if commentScanned.reasons.length > 0}
        <p class="sheet-suspected">{t('sheet.suspected')}</p>
      {/if}
      <p class="sheet-preview" aria-label={t('sheet.willSend')}>{#each commentScanned.segments as segment,
          index (index)}{#if segment.suspected}<mark class="sheet-mark">{segment.text}</mark>{:else}{segment.text}{/if}{/each}</p>
    {/if}

    {#if failure !== null}
      <p class="preview-problem" role="alert">{failure}</p>
    {/if}

    <div class="preview-actions">
      {#if found === null || found.imagesInResults}
        <button type="button" class="button" disabled={!ready} onclick={() => void send('image')}>
          {t('preview.sendImage')}
        </button>
      {/if}
      <button type="button" class="button" disabled={!canSendText} onclick={() => void send('text')}>
        {t('preview.sendText')}
      </button>
      <button type="button" class="button button-quiet" onclick={() => void discardPreview()}>
        {t('preview.discard')}
      </button>
    </div>
  {:else}
    {#if shown.status === 'denied'}
      <p class="preview-problem" role="status">{t('onboarding.screenRecordingText')}</p>
      <button
        type="button"
        class="button"
        onclick={() => void bridge().openScreenRecordingSettings()}
      >
        {t('onboarding.screenRecordingOpen')}
      </button>
    {:else if shown.status === 'failed'}
      <p class="preview-problem" role="status">{t('preview.failed', { message: shown.message })}</p>
    {:else}
      <p class="preview-empty">{t('preview.empty')}</p>
    {/if}

    <div class="preview-actions">
      <button type="button" class="button button-quiet" onclick={() => void discardPreview()}>
        {t('preview.discard')}
      </button>
    </div>
  {/if}
</section>
