/**
 * A Cursor session shows in the overlay with the right project (T-070's acceptance; §5.6, §7.5,
 * §7.7, §14 R-12, OPEN-02): the editor half of the Cursor subset.
 *
 * Cursor's editor cannot be driven from a script, but the moment this scenario is about can: the
 * editor starts every server of `~/.cursor/mcp.json` as a window opens, before any chat. So the
 * harness launches an editor of its own — a fresh user-data and extensions folder, and
 * `USERPROFILE`/`HOME` pointed at a folder of the run whose `.cursor/mcp.json` holds the entry the
 * installer writes, with the run's `HANDOFF_HOME` beside it — on a project folder with a name of
 * its own, and reads what registered. It spends no agent request and never touches the user's
 * own Cursor: a separate user-data folder is a separate instance, closed with everything it
 * started when the scenario ends. It does open a window for the seconds it takes. The recipe is
 * the Cursor canary's (`handoff-mcp/test/canary/agents/cursor/editor-runner.ts`, T-069), copied
 * rather than imported (§3.1 rule 3).
 *
 * What it proves, from the app's side of the channel: the session is Cursor's editor
 * (`cursor-vscode`), keyed at the editor (`ancestor_chain:editor`) with the launched editor in its
 * chain, and named after its workspace folder — which is what the tab, the request sheet and the
 * FM-22 picker print — while its working directory is the home folder the editor starts its
 * servers in. And that the editor's window title names that folder, which is what
 * `requests::focus` reads to bring the right editor window in front of a request.
 *
 * The log-invariant check that follows every scenario needs something planted: here it is a value
 * in the server's environment, which nothing on the channel may carry and nothing may write down.
 */
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { sleep } from '../automation.ts';
import { check, type Assertion } from '../classify.ts';
import { CURSOR_AGENT_ID, cursorEntry } from '../cursor.ts';
import type { Scenario } from '../scenario.ts';

/** The folder the editor opens: a name of its own, so that a title naming it names nothing else. */
export const EDITOR_PROJECT = 'baton-cursor-window';

/** The `clientInfo.name` of Cursor's editor (T-069). */
export const EDITOR_CLIENT_NAME = 'cursor-vscode';

/** How long the editor has to start our server and let it register (T-069 measured about ten seconds). */
export const REGISTRATION_TIMEOUT_MS = 90_000;

/** How long the window has to put its folder in its title once the session is there. */
export const TITLE_TIMEOUT_MS = 30_000;

/** Where Cursor's editor is installed; `HANDOFF_E2E_CURSOR_EDITOR` overrides it. */
export function cursorEditorPath(
  env: Readonly<Record<string, string | undefined>> = process.env,
  platform: NodeJS.Platform = process.platform,
): string {
  const override = env['HANDOFF_E2E_CURSOR_EDITOR']?.trim();
  if (override !== undefined && override !== '') return override;
  if (platform === 'win32') return join(env['LOCALAPPDATA'] ?? '', 'Programs', 'cursor', 'Cursor.exe');
  if (platform === 'darwin') return '/Applications/Cursor.app/Contents/MacOS/Cursor';
  return 'cursor';
}

/**
 * The editor's environment: the parent's, with the home folder moved to the run's, which is where
 * the editor looks for `~/.cursor/mcp.json`. Whatever an editor or a Claude Code session running
 * the harness left in the environment is dropped: `ELECTRON_RUN_AS_NODE` would start the editor as
 * a bare Node, and a `VSCODE_*` of another editor is not this one's to inherit.
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

/** Ends the editor it launched and everything the editor started, our server among them. */
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

export const cursorEditorScenario: Scenario = {
  id: 'cursor-editor-session',
  covers: 'R-12',
  title: "a session Cursor's editor starts shows under its window's folder, keyed at the editor",
  timeoutMs: 180_000,

  async run({ workspace, app, forbidden, facts, say }) {
    const editor = cursorEditorPath();
    if (process.platform !== 'linux' && !existsSync(editor)) {
      return [
        check(
          'R-12',
          "Cursor's editor is installed where the harness looks",
          'protocol',
          false,
          `${editor} does not exist; set HANDOFF_E2E_CURSOR_EDITOR`,
        ),
      ];
    }

    const project = join(workspace.root, EDITOR_PROJECT);
    const home = join(workspace.root, 'cursor-home');
    const userData = join(workspace.root, 'cursor-user-data');
    const extensions = join(workspace.root, 'cursor-extensions');
    for (const folder of [project, join(home, '.cursor'), userData, extensions]) {
      mkdirSync(folder, { recursive: true });
    }

    const planted = `baton-e2e-editor-${Math.random().toString(36).slice(2, 12)}`;
    forbidden.push(planted);
    const entry = cursorEntry(workspace);
    const env = { ...(entry['env'] as Record<string, string>), HANDOFF_E2E_PLANTED: planted };
    writeFileSync(
      join(home, '.cursor', 'mcp.json'),
      `${JSON.stringify({ mcpServers: { handoff: { ...entry, env } } }, null, 2)}\n`,
      'utf8',
    );

    say("launching an editor of the harness's own on a throw-away project");
    const launched = Date.now();
    const child = spawn(
      editor,
      ['--user-data-dir', userData, '--extensions-dir', extensions, '--new-window', project],
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
          "the server Cursor's editor started registered",
          (seen) => seen.sessions.some((one) => one.agentId === CURSOR_AGENT_ID && one.connected),
          REGISTRATION_TIMEOUT_MS,
        )
        .catch(() => undefined);
      const session = registered?.sessions.find(
        (one) => one.agentId === CURSOR_AGENT_ID && one.connected,
      );
      facts['registered_after_ms'] = session === undefined ? null : Date.now() - launched;
      say(session === undefined ? 'no session registered' : `registered as ${session.label}`);

      let title = '';
      if (session !== undefined && child.pid !== undefined) {
        const deadline = Date.now() + TITLE_TIMEOUT_MS;
        while (Date.now() < deadline) {
          title = windowTitleOf(child.pid);
          if (titleNamesFolder(title, EDITOR_PROJECT)) break;
          await sleep(1_000);
        }
      }
      facts['window_title'] = title;
      facts['session_identity'] = session?.sessionIdentity ?? null;
      facts['chain'] = session?.pidChain.map((ancestor) => ancestor.name) ?? [];

      return [
        check(
          'R-12',
          "a session registered from Cursor's editor, as Cursor's editor (SRV-20, ADPT-02)",
          'protocol',
          session !== undefined && session.clientName === EDITOR_CLIENT_NAME,
          `sessions: ${JSON.stringify(
            (registered?.sessions ?? []).map((one) => ({ agent: one.agentId, client: one.clientName })),
          )}`,
        ),
        check(
          'R-12',
          'it is keyed at the editor, with the launched editor in its chain (§5.6, T-069)',
          'protocol',
          session?.sessionIdentity === 'ancestor_chain:editor' &&
            session.pidChain.some((ancestor) => ancestor.pid === child.pid),
          `session_identity: ${JSON.stringify(session?.sessionIdentity ?? null)}; editor pid ${String(
            child.pid,
          )}; chain: ${JSON.stringify(session?.pidChain ?? [])}`,
        ),
        check(
          'OPEN-02',
          "the overlay names it Cursor and the window's folder, as the tab and the request sheet do",
          'protocol',
          session?.label === `Cursor · ${EDITOR_PROJECT}`,
          `label: ${JSON.stringify(session?.label ?? null)}`,
        ),
        check(
          'R-12',
          'its project is the workspace folder, while it runs in the home folder the editor gave it',
          'protocol',
          sameFolder(session?.projectDir, project) && sameFolder(session?.cwd, home),
          `project_dir: ${JSON.stringify(session?.projectDir ?? null)}; cwd: ${JSON.stringify(
            session?.cwd ?? null,
          )}`,
        ),
        check(
          'OPEN-05',
          "the editor's window title names that folder, which is how a request finds the window (§7.7)",
          'protocol',
          titleNamesFolder(title, EDITOR_PROJECT),
          `title: ${JSON.stringify(title)}`,
        ),
      ] satisfies Assertion[];
    } finally {
      closeEditor(child.pid);
      // The editor's processes can hold a file of the run's folder for a moment after they
      // were ended, and the cleanup that follows removes that folder.
      await sleep(1_500);
    }
  },
};
