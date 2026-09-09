<!--
  Settings → Log (§7.6, §7.11, LOG-01..05).

  The record of what has finished, and the three things LOG-04 says a person may do with it,
  because *it is their data*: read one entry whole, delete (one, or everything), export.

  Two habits shape the page:

  - **It shows what the log holds, not what the store remembers.** The spec here is the
    stored one, with `[treated as secret: <kind>]` wherever the certain detector matched at
    ingress (LOG-02, DET-04): there is no **Show** on this page and there never will be, and
    the line under the spec says so rather than leaving a reader to wonder what a mask is.
  - **Every destructive control asks first, in place.** The confirmation is a second button
    on the same row rather than a modal: the panel is one window (MULTI-04) and a dialog over
    it would be a second one. Pressing anything else takes the question away.

  There is no retention control (LOG-05) and the page says why: nothing is ever deleted
  automatically. That sentence is the feature.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { moment } from '../datetime';
  import { t } from '../i18n';
  import type { LogDetailView, LogEntryView } from '../model';

  let entries = $state<LogEntryView[]>([]);
  let detail = $state<LogDetailView | null>(null);
  /** The id whose **Delete** is waiting for a confirmation, or `'all'`, or nothing. */
  let confirming = $state<string | null>(null);
  let problem = $state<string | null>(null);
  let said = $state<string | null>(null);
  let busy = $state(false);

  onMount(() => {
    void load();
  });

  /**
   * The list, read from the core rather than remembered anywhere.
   *
   * It deliberately does **not** clear `problem`: every action re-reads the list when it is
   * over, including the ones that failed, and a reload that wiped the message would answer
   * a refused deletion with a page that looks as if nothing had been asked.
   */
  async function load(): Promise<void> {
    try {
      entries = await bridge().logEntries();
    } catch (error) {
      entries = [];
      problem = t('install.failed', { reason: String(error) });
    }
  }

  /** Opens one entry whole; a second press on the same row closes it again. */
  async function open(id: string): Promise<void> {
    confirming = null;
    if (detail?.entry.id === id) {
      detail = null;
      return;
    }
    try {
      detail = await bridge().logDetail(id);
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    }
  }

  async function removeOne(id: string): Promise<void> {
    busy = true;
    problem = null;
    said = null;
    try {
      await bridge().deleteLogEntry(id);
      if (detail?.entry.id === id) {
        detail = null;
      }
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    }
    confirming = null;
    busy = false;
    await load();
  }

  async function removeEverything(): Promise<void> {
    busy = true;
    problem = null;
    said = null;
    try {
      await bridge().deleteLog();
      detail = null;
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    }
    confirming = null;
    busy = false;
    await load();
  }

  /** LOG-04's export. A cancelled dialog says nothing: it is not a failure. */
  async function exportEverything(): Promise<void> {
    busy = true;
    problem = null;
    said = null;
    try {
      const path = await bridge().exportLog();
      if (path !== null) {
        said = t('log.exported', { path });
      }
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    }
    busy = false;
  }

  /** Agent · project, as the tab strip labels the same handoff (OPEN-02). */
  function label(entry: LogEntryView): string {
    return [entry.agent, entry.project].filter((part) => part !== null).join(' · ');
  }
</script>

<section class="settings-section" data-settings="log">
  <h2>{t('settings.log')}</h2>
  <p class="settings-explain">{t('log.retention')}</p>

  {#if entries.length === 0}
    <p class="settings-explain">{t('log.empty')}</p>
  {:else}
    <ul class="log-list">
      {#each entries as entry (entry.id)}
        <li class="log-entry" data-log-entry={entry.id}>
          <p class="log-goal">{entry.goal ?? entry.id}</p>
          <p class="log-meta">
            {label(entry)}
            {#if entry.closedAt !== null}
              · {t('log.closed', { at: moment(entry.closedAt) })}
            {/if}
            · {t(entry.stateKey)} · {t('log.rounds', { count: entry.rounds })}
          </p>
          {#if !entry.delivered}
            <p class="log-meta">{t('log.notCollected')}</p>
          {/if}
          <div class="log-actions">
            <button
              type="button"
              class="button button-quiet"
              aria-expanded={detail?.entry.id === entry.id}
              onclick={() => void open(entry.id)}
            >
              {detail?.entry.id === entry.id ? t('log.back') : t('log.open')}
            </button>
            {#if confirming === entry.id}
              <button
                type="button"
                class="button button-primary"
                disabled={busy}
                onclick={() => void removeOne(entry.id)}
              >
                {t('log.confirm')}
              </button>
              <button type="button" class="button button-quiet" onclick={() => (confirming = null)}>
                {t('sheet.cancel')}
              </button>
            {:else}
              <button
                type="button"
                class="button button-quiet"
                onclick={() => (confirming = entry.id)}
              >
                {t('log.delete')}
              </button>
            {/if}
          </div>
          {#if confirming === entry.id}
            <p class="settings-explain">{t('log.deleteConfirm')}</p>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  {#if detail !== null}
    <section class="log-detail" data-log-detail={detail.entry.id}>
      <p class="log-meta">
        {t('log.opened', { at: moment(detail.entry.createdAt) })}
        {#if detail.entry.closedAt !== null}
          · {t('log.closed', { at: moment(detail.entry.closedAt) })}
        {/if}
      </p>

      {#if detail.requestText !== null}
        <h3>{t('log.request')}</h3>
        <p class="log-text">{detail.requestText}</p>
      {/if}

      {#if detail.spec !== null}
        <h3>{t('log.spec')}</h3>
        <p class="settings-explain">{t('log.masked')}</p>
        <p class="log-text">{t('log.where')}: {detail.spec.location}</p>
        <p class="log-text">{t('log.whyHuman')}: {detail.spec.whyHuman}</p>
        {#if detail.spec.values.length > 0}
          <h4>{t('log.values')}</h4>
          <ul class="log-plain">
            {#each detail.spec.values as value (value.name)}
              <li><span class="log-key">{value.name}</span>: {value.items.join(', ')}</li>
            {/each}
          </ul>
        {/if}
        {#if detail.spec.secrets.length > 0}
          <h4>{t('log.secrets')}</h4>
          <ul class="log-plain">
            {#each detail.spec.secrets as secret (secret.name)}
              <li><span class="log-key">{secret.name}</span>: {secret.file}</li>
            {/each}
          </ul>
        {/if}
        <h4>{t('log.steps')}</h4>
        <ol class="log-plain">
          {#each detail.spec.steps as step (step.index)}
            <li>
              {step.text}
              {#if step.warning !== null}
                <span class="tag tag-failed">{step.warning}</span>
              {/if}
            </li>
          {/each}
        </ol>
        {#if detail.spec.verify !== null}
          <p class="log-text">{t('log.verify')}: {detail.spec.verify}</p>
        {/if}
      {/if}

      {#if detail.outcomeInstruction !== null}
        <h3>{t('log.outcome')}</h3>
        <p class="log-text">{detail.outcomeInstruction}</p>
        {#if detail.outcomeUserText !== null}
          <p class="log-text">{detail.outcomeUserText}</p>
        {/if}
      {/if}

      {#if detail.rounds.length > 0}
        <h3>{t('log.roundsTitle')}</h3>
        {#each detail.rounds as round (round.no)}
          <section class="log-round" data-log-round={round.no}>
            <h4>{t('log.round', { no: round.no })}</h4>
            <p class="log-meta">{t('log.opened', { at: moment(round.startedAt) })}</p>
            <ol class="log-plain">
              {#each round.steps as text, index (index)}
                <li>{text}</li>
              {/each}
            </ol>
            {#if round.verifyDetail !== null || round.verifyOk !== null}
              <p class="log-text">
                {round.verifyDetail ?? t('log.noReport')}
                <span class="tag">{t('overlay.declaredByAgent')}</span>
                {#if round.verifyLate}
                  <span class="tag tag-late">{t('overlay.late')}</span>
                {/if}
              </p>
            {:else}
              <p class="log-text">{t('log.noReport')}</p>
            {/if}
          </section>
        {/each}
      {/if}

      {#if detail.sends.length > 0}
        <h3>{t('log.sends')}</h3>
        <ul class="log-plain">
          {#each detail.sends as send, index (index)}
            <li>
              {#if send.text !== null}
                {send.text}
              {/if}
              {#if send.imageSha256 !== null}
                <span class="log-meta">
                  {t('log.image', {
                    width: send.imageW ?? 0,
                    height: send.imageH ?? 0,
                    boxes: send.redactionBoxes,
                  })}
                  · {t('log.imageHash', { hash: send.imageSha256 })}
                </span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}

  <div class="log-actions log-whole">
    <button
      type="button"
      class="button button-quiet"
      disabled={busy}
      onclick={() => void exportEverything()}
    >
      {t('log.export')}
    </button>
    {#if confirming === 'all'}
      <button
        type="button"
        class="button button-primary"
        disabled={busy}
        onclick={() => void removeEverything()}
      >
        {t('log.confirm')}
      </button>
      <button type="button" class="button button-quiet" onclick={() => (confirming = null)}>
        {t('sheet.cancel')}
      </button>
    {:else}
      <button type="button" class="button button-quiet" onclick={() => (confirming = 'all')}>
        {t('log.deleteAll')}
      </button>
    {/if}
  </div>

  {#if confirming === 'all'}
    <p class="settings-explain">{t('log.deleteAllConfirm')}</p>
  {/if}
  {#if said !== null}
    <p class="settings-said">{said}</p>
  {/if}
  {#if problem !== null}
    <p class="settings-problem" role="alert">{problem}</p>
  {/if}
</section>
