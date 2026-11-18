param(
    [string]$UpdateVersion = "0.2.2",
    [string]$BaseVersion = "0.2.1",
    [switch]$NoLaunch
)
# Full interaction test with an *installed* baseline:
# 1) build WinIsle-Setup-<BaseVersion>.exe from the current working tree
# 2) silently install it to %LOCALAPPDATA%\Programs\WinIsle
# 3) point data\latest_test.json at WinIsle-Setup-<UpdateVersion>.exe
# 4) launch the INSTALLED baseline with the local manifest
# Clicking the update row then performs the real replace-and-restart flow.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$cargo = Join-Path $root "Cargo.toml"
$apprc = Join-Path $root "app.rc"
$cargoBak = Join-Path $root "data\_ver_cargo.bak"
$apprcBak = Join-Path $root "data\_ver_apprc.bak"
Copy-Item $cargo $cargoBak -Force
Copy-Item $apprc $apprcBak -Force
try {
    & "$PSScriptRoot\pack.ps1" -Version $BaseVersion
} finally {
    Copy-Item $cargoBak $cargo -Force
    Copy-Item $apprcBak $apprc -Force
    Remove-Item $cargoBak, $apprcBak -ErrorAction SilentlyContinue
}

$baseSetup = Join-Path $root "dist\WinIsle-Setup-$BaseVersion.exe"
if (!(Test-Path $baseSetup)) { throw "baseline setup missing: $baseSetup" }
Write-Host "installing baseline $baseSetup ..."
$p = Start-Process $baseSetup -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-" -PassThru -Wait
if ($p.ExitCode -ne 0) { throw "baseline install failed: $($p.ExitCode)" }

$installed = Join-Path $env:LOCALAPPDATA "Programs\WinIsle\WinIsle.exe"
if (!(Test-Path $installed)) { throw "installed exe missing: $installed" }
Write-Host "baseline installed -> $installed"

$setup = Join-Path $root "dist\WinIsle-Setup-$UpdateVersion.exe"
if (!(Test-Path $setup)) { throw "update setup missing: $setup (run test_update.ps1 -NoLaunch first)" }
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
$size = (Get-Item $setup).Length
$manifest = [ordered]@{
    version   = $UpdateVersion
    notes     = "local full interaction test $UpdateVersion"
    url       = "file:///" + ($setup -replace "\\", "/")
    sha256    = $hash
    size      = $size
    mandatory = $false
}
$manifestPath = Join-Path $root "data\latest_test.json"
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Compress), (New-Object Text.UTF8Encoding($false)))

$cfg = Join-Path $root "data\config.ini"
if (Test-Path $cfg) {
    $lines = Get-Content $cfg -Encoding UTF8 | Where-Object { $_ -notmatch "^upd_checked_day=" }
    [IO.File]::WriteAllLines($cfg, $lines, (New-Object Text.UTF8Encoding($false)))
}

Get-Process WinIsle -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800
if (!$NoLaunch) {
    $env:WINISLE_UPDATE_URL = $manifestPath
    $env:WINISLE_PAGE = "settings"
    $env:WINISLE_FORCE_EXPAND = "1"
    Start-Process $installed
    Write-Host "launched INSTALLED baseline $installed"
    Write-Host "update target: $setup"
}
