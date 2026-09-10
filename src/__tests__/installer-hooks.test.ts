// @vitest-environment node
/**
 * The Windows installer's own code (T-054: FM-24, SRV-25, TECHNICAL-DESIGN §3.5).
 *
 * Tauri generates the installer and runs `installer/windows/hooks.nsh` inside it, so the only
 * way to know what the hooks do is to compile them with the real `makensis` and run them.
 * `installer/windows/test/hooks-test.nsi` is the smallest installer that declares the names
 * Tauri's template gives the hooks and inserts one of them per run. This suite builds it once
 * and runs it against folders of its own, with the registry keys and the data folder the
 * hooks touch pointed at a throwaway name, so nothing of a real installation is ever in reach.
 *
 * The one thing a file on disk cannot stand in for is an agent's session. An executable that
 * is **running** can be renamed and cannot be deleted, which is the whole of FM-24, and an
 * open handle behaves differently on both counts. So the server of a session is this very
 * `node.exe`, started and kept alive for as long as the case needs it: signed, present on
 * every Windows machine and runner, and a real mapped image. Each session runs a copy of its
 * own and never a hard link: one name of a file that stays reachable through another can be
 * deleted while the image runs, which is not how an installed server behaves.
 *
 * `makensis` is downloaded by the Tauri CLI the first time it bundles. `pnpm test` runs before
 * that in CI and on a fresh machine, so there the compiled half is skipped with a line saying
 * why; the `windows` job of `ci.yml` runs this file again after its bundle with
 * `BATON_REQUIRE_MAKENSIS=1`, which turns a missing `makensis` into a failure.
 */
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterAll, beforeAll, describe, expect, it } from 'vitest';

// `import.meta.url` is not a file URL under vitest (T-028), so every path starts at the root.
const ROOT = process.cwd();
const HOOKS = join(ROOT, 'installer', 'windows', 'hooks.nsh');
const DRIVER = join(ROOT, 'installer', 'windows', 'test', 'hooks-test.nsi');

interface TauriConfig {
  productName: string;
  bundle: {
    targets: string[];
    externalBin: string[];
    windows?: { nsis?: { installMode?: string; installerHooks?: string } };
  };
}

const config = JSON.parse(
  readFileSync(join(ROOT, 'src-tauri', 'tauri.conf.json'), 'utf8'),
) as TauriConfig;

/** A `pub const NAME: &str = "…";` of the Rust sources, so the two languages cannot drift. */
function rustConstant(file: string, name: string): string {
  const source = readFileSync(join(ROOT, 'src-tauri', 'src', ...file.split('/')), 'utf8');
  const value = new RegExp(`pub const ${name}: &str = "([^"]*)";`).exec(source)?.[1];
  if (value === undefined) throw new Error(`src-tauri/src/${file} declares no ${name}`);
  return value;
}

/** What the application's launch cleanup deletes (`install::cleanup::is_superseded_binary`). */
const STEM = rustConstant('install/fixed_path.rs', 'SERVER_STEM');
const SUFFIX = rustConstant('install/cleanup.rs', 'OLD_BINARY_SUFFIX');
function isSuperseded(name: string): boolean {
  const lowered = name.toLowerCase();
  return lowered.startsWith(STEM) && lowered.endsWith(SUFFIX);
}

/** The code of `hooks.nsh` with its comments left out: what it does, not what it says. */
function hooksCode(): string {
  return readFileSync(HOOKS, 'utf8')
    .split(/\r?\n/)
    .filter((line) => !line.trimStart().startsWith(';'))
    .join('\n');
}

describe('the bundle the fixed path depends on', () => {
  it('installs per user, under the product name, with the server beside the application', () => {
    // SRV-25: every agent configuration names `%LOCALAPPDATA%\Baton\handoff-mcp.exe`. The
    // template installs a `currentUser` bundle into `$LOCALAPPDATA\<productName>` and an
    // external binary beside the main one under its own name, so these values are that path.
    expect(config.productName).toBe('Baton');
    expect(config.bundle.targets).toContain('nsis');
    expect(config.bundle.windows?.nsis?.installMode).toBe('currentUser');
    expect(config.bundle.externalBin).toEqual([`binaries/${STEM}`]);
  });

  it('hands the hooks to the installer Tauri generates', () => {
    const hooks = config.bundle.windows?.nsis?.installerHooks;
    expect(hooks).toBeDefined();
    expect(join(ROOT, 'src-tauri', hooks ?? '')).toBe(HOOKS);
    for (const hook of ['NSIS_HOOK_PREINSTALL', 'NSIS_HOOK_PREUNINSTALL', 'NSIS_HOOK_POSTUNINSTALL']) {
      expect(hooksCode()).toMatch(new RegExp(`^!macro ${hook}$`, 'm'));
    }
  });

  it('moves the server aside under the name the launch cleanup looks for', () => {
    expect(hooksCode()).toContain(`!define BATON_SERVER "${STEM}"`);
    expect(SUFFIX).toBe('.old.exe');
    expect(hooksCode()).toContain(`.$0${SUFFIX}"`);
    expect(hooksCode()).toContain(`.*${SUFFIX}"`);
  });

  it('never names the folder the server shares, nor an agent configuration', () => {
    // RUN-03: the runbooks survive an uninstall. §7.15: only Baton itself edits an agent's
    // settings, and only with the user's consent.
    expect(hooksCode()).not.toMatch(/\.handoff|\.claude|USERPROFILE|\$PROFILE/i);
  });
});

function findMakensis(): string | undefined {
  const localAppData = process.env.LOCALAPPDATA;
  const candidates = [
    process.env.MAKENSIS,
    localAppData === undefined ? undefined : join(localAppData, 'tauri', 'NSIS', 'makensis.exe'),
  ];
  for (const candidate of candidates) {
    if (candidate !== undefined && candidate !== '' && existsSync(candidate)) return candidate;
  }
  const where = spawnSync('where', ['makensis'], { encoding: 'utf8' });
  const first = where.status === 0 ? where.stdout.split(/\r?\n/)[0]?.trim() : undefined;
  return first === undefined || first === '' ? undefined : first;
}

const MAKENSIS = process.platform === 'win32' ? findMakensis() : undefined;
const REQUIRED = process.env.BATON_REQUIRE_MAKENSIS === '1';

describe.runIf(MAKENSIS === undefined)('without makensis', () => {
  it('skips the compiled half and says so, unless the run requires it', () => {
    const reason =
      process.platform === 'win32'
        ? 'no makensis on this machine: it arrives with the first `pnpm tauri build`'
        : 'the installer is Windows-only';
    if (REQUIRED) throw new Error(`BATON_REQUIRE_MAKENSIS=1, but ${reason}`);
    console.warn(`installer-hooks: hooks.nsh was not compiled or run (${reason})`);
  });
});

function registry(...args: string[]): { status: number | null } {
  return spawnSync('reg', args, { encoding: 'utf8' });
}

/** Resolves once `child` has exited, killing it first if it is still running. */
function stop(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
  return new Promise((resolve) => {
    child.once('exit', () => resolve());
    child.kill();
  });
}

// Each case starts processes and runs the compiled driver a few times; the default five
// seconds is a guess about a machine this suite does not control.
describe.skipIf(MAKENSIS === undefined)('hooks.nsh, compiled and run', { timeout: 30_000 }, () => {
  const product = `BatonHooksTest-${randomBytes(4).toString('hex')}`;
  const testKey = `Software\\${product}`;
  const uninstallKey = `${testKey}\\Uninstall`;
  const approvalKey = `${testKey}\\StartupApproved\\Run`;
  const running: ChildProcess[] = [];
  let work = '';
  let driver = '';
  let appData = '';

  beforeAll(() => {
    work = mkdtempSync(join(tmpdir(), 'baton-hooks-'));
    // NSIS takes `/D=` verbatim and unquoted, and Node quotes an argument with a space in it.
    if (work.includes(' ')) throw new Error(`${work} has a space in it; point TEMP elsewhere`);
    driver = join(work, 'hooks-test.exe');
    appData = join(work, 'appdata');
    const built = spawnSync(
      MAKENSIS ?? 'makensis',
      [
        '/V2',
        '/WX',
        `/DOUTFILE=${driver}`,
        `/DHOOKS=${HOOKS}`,
        `/DPRODUCTNAME=${product}`,
        `/DUNINSTKEY=${uninstallKey}`,
        `/DBATON_APP_DATA_DIR=${appData}`,
        `/DBATON_STARTUP_APPROVED_KEY=${approvalKey}`,
        DRIVER,
      ],
      { encoding: 'utf8' },
    );
    if (built.status !== 0) {
      throw new Error(`makensis refused the hooks:\n${built.stdout}\n${built.stderr}`);
    }
  });

  afterAll(async () => {
    await Promise.all(running.map(stop));
    registry('delete', `HKCU\\${testKey}`, '/f');
    if (work !== '') rmSync(work, { recursive: true, force: true });
  });

  /** One run of one hook against `folder`: the driver's exit status, 0 when it ran. */
  function hook(step: string, folder: string, ...flags: string[]): number {
    const run = spawnSync(driver, [`/STEP=${step}`, ...flags, `/D=${folder}`], {
      encoding: 'utf8',
    });
    if (run.error !== undefined) throw run.error;
    return run.status ?? -1;
  }

  /** A folder standing in for `%LOCALAPPDATA%\Baton\`, with files nobody is running. */
  function installFolder(name: string, files: string[] = []): string {
    const folder = join(work, name);
    mkdirSync(folder, { recursive: true });
    for (const file of files) writeFileSync(join(folder, file), 'nobody runs this\n');
    return folder;
  }

  /** An agent's session: the file at `path` becomes a running executable until the end. */
  async function sessionRunning(path: string): Promise<ChildProcess> {
    copyFileSync(process.execPath, path);
    const child = spawn(path, ['-e', 'process.stdin.resume()'], {
      stdio: ['pipe', 'ignore', 'ignore'],
    });
    running.push(child);
    await new Promise<void>((resolve, reject) => {
      child.once('spawn', () => resolve());
      child.once('error', reject);
    });
    return child;
  }

  /** The version the uninstall key records, as an earlier installation left it. */
  function installedVersion(version: string | undefined): void {
    if (version === undefined) {
      registry('delete', `HKCU\\${uninstallKey}`, '/v', 'DisplayVersion', '/f');
    } else {
      registry('add', `HKCU\\${uninstallKey}`, '/v', 'DisplayVersion', '/d', version, '/f');
    }
  }

  const listing = (folder: string): string[] => readdirSync(folder).sort();

  it('dispatches on the step, so a misspelt one cannot pass for a hook that ran', () => {
    expect(hook('no-such-step', installFolder('dispatch'))).toBe(2);
  });

  it('deletes a server nobody is running, and the update writes it as on a first install', () => {
    const folder = installFolder('idle', ['handoff-mcp.exe', 'handoff-app.exe']);
    installedVersion('0.1.0');

    expect(hook('preinstall', folder)).toBe(0);
    expect(listing(folder)).toEqual(['handoff-app.exe']);
  });

  it('moves a server a session is running aside, named after the installed version', async () => {
    const folder = installFolder('in-use');
    installedVersion('0.1.0');
    const session = await sessionRunning(join(folder, 'handoff-mcp.exe'));

    expect(hook('preinstall', folder)).toBe(0);

    expect(listing(folder)).toEqual(['handoff-mcp.0.1.0.old.exe']);
    expect(isSuperseded('handoff-mcp.0.1.0.old.exe')).toBe(true);
    // The session keeps the binary it opened, and the fixed path is free for the new one.
    expect(session.exitCode).toBeNull();
    writeFileSync(join(folder, 'handoff-mcp.exe'), 'the new server\n');
    expect(listing(folder)).toEqual(['handoff-mcp.0.1.0.old.exe', 'handoff-mcp.exe']);
  });

  it('does not reuse a name a session still holds', async () => {
    const folder = installFolder('twice');
    installedVersion('0.1.0');
    // A reinstall of this same version already moved one server aside, and its session runs on.
    const first = await sessionRunning(join(folder, 'handoff-mcp.0.1.0.old.exe'));
    const second = await sessionRunning(join(folder, 'handoff-mcp.exe'));

    expect(hook('preinstall', folder)).toBe(0);

    expect(listing(folder)).toEqual(['handoff-mcp.0.1.0-1.old.exe', 'handoff-mcp.0.1.0.old.exe']);
    expect(first.exitCode).toBeNull();
    expect(second.exitCode).toBeNull();
    for (const name of listing(folder)) expect(isSuperseded(name)).toBe(true);
  });

  it('takes the name back from a superseded server nobody holds any more', async () => {
    const folder = installFolder('stale', ['handoff-mcp.0.1.0.old.exe']);
    installedVersion('0.1.0');
    await sessionRunning(join(folder, 'handoff-mcp.exe'));

    expect(hook('preinstall', folder)).toBe(0);
    expect(listing(folder)).toEqual(['handoff-mcp.0.1.0.old.exe']);
  });

  it('says "unknown" when no installation recorded a version', async () => {
    const folder = installFolder('unrecorded');
    installedVersion(undefined);
    await sessionRunning(join(folder, 'handoff-mcp.exe'));

    expect(hook('preinstall', folder)).toBe(0);
    expect(listing(folder)).toEqual(['handoff-mcp.unknown.old.exe']);
    expect(isSuperseded('handoff-mcp.unknown.old.exe')).toBe(true);
  });

  it('leaves a first installation alone', () => {
    const folder = installFolder('fresh');
    expect(hook('preinstall', folder)).toBe(0);
    expect(listing(folder)).toEqual([]);
  });

  it('uninstalls the superseded servers nobody holds, and nothing else', async () => {
    const folder = installFolder('uninstall', [
      'handoff-mcp.0.1.0.old.exe',
      'handoff-mcp.exe',
      'notes.old.exe',
    ]);
    const session = await sessionRunning(join(folder, 'handoff-mcp.0.2.0.old.exe'));

    expect(hook('preuninstall', folder)).toBe(0);

    // The current server and the program are the template's to remove, and a file a session
    // still runs cannot be deleted by anyone.
    expect(listing(folder)).toEqual(['handoff-mcp.0.2.0.old.exe', 'handoff-mcp.exe', 'notes.old.exe']);
    expect(session.exitCode).toBeNull();
  });

  it('removes the application data only when the box was ticked, never during an update', () => {
    const shared = join(work, 'home', '.handoff');
    mkdirSync(shared, { recursive: true });
    writeFileSync(join(shared, 'channel.token'), 'shared with the server\n');
    mkdirSync(appData, { recursive: true });
    writeFileSync(join(appData, 'handoff.sqlite'), 'the log\n');
    const folder = installFolder('app-data');

    // Unticked: the default, and every silent uninstall.
    expect(hook('postuninstall', folder)).toBe(0);
    expect(existsSync(join(appData, 'handoff.sqlite'))).toBe(true);

    // A newer setup replacing this one runs the uninstaller with `/UPDATE`.
    expect(hook('postuninstall', folder, '/UPDATE', '/DELETEAPPDATA')).toBe(0);
    expect(existsSync(join(appData, 'handoff.sqlite'))).toBe(true);

    expect(hook('postuninstall', folder, '/DELETEAPPDATA')).toBe(0);
    expect(existsSync(appData)).toBe(false);
    // RUN-03: the folder the server shares is not the application's data.
    expect(existsSync(join(shared, 'channel.token'))).toBe(true);
  });

  it('removes the login approval marker with the application, and keeps it through an update', () => {
    const folder = installFolder('login');
    registry('add', `HKCU\\${approvalKey}`, '/v', product, '/t', 'REG_BINARY', '/d', '02', '/f');
    const marked = () => registry('query', `HKCU\\${approvalKey}`, '/v', product).status === 0;
    expect(marked()).toBe(true);

    expect(hook('postuninstall', folder, '/UPDATE')).toBe(0);
    expect(marked()).toBe(true);

    expect(hook('postuninstall', folder)).toBe(0);
    expect(marked()).toBe(false);
  });
});
