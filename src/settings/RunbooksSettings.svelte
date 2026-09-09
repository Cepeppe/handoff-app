<!--
  Settings → Runbooks (§7.6, §7.12, RUN-01, RUN-03, RUN-09).

  The folder `~/.handoff/runbooks/`, listed as it is on disk. There is no editor here: RUN-03
  says the files are the user's, so what this page offers is to look at them, to open the
  folder in the system's own file manager, and to delete one.

  **Delete moves the file to the operating system's trash**, not to nowhere: a runbook is a
  recipe a whole handoff produced, and the trash is where a person already knows to look for
  something they removed by mistake. The confirmation is a second button on the same row, as
  on the Log page and for the same reason — the panel is one window (MULTI-04).

  Two things a row may show that are not defects (`DEVIATIONS.md`, T-044): a **last run
  failed** date beside a newer **last verified** one, because a run that failed and was then
  corrected keeps both and RUN-09 never unmarks the first; and, inside the file, a step text
  carrying `[treated as secret: <kind>]` where the ingress detector matched outside a declared
  value.

  The update proposals of RUN-09 are listed at the top, because §7.6 puts them on this page:
  the same question is also drawn on the handoff that produced it (`OverlayView`), where the
  user can see the correction it came from, and both call the same command. Accepting rewrites
  the file through the writer; declining keeps both, which is what §7.12 means by "the user
  decides" — the new sequence was already written as a runbook of its own.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { moment } from '../datetime';
  import { t } from '../i18n';
  import type { PendingRunbookProposalView, RunbookEntryView } from '../model';

  let runbooks = $state<RunbookEntryView[]>([]);
  let proposals = $state<PendingRunbookProposalView[]>([]);
  /** The file name whose **Delete** is waiting for a confirmation. */
  let confirming = $state<string | null>(null);
  let problem = $state<string | null>(null);
  let said = $state<string | null>(null);
  let busy = $state(false);

  /** The two trust labels of RUN-01, as catalogue keys. */
  const TRUST_KEYS = {
    verified: 'runbooks.trustVerified',
    confirmed_by_user: 'runbooks.trustConfirmedByUser',
  } as const;

  onMount(() => {
    void load();
  });

  /**
   * The list. It does not clear `problem`: **Delete** re-reads when it is over, including
   * when it failed, and a reload that wiped the message would hide the refusal.
   */
  async function load(): Promise<void> {
    try {
      runbooks = await bridge().runbooks();
    } catch (error) {
      runbooks = [];
      problem = t('install.failed', { reason: String(error) });
    }
    try {
      proposals = await bridge().runbookProposals();
    } catch (error) {
      proposals = [];
      problem = t('install.failed', { reason: String(error) });
    }
  }

  /**
   * The user answered a rewrite (RUN-09).
   *
   * The store clears the question only when an accepted rewrite reached the disk, so a
   * failure leaves the row where it is with the writer's own message beside it.
   */
  async function answer(handoffId: string, accept: boolean): Promise<void> {
    busy = true;
    problem = null;
    said = null;
    try {
      await bridge().resolveRunbookProposal(handoffId, accept);
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    }
    busy = false;
    await load();
  }

  async function openFolder(): Promise<void> {
    problem = null;
    try {
      await bridge().openRunbooksFolder();
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    }
  }

  async function remove(fileName: string): Promise<void> {
    busy = true;
    problem = null;
    said = null;
    try {
      await bridge().deleteRunbook(fileName);
      said = t('runbooks.deleted', { name: fileName });
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    }
    confirming = null;
    busy = false;
    await load();
  }
</script>

<section class="settings-section" data-settings="runbooks">
  <h2>{t('settings.runbooks')}</h2>

  {#each proposals as proposal (proposal.handoffId)}
    <section class="runbook-proposal" data-runbook-proposal={proposal.runbookId}>
      <p>{t('overlay.runbookProposal', { name: proposal.fileName })}</p>
      <div class="runbook-proposal-actions">
        <button
          type="button"
          class="button button-primary"
          disabled={busy}
          onclick={() => void answer(proposal.handoffId, true)}
        >
          {t('overlay.runbookAccept')}
        </button>
        <button
          type="button"
          class="button button-quiet"
          disabled={busy}
          onclick={() => void answer(proposal.handoffId, false)}
        >
          {t('overlay.runbookDecline')}
        </button>
      </div>
    </section>
  {/each}

  {#if runbooks.length === 0}
    <p class="settings-explain">{t('runbooks.empty')}</p>
  {:else}
    <ul class="log-list">
      {#each runbooks as runbook (runbook.fileName)}
        <li class="log-entry" data-runbook={runbook.fileName}>
          <p class="log-goal">{runbook.goal}</p>
          <p class="log-meta">{runbook.location}</p>
          <p class="log-meta">
            {t(TRUST_KEYS[runbook.trust])} · {t('runbooks.runs', { count: runbook.runs })} · {t(
              'runbooks.steps',
              { count: runbook.steps },
            )}
          </p>
          <p class="log-meta">{t('runbooks.lastVerified', { at: moment(runbook.lastVerifiedAt) })}</p>
          {#if runbook.lastRunFailedAt !== null}
            <p class="log-meta">
              <span class="tag tag-failed"
                >{t('runbooks.lastRunFailed', { at: moment(runbook.lastRunFailedAt) })}</span
              >
            </p>
          {/if}
          <p class="log-meta log-file">{runbook.fileName}</p>
          <div class="log-actions">
            {#if confirming === runbook.fileName}
              <button
                type="button"
                class="button button-primary"
                disabled={busy}
                onclick={() => void remove(runbook.fileName)}
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
                onclick={() => (confirming = runbook.fileName)}
              >
                {t('runbooks.delete')}
              </button>
            {/if}
          </div>
          {#if confirming === runbook.fileName}
            <p class="settings-explain">
              {t('runbooks.deleteConfirm', { name: runbook.fileName })}
            </p>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  <div class="log-actions log-whole">
    <button type="button" class="button button-quiet" onclick={() => void openFolder()}>
      {t('runbooks.openFolder')}
    </button>
  </div>

  {#if said !== null}
    <p class="settings-said">{said}</p>
  {/if}
  {#if problem !== null}
    <p class="settings-problem" role="alert">{problem}</p>
  {/if}
</section>
