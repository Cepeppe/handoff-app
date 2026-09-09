<!--
  Onboarding (§7.6, F-13, INST-01..03, INST-06, APP-01, CAP-04, OPEN-03).

  The one screen a person sees before Baton has done anything, and the only place it asks for
  something. Welcome → [move to Applications] → agents and consent → autostart → [screen
  recording] → shortcut → done, with the two bracketed steps on macOS alone.

  **The step list comes from the Rust side** (`ui_bridge::install::onboarding`) and is drawn
  here in the order it arrives. That is deliberate: which steps exist is a fact about the
  platform and about where the application is installed, and a rule written on both sides is
  a rule that disagrees with itself the first time one of them is edited.

  What each step does, and what it deliberately does not:

  - **Move** (macOS, bundle outside `/Applications`) comes *before* consent, because the
    fixed launcher path written into every agent configuration is derived from where the
    application is (SRV-25): registering first and moving afterwards is FM-23 arranged in
    advance. It cannot move itself — an application cannot move its own running bundle — so
    it explains and the user does it.
  - **Agents** registers in user scope only (INST-06 offers project scope in settings, not
    here) and one agent at a time, each behind its own consent screen. Nothing is written
    until the button is pressed, and a machine with no supported agent says so and moves on.
  - **Autostart** asks the question of APP-01 with the box pre-checked; the answer is stored
    and the plugin is wired up by the General settings page (T-041).
  - **Screen recording** (macOS) explains and opens the pane. It never triggers the system
    prompt: CAP-04 puts the permission here precisely so that it is never asked for in the
    middle of a handoff (FM-17), and the call itself is T-059.
  - **Shortcut** shows the combination, and the FM-18 recorder when the system refused it.

  Finishing writes `onboarded`, so a second launch comes straight to the overlay.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { AgentStatus, ConsentView, OnboardingStep, ShortcutStatus } from '../model';
  import ShortcutDialog from '../overlay/ShortcutDialog.svelte';
  import ConsentLines from '../settings/ConsentLines.svelte';
  import { resetView } from '../view-state.svelte';

  /** The scope onboarding registers in. INST-06 keeps project scope out of this screen. */
  const USER_SCOPE = { kind: 'user' } as const;

  let steps = $state<OnboardingStep[]>([]);
  let at = $state(0);
  let autostart = $state(true);
  let agents = $state<AgentStatus[]>([]);
  let plan = $state<ConsentView | null>(null);
  let registered = $state<Record<string, boolean>>({});
  let shortcut = $state<ShortcutStatus | null>(null);
  let problem = $state<string | null>(null);
  let busy = $state(false);

  const step = $derived<OnboardingStep | null>(steps[at] ?? null);
  const first = $derived(at === 0);
  const last = $derived(at >= steps.length - 1);

  onMount(() => {
    void start();
  });

  async function start(): Promise<void> {
    const flow = await bridge().onboarding();
    steps = flow.steps;
    // The two steps that need something read before they are reached. Both are cheap, and
    // reading them now means the step draws with its answer rather than blank.
    agents = await bridge().agents(USER_SCOPE);
    shortcut = await bridge().shortcutStatus();
  }

  function back(): void {
    problem = null;
    plan = null;
    at = Math.max(0, at - 1);
  }

  async function next(): Promise<void> {
    problem = null;
    plan = null;
    if (!last) {
      at += 1;
      return;
    }
    await finish();
  }

  /** The last step: remember the answers, and get out of the way. */
  async function finish(): Promise<void> {
    if (busy) {
      return;
    }
    busy = true;
    try {
      await bridge().finishOnboarding(autostart);
      resetView();
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    } finally {
      busy = false;
    }
  }

  /** Opens the consent screen for one agent (INST-01). Nothing is written yet. */
  async function propose(agentId: string): Promise<void> {
    try {
      plan = await bridge().consentPlan(agentId, USER_SCOPE);
      problem = null;
    } catch (error) {
      plan = null;
      problem = t('install.failed', { reason: String(error) });
    }
  }

  /** Writes the plan on screen, then re-reads what the machine now holds. */
  async function accept(): Promise<void> {
    const shown = plan;
    if (shown === null || busy) {
      return;
    }
    busy = true;
    try {
      await bridge().installAgent(shown.agentId, USER_SCOPE, shown.digest);
      registered = { ...registered, [shown.agentId]: true };
      plan = null;
      agents = await bridge().agents(USER_SCOPE);
    } catch (error) {
      // The file moved on between the screen and the button, and nothing was written
      // (INST-01). The new plan is the answer, so the screen stays and asks again.
      problem = t('install.failed', { reason: String(error) });
      await propose(shown.agentId);
    } finally {
      busy = false;
    }
  }

  async function openScreenRecording(): Promise<void> {
    try {
      await bridge().openScreenRecordingSettings();
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    }
  }

  /** The agents worth showing on the consent step: the ones actually on this machine. */
  const found = $derived(agents.filter((agent) => agent.found));

  function alreadyDone(agent: AgentStatus): boolean {
    return registered[agent.agentId] === true || agent.registration.kind === 'registered';
  }
</script>

<section class="view" data-view="onboarding">
  {#if step !== null}
    <h1>{t(`onboarding.${step}Title`)}</h1>

    {#if step === 'welcome'}
      <p>{t('onboarding.welcomeText')}</p>
    {:else if step === 'move'}
      <p>{t('onboarding.moveText')}</p>
    {:else if step === 'agents'}
      {#if plan !== null}
        <ConsentLines {plan} />
        <div class="actions" role="group" aria-label={t('overlay.actions')}>
          <button
            type="button"
            class="button button-primary"
            disabled={busy}
            onclick={() => void accept()}
          >
            {t('install.accept')}
          </button>
          <button type="button" class="button button-quiet" onclick={() => (plan = null)}>
            {t('install.later')}
          </button>
        </div>
      {:else}
        <p>{t('onboarding.agentsText')}</p>
        {#if found.length === 0}
          <p class="onboarding-none" role="status">{t('onboarding.agentsNone')}</p>
        {:else}
          <ul class="agent-list">
            {#each found as agent (agent.agentId)}
              <li class="agent">
                <p class="agent-name">{t(agent.nameKey)}</p>
                {#if alreadyDone(agent)}
                  <p class="agent-status">{t('install.statusRegistered')}</p>
                {:else}
                  <button
                    type="button"
                    class="button"
                    disabled={busy}
                    onclick={() => void propose(agent.agentId)}
                  >
                    {t('install.register')}
                  </button>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    {:else if step === 'autostart'}
      <p>{t('onboarding.autostartText')}</p>
      <label class="onboarding-autostart">
        <input type="checkbox" bind:checked={autostart} />
        {t('onboarding.autostart')}
      </label>
    {:else if step === 'screenRecording'}
      <p>{t('onboarding.screenRecordingText')}</p>
      <button
        type="button"
        class="button button-quiet"
        onclick={() => void openScreenRecording()}
      >
        {t('onboarding.screenRecordingOpen')}
      </button>
    {:else if step === 'shortcut'}
      {#if shortcut !== null && !shortcut.registered}
        <p class="onboarding-problem" role="status">
          {t('onboarding.shortcutTaken', { accelerator: shortcut.accelerator })}
        </p>
        <ShortcutDialog
          accelerator={shortcut.accelerator}
          ondone={() => void bridge().shortcutStatus().then((status) => (shortcut = status))}
        />
      {:else}
        <p>{t('onboarding.shortcutText', { accelerator: shortcut?.accelerator ?? '' })}</p>
      {/if}
    {:else if step === 'done'}
      <p>{t('onboarding.doneText')}</p>
    {/if}

    {#if problem !== null}
      <p class="onboarding-problem" role="alert">{problem}</p>
    {/if}

    <div class="onboarding-actions" role="group" aria-label={t('overlay.actions')}>
      {#if !first}
        <button type="button" class="button button-quiet" onclick={back}>
          {t('onboarding.back')}
        </button>
      {/if}
      <button
        type="button"
        class="button button-primary"
        disabled={busy}
        onclick={() => void next()}
      >
        {last ? t('onboarding.finish') : t('onboarding.next')}
      </button>
    </div>
  {/if}
</section>
