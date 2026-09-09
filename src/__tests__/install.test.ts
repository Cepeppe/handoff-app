/**
 * Onboarding, the consent screen and the Agents settings page (§7.6, §7.15, F-13).
 *
 * Rendered under jsdom with a fake core behind it, like the rest of the frontend suite: what
 * is checked is what the window *shows* and what it *asks the core to do*. Whether the files
 * then come out right is the golden-file suite's, on the other side of the bridge.
 *
 * Three things here are promises to the user rather than details of a component, and each
 * has a case of its own:
 *
 * - **Nothing is written before it has been shown** (INST-01). The Show control reveals the
 *   real diff, and Accept sends back the digest of the plan that was on screen.
 * - **Three modifications on two rows** (INST-02), with no fourth line about a global
 *   timeout variable anywhere (T-026, Option B).
 * - **A launch decides for itself what is worth opening the window for** (§7.2): onboarding
 *   on a first launch, the repair offer when the bundle moved (FM-23), a discreet notice for
 *   a newly found agent (INST-05) and nothing at all otherwise.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import App from '../App.svelte';
import { setBridge } from '../bridge';
import { catalogue, DEFAULT_LANGUAGE, LANGUAGES, setLanguage, t } from '../i18n';
import type { AgentStatus, ConsentView, OnboardingStep, ScanReport } from '../model';
import { currentNotice, resetOverlay } from '../overlay/state.svelte';
import AgentsSettings from '../settings/AgentsSettings.svelte';
import { resetView, view } from '../view-state.svelte';
import OnboardingView from '../views/OnboardingView.svelte';
import { fakeBridge } from './fake-bridge';

const MCP_DIFF = '--- .claude.json · mcpServers.handoff (absent)\n+++ .claude.json\n+{}\n';
const HOOKS_DIFF =
  '--- settings.json · hooks.Stop (absent)\n+++ settings.json\n+[]\n' +
  '--- settings.json · hooks.SubagentStop (absent)\n+++ settings.json\n+[]\n';

/** The plan of an empty machine: three modifications, two rows, nothing in order yet. */
function plan(overrides: Partial<ConsentView> = {}): ConsentView {
  return {
    agentId: 'claude-code',
    nameKey: 'agent.claudeCode',
    modificationCount: 3,
    digest: 'd1',
    alreadyInOrder: false,
    lines: [
      {
        description: {
          key: 'install.claudeCode.mcpEntry',
          args: { file: '.claude.json', server: '/apps/Baton/handoff-mcp', minutes: '30' },
        },
        locations: ['.claude.json · mcpServers.handoff'],
        diff: MCP_DIFF,
        isNoop: false,
      },
      {
        description: { key: 'install.claudeCode.hooks', args: { file: 'settings.json' } },
        locations: ['settings.json · hooks.Stop', 'settings.json · hooks.SubagentStop'],
        diff: HOOKS_DIFF,
        isNoop: false,
      },
    ],
    ...overrides,
  };
}

function agent(overrides: Partial<AgentStatus> = {}): AgentStatus {
  return {
    agentId: 'claude-code',
    nameKey: 'agent.claudeCode',
    found: true,
    configFiles: ['/home/x/.claude.json', '/home/x/.claude/settings.json'],
    registration: { kind: 'not_registered' },
    ...overrides,
  };
}

const NOTHING: ScanReport = { newAgents: [], moved: [] };

beforeEach(() => {
  resetView();
  resetOverlay();
  setLanguage(DEFAULT_LANGUAGE);
});

afterEach(() => {
  cleanup();
  setBridge(null);
});

describe('the consent screen (INST-01, INST-02)', () => {
  it('says how many modifications there are and draws the hooks on one row', async () => {
    setBridge(
      fakeBridge({
        agents: vi.fn(async () => [agent()]),
        consentPlan: vi.fn(async () => plan()),
      }),
    );
    render(AgentsSettings);

    fireEvent.click(await screen.findByText(t('install.register')));

    // INST-02: the count is three and the rows are two. The two numbers are different on
    // purpose, and a screen that derived one from the other would print two changes.
    await screen.findByText(
      t('install.consentIntro', { count: 3, agent: t('agent.claudeCode') }),
    );
    expect(screen.getAllByText(t('install.show'))).toHaveLength(2);
    expect(
      screen.getByText(t('install.claudeCode.hooks', { file: 'settings.json' })),
    ).toBeTruthy();
  });

  it('reveals the exact diff behind Show, and both hooks behind the hooks row', async () => {
    setBridge(
      fakeBridge({
        agents: vi.fn(async () => [agent()]),
        consentPlan: vi.fn(async () => plan()),
      }),
    );
    render(AgentsSettings);
    fireEvent.click(await screen.findByText(t('install.register')));

    const shows = await screen.findAllByText(t('install.show'));
    // Closed until asked: the summary is the screen, the diff is the detail.
    expect(shows[0].getAttribute('aria-expanded')).toBe('false');
    expect(document.body.textContent).not.toContain('mcpServers.handoff (absent)');

    fireEvent.click(shows[0]);
    await waitFor(() => expect(document.body.textContent).toContain(MCP_DIFF.trim()));

    fireEvent.click((await screen.findAllByText(t('install.show')))[0]);
    await waitFor(() => {
      expect(document.body.textContent).toContain('hooks.Stop');
      expect(document.body.textContent).toContain('hooks.SubagentStop');
    });
  });

  it('sends back the digest of the plan it showed, and nothing else', async () => {
    const installAgent = vi.fn(async () => {});
    setBridge(
      fakeBridge({
        agents: vi.fn(async () => [agent()]),
        consentPlan: vi.fn(async () => plan({ digest: 'the-one-shown' })),
        installAgent,
      }),
    );
    render(AgentsSettings);

    fireEvent.click(await screen.findByText(t('install.register')));
    fireEvent.click(await screen.findByText(t('install.accept')));

    await waitFor(() =>
      expect(installAgent).toHaveBeenCalledWith(
        'claude-code',
        { kind: 'user' },
        'the-one-shown',
      ),
    );
  });

  it('shows the new plan instead of writing when the file moved on (INST-01)', async () => {
    // The core refuses a plan whose fingerprint no longer matches. The screen's answer is to
    // ask again, never to close as though something had happened.
    const consentPlan = vi
      .fn<() => Promise<ConsentView>>()
      .mockResolvedValueOnce(plan({ digest: 'stale' }))
      .mockResolvedValue(plan({ digest: 'fresh' }));
    setBridge(
      fakeBridge({
        agents: vi.fn(async () => [agent()]),
        consentPlan,
        installAgent: vi.fn(async () => {
          throw new Error('the configuration changed since it was shown');
        }),
      }),
    );
    render(AgentsSettings);

    fireEvent.click(await screen.findByText(t('install.register')));
    fireEvent.click(await screen.findByText(t('install.accept')));

    await waitFor(() => expect(consentPlan).toHaveBeenCalledTimes(2));
    expect(screen.getByRole('alert').textContent).toContain('changed since it was shown');
    // Still on the consent screen, with the plan that is true now.
    expect(screen.getByText(t('install.accept'))).toBeTruthy();
  });

  it('has no fourth line and no sentence about every MCP server (T-026, Option B)', () => {
    // The global `MCP_TOOL_TIMEOUT` was dropped on 2026-09-08 with its consent line. The
    // check is over the catalogues, because that is where such a sentence would have to live
    // (T-028) and it is the one place a stray one would survive a component being rewritten.
    for (const language of LANGUAGES) {
      const texts = Object.entries(catalogue(language));
      expect(texts.filter(([key]) => key.startsWith('install.claudeCode.'))).toHaveLength(4);
      for (const [key, value] of texts) {
        expect(value.toLowerCase(), `${language}.${key}`).not.toContain('mcp_tool_timeout');
        expect(value.toLowerCase(), `${language}.${key}`).not.toContain('all mcp servers');
        expect(value.toLowerCase(), `${language}.${key}`).not.toContain('tutti i server mcp');
      }
    }
  });
});

describe('the Agents settings page (INST-04, INST-05, INST-06, FM-10, FM-23)', () => {
  it('names each state and offers Repair rather than Register when there is one', async () => {
    setBridge(
      fakeBridge({
        agents: vi.fn(async () => [
          agent({ registration: { kind: 'partial', missing: ['settings.json · hooks.Stop'] } }),
        ]),
      }),
    );
    render(AgentsSettings);

    await screen.findByText(t('install.statusPartial'));
    expect(
      screen.getByText(t('install.missing', { locations: 'settings.json · hooks.Stop' })),
    ).toBeTruthy();
    expect(screen.getByText(t('install.repair'))).toBeTruthy();
    expect(screen.queryByText(t('install.register'))).toBeNull();
  });

  it('names both paths when the bundle moved (FM-23)', async () => {
    setBridge(
      fakeBridge({
        agents: vi.fn(async () => [
          agent({
            registration: { kind: 'path_mismatch', registered: '/old/x', current: '/new/x' },
          }),
        ]),
      }),
    );
    render(AgentsSettings);

    await screen.findByText(t('install.statusPathMismatch'));
    expect(
      screen.getByText(t('install.movedFrom', { registered: '/old/x', current: '/new/x' })),
    ).toBeTruthy();
  });

  it('says an agent that is not on the machine cannot be registered', async () => {
    setBridge(fakeBridge({ agents: vi.fn(async () => [agent({ found: false })]) }));
    render(AgentsSettings);

    await screen.findByText(t('install.statusNotFound'));
    expect(screen.getByText(t('install.register')).hasAttribute('disabled')).toBe(true);
  });

  it('removes only our entries, and re-reads afterwards', async () => {
    const uninstallAgent = vi.fn(async () => {});
    const agents = vi.fn(async () => [agent({ registration: { kind: 'registered' } })]);
    setBridge(fakeBridge({ agents, uninstallAgent }));
    render(AgentsSettings);

    fireEvent.click(await screen.findByText(t('install.uninstall')));

    await waitFor(() =>
      expect(uninstallAgent).toHaveBeenCalledWith('claude-code', { kind: 'user' }),
    );
    await waitFor(() => expect(agents).toHaveBeenCalledTimes(2));
    expect(screen.getByRole('status').textContent).toContain(t('agent.claudeCode'));
  });

  it('re-reads the list for the project folder the picker returned (INST-06)', async () => {
    const agents = vi.fn(async () => [agent()]);
    setBridge(
      fakeBridge({ agents, pickProjectFolder: vi.fn(async () => 'C:\\work\\shop') }),
    );
    render(AgentsSettings);
    await screen.findByText(t('install.register'));

    fireEvent.click(screen.getByLabelText(t('install.scopeProject')));
    // Nothing is read for a project scope with no folder: there is no project yet.
    await waitFor(() => expect(screen.getByText(t('install.noFolder'))).toBeTruthy());

    fireEvent.click(screen.getByText(t('install.chooseFolder')));
    await waitFor(() =>
      expect(agents).toHaveBeenLastCalledWith({ kind: 'project', path: 'C:\\work\\shop' }),
    );
  });

  it('regenerates the token and says so (FM-10)', async () => {
    const repairToken = vi.fn(async () => {});
    setBridge(fakeBridge({ agents: vi.fn(async () => [agent()]), repairToken }));
    render(AgentsSettings);

    fireEvent.click(await screen.findByText(t('install.repairToken')));

    await waitFor(() => expect(repairToken).toHaveBeenCalledTimes(1));
    expect((await screen.findByRole('status')).textContent).toBe(t('install.tokenRepaired'));
  });
});

describe('onboarding (§7.6, F-13)', () => {
  /** A flow with the steps the Rust side would give a Windows machine. */
  function windowsFlow(overrides = {}) {
    return fakeBridge({
      onboarding: vi.fn(async () => ({
        needed: true,
        steps: ['welcome', 'agents', 'autostart', 'shortcut', 'done'] as OnboardingStep[],
      })),
      agents: vi.fn(async () => [agent()]),
      ...overrides,
    });
  }

  it('walks the steps the core gave it, in order', async () => {
    setBridge(windowsFlow());
    render(OnboardingView);

    await screen.findByText(t('onboarding.welcomeTitle'));
    for (const step of ['agents', 'autostart', 'shortcut', 'done']) {
      fireEvent.click(screen.getByText(t('onboarding.next')));
      await screen.findByText(t(`onboarding.${step}Title`));
    }
    // The last step finishes rather than going on.
    expect(screen.queryByText(t('onboarding.next'))).toBeNull();
    expect(screen.getByText(t('onboarding.finish'))).toBeTruthy();
  });

  it('registers in user scope only, behind the consent screen (INST-06)', async () => {
    const installAgent = vi.fn(async () => {});
    setBridge(
      windowsFlow({ consentPlan: vi.fn(async () => plan({ digest: 'x' })), installAgent }),
    );
    render(OnboardingView);

    await screen.findByText(t('onboarding.welcomeTitle'));
    fireEvent.click(screen.getByText(t('onboarding.next')));
    fireEvent.click(await screen.findByText(t('install.register')));

    // The plan is on screen and nothing has been written.
    await screen.findByText(
      t('install.consentIntro', { count: 3, agent: t('agent.claudeCode') }),
    );
    expect(installAgent).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText(t('install.accept')));
    await waitFor(() =>
      expect(installAgent).toHaveBeenCalledWith('claude-code', { kind: 'user' }, 'x'),
    );
  });

  it('says so and moves on when there is no agent to register', async () => {
    setBridge(windowsFlow({ agents: vi.fn(async () => [agent({ found: false })]) }));
    render(OnboardingView);

    await screen.findByText(t('onboarding.welcomeTitle'));
    fireEvent.click(screen.getByText(t('onboarding.next')));

    await screen.findByText(t('onboarding.agentsNone'));
    expect(screen.queryByText(t('install.register'))).toBeNull();
  });

  it('offers autostart pre-checked and remembers the answer (APP-01)', async () => {
    const finishOnboarding = vi.fn(async () => {});
    setBridge(windowsFlow({ finishOnboarding }));
    render(OnboardingView);

    await screen.findByText(t('onboarding.welcomeTitle'));
    fireEvent.click(screen.getByText(t('onboarding.next')));
    fireEvent.click(await screen.findByText(t('onboarding.next')));

    const box = await screen.findByLabelText<HTMLInputElement>(t('onboarding.autostart'));
    expect(box.checked).toBe(true);
    // APP-01's own sentence, and it is not the checkbox's label.
    expect(screen.getByText(t('onboarding.autostartText'))).toBeTruthy();

    fireEvent.click(box);
    fireEvent.click(screen.getByText(t('onboarding.next')));
    fireEvent.click(await screen.findByText(t('onboarding.next')));
    fireEvent.click(await screen.findByText(t('onboarding.finish')));

    await waitFor(() => expect(finishOnboarding).toHaveBeenCalledWith(false));
  });

  it('offers the recorder when the shortcut was refused (FM-18)', async () => {
    setBridge(
      windowsFlow({
        shortcutStatus: vi.fn(async () => ({
          accelerator: 'Control+Alt+H',
          registered: false,
          askForAnother: true,
        })),
      }),
    );
    render(OnboardingView);

    await screen.findByText(t('onboarding.welcomeTitle'));
    for (const _ of [0, 1, 2]) {
      fireEvent.click(await screen.findByText(t('onboarding.next')));
    }

    await screen.findByText(t('onboarding.shortcutTitle'));
    expect(
      screen.getByText(t('onboarding.shortcutTaken', { accelerator: 'Control+Alt+H' })),
    ).toBeTruthy();
    expect(screen.getByText(t('shortcut.record'))).toBeTruthy();
  });
});

describe('what a launch decides to open the window for (§7.2, INST-05, FM-23)', () => {
  it('opens onboarding on a first launch, and scans nothing', async () => {
    const showWindow = vi.fn(async () => {});
    const scanAgents = vi.fn(async () => NOTHING);
    setBridge(
      fakeBridge({
        onboarding: vi.fn(async () => ({
          needed: true,
          steps: ['welcome', 'done'] as OnboardingStep[],
        })),
        showWindow,
        scanAgents,
      }),
    );
    render(App);

    await waitFor(() => expect(view()).toBe('onboarding'));
    expect(showWindow).toHaveBeenCalledTimes(1);
    // The consent screen is about to show the agent; announcing it as a discovery first
    // would be the same news twice, and `finishOnboarding` records it as known anyway.
    expect(scanAgents).not.toHaveBeenCalled();
  });

  it('opens the Agents page when the bundle moved (FM-23)', async () => {
    const showWindow = vi.fn(async () => {});
    setBridge(
      fakeBridge({
        showWindow,
        scanAgents: vi.fn(async () => ({
          newAgents: [],
          moved: [
            {
              agentId: 'claude-code',
              nameKey: 'agent.claudeCode',
              registered: '/old/x',
              current: '/new/x',
            },
          ],
        })),
      }),
    );
    render(App);

    await waitFor(() => expect(view()).toBe('settings'));
    expect(showWindow).toHaveBeenCalledTimes(1);
  });

  it('shows one discreet notice for a newly found agent and leaves the window alone', async () => {
    const showWindow = vi.fn(async () => {});
    setBridge(
      fakeBridge({
        showWindow,
        scanAgents: vi.fn(async () => ({
          newAgents: [{ agentId: 'claude-code', nameKey: 'agent.claudeCode' }],
          moved: [],
        })),
      }),
    );
    render(App);

    await waitFor(() =>
      expect(currentNotice()?.text).toBe(
        t('install.newAgent', { agent: t('agent.claudeCode') }),
      ),
    );
    expect(view()).toBe('overlay');
    expect(showWindow).not.toHaveBeenCalled();
  });

  it('opens nothing and says nothing on an ordinary launch', async () => {
    const showWindow = vi.fn(async () => {});
    const scanAgents = vi.fn(async () => NOTHING);
    setBridge(fakeBridge({ showWindow, scanAgents }));
    render(App);

    await waitFor(() => expect(scanAgents).toHaveBeenCalledTimes(1));
    expect(view()).toBe('overlay');
    expect(showWindow).not.toHaveBeenCalled();
    expect(currentNotice()).toBeNull();
  });
});
