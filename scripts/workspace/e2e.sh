#!/usr/bin/env bash
# Run the cross-repository end-to-end suite against local builds of both repositories
# (TECHNICAL-DESIGN §3.1, §11.5; T-043).
#
# The POSIX half of scripts/workspace/e2e.ps1, same steps in the same order:
#
#   1. the frontend bundle (dist/), because the app is built in the release profile and a
#      release build loads frontendDist rather than the Vite dev server;
#   2. the server the agent will run — the pinned release artifact, or a local build of
#      handoff-mcp with --dev-link;
#   3. the app binary with --features e2e, which is what opens the automation channel;
#   4. claude on PATH, logged in.
#
# Usage, from handoff-app:
#   bash scripts/workspace/e2e.sh                       every scenario
#   bash scripts/workspace/e2e.sh e2e-01-verified       only these
#   bash scripts/workspace/e2e.sh --dev-link            against a local build of handoff-mcp
#   bash scripts/workspace/e2e.sh --skip-build          reuse what is already built
#
# macOS is deferred (implementation decision 7), so this script has never been run there; it is
# written now because the app is written for both platforms and a suite that only exists on
# one of them is a suite that will not exist on the other.
set -euo pipefail

# This file is handoff-app/scripts/workspace/e2e.sh; handoff-mcp sits beside handoff-app.
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app="$(cd "$here/../.." && pwd)"
root="$(dirname "$app")"
server="$root/handoff-mcp"

dev_link=0
skip_build=0
scenarios=()
for argument in "$@"; do
  case "$argument" in
    --dev-link) dev_link=1 ;;
    --skip-build) skip_build=1 ;;
    -*) echo "e2e: unknown option $argument" >&2; exit 2 ;;
    *) scenarios+=("$argument") ;;
  esac
done

step() { printf '==> %s\n' "$1"; }

if [ "$dev_link" -eq 1 ] && [ ! -d "$server" ]; then
  echo "e2e: $server does not exist. Run scripts/workspace/bootstrap.sh first." >&2
  exit 2
fi
command -v claude >/dev/null 2>&1 || {
  echo 'e2e: claude is not on PATH. The e2e suite drives the real Claude Code.' >&2
  exit 2
}

if [ "$dev_link" -eq 1 ]; then
  step 'building handoff-mcp and linking it into the app (dev-link)'
  (cd "$server" && pnpm install --frozen-lockfile && pnpm build)
  sh "$here/dev-link.sh"
fi

cd "$app"
if [ "$skip_build" -eq 0 ]; then
  step 'installing the frontend dependencies'
  pnpm install --frozen-lockfile

  if [ "$dev_link" -eq 0 ]; then
    step 'fetching the pinned server (§3.5)'
    node scripts/fetch-server.mjs
  fi

  step 'building the frontend'
  pnpm build

  step 'building the app with --features e2e (release)'
  (cd src-tauri && cargo build --release --features e2e --bin handoff-app)
fi

step 'running the scenarios'
if [ "${#scenarios[@]}" -eq 0 ]; then
  pnpm e2e
else
  pnpm e2e -- "${scenarios[@]}"
fi
