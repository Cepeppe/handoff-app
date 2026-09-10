<!--
  Settings (§7.6): General, Agents, Network, Log, Runbooks, Updates.

  A mode of the one window and not a second one (DD-10, MULTI-04), and inside it a list of
  sections rather than a single page: §7.6 names six, they arrive with the tasks that own
  them, and a container that already lists them keeps each of those to one entry here.

  All six exist now: General (T-041), Agents (T-040), Log and Runbooks (T-045), Network and
  Updates (T-051). They are listed in the order §7.6 names them. Updates is one sentence in
  this build — it does not check — and it is here rather than left out because "does this
  thing phone home" is a question a person looks for under that word; T-078 fills it in.

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
  import LogSettings from '../settings/LogSettings.svelte';
  import NetworkSettings from '../settings/NetworkSettings.svelte';
  import RunbooksSettings from '../settings/RunbooksSettings.svelte';
  import UpdatesSettings from '../settings/UpdatesSettings.svelte';

  const SECTIONS = [
    { name: 'general', titleKey: 'settings.general', view: GeneralSettings },
    { name: 'agents', titleKey: 'install.agents', view: AgentsSettings },
    { name: 'network', titleKey: 'settings.network', view: NetworkSettings },
    { name: 'log', titleKey: 'settings.log', view: LogSettings },
    { name: 'runbooks', titleKey: 'settings.runbooks', view: RunbooksSettings },
    { name: 'updates', titleKey: 'settings.updates', view: UpdatesSettings },
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
