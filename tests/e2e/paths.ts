/**
 * Where the e2e suite finds the things it drives (T-043, TECHNICAL-DESIGN §11.5).
 *
 * Three of them, and each one is a decision rather than a lookup:
 *
 * - **The app** is the `--features e2e` build of *this* checkout, in the **release**
 *   profile. Release and not debug for one concrete reason: `tauri::generate_context!`
 *   embeds `devUrl` in a debug build, so a debug binary looks for a Vite dev server on port
 *   1420 and comes up with an empty webview when there is none (T-040's handoff entry). A
 *   release binary loads `dist/`, which is what ships and what a scenario should be
 *   driving.
 * - **The server** is the pinned release artifact under `src-tauri/binaries/`, fetched by
 *   `scripts/fetch-server.mjs` — the same binary the installer registers (§3.5). Not a
 *   local build of `handoff-mcp`: `scripts/dev-link` in the workspace root is how a change
 *   under test in the server gets here, and it writes into the same place.
 * - **The runs** go under a temporary directory that holds the isolated `HANDOFF_HOME` and
 *   `HANDOFF_APP_DATA_DIR` of each scenario, so nothing touches the installation the owner
 *   uses every day (`TASKS.md` §0.4 item 4).
 */
import { existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The repository root, from this file. */
export const REPO_ROOT = fileURLToPath(new URL('../../', import.meta.url));

/** Where the `--features e2e` release binary lands. */
export const APP_BINARY = join(
  REPO_ROOT,
  'src-tauri',
  'target',
  'release',
  process.platform === 'win32' ? 'handoff-app.exe' : 'handoff-app',
);

/** Where `scripts/fetch-server.mjs` puts the pinned server. */
export const BINARIES_DIR = join(REPO_ROOT, 'src-tauri', 'binaries');

/**
 * The pinned server executable, whatever target triple it was fetched for.
 *
 * The name carries the triple (`handoff-mcp-x86_64-pc-windows-msvc.exe`) because Tauri
 * appends it to the `externalBin` declaration, so the file is found by shape rather than by
 * a name this file would have to keep in step with `server.lock.json`.
 */
export function serverBinary(): string {
  const override = process.env['HANDOFF_E2E_SERVER']?.trim();
  if (override !== undefined && override !== '') return override;
  if (!existsSync(BINARIES_DIR)) return join(BINARIES_DIR, 'handoff-mcp');
  const found = readdirSync(BINARIES_DIR)
    .filter((name) => name.startsWith('handoff-mcp'))
    .sort();
  const first = found[0];
  return first === undefined ? join(BINARIES_DIR, 'handoff-mcp') : join(BINARIES_DIR, first);
}

/** What is missing before a run can start, in the order it is worth fixing. */
export function missingPrerequisites(): string[] {
  const missing: string[] = [];
  if (!existsSync(APP_BINARY)) {
    missing.push(
      `${APP_BINARY} is missing. Build it: pnpm build && cargo build --release --features e2e --bin handoff-app (in src-tauri), ` +
        'or run scripts/e2e.ps1 from the workspace root.',
    );
  }
  if (!existsSync(serverBinary())) {
    missing.push(
      `${serverBinary()} is missing. Fetch the pinned server: node scripts/fetch-server.mjs.`,
    );
  }
  return missing;
}
