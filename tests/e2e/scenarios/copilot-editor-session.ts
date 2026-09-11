/**
 * A session VS Code starts for Copilot's chat shows in the overlay with the right project (T-072's
 * acceptance; §5.6, §7.5, §7.7, §14 R-12, OPEN-02): the editor half of the GitHub Copilot subset.
 *
 * VS Code's chat cannot be driven from a script, and unlike Cursor's editor VS Code starts no MCP
 * server when a window opens: it starts one when a chat request needs it, or when the command
 * `workbench.mcp.startServer` is run. So the harness launches a VS Code of its own — a fresh
 * `--user-data-dir` whose `User/mcp.json` holds the entry the installer writes, with the run's
 * `HANDOFF_HOME` beside it, a fresh extensions folder, and `USERPROFILE`/`HOME` pointed at a
 * folder of the run — on a project folder with a name of its own, together with a two-file
 * extension in development mode that runs that command once the window is up, with
 * `{ autoTrustChanges: true }`: the argument VS Code's own "Start" link in `mcp.json` passes,
 * which starts a server of the user's configuration without the trust prompt. It spends nothing
 * and never touches the user's own VS Code: a separate user-data folder is a separate instance,
 * closed with everything it started when the scenario ends. It does open a window for the seconds
 * it takes. The recipe is the Copilot canary's
 * (`handoff-mcp/test/canary/agents/copilot/editor-runner.ts`, T-072), copied rather than imported
 * (§3.1 rule 3); the extension is written into the run's folder, so the one piece of CommonJS the
 * harness needs is two strings here.
 *
 * What it proves, from the app's side of the channel: the session is VS Code's (`Visual Studio
 * Code`), keyed at the editor (`ancestor_chain:editor`) with the launched VS Code in its chain, and
 * named after its workspace folder — which VS Code names in no variable, only as the roots of its
 * MCP client, and which the server asks for before `hello` — while its working directory is the
 * home folder VS Code starts its servers in. And that the window's title names that folder, which
 * is what `requests::focus` reads to bring the right window in front of a request. A window
 * running an extension in development mode has its title prefixed with `[Extension Development
 * Host]`, which a user's window never shows: the title is read without it.
 *
 * The log-invariant check that follows every scenario needs something planted: here it is a value
 * in the server's environment, which nothing on the channel may carry and nothing may write down.
 */
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { sleep } from '../automation.ts';
import { check, type Assertion } from '../classify.ts';
import { COPILOT_AGENT_ID, COPILOT_VSCODE_CLIENT_NAME, copilotVscodeEntry } from '../copilot.ts';
import type { Scenario } from '../scenario.ts';

/** The folder VS Code opens: a name of its own, so that a title naming it names nothing else. */
export const EDITOR_PROJECT = 'baton-copilot-window';

/** How long VS Code has to start our server and let it register (T-072 measured about seven seconds). */
export const REGISTRATION_TIMEOUT_MS = 90_000;

/** How long the window has to put its folder in its title once the session is there. */
export const TITLE_TIMEOUT_MS = 30_000;

/** What VS Code puts before the title of a window running an extension in development mode. */
export const DEVELOPMENT_HOST_PREFIX = '[Extension Development Host]';

/** Where VS Code is installed; `HANDOFF_E2E_VSCODE` overrides it. */
export function vscodePath(
  env: Readonly<Record<string, string | undefined>> = process.env,
  platform: NodeJS.Platform = process.platform,
): string {
  const override = env['HANDOFF_E2E_VSCODE']?.trim();
  if (override !== undefined && override !== '') return override;
  if (platform === 'win32') {
    return join(env['LOCALAPPDATA'] ?? '', 'Programs', 'Microsoft VS Code', 'Code.exe');
  }
  if (platform === 'darwin') return '/Applications/Visual Studio Code.app/Contents/MacOS/Code';
  return 'code';
}

/** The run's user settings: nothing that would put a dialog, a download or another server in the way. */
export const VSCODE_USER_SETTINGS = {
  'security.workspace.trust.enabled': false,
  'workbench.startupEditor': 'none',
  'update.mode': 'none',
  'telemetry.telemetryLevel': 'off',
  'extensions.autoCheckUpdates': false,
  'extensions.autoUpdate': false,
  'chat.mcp.discovery.enabled': false,
} as const;

/** The starter extension's manifest: activated once the window has started, and nothing else. */
export const STARTER_MANIFEST = {
  name: 'handoff-e2e-mcp-starter',
  displayName: 'Baton e2e: start the MCP servers of the profile',
  publisher: 'handoff-e2e',
  version: '0.0.1',
  engines: { vscode: '^1.99.0' },
  main: './extension.js',
  activationEvents: ['onStartupFinished'],
} as const;

/**
 * The starter extension itself: it asks VS Code to start every server of the profile — ours is
 * the only one — every three seconds for a minute, because the user configuration may not be read
 * yet the first time; a server already running is left alone.
 */
export const STARTER_SCRIPT = [
  "const vscode = require('vscode');",
  '',
  'exports.activate = async function activate() {',
  '  const deadline = Date.now() + 60000;',
  '  while (Date.now() < deadline) {',
  '    try {',
  "      await vscode.commands.executeCommand('workbench.mcp.startServer', '*', {",
  '        autoTrustChanges: true,',
  '      });',
  '    } catch {',
  '      // The MCP service is not up yet: the next round asks again.',
  '    }',
  '    await new Promise((resolve) => setTimeout(resolve, 3000));',
  '  }',
  '};',
  '',
  'exports.deactivate = function deactivate() {};',
  '',
].join('\n');

/**
 * VS Code's environment: the parent's, with the home folder moved to the run's. Whatever an editor
 * or a Claude Code session running the harness left in the environment is dropped:
 * `ELECTRON_RUN_AS_NODE` would start VS Code as a bare Node, and a `VSCODE_*` of another editor is
 * not this one's to inherit.
 */
function editorEnvironment(home: string): Record<string, string> {
  const child: Record<string, string> = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (value === undefined) continue;
    if (name === 'CLAUDECODE' || name.startsWith('ELECTRON_') || name.startsWith('VSCODE_')) {
      continue;
    }
    child[name] = value;
  }
  child['USERPROFILE'] = home;
  child['HOME'] = home;
  return child;
}

/** The title of the main window of `pid`, as Windows reports it; empty while there is none. */
function windowTitleOf(pid: number): string {
  if (process.platform !== 'win32') return '';
  try {
    return execFileSync(
      'powershell.exe',
      [
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        '[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; ' +
          `(Get-Process -Id ${String(pid)} -ErrorAction Stop).MainWindowTitle`,
      ],
      { encoding: 'utf8', windowsHide: true, timeout: 30_000 },
    ).trim();
  } catch {
    return '';
  }
}

/** A title as a user's window would show it: without the prefix of a development-mode window. */
export function userTitle(title: string): string {
  const trimmed = title.trim();
  return trimmed.startsWith(DEVELOPMENT_HOST_PREFIX)
    ? trimmed.slice(DEVELOPMENT_HOST_PREFIX.length).trim()
    : trimmed;
}

/** The rule of `requests::focus::title_names_folder`, in the harness's own words. */
function titleNamesFolder(title: string, folder: string): boolean {
  return title
    .replace(/ — | – /gu, ' - ')
    .split(' - ')
    .some((part) => part.trim().toLowerCase() === folder.toLowerCase());
}

/** Two spellings of one folder: separators, a trailing one and the case do not count. */
function sameFolder(left: string | null | undefined, right: string): boolean {
  const normal = (path: string): string =>
    path.replace(/\//gu, '\\').replace(/\\+$/u, '').toLowerCase();
  return typeof left === 'string' && normal(left) === normal(right);
}

/** Ends the VS Code it launched and everything it started, our server among them. */
function closeEditor(pid: number | undefined): void {
  if (pid === undefined) return;
  try {
    if (process.platform === 'win32') {
      execFileSync('taskkill', ['/PID', String(pid), '/T', '/F'], {
        stdio: 'ignore',
        windowsHide: true,
      });
    } else {
      process.kill(-pid, 'SIGTERM');
    }
  } catch {
    // Already gone: nothing left to close.
  }
}

function writeJson(file: string, value: unknown): void {
  writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, 'utf8');
}

export const copilotEditorScenario: Scenario = {
  id: 'copilot-editor-session',
  covers: 'R-12',
  title: "a session VS Code starts for Copilot shows under its window's folder, keyed at the editor",
  timeoutMs: 180_000,

  async run({ workspace, app, forbidden, facts, say }) {
    const editor = vscodePath();
    if (process.platform !== 'linux' && !existsSync(editor)) {
      return [
        check(
          'R-12',
          'VS Code is installed where the harness looks',
          'protocol',
          false,
          `${editor} does not exist; set HANDOFF_E2E_VSCODE`,
        ),
      ];
    }

    const project = join(workspace.root, EDITOR_PROJECT);
    const home = join(workspace.root, 'vscode-home');
    const userData = join(workspace.root, 'vscode-user-data');
    const extensions = join(workspace.root, 'vscode-extensions');
    const starter = join(workspace.root, 'vscode-starter');
    for (const folder of [project, home, join(userData, 'User'), extensions, starter]) {
      mkdirSync(folder, { recursive: true });
    }

    const planted = `baton-e2e-editor-${Math.random().toString(36).slice(2, 12)}`;
    forbidden.push(planted);
    const entry = copilotVscodeEntry(workspace);
    const env = { ...(entry['env'] as Record<string, string>), HANDOFF_E2E_PLANTED: planted };
    writeJson(join(userData, 'User', 'mcp.json'), { servers: { handoff: { ...entry, env } } });
    writeJson(join(userData, 'User', 'settings.json'), VSCODE_USER_SETTINGS);
    writeJson(join(starter, 'package.json'), STARTER_MANIFEST);
    writeFileSync(join(starter, 'extension.js'), STARTER_SCRIPT, 'utf8');

    say("launching a VS Code of the harness's own on a throw-away project");
    const launched = Date.now();
    const child = spawn(
      editor,
      [
        '--user-data-dir',
        userData,
        '--extensions-dir',
        extensions,
        `--extensionDevelopmentPath=${starter}`,
        '--disable-workspace-trust',
        '--new-window',
        project,
      ],
      {
        env: editorEnvironment(home),
        stdio: 'ignore',
        detached: process.platform !== 'win32',
        windowsHide: false,
      },
    );
    child.on('error', () => {
      // Reported below as a session that never registered.
    });

    try {
      const registered = await app
        .waitFor(
          'the server VS Code started registered',
          (seen) => seen.sessions.some((one) => one.agentId === COPILOT_AGENT_ID && one.connected),
          REGISTRATION_TIMEOUT_MS,
        )
        .catch(() => undefined);
      const session = registered?.sessions.find(
        (one) => one.agentId === COPILOT_AGENT_ID && one.connected,
      );
      facts['registered_after_ms'] = session === undefined ? null : Date.now() - launched;
      say(session === undefined ? 'no session registered' : `registered as ${session.label}`);

      let title = '';
      if (session !== undefined && child.pid !== undefined) {
        const deadline = Date.now() + TITLE_TIMEOUT_MS;
        while (Date.now() < deadline) {
          title = windowTitleOf(child.pid);
          if (titleNamesFolder(userTitle(title), EDITOR_PROJECT)) break;
          await sleep(1_000);
        }
      }
      facts['window_title'] = title;
      facts['session_identity'] = session?.sessionIdentity ?? null;
      facts['chain'] = session?.pidChain.map((ancestor) => ancestor.name) ?? [];

      return [
        check(
          'R-12',
          'a session registered from VS Code, as VS Code, under the Copilot row (SRV-20, ADPT-02)',
          'protocol',
          session !== undefined && session.clientName === COPILOT_VSCODE_CLIENT_NAME,
          `sessions: ${JSON.stringify(
            (registered?.sessions ?? []).map((one) => ({ agent: one.agentId, client: one.clientName })),
          )}`,
        ),
        check(
          'R-12',
          'it is keyed at the editor, with the launched VS Code in its chain (§5.6, T-072)',
          'protocol',
          session?.sessionIdentity === 'ancestor_chain:editor' &&
            session.pidChain.some((ancestor) => ancestor.pid === child.pid),
          `session_identity: ${JSON.stringify(session?.sessionIdentity ?? null)}; editor pid ${String(
            child.pid,
          )}; chain: ${JSON.stringify(session?.pidChain ?? [])}`,
        ),
        check(
          'OPEN-02',
          "the overlay names it GitHub Copilot and the window's folder, as the tab and the request sheet do",
          'protocol',
          session?.label === `GitHub Copilot · ${EDITOR_PROJECT}`,
          `label: ${JSON.stringify(session?.label ?? null)}`,
        ),
        check(
          'R-12',
          "its project is the window's folder, from the client's roots, while it runs in the home folder",
          'protocol',
          sameFolder(session?.projectDir, project) && sameFolder(session?.cwd, home),
          `project_dir: ${JSON.stringify(session?.projectDir ?? null)}; cwd: ${JSON.stringify(
            session?.cwd ?? null,
          )}`,
        ),
        check(
          'OPEN-05',
          "the window's title names that folder, which is how a request finds the window (§7.7)",
          'protocol',
          titleNamesFolder(userTitle(title), EDITOR_PROJECT),
          `title: ${JSON.stringify(title)}`,
        ),
      ] satisfies Assertion[];
    } finally {
      closeEditor(child.pid);
      // VS Code's processes can hold a file of the run's folder for a moment after they were
      // ended, and the cleanup that follows removes that folder.
      await sleep(1_500);
    }
  },
};
