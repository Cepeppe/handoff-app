# `scripts/` — the pinned server artifact

`handoff-app` consumes `handoff-mcp` as **one signed release artifact and nothing else**:
no source import, no submodule, no path dependency. `fetch-server.mjs` is what turns that
pin into files on disk, and `--check` is the gate the build runs before it packages
anything.

## `fetch-server.mjs`

```sh
node scripts/fetch-server.mjs            # download, verify and unpack the pinned release
node scripts/fetch-server.mjs --check    # verify what is already in vendor/, download nothing
```

It needs Node 22 or newer and has no dependencies: the minisign verification is Ed25519
through `node:crypto` and the tar reader is part of the script.

### What a run does

1. Reads `server.lock.json`: the pinned `version`, one `sha256` per asset, and the channel
   `protocol_version`.
2. Downloads, from the release tagged `v<version>`, the binary for the current platform,
   the format tarball, `SHA256SUMS` and `SHA256SUMS.minisig`.
3. Verifies `SHA256SUMS` against `keys/handoff-mcp-release.pub`, the owner's minisign
   public key — both the signature over the file and the global signature over the trusted
   comment, so the comment cannot be rewritten around a valid signature.
4. Verifies each asset against **two** authorities that must agree: the signed
   `SHA256SUMS` (this is the owner's release) and the `sha256` of the lock (this is the
   release this checkout was built against). Either mismatch is a refusal.
5. Unpacks into a staging directory, checks that the artifact carries the five format
   directories, that its `FORMAT-VERSION` was built from the pinned version, and that its
   channel `protocol_version` is the one the lock pins — exact equality, settled at build
   time and never at run time. Only then does it replace `vendor/`, so a failed run leaves
   the previous artifact intact.

### What it leaves behind

```
vendor/handoff-mcp/VERSION                        0.1.0
vendor/handoff-mcp/bin/win32-x64/handoff-mcp.exe  the standalone server
vendor/handoff-mcp/format/                        schemas patterns protocol fixtures docs
src-tauri/binaries/handoff-mcp-<target-triple>    the same binary, named for Tauri
```

Both trees are git-ignored. The copy under `src-tauri/binaries/` is where Tauri looks for
an `externalBin`, so the server is signed and notarized together with the app; the name
carries the Rust target triple (`x86_64-pc-windows-msvc`, `x86_64-apple-darwin`,
`aarch64-apple-darwin`).

The app never edits anything under `vendor/`. A change to a schema, a pattern file or the
channel definition happens in `handoff-mcp`, is released, and reaches the app by bumping
`server.lock.json`.

### Why a token is needed

`Cepeppe/handoff-mcp` is private for now, so the GitHub API refuses an anonymous read of
the release and its assets. The script looks for a token in this order:

| Source | Where it is used |
|---|---|
| `HANDOFF_MCP_READ_TOKEN` | CI: a fine-grained PAT with Contents **read-only** on `Cepeppe/handoff-mcp` alone, stored as a repository secret |
| `GH_TOKEN` | a shell that already exports one |
| `gh auth token` | a developer machine logged in with the GitHub CLI |

The default `GITHUB_TOKEN` of an Actions run is deliberately **not** used: it is scoped to
this repository and cannot read the other one, so it would fail with a confusing 404. A
404 from the release endpoint almost always means the token cannot see `handoff-mcp`.

`HANDOFF_MCP_READ_TOKEN` expires on **2027-09-07**; after that date the app CI cannot
download the release assets until it is regenerated with the same shape. It becomes
unnecessary if the server repository is ever made public.

### Options

| Option | Effect |
|---|---|
| `--check` | verify `vendor/` against the lock and exit; downloads nothing, needs no token |
| `--repo <owner/name>` | read the release from another repository (default `Cepeppe/handoff-mcp`) |
| `--dir <dir>` | keep the downloaded assets here instead of a temporary directory |
| `--keep` | do not delete that directory afterwards |
| `--offline` | verify and unpack the assets already in `--dir` instead of downloading |

Exit status is 0 on success and 1 on any refusal, so `--check` is usable as a build gate.
Both modes print the vendored version on stdout and everything else on stderr.

## Development builds and `--check`

`scripts/dev-link` at the **workspace root** (not in this repository) fills the same
layout from a local build of `../handoff-mcp` and writes `VERSION = dev-<git sha>`. It
bypasses the lock on purpose and refuses to run when `CI` is set.

`--check` treats that version accordingly:

- on a developer machine it reports `[dev] …` and succeeds, so a linked checkout can be
  built and run;
- wherever `CI` is set it fails, so no packaged build can come from a locally built
  server.

`CI=1 node scripts/fetch-server.mjs --check` is therefore the strict form, usable by hand.
`node scripts/fetch-server.mjs` puts a dev-linked checkout back on the pinned artifact.
