/**
 * The overlay under test: one isolated instance per scenario (T-043, §11.5).
 *
 * Every run gets its own temporary root with its own `HANDOFF_HOME` and
 * `HANDOFF_APP_DATA_DIR`, which is what keeps it off the installation the owner uses every
 * day: the pipe name is a digest of the `HANDOFF_HOME` **string** (implementation decision 4),
 * the database and the settings follow `HANDOFF_APP_DATA_DIR`, and the runbooks and the
 * token live under the home. Nothing is redirected that does not have to be — in particular
 * **not `HOME` or `USERPROFILE`**: the agent's credentials are there (the T-039 handoff
 * entry), and this suite never runs the installation adapter, so there is nothing of ours
 * that would be written into a real profile.
 *
 * The app is launched with `--hidden`, the argument the login entry carries: it leaves the
 * panel closed (T-041's deviation) and an unattended suite has nobody to show it to.
 */
import { spawn, type ChildProcess, type SpawnOptions } from 'node:child_process';
import { createWriteStream, mkdirSync, mkdtempSync, rmSync, type WriteStream } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { Automation, sleep } from './automation.ts';
import { APP_BINARY } from './paths.ts';

/** The argument `tauri-plugin-autostart` writes into the login entry (`general::HIDDEN_ARG`). */
export const HIDDEN_ARG = '--hidden';

/** How long the app has to bind its endpoints before a scenario gives up on it. */
export const START_TIMEOUT_MS = 60_000;

/** How long the process has to leave after `quit` before it is killed. */
export const QUIT_TIMEOUT_MS = 15_000;

/** The isolated folders of one run. */
export interface Workspace {
  /** The temporary root; everything else is under it. */
  readonly root: string;
  /** `HANDOFF_HOME`: the token, the endpoints, the runbooks. */
  readonly home: string;
  /** `HANDOFF_APP_DATA_DIR`: the database, the settings, `crashes/`. */
  readonly appData: string;
  /** The project folder the agent is run in. */
  readonly project: string;
}

/** A running app, its automation channel, and its log. */
export interface RunningApp {
  readonly workspace: Workspace;
  readonly automation: Automation;
  /** Everything the app wrote on stdout and stderr, so far. */
  log(): string;
  /** Asks it to leave, waits, and kills it if it does not. */
  stop(): Promise<void>;
}

/** A temporary root with the four folders a scenario needs. */
export function makeWorkspace(scenario: string): Workspace {
  const root = mkdtempSync(join(tmpdir(), `baton-e2e-${scenario}-`));
  const workspace: Workspace = {
    root,
    home: join(root, 'home'),
    appData: join(root, 'appdata'),
    project: join(root, 'project'),
  };
  for (const folder of [workspace.home, workspace.appData, workspace.project]) {
    mkdirSync(folder, { recursive: true });
  }
  return workspace;
}

/** Removes a workspace, unless `HANDOFF_E2E_KEEP=1` says to leave it for reading. */
export function cleanUp(workspace: Workspace): void {
  if (process.env['HANDOFF_E2E_KEEP'] === '1') return;
  rmSync(workspace.root, { recursive: true, force: true });
}

/** The environment the app is given: the two redirections, and the log level. */
export function appEnvironment(workspace: Workspace): Record<string, string> {
  const child: Record<string, string> = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (value !== undefined) child[name] = value;
  }
  child['HANDOFF_HOME'] = workspace.home;
  child['HANDOFF_APP_DATA_DIR'] = workspace.appData;
  child['RUST_LOG'] = process.env['HANDOFF_E2E_RUST_LOG'] ?? 'handoff_app_lib=debug';
  return child;
}

/**
 * Spawns the app, retrying once when Smart App Control refuses the binary.
 *
 * On this machine SAC blocks a freshly built executable the first time it is started and
 * lets it through afterwards, at random. From Node the refusal is not
 * `os error 4551` but `spawn UNKNOWN` with `errno -4094`, which says nothing at all, and an
 * unattended suite that treated it as a real failure would go red on the first run after
 * every build. One retry is the documented remedy and it is the whole of this function.
 */
async function spawnApp(workspace: Workspace): Promise<ChildProcess> {
  const options: SpawnOptions = {
    cwd: workspace.root,
    env: appEnvironment(workspace),
    windowsHide: true,
    stdio: ['ignore', 'pipe', 'pipe'],
  };
  for (let attempt = 1; ; attempt += 1) {
    const child = spawn(APP_BINARY, [HIDDEN_ARG], options);
    const started = await new Promise<Error | undefined>((resolve) => {
      child.once('spawn', () => resolve(undefined));
      child.once('error', (cause) => resolve(cause));
    });
    if (started === undefined) return child;
    const blocked = (started as NodeJS.ErrnoException).code === 'UNKNOWN';
    if (!blocked || attempt >= 3) throw started;
    process.stderr.write(
      `             · the app would not start (${started.message}); ` +
        'retrying — Smart App Control blocks a freshly built binary at random\n',
    );
    await sleep(1000);
  }
}

/**
 * Starts the app and waits until its automation channel answers.
 *
 * The wait is on the channel and not on the process: a Tauri application that is "running"
 * has not necessarily bound anything yet, and every scenario's first act is a `state`.
 */
export async function startApp(workspace: Workspace): Promise<RunningApp> {
  const logFile = join(workspace.root, 'app.log');
  const sink: WriteStream = createWriteStream(logFile, { flags: 'a' });
  let text = '';

  const child: ChildProcess = await spawnApp(workspace);
  const collect = (chunk: Buffer): void => {
    const piece = chunk.toString('utf8');
    text += piece;
    sink.write(piece);
  };
  child.stdout?.on('data', collect);
  child.stderr?.on('data', collect);

  let exited: number | null | undefined;
  child.on('exit', (code) => (exited = code));

  let automation: Automation;
  try {
    automation = await Automation.open(workspace.home, START_TIMEOUT_MS);
  } catch (cause) {
    child.kill();
    const why = exited === undefined ? '' : ` The app exited with ${String(exited)}.`;
    throw new Error(
      `${cause instanceof Error ? cause.message : String(cause)}.${why}\n` +
        `--- ${logFile} ---\n${text.slice(-4000)}`,
    );
  }

  return {
    workspace,
    automation,
    log: () => text,
    async stop() {
      if (exited !== undefined) return;
      await automation.quit();
      const deadline = Date.now() + QUIT_TIMEOUT_MS;
      while (exited === undefined && Date.now() < deadline) await sleep(100);
      if (exited === undefined) {
        // A kill leaves a `sessions` row flagged connected, which is exactly the state
        // `Registry::open` repairs at the next start — so it costs the scenario nothing, but
        // it is a fact worth having in the log rather than a silent difference.
        text += '\n[e2e] the app did not leave on quit and was killed\n';
        child.kill();
      }
      sink.end();
    },
  };
}
