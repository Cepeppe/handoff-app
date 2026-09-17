<!--
  The tab strip: one tab per handoff (MULTI-01), with the orphan and parked ones in a
  collapsible "waiting" group (§7.6, SRV-23, RESP-07).

  Switching tabs returns no tool call and never touches the agent (MULTI-01): it is a
  selection here and nothing else. The badge is the window's own count of what changed while
  the user was looking elsewhere (MULTI-03); `state.svelte.ts` says why it cannot be the
  core's.

  The waiting group carries its own buttons, because SRV-23 gives the user three things to
  do **from the list**: view it (select the tab), close it by hand, or copy its id to resume
  it in a new session. RESP-07 adds the fourth for a parked handoff — resume it from the
  overlay. Which of them a given entry offers comes from the core, in the same `actions`
  block the whole tab carries: the store refuses an action a state does not have, and §7.4
  calls a button that produces that refusal a defect of the view. They are always drawn, never
  on hover: a keyboard and a touch screen have no hover to reveal them with.

  **The group is a `<details>` and never a popover.** The window's height follows its content
  (WIN-02), so a list that floated over the panel would open into a window that is not tall
  enough to hold it and be clipped. Opening it therefore takes a line of its own and pushes
  the step down, which is a window that grew — exactly what the resize observer is for.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { ActionName, TabView } from '../model';
  import Icon from './Icon.svelte';
  import WaitingEntry from './WaitingEntry.svelte';
  import { badge, select, selectedId } from './state.svelte';

  const {
    tabs,
    onact,
  }: {
    tabs: TabView[];
    /** Runs a user action on a named handoff, which for the waiting group is not the selected one. */
    onact: (id: string, action: ActionName) => void;
  } = $props();

  const open = $derived(tabs.filter((tab) => tab.group === 'open'));
  const waiting = $derived(tabs.filter((tab) => tab.group === 'waiting'));
</script>

<nav class="tab-strip" aria-label={t('overlay.tabs')}>
  {#each open as entry (entry.id)}
    <button
      type="button"
      class="tab"
      class:tab-orphan={entry.orphan}
      aria-current={selectedId() === entry.id ? 'true' : undefined}
      title={entry.goal ?? entry.label}
      onclick={() => void select(entry.id)}
    >
      {#if selectedId() === entry.id}
        <span class="tab-dot"></span>
      {/if}
      <span class="tab-label">{entry.label}</span>
      {#if badge(entry.id) > 0}
        <span class="tab-badge" aria-label={t('overlay.unseen', { count: badge(entry.id) })}>
          {badge(entry.id)}
        </span>
      {/if}
    </button>
  {/each}

  {#if waiting.length > 0}
    <details class="waiting-group">
      <summary class="tab tab-waiting">
        <span class="tab-label">{t('overlay.waiting')} · {waiting.length}</span>
        <Icon name="chevron" size={12} />
      </summary>
      <div class="waiting-list">
        {#each waiting as entry (entry.id)}
          <WaitingEntry {entry} {onact} oncopyid={(id) => void bridge().copyHandoffId(id)} />
        {/each}
      </div>
    </details>
  {/if}
</nav>
