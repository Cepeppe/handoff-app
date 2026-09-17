#!/bin/sh
# Workspace bootstrap: clone handoff-mcp beside this checkout of handoff-app and report
# missing toolchains. Developer convenience only; nothing here is needed to build or run
# either repository. This script never installs anything.
#
# Usage: sh scripts/workspace/bootstrap.sh   (run from anywhere)

set -u

# This file is handoff-app/scripts/workspace/bootstrap.sh; the workspace is the folder that
# holds handoff-app, and the two repositories sit in it side by side.
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
OWNER=Cepeppe
MISSING=0

say() { printf '%s\n' "$*"; }

have() { command -v "$1" >/dev/null 2>&1; }

# report NAME COMMAND INSTALL_HINT [version-flag] [optional]
report() {
  name=$1
  cmd=$2
  hint=$3
  flag=${4:---version}
  optional=${5:-}
  if have "$cmd"; then
    version=$("$cmd" "$flag" 2>/dev/null | head -n 1)
    [ -n "$version" ] || version="present"
    say "  ok       $name: $version"
  elif [ -n "$optional" ]; then
    say "  optional $name: not found -> $hint"
  else
    say "  MISSING  $name -> $hint"
    MISSING=$((MISSING + 1))
  fi
}

# clang is needed by bindgen, which finds it through LIBCLANG_PATH rather than PATH,
# so an LLVM installation that is not on PATH still counts as present.
check_clang() {
  if have clang; then
    say "  ok       clang: $(clang --version 2>/dev/null | head -n 1)"
    return 0
  fi
  found=
  if [ -n "${LIBCLANG_PATH:-}" ] && have cygpath; then
    dir=$(cygpath -u "$LIBCLANG_PATH" 2>/dev/null)
    [ -n "$dir" ] && [ -x "$dir/clang.exe" ] && found="$dir/clang.exe"
  fi
  if [ -z "$found" ] && [ -x "/c/Program Files/LLVM/bin/clang.exe" ]; then
    found="/c/Program Files/LLVM/bin/clang.exe"
  fi
  if [ -n "$found" ]; then
    say "  ok       clang: $("$found" --version 2>/dev/null | head -n 1) (not on PATH, found at $found)"
    return 0
  fi
  say "  MISSING  clang -> $LLVM_HINT"
  MISSING=$((MISSING + 1))
}

clone() {
  repo=$1
  if [ -d "$ROOT/$repo/.git" ]; then
    say "  ok       $repo already cloned"
    return 0
  fi
  if [ -e "$ROOT/$repo" ]; then
    say "  MISSING  $repo exists but is not a git clone: inspect it by hand"
    MISSING=$((MISSING + 1))
    return 0
  fi
  say "  cloning  $OWNER/$repo"
  if have gh; then
    gh repo clone "$OWNER/$repo" "$ROOT/$repo" || {
      say "  MISSING  $repo could not be cloned (are you logged in? run: gh auth login)"
      MISSING=$((MISSING + 1))
    }
  elif have git; then
    git clone "https://github.com/$OWNER/$repo.git" "$ROOT/$repo" || {
      say "  MISSING  $repo could not be cloned"
      MISSING=$((MISSING + 1))
    }
  else
    say "  MISSING  $repo not cloned: install git first"
    MISSING=$((MISSING + 1))
  fi
}

case $(uname -s 2>/dev/null) in
  MINGW*|MSYS*|CYGWIN*) WINDOWS=1 ;;
  *) WINDOWS= ;;
esac

if [ -n "$WINDOWS" ]; then
  RUSTUP_HINT='winget install Rustlang.Rustup, then rustup default stable-x86_64-pc-windows-msvc'
  CMAKE_HINT='winget install Kitware.CMake'
  LLVM_HINT='winget install LLVM.LLVM (set LIBCLANG_PATH to its bin folder if bindgen fails)'
  MINISIGN_HINT='winget install jedisct1.minisign'
  NODE_HINT='winget install OpenJS.NodeJS.LTS'
  GH_HINT='winget install GitHub.cli, then gh auth login'
  GIT_HINT='winget install Git.Git'
  PWSH_HINT='winget install Microsoft.PowerShell'
else
  RUSTUP_HINT='https://rustup.rs, then rustup default stable'
  CMAKE_HINT='install cmake with your package manager'
  LLVM_HINT='install llvm/clang with your package manager'
  MINISIGN_HINT='install minisign with your package manager'
  NODE_HINT='install Node 22 LTS or newer with your package manager'
  GH_HINT='install the GitHub CLI, then gh auth login'
  GIT_HINT='install git with your package manager'
  PWSH_HINT='install PowerShell 7 (optional on this platform)'
fi

say "Workspace: $ROOT"
say ""
say "Repositories"
clone handoff-mcp
clone handoff-app

say ""
say "Toolchains (nothing is installed for you)"
report "git" git "$GIT_HINT"
report "gh" gh "$GH_HINT"
report "node" node "$NODE_HINT"
report "pnpm" pnpm "npm install -g pnpm"
report "rustup" rustup "$RUSTUP_HINT"
report "cargo" cargo "$RUSTUP_HINT"
report "rustc" rustc "$RUSTUP_HINT"
report "cargo-clippy" cargo-clippy "rustup component add clippy"
report "rustfmt" rustfmt "rustup component add rustfmt"
report "cmake" cmake "$CMAKE_HINT"
check_clang
report "minisign" minisign "$MINISIGN_HINT" -v
report "pwsh" pwsh "$PWSH_HINT" --version optional

if have cargo; then
  if cargo tauri --version >/dev/null 2>&1; then
    say "  ok       tauri-cli: $(cargo tauri --version 2>/dev/null | head -n 1)"
  else
    say "  MISSING  tauri-cli -> cargo install tauri-cli --version \"^2\""
    MISSING=$((MISSING + 1))
  fi
fi

if [ -n "$WINDOWS" ]; then
  VSWHERE="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
  if [ -x "$VSWHERE" ] && [ -n "$("$VSWHERE" -latest -products '*' \
      -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>/dev/null)" ]; then
    say "  ok       Visual Studio C++ build tools"
  else
    say "  MISSING  Visual Studio C++ build tools -> winget install Microsoft.VisualStudio.2022.BuildTools --override \"--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended\""
    MISSING=$((MISSING + 1))
  fi
fi

say ""
if [ "$MISSING" -gt 0 ]; then
  say "$MISSING item(s) missing. Install them, open a new shell (installers do not"
  say "refresh PATH in this one), then run this script again."
  exit 1
fi
say "All checks passed."
