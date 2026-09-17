# Run the cross-repository end-to-end suite against local builds of both repositories
# (TECHNICAL-DESIGN §3.1, §11.5; T-043, T-067).
#
# Four things have to exist before a scenario can run, and this script makes all four:
#
#   1. the frontend bundle (`dist/`), because the app is built in the release profile and a
#      release build loads `frontendDist` rather than the Vite dev server;
#   2. the server the agent will run — the pinned release artifact by default, or a local
#      build of `handoff-mcp` when -DevLink is given;
#   3. the app binary with `--features e2e`, which is what opens the automation channel;
#   4. the agent on PATH, logged in: `claude`, `codex` for the Codex subset, `opencode` for the
#      OpenCode subset, `cursor-agent` (Cursor's Agent CLI) for the Cursor subset, `copilot`
#      (the GitHub Copilot CLI) for the Copilot subset, or `kilo` (the Kilo CLI) for the Kilo
#      Code subset.
#
# Usage, from handoff-app:
#   scripts\workspace\e2e.ps1                       every scenario, against Claude Code
#   scripts\workspace\e2e.ps1 e2e-01-verified       only these
#   scripts\workspace\e2e.ps1 -Agent codex          the Codex subset, against the Codex CLI (T-067)
#   scripts\workspace\e2e.ps1 -Agent opencode       the OpenCode subset, against OpenCode (T-074)
#   scripts\workspace\e2e.ps1 -Agent cursor         the Cursor subset, against Cursor's editor and CLI (T-070)
#   scripts\workspace\e2e.ps1 -Agent copilot        the GitHub Copilot subset, against VS Code and the Copilot CLI (T-072)
#   scripts\workspace\e2e.ps1 -Agent kilo-code      the Kilo Code subset, against the Kilo CLI (T-081)
#   scripts\workspace\e2e.ps1 -DevLink              against a local build of handoff-mcp
#   scripts\workspace\e2e.ps1 -SkipBuild            reuse what is already built
#
# The suite talks to a real agent and costs real usage. It is run by hand
# (implementation decision 9); `e2e.yml` exists for the day that changes.

[CmdletBinding()]
param(
    # Scenario ids, or none for all of them.
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $Scenarios,

    # The agent to run against: Claude Code (every scenario), or Codex, OpenCode, Cursor,
    # GitHub Copilot or Kilo Code (their subsets).
    [ValidateSet('claude-code', 'codex', 'opencode', 'cursor', 'copilot', 'kilo-code')]
    [string] $Agent = 'claude-code',

    # Point the app at a local build of handoff-mcp instead of the pinned release.
    [switch] $DevLink,

    # Reuse whatever is already built. For a second run while a scenario is being written.
    [switch] $SkipBuild
)

$ErrorActionPreference = 'Stop'
# This file is handoff-app\scripts\workspace\e2e.ps1; handoff-mcp sits beside handoff-app.
$app = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$root = Split-Path -Parent $app
$server = Join-Path $root 'handoff-mcp'

function Step($what) { Write-Host "==> $what" -ForegroundColor Cyan }

if ($DevLink -and -not (Test-Path $server)) {
    throw "$server does not exist. Run scripts\workspace\bootstrap.ps1 first."
}
$program = switch ($Agent) {
    'claude-code' { 'claude' }
    'cursor' { 'cursor-agent' }
    'kilo-code' { 'kilo' }
    default { $Agent }
}
if (-not (Get-Command $program -ErrorAction SilentlyContinue)) {
    throw "$program is not on PATH. The e2e suite drives the real agent."
}

if ($DevLink) {
    Step 'building handoff-mcp and linking it into the app (dev-link)'
    Push-Location $server
    try {
        & pnpm install --frozen-lockfile
        if ($LASTEXITCODE -ne 0) { throw 'pnpm install failed in handoff-mcp' }
        & pnpm build
        if ($LASTEXITCODE -ne 0) { throw 'pnpm build failed in handoff-mcp' }
    } finally { Pop-Location }
    & (Join-Path $PSScriptRoot 'dev-link.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'dev-link failed' }
}

Push-Location $app
try {
    if (-not $SkipBuild) {
        Step 'installing the frontend dependencies'
        & pnpm install --frozen-lockfile
        if ($LASTEXITCODE -ne 0) { throw 'pnpm install failed in handoff-app' }

        if (-not $DevLink) {
            Step 'fetching the pinned server (§3.5)'
            & node scripts/fetch-server.mjs
            if ($LASTEXITCODE -ne 0) { throw 'fetch-server failed' }
        }

        Step 'building the frontend'
        & pnpm build
        if ($LASTEXITCODE -ne 0) { throw 'pnpm build failed in handoff-app' }

        Step 'building the app with --features e2e (release)'
        Push-Location (Join-Path $app 'src-tauri')
        try {
            & cargo build --release --features e2e --bin handoff-app
            # Smart App Control blocks a freshly linked binary at random on a machine where it is
            # on (T-002, T-035): re-running the identical command is the first thing to try, and
            # deleting the executable so cargo relinks it is the second.
            if ($LASTEXITCODE -ne 0) {
                Write-Warning 'the build failed once; retrying (Smart App Control blocks a fresh binary at random)'
                & cargo build --release --features e2e --bin handoff-app
            }
            if ($LASTEXITCODE -ne 0) { throw 'cargo build --features e2e failed' }
        } finally { Pop-Location }
    }

    Step "running the scenarios against $Agent"
    if ($Scenarios) { & pnpm e2e -- --agent $Agent @Scenarios } else { & pnpm e2e -- --agent $Agent }
    exit $LASTEXITCODE
} finally { Pop-Location }
