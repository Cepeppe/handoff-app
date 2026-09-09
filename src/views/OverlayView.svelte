<!--
  The overlay: the tab strip, the banner of §8.4, whichever of the views of §7.6 the tab's
  state calls for, the action bar and the sheets behind Ask, Note, Defer and Abandon.

  What it owns is the wiring, and nothing else: which tab is selected and what changed while
  the user was elsewhere live in `state.svelte.ts`, what a tab *is* comes from the core in
  one object (`getHandoffView`), and every button is drawn from the `actions` block that
  object carries — the store refuses an action a state does not offer, and §7.4 calls a
  button that produces that refusal a defect of this file.

  **Which view is showing is decided by `uiState` and by nothing else.** The core resolved
  the rows of §8.4 in one place, precedence included (`ui_bridge/view.rs`), so a second
  reading of "is it detached or is it deferred" here would be a second answer to a question
  that already has one. Waiting-for-spec, Question-pending and Verifying are the three rows
  that replace the step view; everything else guides, and a final tab shows its outcome.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { ActionName, HandoffView, RequestChoice } from '../model';
  import ActionBar from '../overlay/ActionBar.svelte';
  import CrashNotice from '../overlay/CrashNotice.svelte';
  import History from '../overlay/History.svelte';
  import QuestionPending from '../overlay/QuestionPending.svelte';
  import SessionPicker from '../overlay/SessionPicker.svelte';
  import StepView from '../overlay/StepView.svelte';
  import TabStrip from '../overlay/TabStrip.svelte';
  import TextSheet from '../overlay/TextSheet.svelte';
  import Verifying from '../overlay/Verifying.svelte';
  import WaitingForSpec from '../overlay/WaitingForSpec.svelte';
  import {
    allTabs,
    answerSessionPicker,
    currentNotice,
    currentView,
    handoffChanged,
    hideEverything,
    refreshCurrent,
    refreshSessions,
    refreshTabs,
    sessionChoices,
    showNotice,
  } from '../overlay/state.svelte';

  /** The sheet on screen, when one is open (RESP-02: Ask and Note are distinct). */
  let sheet = $state<ActionName | null>(null);

  /**
   * The requests the **Change** control of FM-20 is offering, while it is open.
   *
   * `null` means the control has not been pressed. It is read on demand rather than carried
   * in the view: a mis-link is rare (§12.4 calls it an accepted case) and the queue is the
   * core's, so asking when the user asks keeps one source of truth.
   */
  let relinking = $state<RequestChoice[] | null>(null);

  const view = $derived(currentView());
  const tabs = $derived(allTabs());
  const notice = $derived(currentNotice());
  const choices = $derived(sessionChoices());

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
    await runOn(current.tab.id, action, payload);
  }

  /** Opens the FM-20 list, or closes it when it is already open. */
  async function offerRelink(id: string): Promise<void> {
    if (relinking !== null) {
      relinking = null;
      return;
    }
    relinking = await bridge().openRequests(id);
  }

  /**
   * One action on a named handoff.
   *
   * Named rather than "the selected one", because the waiting group of §7.6 acts on entries
   * the user is not looking at: closing an orphan from the list must not require selecting
   * it first (SRV-23).
   */
  async function runOn(id: string, action: ActionName, payload?: string): Promise<void> {
    sheet = null;
    relinking = null;
    try {
      await bridge().act(id, action, payload);
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
      .onSessionsChanged(() => {
        void refreshTabs();
        // FM-22: the registry announces the picker through this same event, with no
        // payload, so the question is re-read rather than delivered.
        void refreshSessions();
      })
      .then(keep);
    void bridge().onNotice(showNotice).then(keep);

    void refreshTabs();
    void refreshSessions();

    return () => {
      hideEverything();
      for (const stop of stopping) {
        stop();
      }
    };
  });
</script>

<section class="view" data-view="overlay">
  <TabStrip {tabs} onact={(id, action) => void runOn(id, action)} />

  <SessionPicker {choices} onanswer={(sessionRef) => void answerSessionPicker(sessionRef)} />

  <!--
    §7.14: the launch after a crash. Above the tab it is about nothing in particular — it is
    a fact about the application and not about a handoff — and below the strip, so it never
    takes the place of the work the user came for.
  -->
  <CrashNotice onfailed={(text) => showNotice({ kind: 'error', text })} />

  {#if notice !== null}
    <p class="notice" data-notice={notice.kind} role="status">{notice.text}</p>
  {/if}

  {#if view === null}
    <p class="empty">{t('overlay.empty')}</p>
  {:else}
    <!--
      The banner of §8.4, except for the verifying row: there `Verifying.svelte` says the
      same sentence with the spec's own text quoted under it, and one screen should not
      carry "the agent should now check" twice. The "declared by agent" label of the final
      row travels with the report, for the same reason.
    -->
    {#if view.banner !== null && view.uiState !== 'verifying'}
      <p class="banner" data-ui-state={view.uiState}>
        {t(view.banner.key, view.banner.arg === null ? undefined : { text: view.banner.arg })}
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
    <!--
      The request this tab answers, and the one-click correction of FM-20. A spec that quoted
      no `request_id` was linked to the session's oldest open request (OPEN-08), which two
      open requests can get the wrong way round (§12.4); **Change** lists the others and
      relinking is a single action the store applies for both sides at once.
    -->
    {#if view.requestText !== null && view.uiState !== 'waitingForSpec'}
      <p class="request-text">
        {view.linkedRequest === null ? t('overlay.request') : t('overlay.linkedRequest')}:
        {view.requestText}
        <button
          type="button"
          class="button button-quiet"
          onclick={() => void offerRelink(view.tab.id)}
        >
          {t('overlay.changeRequest')}
        </button>
      </p>
    {/if}

    {#if relinking !== null}
      <section class="relink" role="group" aria-label={t('overlay.chooseRequest')}>
        <p class="relink-title">{t('overlay.chooseRequest')}</p>
        {#if relinking.length === 0}
          <p class="relink-empty">{t('overlay.noOtherRequest')}</p>
        {:else}
          {#each relinking as choice (choice.id)}
            <button
              type="button"
              class="button"
              aria-current={choice.id === view.linkedRequest?.id ? 'true' : undefined}
              onclick={() => void runOn(view.tab.id, 'relink', choice.id)}
            >
              {choice.text}
            </button>
          {/each}
        {/if}
        <button type="button" class="button button-quiet" onclick={() => (relinking = null)}>
          {t('sheet.cancel')}
        </button>
      </section>
    {/if}

    {#if view.uiState === 'waitingForSpec'}
      <!-- Abandon opens the same sheet here as anywhere else (RESP-08). -->
      {#if sheet === null}
        <WaitingForSpec {view} onact={start} />
      {:else}
        <TextSheet
          action={sheet}
          optional={OPTIONAL.has(sheet)}
          scanned={SCANNED.has(sheet)}
          onsend={(text) => void run(sheet ?? 'abandon', text)}
          oncancel={() => (sheet = null)}
        />
      {/if}
    {:else}
      {#if view.pending !== null}
        <QuestionPending pending={view.pending} />
      {/if}

      {#if view.uiState === 'verifying' || view.verifyResult !== null}
        <Verifying verify={view.verify} result={view.verifyResult} />
      {/if}

      <StepView {view} />

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
    {/if}

    <History rounds={view.history} />
  {/if}
</section>
