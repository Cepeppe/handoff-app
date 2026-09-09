<!--
  The tab strip: one tab per handoff (MULTI-01), with the orphan and parked ones in a
  collapsible "waiting" group (§7.6, SRV-23).

  Switching tabs returns no tool call and never touches the agent (MULTI-01): it is a
  selection here and nothing else. The badge is the window's own count of what changed while
  the user was looking elsewhere (MULTI-03); `state.svelte.ts` says why it cannot be the
  core's.
-->
<script lang="ts">
  import { t } from '../i18n';
  import type { TabView } from '../model';
  import { badge, select, selectedId } from './state.svelte';

  const { tabs }: { tabs: TabView[] } = $props();

  const open = $derived(tabs.filter((tab) => tab.group === 'open'));
  const waiting = $derived(tabs.filter((tab) => tab.group === 'waiting'));
</script>

{#snippet tab(entry: TabView)}
  <button
    type="button"
    class="tab"
    class:tab-orphan={entry.orphan}
    aria-current={selectedId() === entry.id ? 'true' : undefined}
    title={entry.goal ?? entry.label}
    onclick={() => void select(entry.id)}
  >
    <span class="tab-label">{entry.label}</span>
    {#if badge(entry.id) > 0}
      <span class="tab-badge" aria-label={t('overlay.unseen', { count: badge(entry.id) })}>
        {badge(entry.id)}
      </span>
    {/if}
  </button>
{/snippet}

<nav class="tab-strip" aria-label={t('overlay.tabs')}>
  {#each open as entry (entry.id)}
    {@render tab(entry)}
  {/each}
</nav>

{#if waiting.length > 0}
  <details class="waiting-group">
    <summary>{t('overlay.waiting')} ({waiting.length})</summary>
    <nav class="tab-strip" aria-label={t('overlay.waiting')}>
      {#each waiting as entry (entry.id)}
        {@render tab(entry)}
      {/each}
    </nav>
  </details>
{/if}
