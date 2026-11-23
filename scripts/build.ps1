# Builds Umbra from scratch to its installer:
#   1. the FFmpeg and libclang build dependencies (fetch-deps.ps1, skipped when present)
#   2. the UI (npm ci when the lock file changed, then npm run build)
#   3. the app and its NSIS installer (cargo tauri build; the Tauri CLI is installed if missing)
#   4. the files for a GitHub release, in out\release\<version>: the installer, a portable zip,
#      SHA256SUMS.txt and release-notes.md (that version's section of CHANGELOG.md)
#
#   powershell -ExecutionPolicy Bypass -File scripts\build.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$ui = Join-Path $root 'app\ui'

# Native programs don't stop the script on failure by themselves.
function Run([string]$what, [scriptblock]$command) {
    Write-Host "`n== $what" -ForegroundColor Cyan
    & $command
    if ($LASTEXITCODE -ne 0) { throw "$what failed (exit code $LASTEXITCODE)" }
}

foreach ($tool in 'cargo', 'npm') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "$tool is not installed or not on PATH (see README, Building)"
    }
}

Write-Host '== Build dependencies' -ForegroundColor Cyan
& (Join-Path $PSScriptRoot 'fetch-deps.ps1')

Push-Location $ui
try {
    # npm keeps a copy of the lock file it installed from; reinstall only when it differs.
    $installed = Join-Path $ui 'node_modules\.package-lock.json'
    $lock = Join-Path $ui 'package-lock.json'
    if (-not (Test-Path $installed) -or (Get-Item $lock).LastWriteTime -gt (Get-Item $installed).LastWriteTime) {
        Run 'UI packages' { npm ci --no-audit --no-fund }
    }
    Run 'UI' { npm run build }
} finally {
    Pop-Location
}

if (-not (Get-Command cargo-tauri -ErrorAction SilentlyContinue)) {
    Run 'Tauri CLI' { cargo install tauri-cli --version '^2' --locked }
}

Push-Location (Join-Path $root 'app\crates\umbra-app')
try {
    Run 'App and installer' { cargo tauri build }
} finally {
    Pop-Location
}

Write-Host "`n== Release files" -ForegroundColor Cyan
$conf = Get-Content (Join-Path $root 'app\crates\umbra-app\tauri.conf.json') -Raw | ConvertFrom-Json
$version = $conf.version
$built = Join-Path $root 'app\target\release'
$installer = Join-Path $built "bundle\nsis\Umbra_${version}_x64-setup.exe"
if (-not (Test-Path $installer)) { throw "the installer $installer was not built" }

$release = Join-Path $root "out\release\$version"
if (Test-Path $release) { Remove-Item -Recurse -Force $release }
New-Item -ItemType Directory -Force $release | Out-Null
Copy-Item $installer $release

# The portable zip: the app with the files the installer puts next to it (the bundle's
# resources: the FFmpeg DLLs and their license) and Umbra's own license.
$stage = Join-Path $release 'portable'
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item (Join-Path $built 'umbra.exe') $stage
foreach ($target in $conf.bundle.resources.PSObject.Properties.Value) {
    $to = Join-Path $stage $target
    New-Item -ItemType Directory -Force (Split-Path $to) | Out-Null
    Copy-Item (Join-Path $built $target) $to
}
New-Item -ItemType Directory -Force (Join-Path $stage 'licenses') | Out-Null
Copy-Item (Join-Path $root 'LICENSE') (Join-Path $stage 'licenses\Umbra-LICENSE.txt')
$portable = Join-Path $release "Umbra_${version}_x64-portable.zip"
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $portable
Remove-Item -Recurse -Force $stage

# The text for the release page: this version's section of the changelog, without its heading.
$changelog = Get-Content (Join-Path $root 'CHANGELOG.md') -Encoding UTF8
$start = [array]::IndexOf($changelog, "## $version")
if ($start -lt 0) { throw "CHANGELOG.md has no '## $version' section" }
$end = $start + 1
while ($end -lt $changelog.Count -and -not $changelog[$end].StartsWith('## ')) { $end++ }
$notes = ($changelog[($start + 1)..($end - 1)] -join "`n").Trim()
[IO.File]::WriteAllText((Join-Path $release 'release-notes.md'), "$notes`n")

$sums = Get-ChildItem $release -File | Where-Object Extension -in '.exe', '.zip' | ForEach-Object {
    "$((Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower())  $($_.Name)"
}
[IO.File]::WriteAllText((Join-Path $release 'SHA256SUMS.txt'), ($sums -join "`n") + "`n")

Write-Host "`nRelease files for $version in $release" -ForegroundColor Green
Get-ChildItem $release | ForEach-Object { Write-Host "  $($_.Name)" }

Start-Process explorer.exe "/select,`"$release`""
