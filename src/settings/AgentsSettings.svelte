<!--
  Settings → Agents (§7.6, INST-01, INST-04, INST-05, INST-06, FM-10, FM-23).

  What onboarding does once, this page does for ever: one row per adapter, saying what is
  registered in the chosen scope and offering the three things a person can do about it.

  - **Register** and **Repair** are the same operation seen from two states. `plan` always
    returns every modification, no-ops included, and `apply` writes only what really differs
    (T-039), so a half-registered machine and a moved bundle (FM-23) are both repaired by
    applying the plan. The button changes its name and nothing else.
  - **Remove** takes out exactly our entries and leaves the rest of the file as it was
    (INST-04). No consent screen: nothing of the user's is written.
  - **Repair the token** regenerates `~/.handoff/channel.token` (FM-10). It is the one repair
    that has nothing to do with an agent's configuration, which is why it sits under the
    list rather than in a row.

  The scope selector is INST-06: user by default, one project folder as the alternative, and
  the folder comes from the system picker rather than from a text field — a path typed by
  hand is a path that is wrong once in ten. Every read and every write on this page is for
  the scope that is selected, so switching it re-reads the list.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { AgentStatus, ConsentView, Registration, Scope } from '../model';

  import ConsentLines from './ConsentLines.svelte';

  let scopeKind = $state<'user' | 'project'>('user');
  let folder = $state<string | null>(null);
  let agents = $state<AgentStatus[]>([]);
  let plan = $state<ConsentView | null>(null);
  let problem = $state<string | null>(null);
  let said = $state<string | null>(null);
  let busy = $state(false);

  /** The scope every read and write on this page uses (INST-06). */
  const scope = $derived<Scope>(
    scopeKind === 'project' && folder !== null ? { kind: 'project', path: folder } : { kind: 'user' },
  );

  /** A project scope with no folder chosen yet can be read but not written to. */
  const ready = $derived(scopeKind === 'user' || folder !== null);

  onMount(() => {
    void load();
  });

  /** The list, for the scope in force. Also the "Find agents" action (INST-05). */
  async function load(): Promise<void> {
    problem = null;
    try {
      agents = await bridge().agents(scope);
    } catch (error) {
      agents = [];
      problem = t('install.failed', { reason: String(error) });
    }
  }

  /** The catalogue key of a registration state, for the row's status line. */
  function statusKey(agent: AgentStatus): string {
    if (!agent.found) {
      return 'install.statusNotFound';
    }
    switch (agent.registration.kind) {
      case 'registered':
        return 'install.statusRegistered';
      case 'partial':
        return 'install.statusPartial';
      case 'path_mismatch':
        return 'install.statusPathMismatch';
      default:
        return 'install.statusNotRegistered';
    }
  }

  /** What the write button is called in this state: the same operation, two names. */
  function actionKey(registration: Registration): string {
    return registration.kind === 'registered' || registration.kind === 'not_registered'
      ? 'install.register'
      : 'install.repair';
  }

  /** Opens the consent screen for one agent (INST-01). Nothing is written yet. */
  async function propose(agentId: string): Promise<void> {
    said = null;
    try {
      plan = await bridge().consentPlan(agentId, scope);
    } catch (error) {
      plan = null;
      problem = t('install.failed', { reason: String(error) });
    }
  }

  /**
   * Writes the plan the user is looking at (INST-01).
   *
   * A refusal here is very often the honest one: the file moved on between the screen and
   * the button, and the Rust side refused rather than write something nobody saw. Showing
   * the new plan is the answer, so the screen re-plans instead of closing.
   */
  async function accept(): Promise<void> {
    const shown = plan;
    if (shown === null || busy) {
      return;
    }
    busy = true;
    try {
      await bridge().installAgent(shown.agentId, scope, shown.digest);
      said = t('install.applied', { agent: t(shown.nameKey) });
      plan = null;
      await load();
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
      await propose(shown.agentId);
    } finally {
      busy = false;
    }
  }

  /** Takes our entries out and leaves everything else (INST-04). */
  async function remove(agent: AgentStatus): Promise<void> {
    if (busy) {
      return;
    }
    busy = true;
    problem = null;
    try {
      await bridge().uninstallAgent(agent.agentId, scope);
      said = t('install.removed', { agent: t(agent.nameKey) });
      await load();
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    } finally {
      busy = false;
    }
  }

  /** FM-10: the token file, regenerated. The server re-reads it at its next attempt. */
  async function repairToken(): Promise<void> {
    problem = null;
    try {
      await bridge().repairToken();
      said = t('install.tokenRepaired');
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    }
  }

  /** INST-06: the project folder, from the system picker. */
  async function chooseFolder(): Promise<void> {
    const chosen = await bridge().pickProjectFolder();
    if (chosen !== null) {
      folder = chosen;
      await load();
    }
  }

  async function chooseScope(kind: 'user' | 'project'): Promise<void> {
    scopeKind = kind;
    plan = null;
    said = null;
    if (kind === 'user' || folder !== null) {
      await load();
    }
  }
</script>

<section class="settings-section" data-settings="agents">
  <h2>{t('install.agents')}</h2>

  <fieldset class="scope">
    <legend>{t('install.scope')}</legend>
    <label>
      <input
        type="radio"
        name="scope"
        value="user"
        checked={scopeKind === 'user'}
        onchange={() => void chooseScope('user')}
      />
      {t('install.scopeUser')}
    </label>
    <label>
      <input
        type="radio"
        name="scope"
        value="project"
        checked={scopeKind === 'project'}
        onchange={() => void chooseScope('project')}
      />
      {t('install.scopeProject')}
    </label>
    {#if scopeKind === 'project'}
      <p class="scope-folder">{folder ?? t('install.noFolder')}</p>
      <button type="button" class="button button-quiet" onclick={() => void chooseFolder()}>
        {t('install.chooseFolder')}
      </button>
    {/if}
  </fieldset>

  {#if plan !== null}
    <div class="consent" role="group" aria-label={t('install.agents')}>
      <ConsentLines {plan} />
      <div class="actions" role="group" aria-label={t('overlay.actions')}>
        <button
          type="button"
          class="button button-primary"
          disabled={busy || !ready}
          onclick={() => void accept()}
        >
          {t('install.accept')}
        </button>
        <button type="button" class="button button-quiet" onclick={() => (plan = null)}>
          {t('install.later')}
        </button>
      </div>
    </div>
  {:else}
    <ul class="agent-list">
      {#each agents as agent (agent.agentId)}
        <li class="agent">
          <p class="agent-name">{t(agent.nameKey)}</p>
          <p class="agent-status">{t(statusKey(agent))}</p>
          {#if agent.registration.kind === 'partial'}
            <p class="agent-detail">
              {t('install.missing', { locations: agent.registration.missing.join(', ') })}
            </p>
          {:else if agent.registration.kind === 'path_mismatch'}
            <p class="agent-detail">
              {t('install.movedFrom', {
                registered: agent.registration.registered,
                current: agent.registration.current,
              })}
            </p>
          {/if}
          <p class="agent-detail">{t('install.files')}: {agent.configFiles.join(' · ')}</p>
          <div class="actions" role="group" aria-label={t('overlay.actions')}>
            <button
              type="button"
              class="button"
              disabled={!agent.found || !ready || busy}
              onclick={() => void propose(agent.agentId)}
            >
              {t(actionKey(agent.registration))}
            </button>
            <button
              type="button"
              class="button button-quiet"
              disabled={agent.registration.kind === 'not_registered' || !ready || busy}
              onclick={() => void remove(agent)}
            >
              {t('install.uninstall')}
            </button>
          </div>
        </li>
      {/each}
    </ul>

    <button type="button" class="button button-quiet" onclick={() => void load()}>
      {t('install.findAgents')}
    </button>
  {/if}

  {#if said !== null}
    <p class="settings-said" role="status">{said}</p>
  {/if}
  {#if problem !== null}
    <p class="settings-problem" role="alert">{problem}</p>
  {/if}

  <div class="token-repair">
    <p class="settings-explain">{t('install.repairTokenText')}</p>
    <button type="button" class="button button-quiet" onclick={() => void repairToken()}>
      {t('install.repairToken')}
    </button>
  </div>
</section>
