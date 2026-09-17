<!--
  The handoff list of the expanded view (§7.6): the tab strip, given a column of its own.

  At 720 pixels the chips of the narrow panel would be a row of four truncated labels, and
  the room the extra width bought would go to nothing. Here each handoff gets two lines — the
  goal, which is what a person remembers it by, and the agent and project underneath — so the
  list answers "which one is this" without a tooltip.

  It lists the same two groups as the strip and offers the same buttons (MULTI-01, SRV-23,
  RESP-07); nothing is hover-only. Underneath, the two doors out of the overlay that the tray
  menu also has: **New request**, with the combination in force beside it (OPEN-03), and
  **Settings**. Both switch the view, which narrows or widens the window through the one rule
  in `App.svelte` — and coming back to the overlay comes back to this shape, because the form
  is remembered for the session.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import { keycaps } from '../keys';
  import type { ActionName, TabView } from '../model';
  import { showView } from '../view-state.svelte';
  import Icon from './Icon.svelte';
  import WaitingEntry from './WaitingEntry.svelte';
  import { badge, select, selectedId } from './state.svelte';

  const {
    tabs,
    onact,
  }: {
    tabs: TabView[];
    onact: (id: string, action: ActionName) => void;
  } = $props();

  const open = $derived(tabs.filter((tab) => tab.group === 'open'));
  const waiting = $derived(tabs.filter((tab) => tab.group === 'waiting'));

  /**
   * The combination the system accepted, as the keys a person presses (OPEN-03, FM-18).
   *
   * One keycap holding the whole combination rather than one per key: the column is 220
   * pixels wide and the label beside it has to stay readable, which three caps and their
   * gaps do not leave room for. The settings page, which has the width, prints them apart.
   */
  let shortcut = $state('');

  onMount(() => {
    void bridge()
      .shortcutStatus()
      .then((status) => {
        // A refused combination is not printed: the keycap would name a shortcut that does
        // nothing, and `New request` here works either way (FM-18).
        shortcut = status.registered ? keycaps(status.accelerator).join(' ') : '';
      })
      .catch(() => {
        shortcut = '';
      });
  });
</script>

<nav class="rail" aria-label={t('overlay.tabs')}>
  {#if open.length > 0}
    <span class="section-label rail-label">{t('overlay.inProgressGroup')}</span>
    {#each open as entry (entry.id)}
      <button
        type="button"
        class="rail-item"
        aria-current={selectedId() === entry.id ? 'true' : undefined}
        title={entry.goal ?? entry.label}
        onclick={() => void select(entry.id)}
      >
        <span class="rail-line">
          <span class="rail-dot" class:rail-dot-on={selectedId() === entry.id}></span>
          <span class="rail-goal" class:rail-orphan={entry.orphan}>{entry.goal ?? entry.label}</span>
          {#if badge(entry.id) > 0}
            <span class="tab-badge" aria-label={t('overlay.unseen', { count: badge(entry.id) })}>
              {badge(entry.id)}
            </span>
          {/if}
        </span>
        <span class="rail-meta">{entry.label}</span>
      </button>
    {/each}
  {/if}

  {#if waiting.length > 0}
    <span class="section-label rail-label">{t('overlay.waiting')}</span>
    {#each waiting as entry (entry.id)}
      <WaitingEntry {entry} {onact} oncopyid={(id) => void bridge().copyHandoffId(id)} />
    {/each}
  {/if}

  <span class="rail-gap"></span>

  <button type="button" class="button rail-new" onclick={() => showView('request')}>
    <span class="rail-new-label">
      <Icon name="plus" size={15} />
      {t('tray.newRequest')}
    </span>
    {#if shortcut.length > 0}
      <span class="kbd">{shortcut}</span>
    {/if}
  </button>
  <button type="button" class="button button-quiet rail-settings" onclick={() => showView('settings')}>
    <Icon name="settings" size={15} />
    {t('view.settings')}
  </button>
</nav>
