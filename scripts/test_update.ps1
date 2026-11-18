param(
    [string]$Version = "0.2.2",
    [switch]$NoLaunch
)
# Local update test:
# 1) build a real WinIsle-Setup-<Version>.exe from the current working tree
#    (temporarily sets the version, then restores Cargo.toml/app.rc)
# 2) write data\latest_test.json pointing at that setup via file://
# 3) reset today's update-check marker
# 4) launch the currently installed (older) WinIsle.exe with the local manifest
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
    & "$PSScriptRoot\pack.ps1" -Version $Version
} finally {
    Copy-Item $cargoBak $cargo -Force
    Copy-Item $apprcBak $apprc -Force
    Remove-Item $cargoBak, $apprcBak -ErrorAction SilentlyContinue
}

$setup = Join-Path $root "dist\WinIsle-Setup-$Version.exe"
if (!(Test-Path $setup)) { throw "setup not produced: $setup" }
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
$size = (Get-Item $setup).Length
$url = "file:///" + ($setup -replace "\\", "/")
$manifest = [ordered]@{
    version   = $Version
    notes     = "local update test $Version"
    url       = $url
    sha256    = $hash
    size      = $size
    mandatory = $false
}
$json = $manifest | ConvertTo-Json -Compress
$manifestPath = Join-Path $root "data\latest_test.json"
[IO.File]::WriteAllText($manifestPath, $json, (New-Object Text.UTF8Encoding($false)))
Write-Host "manifest -> $manifestPath"
Write-Host "setup    -> $setup ($size bytes)"
Write-Host "sha256   -> $hash"

# clear today's upd_checked_day so the row is clickable
$cfg = Join-Path $root "data\config.ini"
if (Test-Path $cfg) {
    $lines = Get-Content $cfg -Encoding UTF8 | Where-Object { $_ -notmatch "^upd_checked_day=" }
    [IO.File]::WriteAllLines($cfg, $lines, (New-Object Text.UTF8Encoding($false)))
}

if (Get-Process WinIsle -ErrorAction SilentlyContinue) {
    Get-Process WinIsle | Stop-Process -Force
    Start-Sleep -Milliseconds 800
}
if (!$NoLaunch) {
    $exe = Join-Path $root "WinIsle.exe"
    $env:WINISLE_UPDATE_URL = $manifestPath
    $env:WINISLE_PAGE = "settings"
    Start-Process $exe
    Write-Host "launched $exe with WINISLE_UPDATE_URL=$manifestPath"
}
