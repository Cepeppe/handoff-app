/**
 * The two drivers the UI suite needs, and where they come from (T-055, §11.4).
 *
 * `tauri-driver` is Tauri's WebDriver server. On Windows it is a thin proxy: it starts the
 * application through `msedgedriver`, the WebDriver of Microsoft Edge, which attaches to the
 * WebView2 the application draws in. Two rules decide where each one comes from:
 *
 * - **`tauri-driver` is pinned.** It is a crate, installed with `cargo install --locked` into
 *   `tests/ui/.cache/` rather than into `~/.cargo/bin`, so the version the suite was written
 *   against is the one it runs with on every machine and nothing outside this repository
 *   changes. CI caches that folder.
 * - **`msedgedriver` follows the machine.** It has to be the build of the WebView2 runtime it
 *   drives — a driver of another major version refuses the session — and the runtime updates
 *   itself with Windows. So the version is read from the machine at run time, from the
 *   registry key the Evergreen runtime writes, and the matching driver is downloaded from
 *   Microsoft's own host into the same cache. A runner image and a developer machine each get
 *   the driver of the runtime they actually have.
 *
 * Neither is fetched behind the suite's back: `pnpm test:ui -- --setup` does it, and a run
 * that finds either missing says which command to type.
 */
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { REPO_ROOT } from '../e2e/paths.ts';

/** The `tauri-driver` release the suite is written against. */
export const TAURI_DRIVER_VERSION = '2.0.6';

/** Where both drivers are kept. Git-ignored. */
export const CACHE_DIR = join(REPO_ROOT, 'tests', 'ui', '.cache');

/** The client id under which the Evergreen WebView2 runtime records its version (`pv`). */
const WEBVIEW2_CLIENT = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

/** The three places that id is written: per machine (32- and 64-bit views), and per user. */
const WEBVIEW2_KEYS = [
  `HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2_CLIENT}`,
  `HKLM\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2_CLIENT}`,
  `HKCU\\Software\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2_CLIENT}`,
];

/** The drivers of one run, and the runtime they were matched to. */
export interface Tools {
  readonly tauriDriver: string;
  readonly edgeDriver: string;
  readonly webView2: string;
}

/** Where `cargo install --root` puts `tauri-driver`. */
function cachedTauriDriver(): string {
  return join(CACHE_DIR, 'tauri-driver', 'bin', 'tauri-driver.exe');
}

/** Where the driver for `version` is kept. */
function cachedEdgeDriver(version: string): string {
  return join(CACHE_DIR, `msedgedriver-${version}`, 'msedgedriver.exe');
}

/** `a.b.c.d` compared as numbers, so `152.0.10` sorts after `152.0.9`. */
function compareVersions(left: string, right: string): number {
  const a = left.split('.').map(Number);
  const b = right.split('.').map(Number);
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    const difference = (a[index] ?? 0) - (b[index] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

/**
 * The version of the WebView2 runtime this machine runs, or `null` when none is installed.
 *
 * The registry first, because it is what the runtime's own updater writes; the installation
 * folder second, for an image that installed the runtime without its updater.
 */
export function webView2Version(): string | null {
  for (const key of WEBVIEW2_KEYS) {
    const answer = spawnSync('reg', ['query', key, '/v', 'pv'], { encoding: 'utf8', windowsHide: true });
    const found = /\bpv\s+REG_SZ\s+(\d+(?:\.\d+){3})/u.exec(answer.stdout ?? '');
    if (found?.[1] !== undefined && found[1] !== '0.0.0.0') return found[1];
  }
  const folder = join(
    process.env['ProgramFiles(x86)'] ?? 'C:\\Program Files (x86)',
    'Microsoft',
    'EdgeWebView',
    'Application',
  );
  if (!existsSync(folder)) return null;
  const versions = readdirSync(folder).filter((name) => /^\d+(?:\.\d+){3}$/u.test(name));
  versions.sort(compareVersions);
  return versions[versions.length - 1] ?? null;
}

/** The drivers this run will use, or what is missing and how to get it. */
export function locateTools(): { tools: Tools | null; missing: string[] } {
  const missing: string[] = [];
  const tauriDriver = process.env['HANDOFF_UI_TAURI_DRIVER']?.trim() || cachedTauriDriver();
  if (!existsSync(tauriDriver)) {
    missing.push(`tauri-driver is missing (${tauriDriver}). Run: pnpm test:ui -- --setup`);
  }
  const webView2 = webView2Version();
  if (webView2 === null) {
    missing.push('no WebView2 runtime is installed on this machine, so there is nothing to drive.');
    return { tools: null, missing };
  }
  const edgeDriver = process.env['HANDOFF_UI_MSEDGEDRIVER']?.trim() || cachedEdgeDriver(webView2);
  if (!existsSync(edgeDriver)) {
    missing.push(
      `msedgedriver ${webView2}, the build of this machine's WebView2, is missing (${edgeDriver}). ` +
        'Run: pnpm test:ui -- --setup',
    );
  }
  return missing.length > 0 ? { tools: null, missing } : { tools: { tauriDriver, edgeDriver, webView2 }, missing };
}

/** `cargo install` of the pinned `tauri-driver`, into the cache. A no-op when it is there. */
function installTauriDriver(): void {
  process.stderr.write(`ui: installing tauri-driver ${TAURI_DRIVER_VERSION} into ${CACHE_DIR}\n`);
  const installed = spawnSync(
    'cargo',
    [
      'install',
      'tauri-driver',
      '--locked',
      '--version',
      TAURI_DRIVER_VERSION,
      '--root',
      join(CACHE_DIR, 'tauri-driver'),
    ],
    { stdio: 'inherit', windowsHide: true },
  );
  if (installed.status !== 0 || !existsSync(cachedTauriDriver())) {
    throw new Error(`cargo install tauri-driver ${TAURI_DRIVER_VERSION} failed (exit ${String(installed.status)})`);
  }
}

/** Downloads and unpacks the `msedgedriver` of `version`, unless it is already cached. */
async function downloadEdgeDriver(version: string): Promise<void> {
  const target = cachedEdgeDriver(version);
  if (existsSync(target)) return;
  const folder = join(CACHE_DIR, `msedgedriver-${version}`);
  mkdirSync(folder, { recursive: true });
  const flavour = process.arch === 'arm64' ? 'arm64' : 'win64';
  const url = `https://msedgedriver.microsoft.com/${version}/edgedriver_${flavour}.zip`;
  process.stderr.write(`ui: downloading ${url}\n`);
  const response = await fetch(url, { signal: AbortSignal.timeout(120_000) });
  if (!response.ok) {
    throw new Error(`${url} answered ${String(response.status)}`);
  }
  const zip = join(folder, `edgedriver_${flavour}.zip`);
  writeFileSync(zip, Buffer.from(await response.arrayBuffer()));
  // Windows PowerShell rather than `tar`: under Git Bash `tar` is GNU tar, which reads no zip.
  const expanded = spawnSync(
    'powershell.exe',
    [
      '-NoProfile',
      '-NonInteractive',
      '-Command',
      `Expand-Archive -LiteralPath '${zip}' -DestinationPath '${folder}' -Force`,
    ],
    { encoding: 'utf8', windowsHide: true },
  );
  rmSync(zip, { force: true });
  if (expanded.status !== 0 || !existsSync(target)) {
    throw new Error(`the driver archive would not unpack: ${expanded.stderr ?? ''}`);
  }
}

/** `--setup`: both drivers, for this machine. */
export async function setup(): Promise<Tools> {
  installTauriDriver();
  const version = webView2Version();
  if (version === null) {
    throw new Error('no WebView2 runtime is installed on this machine, so there is nothing to drive.');
  }
  await downloadEdgeDriver(version);
  const { tools, missing } = locateTools();
  if (tools === null) throw new Error(missing.join('\n'));
  return tools;
}
