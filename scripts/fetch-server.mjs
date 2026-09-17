// Downloads, verifies and unpacks the pinned handoff-mcp release (TECHNICAL-DESIGN §3.5).
//
// This is the only coupling between the two repositories: no source import, no submodule,
// no path dependency — one signed release artifact, pinned by `server.lock.json`. The
// script reads that lock, fetches the platform binary and the format tarball of the
// release, checks every byte against the checksums, checks the checksum file against the
// owner's minisign signature, and only then unpacks:
//
//   vendor/handoff-mcp/bin/<platform>/handoff-mcp[.exe]
//   vendor/handoff-mcp/format/{schemas,patterns,protocol,fixtures,docs,FORMAT-VERSION}
//   vendor/handoff-mcp/VERSION
//   src-tauri/binaries/handoff-mcp-<target-triple>[.exe]   (Tauri externalBin naming)
//
// `--check` is the build-time gate of §3.5: it verifies that vendor/ exists and records
// the version the lock pins, and downloads nothing. `scripts/workspace/dev-link` writes
// the same layout from a local build with a `dev-<sha>` version; that is a
// deliberate bypass of the lock, so `--check` refuses it when `CI` is set and only warns
// otherwise.
//
// `--format-only` fetches and verifies the format tarball alone, leaving `bin/` and
// `src-tauri/binaries/` empty. The tarball is platform-neutral and is pinned like any other
// asset, so nothing about the verification chain changes; what changes is that the run does
// not need a platform binary the lock does not pin. That is the state of macOS while it is
// deferred (implementation decision 7): the app crate embeds the schemas, the channel protocol
// and the pattern file at build time (T-029), so `cargo test` there needs the format and
// nothing else. Combine it with `--check` for the matching gate.
//
// Node ≥ 22, no dependencies: the minisign verification is ported from
// `handoff-mcp/build/verify-release.mjs` (Ed25519 through `crypto.verify`, BLAKE2b-512
// through `crypto.createHash`) and the tar reader below is written here rather than
// shelled out to `tar`, which reads an absolute Windows path as `host:path`.
//
// Usage:
//   node scripts/fetch-server.mjs
//   node scripts/fetch-server.mjs --check
//   node scripts/fetch-server.mjs --format-only
//   node scripts/fetch-server.mjs --dir dist/assets --keep
//   node scripts/fetch-server.mjs --dir dist/assets --offline
//
// Options:
//   --check              verify vendor/ against the lock and exit; downloads nothing
//   --format-only        fetch or verify the format material alone, without a binary
//   --repo <owner/name>  the release repository (default Cepeppe/handoff-mcp)
//   --dir <dir>          where the release assets are written (default a temporary dir)
//   --keep               do not delete that directory afterwards
//   --offline            use the assets already in --dir instead of downloading
//
// Authentication: none is required, because `handoff-mcp` is public. A token is still sent
// when one is at hand, since the GitHub API allows an anonymous address 60 requests an hour
// and a CI runner shares its address with other jobs: `GH_TOKEN` or `GITHUB_TOKEN` (CI passes
// the run's own `github.token`), then `gh auth token`. See `scripts/README.md`.
import { execFileSync } from 'node:child_process';
import { createHash, createPublicKey, verify as verifySignature } from 'node:crypto';
import { createReadStream, createWriteStream } from 'node:fs';
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rename,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { pipeline } from 'node:stream/promises';
import { Readable } from 'node:stream';
import { fileURLToPath } from 'node:url';
import { gunzipSync } from 'node:zlib';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');

const DEFAULT_REPO = 'Cepeppe/handoff-mcp';

const LOCK_FILE = 'server.lock.json';
const PUBLIC_KEY_FILE = join('keys', 'handoff-mcp-release.pub');
const VENDOR_DIR = join('vendor', 'handoff-mcp');
const BINARIES_DIR = join('src-tauri', 'binaries');

const SUMS = 'SHA256SUMS';
const SIGNATURE = 'SHA256SUMS.minisig';
const VERSION_FILE = 'VERSION';
const FORMAT_VERSION_FILE = 'FORMAT-VERSION';

/** The top-level directories the format tarball must carry (§3.5). */
const FORMAT_DIRS = ['schemas', 'patterns', 'protocol', 'fixtures', 'docs'];

/**
 * The release targets of §3.5 and the Rust target triple Tauri appends to the name of an
 * `externalBin`. A platform key of the lock file is one of these.
 */
const TARGET_TRIPLES = {
  'win32-x64': 'x86_64-pc-windows-msvc',
  'darwin-x64': 'x86_64-apple-darwin',
  'darwin-arm64': 'aarch64-apple-darwin',
};

/** minisign's two signature algorithms: legacy over the file, `ED` over its BLAKE2b hash. */
const SIGALG_LEGACY = 'Ed';
const SIGALG_PREHASHED = 'ED';

/** The SPKI wrapper that turns 32 raw Ed25519 bytes into a key `crypto.verify` accepts. */
const ED25519_SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

class FetchError extends Error {}

function fail(message) {
  throw new FetchError(message);
}

function inCI() {
  const value = process.env.CI;
  return value !== undefined && value !== '' && value !== '0' && value !== 'false';
}

// -------------------------------------------------------------------------- lock file

function isWindows(platform) {
  return platform.startsWith('win32-');
}

/** `win32-x64`, `darwin-arm64` or `darwin-x64`; anything else has no published asset. */
function hostPlatform() {
  const platform = `${process.platform}-${process.arch}`;
  if (!(platform in TARGET_TRIPLES)) {
    fail(
      `${platform} is not a release target of TECHNICAL-DESIGN §3.5 ` +
        `(${Object.keys(TARGET_TRIPLES).join(', ')})`,
    );
  }
  return platform;
}

/** The asset names of §3.5. */
function binaryAsset(version, platform) {
  return `handoff-mcp-${version}-${platform}${isWindows(platform) ? '.exe' : ''}`;
}

function formatAsset(version) {
  return `handoff-mcp-${version}-format.tar.gz`;
}

/** The name the server has inside `vendor/`, and the name Tauri wants in `binaries/`. */
function executableName(platform) {
  return `handoff-mcp${isWindows(platform) ? '.exe' : ''}`;
}

function externalBinName(platform) {
  return `handoff-mcp-${TARGET_TRIPLES[platform]}${isWindows(platform) ? '.exe' : ''}`;
}

async function readLock() {
  let lock;
  try {
    lock = JSON.parse(await readFile(join(repoRoot, LOCK_FILE), 'utf8'));
  } catch (error) {
    fail(`${LOCK_FILE} could not be read: ${error.message}`);
  }
  if (typeof lock.version !== 'string' || !/^\d+\.\d+\.\d+(?:[-+].+)?$/.test(lock.version)) {
    fail(`${LOCK_FILE} has no semver "version"`);
  }
  if (!Number.isInteger(lock.protocol_version) || lock.protocol_version < 1) {
    fail(`${LOCK_FILE} has no positive integer "protocol_version"`);
  }
  if (lock.assets === null || typeof lock.assets !== 'object') {
    fail(`${LOCK_FILE} has no "assets" object`);
  }
  for (const [name, entry] of Object.entries(lock.assets)) {
    if (name !== 'format' && !(name in TARGET_TRIPLES)) {
      fail(`${LOCK_FILE} pins "${name}", which is not an asset of §3.5`);
    }
    if (typeof entry?.sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(entry.sha256)) {
      fail(`${LOCK_FILE}: asset "${name}" has no sha256`);
    }
  }
  if (lock.assets.format === undefined) fail(`${LOCK_FILE} does not pin the format asset`);
  return lock;
}

/** The platform entry of the lock, with the message implementation decision 7 makes useful on darwin. */
function pinnedPlatform(lock, platform) {
  if (lock.assets[platform] === undefined) {
    fail(
      `${LOCK_FILE} pins no asset for ${platform}: the release publishes ` +
        `${Object.keys(lock.assets)
          .filter((name) => name !== 'format')
          .join(', ')} only`,
    );
  }
  return lock.assets[platform].sha256;
}

// ---------------------------------------------------------------------------- minisign

/**
 * The key id the way minisign prints it in the comment of a `.pub` file: the eight bytes
 * read big-endian, so that the value can be compared with `keys/handoff-mcp-release.pub`
 * by eye.
 */
function keyIdOf(raw) {
  return Buffer.from(raw.subarray(2, 10)).reverse().toString('hex').toUpperCase();
}

function decodeBase64Line(line, what) {
  const decoded = Buffer.from(line.trim(), 'base64');
  if (decoded.length === 0) fail(`${what} is not base64`);
  return decoded;
}

/** Parses the committed `.pub` file: a comment line and the `RW…` payload. */
async function readPublicKey() {
  const path = join(repoRoot, PUBLIC_KEY_FILE);
  let text;
  try {
    text = await readFile(path, 'utf8');
  } catch {
    fail(`${PUBLIC_KEY_FILE} is missing: the release cannot be verified without it`);
  }
  const candidate = text
    .split(/\r?\n/)
    .map((entry) => entry.trim())
    .filter((entry) => entry !== '' && !entry.startsWith('untrusted comment:'));
  if (candidate.length !== 1) fail(`${PUBLIC_KEY_FILE} does not look like a minisign public key`);

  const raw = decodeBase64Line(candidate[0], 'the public key');
  if (raw.length !== 42) fail(`the public key is ${raw.length} bytes, expected 42`);
  const algorithm = raw.subarray(0, 2).toString('latin1');
  if (algorithm !== SIGALG_LEGACY) fail(`unknown public key algorithm ${algorithm}`);
  return {
    keyId: keyIdOf(raw),
    key: createPublicKey({
      key: Buffer.concat([ED25519_SPKI_PREFIX, raw.subarray(10)]),
      format: 'der',
      type: 'spki',
    }),
  };
}

/** Parses a `.minisig`: the signature, its algorithm, and the trusted comment it covers. */
function parseSignature(text) {
  const lines = text.split(/\r?\n/);
  if (lines.length < 4) fail(`${SIGNATURE} is truncated: ${lines.length} lines, expected 4`);

  const raw = decodeBase64Line(lines[1], 'the signature');
  if (raw.length !== 74) fail(`the signature is ${raw.length} bytes, expected 74`);
  const trustedComment = lines[2].replace(/^trusted comment:\s?/, '');
  if (trustedComment === lines[2]) fail(`${SIGNATURE} has no trusted comment on line 3`);
  const globalSignature = decodeBase64Line(lines[3], 'the global signature');
  if (globalSignature.length !== 64) {
    fail(`the global signature is ${globalSignature.length} bytes, expected 64`);
  }

  const algorithm = raw.subarray(0, 2).toString('latin1');
  if (algorithm !== SIGALG_LEGACY && algorithm !== SIGALG_PREHASHED) {
    fail(`unknown signature algorithm ${algorithm}`);
  }
  return {
    algorithm,
    keyId: keyIdOf(raw),
    signature: raw.subarray(10),
    trustedComment,
    globalSignature,
  };
}

/**
 * Verifies a detached minisign signature over `content`. Both signatures are checked: the
 * one over the file, and the global one over `signature || trusted comment`, which is what
 * stops an attacker from keeping a valid signature and rewriting the comment around it.
 */
function verifyMinisign(content, signatureText, publicKey) {
  const parsed = parseSignature(signatureText);
  if (parsed.keyId !== publicKey.keyId) {
    fail(`${SUMS} was signed by key ${parsed.keyId}, not by ${publicKey.keyId}`);
  }

  const signed =
    parsed.algorithm === SIGALG_PREHASHED
      ? createHash('blake2b512').update(content).digest()
      : content;
  if (!verifySignature(null, signed, publicKey.key, parsed.signature)) {
    fail(`the signature of ${SUMS} does not match its content`);
  }
  if (
    !verifySignature(
      null,
      Buffer.concat([parsed.signature, Buffer.from(parsed.trustedComment, 'utf8')]),
      publicKey.key,
      parsed.globalSignature,
    )
  ) {
    fail('the global signature does not match the trusted comment');
  }
  return parsed;
}

// ------------------------------------------------------------------------------ GitHub

/** The token to send, if one is at hand; `undefined` reads the public release anonymously. */
function token() {
  for (const name of ['GH_TOKEN', 'GITHUB_TOKEN']) {
    const value = process.env[name];
    if (value !== undefined && value !== '') return value;
  }
  try {
    const cli = execFileSync('gh', ['auth', 'token'], { encoding: 'utf8', stdio: 'pipe' }).trim();
    return cli === '' ? undefined : cli;
  } catch {
    // No GitHub CLI, or one that is not logged in.
    return undefined;
  }
}

async function api(path, accept, auth) {
  const request = (bearer) =>
    fetch(`https://api.github.com${path}`, {
      headers: {
        accept,
        ...(bearer === undefined ? {} : { authorization: `Bearer ${bearer}` }),
        'user-agent': 'handoff-app-fetch-server',
        'x-github-api-version': '2022-11-28',
      },
    });
  let response = await request(auth);
  // A stale token must not block a download that needs none.
  if (response.status === 401 && auth !== undefined) response = await request(undefined);
  if (!response.ok) {
    const limited = (response.status === 403 || response.status === 429) && auth === undefined;
    const hint = limited
      ? ' — the anonymous rate limit of the GitHub API: set GH_TOKEN or run `gh auth login`'
      : '';
    fail(`GET ${path} answered ${response.status} ${response.statusText}${hint}`);
  }
  return response;
}

async function releaseAssets(repo, version, auth) {
  const response = await api(
    `/repos/${repo}/releases/tags/v${version}`,
    'application/vnd.github+json',
    auth,
  );
  const release = await response.json();
  return release.assets.map((asset) => ({ name: asset.name, id: asset.id, size: asset.size }));
}

/**
 * Downloads one asset. The API endpoint redirects to storage, and Node's fetch drops the
 * `Authorization` header across origins by itself, which is what the redirect target wants.
 */
async function download(repo, asset, dir, auth) {
  const response = await api(
    `/repos/${repo}/releases/assets/${asset.id}`,
    'application/octet-stream',
    auth,
  );
  const target = join(dir, asset.name);
  await pipeline(Readable.fromWeb(response.body), createWriteStream(target));
  const written = (await stat(target)).size;
  if (written !== asset.size) {
    fail(`${asset.name} downloaded as ${written} bytes, the release says ${asset.size}`);
  }
  console.error(`  downloaded ${asset.name} (${(written / 1024).toFixed(0)} KB)`);
  return target;
}

// ------------------------------------------------------------------------ verification

function sha256(path) {
  const hash = createHash('sha256');
  return pipeline(createReadStream(path), hash).then(() => hash.digest('hex'));
}

/** Parses `sha256sum` output: `<64 hex><two spaces><name>`, one asset per line. */
function parseSums(text) {
  const sums = new Map();
  for (const line of text.split(/\r?\n/)) {
    if (line.trim() === '') continue;
    const match = /^([0-9a-f]{64}) [ *](.+)$/.exec(line);
    if (match === null) fail(`${SUMS} has a line that is not a checksum: ${line}`);
    const name = basename(match[2].trim());
    if (sums.has(name)) fail(`${SUMS} lists ${name} twice`);
    sums.set(name, match[1]);
  }
  if (sums.size === 0) fail(`${SUMS} is empty`);
  return sums;
}

/**
 * Checks one downloaded asset against both authorities that must agree: the signed
 * `SHA256SUMS` of the release and the sha256 the lock file pins. The signature proves the
 * release is the owner's; the lock proves it is the release this checkout was built
 * against. A mismatch on either is a refusal, never a warning.
 */
async function verifyAsset(dir, name, pinned, sums) {
  const actual = await sha256(join(dir, name));
  const signed = sums.get(name);
  if (signed === undefined) fail(`${name} is not listed in ${SUMS}`);
  if (actual !== signed) fail(`${name} hashes to ${actual}, the signed ${SUMS} says ${signed}`);
  if (actual !== pinned) fail(`${name} hashes to ${actual}, ${LOCK_FILE} pins ${pinned}`);
  console.error(`  verified   ${name} — sha256 ${actual}`);
}

// ------------------------------------------------------------------------------- untar

const BLOCK = 512;

/** A NUL-terminated header field, trimmed. */
function tarField(header, start, length) {
  return header
    .subarray(start, start + length)
    .toString('utf8')
    .replace(/\0[\s\S]*$/, '')
    .trim();
}

function tarSize(header) {
  const raw = tarField(header, 124, 12);
  const size = Number.parseInt(raw === '' ? '0' : raw, 8);
  if (!Number.isSafeInteger(size) || size < 0) {
    fail(`the archive has an unreadable entry size "${raw}"`);
  }
  return size;
}

/** Splits an entry name that is guaranteed to stay inside the destination directory. */
function safeEntryPath(name) {
  const cleaned = name.replace(/\/+$/, '');
  if (cleaned === '') fail('the archive has an entry with an empty name');
  if (cleaned.startsWith('/') || cleaned.includes('\\') || /^[A-Za-z]:/.test(cleaned)) {
    fail(`the archive has an absolute entry "${name}"`);
  }
  const parts = cleaned.split('/');
  if (parts.some((part) => part === '.' || part === '..')) {
    fail(`the archive has a traversing entry "${name}"`);
  }
  return parts;
}

/**
 * Unpacks a gzipped ustar archive into `destination`. Written here rather than shelled out
 * to `tar` for three reasons: GNU tar under Git Bash reads an absolute Windows path as
 * `host:path` and tries to open a network connection; the entry names are checked against
 * traversal before anything is written; and the format tarball is a few hundred kilobytes,
 * so holding it in memory costs nothing. Only the entry types GNU tar produces for this
 * archive are accepted; anything else is a refusal rather than a guess.
 */
async function extractTarGz(archive, destination) {
  const buffer = gunzipSync(await readFile(archive));
  let offset = 0;
  let longName;
  let files = 0;

  while (offset + BLOCK <= buffer.length) {
    const header = buffer.subarray(offset, offset + BLOCK);
    if (header.every((byte) => byte === 0)) break;
    if (tarField(header, 257, 6) !== 'ustar') fail('the archive is not a ustar tar');

    const size = tarSize(header);
    if (offset + BLOCK + size > buffer.length) fail('the archive is truncated');
    const flag = header[156] === 0 ? '0' : String.fromCharCode(header[156]);
    const prefix = tarField(header, 345, 155);
    const stored = tarField(header, 0, 100);
    const name = longName ?? (prefix === '' ? stored : `${prefix}/${stored}`);
    longName = undefined;
    const body = buffer.subarray(offset + BLOCK, offset + BLOCK + size);
    offset += BLOCK + Math.ceil(size / BLOCK) * BLOCK;

    if (flag === 'L') {
      longName = body.toString('utf8').replace(/\0[\s\S]*$/, '');
      continue;
    }
    const parts = safeEntryPath(name);
    const target = join(destination, ...parts);
    if (flag === '5') {
      await mkdir(target, { recursive: true });
      continue;
    }
    if (flag !== '0') fail(`the archive has an entry of unsupported type "${flag}" (${name})`);
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, body);
    files += 1;
  }
  if (files === 0) fail('the archive contains no files');
  return files;
}

// ------------------------------------------------------------------------ vendor layout

function parseFormatVersion(text) {
  const values = new Map();
  for (const line of text.split(/\r?\n/)) {
    if (line.trim() === '') continue;
    const index = line.indexOf('=');
    if (index === -1) fail(`${FORMAT_VERSION_FILE} has a line that is not key=value: ${line}`);
    values.set(line.slice(0, index).trim(), line.slice(index + 1).trim());
  }
  return values;
}

async function exists(path) {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

async function readVendorVersion() {
  try {
    return (await readFile(join(repoRoot, VENDOR_DIR, VERSION_FILE), 'utf8')).trim();
  } catch {
    return undefined;
  }
}

/**
 * The `protocol_version` of the lock has to be the one the unpacked artifact carries.
 * §3.6 requires exact equality on the channel protocol and §3.5 settles it at build time,
 * never at run time; the second half of that comparison — against the version compiled
 * into the app's listener — belongs to the listener, which does not exist yet (T-031).
 */
async function verifyFormatDirectory(formatDir, lock, { pinnedVersion = true } = {}) {
  for (const dir of FORMAT_DIRS) {
    if (!(await exists(join(formatDir, dir)))) {
      fail(`the format artifact has no ${dir}/`);
    }
  }
  const versionPath = join(formatDir, FORMAT_VERSION_FILE);
  if (!(await exists(versionPath))) fail(`the format artifact has no ${FORMAT_VERSION_FILE}`);
  const values = parseFormatVersion(await readFile(versionPath, 'utf8'));

  const protocol = values.get('protocol_version');
  if (protocol !== String(lock.protocol_version)) {
    fail(
      `the artifact speaks channel protocol ${protocol}, ${LOCK_FILE} pins ` +
        `${lock.protocol_version} (§3.6 requires exact equality)`,
    );
  }
  const packaged = values.get('package_version');
  if (pinnedVersion && packaged !== lock.version) {
    fail(`the artifact was built from ${packaged}, ${LOCK_FILE} pins ${lock.version}`);
  }
  return values;
}

/** Copies the server where Tauri looks for an `externalBin` (§3.5 placement). */
async function placeExternalBin(source, platform) {
  const dir = join(repoRoot, BINARIES_DIR);
  await mkdir(dir, { recursive: true });
  const target = join(dir, externalBinName(platform));
  await copyFile(source, target);
  if (process.platform !== 'win32') await chmod(target, 0o755);
  return target;
}

/**
 * Writes the whole vendor tree in a staging directory and swaps it in at the end, so that
 * a failure halfway through leaves the previous artifact intact instead of a half-unpacked
 * one that `--check` would then have to recognise.
 *
 * `binaryPath` is null under `--format-only`: the format material is written and `bin/` and
 * `src-tauri/binaries/` are left empty, which is all a job that compiles without bundling
 * needs.
 */
async function fillVendor(lock, platform, binaryPath, formatPath) {
  const vendor = join(repoRoot, VENDOR_DIR);
  const staging = join(repoRoot, 'vendor', `.staging-${process.pid}`);
  let files;
  let versions;
  try {
    await rm(staging, { recursive: true, force: true });

    const formatDir = join(staging, 'format');
    await mkdir(formatDir, { recursive: true });
    files = await extractTarGz(formatPath, formatDir);
    versions = await verifyFormatDirectory(formatDir, lock);

    if (binaryPath !== null) {
      const binDir = join(staging, 'bin', platform);
      await mkdir(binDir, { recursive: true });
      const executable = join(binDir, executableName(platform));
      await copyFile(binaryPath, executable);
      if (process.platform !== 'win32') await chmod(executable, 0o755);
    }

    await writeFile(join(staging, VERSION_FILE), `${lock.version}\n`);

    await rm(vendor, { recursive: true, force: true });
    await mkdir(dirname(vendor), { recursive: true });
    await rename(staging, vendor);
  } finally {
    // The rename above leaves nothing to remove; a refusal halfway through does.
    await rm(staging, { recursive: true, force: true });
  }

  if (binaryPath === null) return { files, versions, placed: null };
  const installed = join(vendor, 'bin', platform, executableName(platform));
  const placed = await placeExternalBin(installed, platform);
  return { files, versions, placed };
}

// ------------------------------------------------------------------------------- check

/**
 * The build-time gate of §3.5. A `dev-…` vendor comes from `scripts/workspace/dev-link`,
 * which is documented as bypassing the lock and refuses to run in CI: it
 * is reported and tolerated on a developer machine, and refused wherever `CI` is set, so
 * no release build can be made from a locally built server.
 */
async function check(lock, platform, options = {}) {
  const vendor = join(repoRoot, VENDOR_DIR);
  if (!(await exists(vendor))) {
    fail(`${VENDOR_DIR} is absent: run \`node scripts/fetch-server.mjs\``);
  }
  const version = await readVendorVersion();
  if (version === undefined || version === '') {
    fail(`${join(VENDOR_DIR, VERSION_FILE)} is absent: run \`node scripts/fetch-server.mjs\``);
  }

  const development = version.startsWith('dev-');
  if (development && inCI()) {
    fail(
      `${VENDOR_DIR} is a development build (${version}) linked by scripts/workspace/dev-link; ` +
        `CI builds only from the pinned ${lock.version}`,
    );
  }
  if (!development && version !== lock.version) {
    fail(`${VENDOR_DIR} is ${version}, ${LOCK_FILE} pins ${lock.version}`);
  }

  if (!options.formatOnly) {
    const executable = join(vendor, 'bin', platform, executableName(platform));
    if (!(await exists(executable))) {
      fail(`${VENDOR_DIR} carries no ${platform} server: run \`node scripts/fetch-server.mjs\``);
    }
    const external = join(BINARIES_DIR, externalBinName(platform));
    if (!(await exists(join(repoRoot, external)))) {
      fail(`${external} is absent: run \`node scripts/fetch-server.mjs\``);
    }
  }
  await verifyFormatDirectory(join(vendor, 'format'), lock, { pinnedVersion: !development });

  const what = options.formatOnly ? 'format material' : `server for ${platform}`;
  if (development) {
    console.error(`[dev] ${VENDOR_DIR} is ${version}, not the pinned ${lock.version}`);
    console.error('      it was filled by scripts/workspace/dev-link, which bypasses the lock file');
  } else {
    console.error(`[ok] ${VENDOR_DIR} is the ${what} of ${version}, as ${LOCK_FILE} pins`);
  }
  process.stdout.write(`${version}\n`);
}

// -------------------------------------------------------------------------------- main

async function fetch_(lock, platform, options) {
  const publicKey = await readPublicKey();
  const binaryName = options.formatOnly ? null : binaryAsset(lock.version, platform);
  const formatName = formatAsset(lock.version);

  const dir =
    options.dir === undefined
      ? await mkdtemp(join(tmpdir(), 'handoff-mcp-asset-'))
      : resolve(repoRoot, options.dir);
  const keep = options.keep || options.offline || options.dir !== undefined;

  try {
    if (options.offline) {
      console.error(`using the assets already in ${dir}`);
    } else {
      const auth = token();
      await mkdir(dir, { recursive: true });
      const published = await releaseAssets(options.repo, lock.version, auth);
      const wanted = [binaryName, formatName, SUMS, SIGNATURE].filter((name) => name !== null);
      for (const name of wanted) {
        const asset = published.find((entry) => entry.name === name);
        if (asset === undefined) fail(`the release v${lock.version} has no ${name}`);
      }
      console.error(`${options.repo} v${lock.version}: ${published.length} assets`);
      for (const name of wanted) {
        await download(options.repo, published.find((entry) => entry.name === name), dir, auth);
      }
    }

    const sumsText = await readFile(join(dir, SUMS), 'utf8');
    const parsed = verifyMinisign(
      Buffer.from(sumsText, 'utf8'),
      await readFile(join(dir, SIGNATURE), 'utf8'),
      publicKey,
    );
    console.error(`  verified   ${SIGNATURE} — key ${publicKey.keyId}, ${parsed.trustedComment}`);

    const sums = parseSums(sumsText);
    if (binaryName !== null) {
      await verifyAsset(dir, binaryName, pinnedPlatform(lock, platform), sums);
    }
    await verifyAsset(dir, formatName, lock.assets.format.sha256, sums);

    const result = await fillVendor(
      lock,
      platform,
      binaryName === null ? null : join(dir, binaryName),
      join(dir, formatName),
    );
    console.error(`  unpacked   ${result.files} format files into ${join(VENDOR_DIR, 'format')}`);
    if (result.placed !== null) {
      console.error(`  placed     ${join(BINARIES_DIR, externalBinName(platform))}`);
    }
    const scope = binaryName === null ? 'format material only' : `for ${platform}`;
    console.error(
      `\nvendor is handoff-mcp ${lock.version} ${scope}, ` +
        `channel protocol ${result.versions.get('protocol_version')}, ` +
        `patterns ${result.versions.get('patterns_version')}`,
    );
    process.stdout.write(`${lock.version}\n`);
  } finally {
    if (!keep) await rm(dir, { recursive: true, force: true });
  }
}

async function main(argv) {
  const flag = (name) => argv.includes(name);
  const value = (name) => {
    const index = argv.indexOf(name);
    if (index === -1) return undefined;
    if (argv[index + 1] === undefined) fail(`${name} needs a value`);
    return argv[index + 1];
  };
  for (const argument of argv) {
    if (
      argument.startsWith('--') &&
      !['--check', '--format-only', '--keep', '--offline', '--repo', '--dir'].includes(argument)
    ) {
      fail(`unknown option ${argument} (scripts/README.md lists them)`);
    }
  }

  const lock = await readLock();
  const platform = hostPlatform();
  // The format tarball is platform-neutral, so `--format-only` never consults the platform
  // entry of the lock: that entry is exactly what a host with no pinned binary lacks.
  const formatOnly = flag('--format-only');
  if (flag('--check')) {
    if (!formatOnly) pinnedPlatform(lock, platform);
    await check(lock, platform, { formatOnly });
    return;
  }

  const options = {
    repo: value('--repo') ?? DEFAULT_REPO,
    dir: value('--dir'),
    keep: flag('--keep'),
    offline: flag('--offline'),
    formatOnly,
  };
  if (options.offline && options.dir === undefined) fail('--offline needs --dir');
  await fetch_(lock, platform, options);
}

try {
  await main(process.argv.slice(2));
} catch (error) {
  if (!(error instanceof FetchError)) throw error;
  console.error(`fetch-server: ${error.message}`);
  process.exitCode = 1;
}
