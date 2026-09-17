# Point handoff-app at a locally built handoff-mcp instead of the pinned release.
#
# It builds ..\handoff-mcp (pnpm build + the standalone binary for this platform) and
# fills handoff-app\vendor\ and handoff-app\src-tauri\binaries\ with the result, writing
# VERSION = dev-<git sha>. That is exactly the layout scripts\fetch-server.mjs produces,
# so the app cannot tell the difference — which is the point, and also the danger:
#
#   this bypasses handoff-app\server.lock.json.
#
# A build made this way is not reproducible from the app repository alone, so the script
# refuses to run when CI is set, and `fetch-server.mjs --check` fails on a dev-linked
# vendor wherever CI is set. Run `node scripts\fetch-server.mjs` inside handoff-app to go
# back to the pinned artifact.
#
# Usage: pwsh -File scripts/workspace/dev-link.ps1   (run from anywhere)

$ErrorActionPreference = 'Stop'

# This file is handoff-app/scripts/workspace/dev-link.ps1; the workspace is the folder that holds
# handoff-app, with handoff-mcp beside it.
$root = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
$mcp = Join-Path $root 'handoff-mcp'
$app = Join-Path $root 'handoff-app'

$formatDirs = @('schemas', 'patterns', 'protocol', 'fixtures', 'docs')

function Stop-DevLink([string]$Message) {
    [Console]::Error.WriteLine("dev-link: $Message")
    exit 1
}

function Invoke-Checked([string]$What, [scriptblock]$Command) {
    & $Command
    if ($LASTEXITCODE -ne 0) { Stop-DevLink "$What failed with exit code $LASTEXITCODE" }
}

if (-not [string]::IsNullOrEmpty($env:CI) -and $env:CI -ne '0' -and $env:CI -ne 'false') {
    Stop-DevLink 'CI is set. A build in CI takes the server from server.lock.json, never from a local build.'
}

if (-not (Test-Path (Join-Path $mcp '.git'))) {
    Stop-DevLink "$mcp is not a clone of handoff-mcp (run scripts/workspace/bootstrap.ps1)"
}
if (-not (Test-Path (Join-Path $app '.git'))) {
    Stop-DevLink "$app is not a clone of handoff-app (run scripts/workspace/bootstrap.ps1)"
}
foreach ($tool in @('node', 'pnpm', 'git')) {
    if ($null -eq (Get-Command $tool -ErrorAction SilentlyContinue)) {
        Stop-DevLink "$tool is required (scripts/workspace/bootstrap.ps1 reports what is missing)"
    }
}

'Building handoff-mcp (pnpm build + standalone binary)'
Push-Location $mcp
try {
    Invoke-Checked 'pnpm build:sea' { pnpm build:sea }

    # The asset name and the platform come from the build script itself, so the naming rule
    # of TECHNICAL-DESIGN §3.5 lives in one place.
    $info = (& node build/sea/build-sea.mjs --print-target) -join "`n" | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { Stop-DevLink 'build-sea.mjs --print-target failed' }
    $formatVersion = (& node build/format-tarball.mjs --print) -join "`n"
    if ($LASTEXITCODE -ne 0) { Stop-DevLink 'format-tarball.mjs --print failed' }
    $sha = (& git rev-parse --short HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { Stop-DevLink 'git rev-parse failed' }
}
finally {
    Pop-Location
}

$target = $info.target
$binary = Join-Path $mcp (Join-Path 'dist/sea' $info.asset)
if (-not (Test-Path $binary)) { Stop-DevLink "$binary was not produced by the build" }

# The Rust target triple Tauri appends to the name of an externalBin.
$triples = @{
    'win32-x64'    = 'x86_64-pc-windows-msvc'
    'darwin-x64'   = 'x86_64-apple-darwin'
    'darwin-arm64' = 'aarch64-apple-darwin'
}
if (-not $triples.ContainsKey($target)) {
    Stop-DevLink "$target is not a release target of TECHNICAL-DESIGN §3.5"
}
$triple = $triples[$target]
$exe = if ($target.StartsWith('win32-')) { '.exe' } else { '' }
$label = "dev-$sha"

# Everything is staged and swapped in at the end, so an interrupted run leaves the previous
# vendor tree alone instead of a half-filled one.
$vendor = Join-Path $app 'vendor/handoff-mcp'
$staging = Join-Path $app 'vendor/.staging-dev-link'
if (Test-Path $staging) { Remove-Item -Recurse -Force $staging }
$stagedFormat = Join-Path $staging 'format'
$stagedBin = Join-Path $staging "bin/$target"
New-Item -ItemType Directory -Force -Path $stagedFormat, $stagedBin | Out-Null

foreach ($dir in $formatDirs) {
    $source = Join-Path $mcp $dir
    if (-not (Test-Path $source)) {
        Stop-DevLink "$source is missing: is this really a handoff-mcp checkout?"
    }
    Copy-Item -Recurse -Force $source (Join-Path $stagedFormat $dir)
}
[IO.File]::WriteAllText((Join-Path $stagedFormat 'FORMAT-VERSION'), $formatVersion.TrimEnd("`n") + "`n")

Copy-Item -Force $binary (Join-Path $stagedBin "handoff-mcp$exe")
[IO.File]::WriteAllText((Join-Path $staging 'VERSION'), "$label`n")

if (Test-Path $vendor) { Remove-Item -Recurse -Force $vendor }
New-Item -ItemType Directory -Force -Path (Join-Path $app 'vendor') | Out-Null
Move-Item $staging $vendor

$binaries = Join-Path $app 'src-tauri/binaries'
New-Item -ItemType Directory -Force -Path $binaries | Out-Null
Copy-Item -Force (Join-Path $vendor "bin/$target/handoff-mcp$exe") (Join-Path $binaries "handoff-mcp-$triple$exe")

''
"  vendor   handoff-app/vendor/handoff-mcp is $label ($target)"
"  binary   handoff-app/src-tauri/binaries/handoff-mcp-$triple$exe"
''
'This bypasses handoff-app/server.lock.json. Check it with'
'  cd handoff-app; node scripts/fetch-server.mjs --check'
'and go back to the pinned release with'
'  cd handoff-app; node scripts/fetch-server.mjs'
