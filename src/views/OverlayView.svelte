<!--
  The overlay: the tab strip, the banner of §8.4, the step view, the action bar and the
  sheets behind Ask, Note, Defer and Abandon (§7.6).

  What it owns is the wiring, and nothing else: which tab is selected and what changed while
  the user was elsewhere live in `state.svelte.ts`, what a tab *is* comes from the core in
  one object (`getHandoffView`), and every button is drawn from the `actions` block that
  object carries — the store refuses an action a state does not offer, and §7.4 calls a
  button that produces that refusal a defect of this file.

  The collapsed bar of WIN-03 and the secondary views are T-037; what is here is the panel as
  it looks while a person is working through a handoff.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { ActionName, HandoffView } from '../model';
  import ActionBar from '../overlay/ActionBar.svelte';
  import StepView from '../overlay/StepView.svelte';
  import TabStrip from '../overlay/TabStrip.svelte';
  import TextSheet from '../overlay/TextSheet.svelte';
  import {
    allTabs,
    currentNotice,
    currentView,
    handoffChanged,
    hideEverything,
    refreshCurrent,
    refreshTabs,
    showNotice,
  } from '../overlay/state.svelte';

  /** The sheet on screen, when one is open (RESP-02: Ask and Note are distinct). */
  let sheet = $state<ActionName | null>(null);

  const view = $derived(currentView());
  const tabs = $derived(allTabs());
  const notice = $derived(currentNotice());

  /** The three actions that send what the user typed, and are therefore scanned (§7.10). */
  const SCANNED: ReadonlySet<ActionName> = new Set<ActionName>(['ask', 'defer', 'abandon']);

  /** The two that take a reason or nothing at all (RESP-05, RESP-08). */
  const OPTIONAL: ReadonlySet<ActionName> = new Set<ActionName>(['defer', 'abandon']);

  function start(action: ActionName): void {
    if (action === 'ask' || action === 'note' || action === 'defer' || action === 'abandon') {
      sheet = action;
      return;
    }
    void run(action);
  }

  async function run(action: ActionName, payload?: string): Promise<void> {
    const current: HandoffView | null = view;
    if (current === null) {
      return;
    }
    sheet = null;
    try {
      await bridge().act(current.tab.id, action, payload);
    } catch {
      // The Rust side already pushed the sentence the user reads (`notice`); what is left
      // here is to draw whatever the core now says, which is what a refused action left
      // unchanged (FM-28).
    }
    await refreshCurrent();
    await refreshTabs();
  }

  onMount(() => {
    const stopping: Array<() => void> = [];
    const keep = (unlisten: () => void): number => stopping.push(unlisten);

    // The three events of §7.6. Two of them carry no state worth the name — the view
    // re-reads the core, which is the one source of truth — and the third *is* its payload.
    void bridge()
      .onHandoffChanged((id) => void handoffChanged(id))
      .then(keep);
    void bridge()
      .onSessionsChanged(() => void refreshTabs())
      .then(keep);
    void bridge().onNotice(showNotice).then(keep);

    void refreshTabs();

    return () => {
      hideEverything();
      for (const stop of stopping) {
        stop();
      }
    };
  });
</script>

<section class="view" data-view="overlay">
  <TabStrip {tabs} />

  {#if notice !== null}
    <p class="notice" data-notice={notice.kind} role="status">{notice.text}</p>
  {/if}

  {#if view === null}
    <p class="empty">{t('overlay.empty')}</p>
  {:else}
    {#if view.banner !== null}
      <p class="banner" data-ui-state={view.uiState}>
        {t(view.banner.key, view.banner.arg === null ? undefined : { text: view.banner.arg })}
        {#if view.uiState === 'final' && view.verifyResult !== null}
          <span class="banner-detail">({t('overlay.declaredByAgent')})</span>
        {/if}
      </p>
    {/if}

    {#if view.tab.orphan}
      <p class="banner banner-orphan">{t('overlay.orphan')}</p>
    {/if}

    {#if view.goal !== null}
      <h1 class="goal">{view.goal}</h1>
    {/if}
    {#if view.location !== null}
      <p class="location">{view.location}</p>
    {/if}
    {#if view.resumedFrom !== null}
      <p class="resumed-from">
        {t('overlay.resumedFrom', {
          label: `${view.resumedFrom.agent} · ${view.resumedFrom.project}`,
        })}
      </p>
    {/if}
    {#if view.requestText !== null}
      <p class="request-text">
        {view.linkedRequest === null ? t('overlay.request') : t('overlay.linkedRequest')}:
        {view.requestText}
      </p>
    {/if}
    {#if view.pending !== null}
      <p class="pending">
        {view.pending.kind === 'question'
          ? t('overlay.pendingQuestion', { step: view.pending.step })
          : t('overlay.pendingScreenshot', { step: view.pending.step })}
      </p>
    {/if}

    <StepView {view} />

    {#if view.verifyResult !== null && view.verifyResult.detail !== null}
      <p class="verify-detail">{view.verifyResult.detail}</p>
    {/if}

    {#if sheet === null}
      <ActionBar actions={view.actions} lastStep={view.step?.last ?? false} onact={start} />
    {:else}
      <TextSheet
        action={sheet}
        optional={OPTIONAL.has(sheet)}
        scanned={SCANNED.has(sheet)}
        onsend={(text) => void run(sheet ?? 'note', text)}
        oncancel={() => (sheet = null)}
      />
    {/if}

    {#if view.history.length > 0}
      <details class="history">
        <summary>{t('overlay.history')}</summary>
        {#each view.history as round (round.no)}
          <section class="history-round">
            <h2>{t('overlay.round', { no: round.no })}</h2>
            <ol>
              {#each round.steps as text, index (index)}
                <li class:done={round.confirmed.includes(index + 1)}>
                  {text}
                  {#if round.skipped.includes(index + 1)}
                    <span class="tag">{t('overlay.skipped')}</span>
                  {/if}
                </li>
              {/each}
            </ol>
            {#each round.notes as note, index (index)}
              <p class="history-note">{note.text}</p>
            {/each}
            {#each round.replies as reply, index (index)}
              <p class="history-reply">{reply.text}</p>
            {/each}
            {#if round.verify !== null && round.verify.detail !== null}
              <p class="history-verify">
                {round.verify.detail} <span class="tag">{t('overlay.declaredByAgent')}</span>
              </p>
            {/if}
          </section>
        {/each}
      </details>
    {/if}
  {/if}
</section>
