<!--
  Settings (§7.6): General, Agents, Network, Log, Runbooks, Updates.

  A mode of the one window and not a second one (DD-10, MULTI-04), and inside it a list of
  sections rather than a single page: §7.6 names six, they arrive with the tasks that own
  them, and a container that already lists them keeps each of those to one entry here.

  Agents is the one that exists (T-040). General is T-041, Log is T-045, Runbooks T-044,
  Network T-051 and Updates T-078; none of them is drawn as an empty placeholder, because a
  section that says nothing is a section the user has to learn to skip.
-->
<script lang="ts">
  import { t } from '../i18n';
  import AgentsSettings from '../settings/AgentsSettings.svelte';

  // TASK: T-041 — General joins this list, and the window widens while settings are open.
  const SECTIONS = [{ name: 'agents', titleKey: 'install.agents', view: AgentsSettings }] as const;

  let current = $state<(typeof SECTIONS)[number]['name']>('agents');
  const Current = $derived(
    SECTIONS.find((section) => section.name === current)?.view ?? AgentsSettings,
  );
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
