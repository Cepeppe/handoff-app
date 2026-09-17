# `scripts/workspace/` — conveniences for both checkouts

These scripts are for a developer who has both repositories checked out side by side:

```
<workspace>/
├── handoff-mcp/
└── handoff-app/        this repository
```

Nothing here is required to build, test, release or run either repository: `handoff-mcp` and
`handoff-app` each build from a clean checkout of itself alone, and their CI pipelines prove it
([TECHNICAL-DESIGN §3.1](../../docs/design/TECHNICAL-DESIGN.md#31-workspace-root-dd-01)). If a
script here ever becomes necessary to produce an artifact, that is a bug in the repository, not a
missing script.

| Script | What it does |
|---|---|
| `bootstrap.sh` / `bootstrap.ps1` | Clone `handoff-mcp` beside this repository if it is missing, then report which developer toolchains are absent, with the exact command that installs each one. It never installs anything itself. |
| `dev-link.sh` / `dev-link.ps1` | Build `../handoff-mcp` and fill `vendor/` and `src-tauri/binaries/` from it, with `VERSION = dev-<git sha>`. **Bypasses `server.lock.json`**; refuses to run when `CI` is set. |
| `e2e.ps1` / `e2e.sh` | Build the app with `--features e2e` and run the end-to-end suite against a real agent ([`docs/dev/e2e.md`](../../docs/dev/e2e.md)). |

The scripts find the workspace from their own location, so they run from any folder:

```sh
sh scripts/workspace/bootstrap.sh          # Git Bash, macOS, Linux
sh scripts/workspace/dev-link.sh
bash scripts/workspace/e2e.sh
```

```powershell
pwsh -File scripts/workspace/bootstrap.ps1  # Windows
pwsh -File scripts/workspace/dev-link.ps1
scripts\workspace\e2e.ps1
```

`bootstrap` changes nothing apart from the clone: it prints what is missing and exits non-zero
if anything required is, so it is safe to run at any time.

## `dev-link` and the pinned artifact

`handoff-app` normally takes the server from one signed release, pinned by `server.lock.json`
and unpacked by [`scripts/fetch-server.mjs`](../README.md). `dev-link` writes the same layout
from a local build, so that a change to the server can be tried in the app before it is
released — and that is precisely why it is dangerous: the result is not reproducible from this
repository alone.

Two guards keep it out of anything that ships. `dev-link` itself refuses to start when `CI` is
set, and the version it writes (`dev-<git sha>`) makes `node scripts/fetch-server.mjs --check`
fail wherever `CI` is set, while on a developer machine that check reports the dev version and
succeeds. To go back to the pinned release:

```sh
node scripts/fetch-server.mjs
```
