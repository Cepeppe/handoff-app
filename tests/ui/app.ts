/**
 * One isolated Baton, started by WebDriver (T-055, §11.4).
 *
 * The launch is not ours: `tauri-driver` starts `msedgedriver`, and `msedgedriver` starts the
 * application and attaches to its WebView2. So everything the application must be given has
 * to be in the environment of `tauri-driver`, which is why one is started per attempt, with
 * the redirections of that attempt, and stopped with it:
 *
 * | Variable | What it moves |
 * |---|---|
 * | `HANDOFF_HOME` | `~/.handoff`: the token, the channel pipe (its name is a digest of this string), the runbooks |
 * | `HANDOFF_APP_DATA_DIR` | the database and the settings |
 * | `PATH` | gains the attempt's own `bin/`, first, where a scenario can put a stand-in `claude` |
 * | `TEMP`, `TMP` | the attempt's own `tmp/`, where `msedgedriver` creates the session's WebView2 profile |
 *
 * The last row is how the driver and the runtime find each other. `msedgedriver` hands the
 * WebView2 a profile folder of its own making under `TEMP` (`scoped_dir…\EBWebView`) with
 * `--remote-debugging-port=0`, and waits for the runtime to write the port it chose into a
 * `DevToolsActivePort` file there. Inside the attempt's root that folder is a known, writable,
 * long path, it goes when the attempt goes, and when a session will not start it is the
 * first place to look — `snapshot` below reads it.
 *
 * `USERPROFILE` is deliberately **not** redirected, although it is where the installation
 * adapter finds `~/.claude.json`. WebView2 and the Windows components it loads follow it too,
 * and in a fresh profile folder `msedgedriver` never finds the WebView2 it started: every
 * session failed after sixty seconds with "DevToolsActivePort file doesn't exist", measured
 * against the same build with and without the redirection. No scenario writes to the
 * adapter's files — the one that reaches the consent screen never presses Accept, and checks
 * that the machine's own files are what they were. `WEBVIEW2_USER_DATA_FOLDER` is not set
 * either: the driver's own profile folder wins over it.
 *
 * Two settings are written through the automation channel before a scenario begins, and the
 * page is reloaded so the window reads them the way it reads them at every launch. The
 * **language** is English whatever the machine's is, so a scenario reads the words a person
 * would read on the runner and on a developer's Italian desktop alike; and **onboarding** is
 * marked as done unless the scenario is about onboarding. That second one is what the
 * onboarding's own last button writes — pressing the button instead would also answer its
 * autostart question, and that writes the real login items of the machine, which no
 * environment variable redirects.
 */
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { Automation } from '../e2e/automation.ts';
import { REPO_ROOT } from '../e2e/paths.ts';
import type { Tools } from './tools.ts';
import { until } from './wait.ts';
import { Session } from './webdriver.ts';

/** The build the suite drives, unless `HANDOFF_UI_APP` names another. */
export const APP_BINARY =
  process.env['HANDOFF_UI_APP']?.trim() ||
  join(REPO_ROOT, 'src-tauri', 'target', 'debug', 'handoff-app.exe');

/**
 * How that build is made.
 *
 * `tauri build` and not `cargo build`, because Tauri chooses between the Vite dev server and
 * the embedded `dist/` by a feature that only `tauri build` turns on: a
 * plain `cargo build` produces a window that looks for `localhost:1420` and draws an error
 * page. `--debug` because it compiles in a fraction of the time and drives the same window;
 * `--features e2e` for the automation channel the setup below writes the settings through,
 * and for the fixture capture backend the preview scenarios take their screenshot from.
 */
export const BUILD_COMMAND = 'pnpm tauri build --debug --no-bundle --features e2e';

/** What an error says when the application could not be started at all. */
export const START_FAILURE = 'the application could not be started under WebDriver';

/** The two settings keys the setup writes (`ui_bridge::general`, `ui_bridge::install`). */
const LANGUAGE_KEY = 'language';
const ONBOARDED_KEY = 'onboarded';

/** The executables a running Baton can be: a development build and an installed one. */
const BATON_IMAGES = ['handoff-app.exe', 'Baton.exe'];

/**
 * How long a session may take to start before the machine is photographed.
 *
 * A healthy start is a second or two on a warm machine and about ten on a cold one;
 * `msedgedriver` itself gives up at sixty.
 */
const SLOW_SESSION_MS = 20_000;

/**
 * A terminal colour code, which the log carries when the subscriber decides it has a terminal.
 *
 * Built from the escape character's code point rather than written as a literal: a literal
 * escape character is invisible in the source.
 */
const COLOUR_CODE = new RegExp(`${String.fromCharCode(27)}\\[[0-9;]*m`, 'gu');

/** The isolated folders of one attempt. */
export interface UiWorkspace {
  /** The temporary root; everything else is under it. */
  readonly root: string;
  /** `HANDOFF_HOME`. */
  readonly home: string;
  /** `HANDOFF_APP_DATA_DIR`. */
  readonly appData: string;
  /** A folder put first on the application's `PATH`, empty unless a scenario fills it. */
  readonly bin: string;
  /** `TEMP` and `TMP` of the drivers and the application. */
  readonly temp: string;
}

/** A running application, its WebDriver session and its automation channel. */
export interface RunningApp {
  readonly workspace: UiWorkspace;
  readonly session: Session;
  readonly automation: Automation;
  /** Where the product channel listens, as the application itself logged it. */
  readonly channelEndpoint: string;
  /** Everything the drivers and the application printed, so far. */
  log(): string;
  /** Asks the application to leave, ends the session, and stops the drivers. */
  stop(): Promise<void>;
}

/** A temporary root with the folders one attempt needs. */
export function makeWorkspace(id: string): UiWorkspace {
  const root = mkdtempSync(join(tmpdir(), `baton-ui-${id}-`));
  const workspace: UiWorkspace = {
    root,
    home: join(root, 'home'),
    appData: join(root, 'appdata'),
    bin: join(root, 'bin'),
    temp: join(root, 'tmp'),
  };
  for (const folder of [workspace.home, workspace.appData, workspace.bin, workspace.temp]) {
    mkdirSync(folder, { recursive: true });
  }
  return workspace;
}

/** Removes a workspace, unless `HANDOFF_UI_KEEP=1` asks to leave it for reading. */
export function cleanUp(workspace: UiWorkspace): void {
  if (process.env['HANDOFF_UI_KEEP'] === '1') return;
  try {
    // The WebView2 processes let go of their profile a moment after the application leaves.
    rmSync(workspace.root, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  } catch {
    // A temporary folder left behind is not a failed scenario.
  }
}

/**
 * Whether a Baton is running on this machine.
 *
 * It matters more than it looks: the single-instance plugin is keyed on the bundle
 * identifier, not on `HANDOFF_HOME`, so a launch while another Baton runs hands itself over to
 * that one and exits — and the session would be driving nothing, or the owner's own window.
 */
export function batonIsRunning(): boolean {
  return BATON_IMAGES.some((image) => {
    // `windowsHide` on every child the harness starts: `scenario.ts` (`clipboard`) says why.
    const listed = spawnSync('tasklist', ['/FI', `IMAGENAME eq ${image}`, '/NH', '/FO', 'CSV'], {
      encoding: 'utf8',
      windowsHide: true,
    });
    return (listed.stdout ?? '').toLowerCase().includes(`"${image.toLowerCase()}"`);
  });
}

/** The environment of `tauri-driver`, and so of `msedgedriver` and of the application. */
function environment(workspace: UiWorkspace): Record<string, string> {
  const child: Record<string, string> = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (value !== undefined) child[name] = value;
  }
  child['HANDOFF_HOME'] = workspace.home;
  child['HANDOFF_APP_DATA_DIR'] = workspace.appData;
  // Windows spells it `Path`, and Node keeps the spelling it found; one entry, whatever it is.
  const pathKey = Object.keys(child).find((name) => name.toUpperCase() === 'PATH') ?? 'Path';
  child[pathKey] = `${workspace.bin};${child[pathKey] ?? ''}`;
  child['TEMP'] = workspace.temp;
  child['TMP'] = workspace.temp;
  child['RUST_LOG'] = process.env['HANDOFF_UI_RUST_LOG'] ?? 'handoff_app_lib=debug';
  child['NO_COLOR'] = '1';
  return child;
}

/** A port nothing listens on, from the operating system. */
function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      const port = typeof address === 'object' && address !== null ? address.port : 0;
      server.close(() => resolve(port));
    });
  });
}

/** Stops `tauri-driver` and everything it started: `msedgedriver`, the application, WebView2. */
function killTree(driver: ChildProcess): void {
  if (driver.pid === undefined || driver.exitCode !== null) return;
  spawnSync('taskkill', ['/PID', String(driver.pid), '/T', '/F'], { stdio: 'ignore', windowsHide: true });
}

/** The log without the colour codes a terminal would have drawn. */
function plain(text: string): string {
  return text.replace(COLOUR_CODE, '');
}

/**
 * What the machine looks like while a session is slow to start.
 *
 * The four things that decide whether `msedgedriver` can reach the WebView2 it launched, read
 * while it is still waiting: the processes of the drivers, of the application and of its
 * WebView2 browser, with their command lines (is `--remote-debugging-port` there, and which
 * profile folder); whether that profile has its `DevToolsActivePort` yet; the tail of the
 * runtime's own `chrome_debug.log`, which the driver's `--enable-logging` writes beside it;
 * and any WebView2 or Edge policy of the machine, which can switch remote debugging off.
 * Only the application's own WebView2 is listed: every other program's is none of ours.
 *
 * Exported so it can be run by hand against a live attempt (`HANDOFF_UI_KEEP=1` keeps the
 * root): a healthy session starts long before it is called, so a suite that passes never
 * exercises it.
 */
export function snapshot(workspace: Pick<UiWorkspace, 'temp'>): string {
  const temp = workspace.temp.replace(/'/gu, "''");
  const script = [
    "$apps = @(Get-CimInstance Win32_Process -Filter \"Name='handoff-app.exe'\" | ForEach-Object { $_.ProcessId })",
    "Get-CimInstance Win32_Process | Where-Object { @('tauri-driver.exe','msedgedriver.exe','handoff-app.exe') -contains $_.Name -or ($_.Name -eq 'msedgewebview2.exe' -and $apps -contains $_.ParentProcessId) } | ForEach-Object { 'process {0} pid={1} parent={2}: {3}' -f $_.Name, $_.ProcessId, $_.ParentProcessId, $_.CommandLine }",
    `Get-ChildItem -LiteralPath '${temp}' -Recurse -Depth 4 -Force -ErrorAction SilentlyContinue | Where-Object { @('DevToolsActivePort','chrome_debug.log') -contains $_.Name } | ForEach-Object { 'file ' + $_.FullName + ' (' + $_.Length + ' bytes)' }`,
    `Get-ChildItem -LiteralPath '${temp}' -Recurse -Depth 4 -Force -Filter chrome_debug.log -ErrorAction SilentlyContinue | Select-Object -First 1 | ForEach-Object { Get-Content -LiteralPath $_.FullName -Tail 30 }`,
    "foreach ($key in @('HKLM:\\SOFTWARE\\Policies\\Microsoft\\Edge\\WebView2','HKLM:\\SOFTWARE\\Policies\\Microsoft\\Edge','HKCU:\\SOFTWARE\\Policies\\Microsoft\\Edge')) { if (Test-Path $key) { 'policy ' + $key; Get-ItemProperty $key | Out-String } }",
  ].join('; ');
  const answer = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
    encoding: 'utf8',
    windowsHide: true,
  });
  return `${answer.stdout ?? ''}${answer.stderr ?? ''}`.trim();
}

/**
 * The product channel's endpoint, from the line the application logs when it binds.
 *
 * Read and never derived: the Windows pipe name is a digest of the user and of `HANDOFF_HOME`
 * (§5.8, implementation decision 4), and computing it a third time here would be one more
 * implementation of the rule to get wrong (`docs/dev/smoke.md` says the same of its script).
 */
function channelEndpointIn(text: string): string | null {
  const found = /the channel is listening\s+endpoint=(\S+)/u.exec(plain(text));
  return found?.[1] ?? null;
}

/** An error carrying what the drivers and the application printed. */
function withLog(what: string, cause: unknown, text: string): Error {
  const why = cause instanceof Error ? `: ${cause.message}` : '';
  return new Error(`${what}${why}\n--- drivers and application ---\n${plain(text).slice(-20_000)}`);
}

/**
 * Starts the application under WebDriver and prepares it for a scenario.
 *
 * Answers once the window has mounted again after the setup, so the first thing a scenario
 * reads is a window in English that has either shown onboarding or not, as asked.
 */
export async function startApp(
  workspace: UiWorkspace,
  tools: Tools,
  options: { onboarded: boolean },
): Promise<RunningApp> {
  const port = await freePort();
  const nativePort = await freePort();
  const driver = spawn(
    tools.tauriDriver,
    ['--port', String(port), '--native-port', String(nativePort), '--native-driver', tools.edgeDriver],
    { cwd: workspace.root, env: environment(workspace), stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true },
  );
  let text = '';
  const collect = (chunk: Buffer): void => {
    text += chunk.toString('utf8');
  };
  driver.stdout?.on('data', collect);
  driver.stderr?.on('data', collect);
  let failedToStart: Error | undefined;
  driver.on('error', (cause) => {
    failedToStart = cause;
  });

  const base = `http://127.0.0.1:${String(port)}`;
  let session: Session | undefined;
  try {
    await until(
      'tauri-driver answers',
      async () => {
        if (failedToStart !== undefined) throw failedToStart;
        return (await fetch(`${base}/status`)).ok;
      },
      20_000,
    );

    // Smart App Control refuses a freshly linked executable at random on the development
    // machine and lets the same bytes through a moment later; through
    // the drivers it arrives as a browser that "failed to start". One more try is the remedy
    // the e2e harness uses for the same reason. Not more: a session that fails twice fails
    // for a reason a third try will not change, and each try can cost the driver's sixty
    // seconds.
    for (let attempt = 1; session === undefined; attempt += 1) {
      const slow = setTimeout(() => {
        text +=
          `\n[ui] no session after ${String(SLOW_SESSION_MS / 1000)} s; the machine meanwhile:\n` +
          `${snapshot(workspace)}\n`;
      }, SLOW_SESSION_MS);
      try {
        session = await Session.create(base, {
          browserName: 'wry',
          'tauri:options': { application: APP_BINARY },
        });
      } catch (cause) {
        if (attempt >= 2) throw cause;
        text += `\n[ui] the session did not start (${cause instanceof Error ? cause.message : String(cause)}); retrying\n`;
      } finally {
        clearTimeout(slow);
      }
    }

    const ready = session;
    // The driver attaches to the webview as soon as it exists, which can be before Tauri has
    // navigated it to the application's own page: the first address is then `about:blank`,
    // and a check made at once reads that. Measured: one attempt in about fifteen.
    const address = await until(
      'the window navigated to its page',
      async () => {
        const now = await ready.url();
        return now === '' || now === 'about:blank' ? null : now;
      },
      20_000,
    );
    if (!address.startsWith('http://tauri.localhost')) {
      throw new Error(
        `the window loaded ${address} instead of the frontend built into the binary. ` +
          `Build the app the suite drives with: ${BUILD_COMMAND}`,
      );
    }

    const automation = await Automation.open(workspace.home, 30_000);
    const channelEndpoint = await until(
      'the application logs where its channel listens',
      () => channelEndpointIn(text),
      30_000,
    );

    await automation.setSetting(LANGUAGE_KEY, 'en');
    if (options.onboarded) {
      await automation.setSetting(ONBOARDED_KEY, true);
    }
    await ready.refresh();
    await until(
      'the window mounted with the settings of the attempt',
      () => ready.execute<boolean>('return document.querySelector("[data-view]") !== null;'),
      20_000,
    );

    return {
      workspace,
      session: ready,
      automation,
      channelEndpoint,
      log: () => plain(text),
      async stop() {
        await automation.quit();
        await ready.delete().catch(() => undefined);
        killTree(driver);
        await until('the application left', () => !batonIsRunning(), 20_000).catch(() => undefined);
      },
    };
  } catch (cause) {
    await session?.delete().catch(() => undefined);
    killTree(driver);
    throw withLog(START_FAILURE, cause, text);
  }
}
