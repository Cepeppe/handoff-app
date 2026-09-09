<!--
  Settings (§7.6): General, Agents, Network, Log, Runbooks, Updates.

  A mode of the one window and not a second one (DD-10, MULTI-04), and inside it a list of
  sections rather than a single page: §7.6 names six, they arrive with the tasks that own
  them, and a container that already lists them keeps each of those to one entry here.

  General (T-041) and Agents (T-040) are the two that exist. Log and Runbooks are both T-045
  (T-044 writes the runbook files; this page lists them), Network is T-051 and Updates T-078;
  none of them is drawn as an empty placeholder, because a section that says nothing is a
  section the user has to learn to skip.

  **The panel is wider here and narrower everywhere else** (§7.6: "settings that need more
  room open the window in a wider layout temporarily"). It is asked for on mount and given
  back on destroy, so leaving the page by any route — the tab strip, the tray, a handoff
  arriving — restores the fixed width of WIN-02 without anyone having to remember to.
-->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import AgentsSettings from '../settings/AgentsSettings.svelte';
  import GeneralSettings from '../settings/GeneralSettings.svelte';

  const SECTIONS = [
    { name: 'general', titleKey: 'settings.general', view: GeneralSettings },
    { name: 'agents', titleKey: 'install.agents', view: AgentsSettings },
  ] as const;

  let current = $state<(typeof SECTIONS)[number]['name']>('general');
  const Current = $derived(
    SECTIONS.find((section) => section.name === current)?.view ?? GeneralSettings,
  );

  onMount(() => {
    void bridge().setWideLayout(true);
  });

  onDestroy(() => {
    void bridge().setWideLayout(false);
  });
</script>

<section class="view" data-view="settings">
  <h1>{t('view.settings')}</h1>

  {#if SECTIONS.length > 1}
    <nav class="settings-nav" aria-label={t('view.settings')}>
      {#each SECTIONS as section (section.name)}
        <button
          type="button"
          class="settings-nav-item"
          aria-current={current === section.name ? 'page' : undefined}
          onclick={() => (current = section.name)}
        >
          {t(section.titleKey)}
        </button>
      {/each}
    </nav>
  {/if}

  <Current />
</section>
