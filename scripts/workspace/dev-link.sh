#!/bin/sh
# Point handoff-app at a locally built handoff-mcp instead of the pinned release.
#
# It builds ../handoff-mcp (pnpm build + the standalone binary for this platform) and
# fills handoff-app/vendor/ and handoff-app/src-tauri/binaries/ with the result, writing
# VERSION = dev-<git sha>. That is exactly the layout scripts/fetch-server.mjs produces,
# so the app cannot tell the difference — which is the point, and also the danger:
#
#   this bypasses handoff-app/server.lock.json.
#
# A build made this way is not reproducible from the app repository alone, so the script
# refuses to run when CI is set, and `fetch-server.mjs --check` fails on a dev-linked
# vendor wherever CI is set. Run `node scripts/fetch-server.mjs` inside handoff-app to go
# back to the pinned artifact.
#
# Usage: sh scripts/workspace/dev-link.sh   (run from anywhere)

set -eu

# This file is handoff-app/scripts/workspace/dev-link.sh; the workspace is the folder that holds
# handoff-app, with handoff-mcp beside it.
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
MCP="$ROOT/handoff-mcp"
APP="$ROOT/handoff-app"

FORMAT_DIRS='schemas patterns protocol fixtures docs'

die() {
  printf 'dev-link: %s\n' "$*" >&2
  exit 1
}

case "${CI:-}" in
  '' | 0 | false) ;;
  *) die 'CI is set. A build in CI takes the server from server.lock.json, never from a local build.' ;;
esac

[ -d "$MCP/.git" ] || die "$MCP is not a clone of handoff-mcp (run scripts/workspace/bootstrap.sh)"
[ -d "$APP/.git" ] || die "$APP is not a clone of handoff-app (run scripts/workspace/bootstrap.sh)"
for tool in node pnpm git; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is required (scripts/workspace/bootstrap.sh reports what is missing)"
done

printf 'Building handoff-mcp (pnpm build + standalone binary)\n'
(cd "$MCP" && pnpm build:sea)

# The asset name and the platform come from the build script itself, so the naming rule of
# TECHNICAL-DESIGN §3.5 lives in one place.
INFO=$(cd "$MCP" && node build/sea/build-sea.mjs --print-target)
TARGET=$(node -p "JSON.parse(process.argv[1]).target" "$INFO")
ASSET=$(node -p "JSON.parse(process.argv[1]).asset" "$INFO")
BINARY="$MCP/dist/sea/$ASSET"
[ -f "$BINARY" ] || die "$BINARY was not produced by the build"

# The Rust target triple Tauri appends to the name of an externalBin.
case "$TARGET" in
  win32-x64) TRIPLE=x86_64-pc-windows-msvc; EXE=.exe ;;
  darwin-x64) TRIPLE=x86_64-apple-darwin; EXE= ;;
  darwin-arm64) TRIPLE=aarch64-apple-darwin; EXE= ;;
  *) die "$TARGET is not a release target of TECHNICAL-DESIGN §3.5" ;;
esac

LABEL="dev-$(git -C "$MCP" rev-parse --short HEAD)"

# Everything is staged and swapped in at the end, so an interrupted run leaves the
# previous vendor tree alone instead of a half-filled one.
VENDOR="$APP/vendor/handoff-mcp"
STAGING="$APP/vendor/.staging-dev-link"
rm -rf "$STAGING"
mkdir -p "$STAGING/format" "$STAGING/bin/$TARGET"

for dir in $FORMAT_DIRS; do
  [ -d "$MCP/$dir" ] || die "$MCP/$dir is missing: is this really a handoff-mcp checkout?"
  cp -R "$MCP/$dir" "$STAGING/format/"
done
(cd "$MCP" && node build/format-tarball.mjs --print) > "$STAGING/format/FORMAT-VERSION"

cp "$BINARY" "$STAGING/bin/$TARGET/handoff-mcp$EXE"
chmod 755 "$STAGING/bin/$TARGET/handoff-mcp$EXE" 2>/dev/null || true
printf '%s\n' "$LABEL" > "$STAGING/VERSION"

rm -rf "$VENDOR"
mkdir -p "$APP/vendor"
mv "$STAGING" "$VENDOR"

mkdir -p "$APP/src-tauri/binaries"
cp "$VENDOR/bin/$TARGET/handoff-mcp$EXE" "$APP/src-tauri/binaries/handoff-mcp-$TRIPLE$EXE"

printf '\n  vendor   handoff-app/vendor/handoff-mcp is %s (%s)\n' "$LABEL" "$TARGET"
printf '  binary   handoff-app/src-tauri/binaries/handoff-mcp-%s%s\n' "$TRIPLE" "$EXE"
printf '\nThis bypasses handoff-app/server.lock.json. Check it with\n'
printf '  cd handoff-app && node scripts/fetch-server.mjs --check\n'
printf 'and go back to the pinned release with\n'
printf '  cd handoff-app && node scripts/fetch-server.mjs\n'
