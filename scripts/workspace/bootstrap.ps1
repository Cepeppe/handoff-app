# Workspace bootstrap: clone handoff-mcp beside this checkout of handoff-app and report
# missing toolchains. Developer convenience only; nothing here is needed to build or run
# either repository. This script never installs anything.
#
# Usage: pwsh -File scripts/workspace/bootstrap.ps1   (run from anywhere)

$ErrorActionPreference = 'Stop'

# This file is handoff-app/scripts/workspace/bootstrap.ps1; the workspace is the folder that
# holds handoff-app, and the two repositories sit in it side by side.
$root = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
$owner = 'Cepeppe'
$script:missing = 0

function Test-Tool([string]$Name) {
    $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

function Report-Tool([string]$Name, [string]$Command, [string]$Hint, [string]$VersionFlag = '--version', [switch]$Optional) {
    if (Test-Tool $Command) {
        $version = ''
        try { $version = (& $Command $VersionFlag 2>$null | Select-Object -First 1) } catch { }
        if ([string]::IsNullOrWhiteSpace($version)) { $version = 'present' }
        "  ok       ${Name}: $version"
    }
    elseif ($Optional) {
        "  optional ${Name}: not found -> $Hint"
    }
    else {
        "  MISSING  $Name -> $Hint"
        $script:missing++
    }
}

# clang is needed by bindgen, which finds it through LIBCLANG_PATH rather than PATH,
# so an LLVM installation that is not on PATH still counts as present.
function Report-Clang([string]$Hint) {
    if (Test-Tool 'clang') {
        "  ok       clang: $(clang --version 2>$null | Select-Object -First 1)"
        return
    }
    $candidates = @()
    if ($env:LIBCLANG_PATH) { $candidates += (Join-Path $env:LIBCLANG_PATH 'clang.exe') }
    if ($env:ProgramFiles) { $candidates += (Join-Path $env:ProgramFiles 'LLVM\bin\clang.exe') }
    $found = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($found) {
        $version = (& $found --version 2>$null | Select-Object -First 1)
        "  ok       clang: $version (not on PATH, found at $found)"
        return
    }
    "  MISSING  clang -> $Hint"
    $script:missing++
}

function Clone-Repo([string]$Repo) {
    $path = Join-Path $root $Repo
    if (Test-Path (Join-Path $path '.git')) {
        "  ok       $Repo already cloned"
        return
    }
    if (Test-Path $path) {
        "  MISSING  $Repo exists but is not a git clone: inspect it by hand"
        $script:missing++
        return
    }
    "  cloning  $owner/$Repo"
    try {
        if (Test-Tool 'gh') { gh repo clone "$owner/$Repo" $path }
        elseif (Test-Tool 'git') { git clone "https://github.com/$owner/$Repo.git" $path }
        else { throw 'neither gh nor git is available' }
        if ($LASTEXITCODE -ne 0) { throw "clone exited with $LASTEXITCODE" }
    }
    catch {
        "  MISSING  $Repo could not be cloned ($_). Logged in? run: gh auth login"
        $script:missing++
    }
}

"Workspace: $root"
""
"Repositories"
Clone-Repo 'handoff-mcp'
Clone-Repo 'handoff-app'

""
"Toolchains (nothing is installed for you)"
Report-Tool 'git'          'git'          'winget install Git.Git'
Report-Tool 'gh'           'gh'           'winget install GitHub.cli, then gh auth login'
Report-Tool 'node'         'node'         'winget install OpenJS.NodeJS.LTS'
Report-Tool 'pnpm'         'pnpm'         'npm install -g pnpm'
Report-Tool 'rustup'       'rustup'       'winget install Rustlang.Rustup, then rustup default stable-x86_64-pc-windows-msvc'
Report-Tool 'cargo'        'cargo'        'winget install Rustlang.Rustup, then rustup default stable-x86_64-pc-windows-msvc'
Report-Tool 'rustc'        'rustc'        'winget install Rustlang.Rustup, then rustup default stable-x86_64-pc-windows-msvc'
Report-Tool 'cargo-clippy' 'cargo-clippy' 'rustup component add clippy'
Report-Tool 'rustfmt'      'rustfmt'      'rustup component add rustfmt'
Report-Tool 'cmake'        'cmake'        'winget install Kitware.CMake'
Report-Clang               'winget install LLVM.LLVM (set LIBCLANG_PATH to its bin folder if bindgen fails)'
Report-Tool 'minisign'     'minisign'     'winget install jedisct1.minisign' '-v'
Report-Tool 'pwsh'         'pwsh'         'winget install Microsoft.PowerShell' '--version' -Optional

if (Test-Tool 'cargo') {
    $tauri = ''
    try { $tauri = (cargo tauri --version 2>$null | Select-Object -First 1) } catch { }
    if ([string]::IsNullOrWhiteSpace($tauri)) {
        '  MISSING  tauri-cli -> cargo install tauri-cli --version "^2"'
        $script:missing++
    }
    else {
        "  ok       tauri-cli: $tauri"
    }
}

if ($IsWindows) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $vcPath = ''
    if (Test-Path $vswhere) {
        $vcPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    }
    if ([string]::IsNullOrWhiteSpace($vcPath)) {
        '  MISSING  Visual Studio C++ build tools -> winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"'
        $script:missing++
    }
    else {
        "  ok       Visual Studio C++ build tools: $vcPath"
    }
}

""
if ($script:missing -gt 0) {
    "$script:missing item(s) missing. Install them, open a new shell (installers do not"
    'refresh PATH in this one), then run this script again.'
    exit 1
}
'All checks passed.'
