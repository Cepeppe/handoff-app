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
  room open the window in a wider layout temporarily"). The width itself is not asked for
  here: `App.svelte` derives the window's layout from the view in one place, so leaving this
  page by any route — the nav, the tray, a handoff arriving — gives the width back without
  anyone having to remember to.

  The pages are a column on the left rather than a row of chips above, because at this width
  six labels in one row are six truncated labels, and because a settings page is the one
  screen in this application a person browses rather than acts on.
-->
<script lang="ts">
  import { t } from '../i18n';
  import Icon from '../overlay/Icon.svelte';
  import AgentsSettings from '../settings/AgentsSettings.svelte';
  import GeneralSettings from '../settings/GeneralSettings.svelte';
  import LogSettings from '../settings/LogSettings.svelte';
  import NetworkSettings from '../settings/NetworkSettings.svelte';
  import RunbooksSettings from '../settings/RunbooksSettings.svelte';
  import UpdatesSettings from '../settings/UpdatesSettings.svelte';
  import { resetView } from '../view-state.svelte';

  const SECTIONS = [
    { name: 'general', titleKey: 'settings.general', icon: 'settings', view: GeneralSettings },
    { name: 'agents', titleKey: 'install.agents', icon: 'agents', view: AgentsSettings },
    { name: 'network', titleKey: 'settings.network', icon: 'network', view: NetworkSettings },
    { name: 'log', titleKey: 'settings.log', icon: 'log', view: LogSettings },
    { name: 'runbooks', titleKey: 'settings.runbooks', icon: 'runbooks', view: RunbooksSettings },
    { name: 'updates', titleKey: 'settings.updates', icon: 'updates', view: UpdatesSettings },
  ] as const;

  let current = $state<(typeof SECTIONS)[number]['name']>('general');
  const Current = $derived(
    SECTIONS.find((section) => section.name === current)?.view ?? GeneralSettings,
  );
</script>

<section class="view view-settings" data-view="settings">
  <nav class="settings-nav" aria-label={t('view.settings')}>
    {#each SECTIONS as section (section.name)}
      <button
        type="button"
        class="settings-nav-item"
        aria-current={current === section.name ? 'page' : undefined}
        onclick={() => (current = section.name)}
      >
        <Icon name={section.icon} size={15} />
        {t(section.titleKey)}
      </button>
    {/each}

    <span class="settings-nav-gap"></span>

    <button type="button" class="button button-quiet settings-back" onclick={resetView}>
      <Icon name="back" size={14} />
      {t('settings.back')}
    </button>
  </nav>

  <div class="column settings-page">
    <h1>{t('view.settings')}</h1>
    <Current />
  </div>
</section>
