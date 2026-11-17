param(
    [Parameter(Mandatory = $true)][string]$Version
)
# 统一把版本号写进 Cargo.toml 与 app.rc（CI 打 tag 时调用；本地也可用）。
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$Version = $Version.TrimStart('v')
if ($Version -notmatch '^\d+\.\d+\.\d+') { throw "版本号需形如 1.2.3，收到: $Version" }
$parts = $Version.Split('.')
while ($parts.Count -lt 4) { $parts += '0' }
$quad = ($parts[0..3] -join ',')

$cargo = Join-Path $root 'Cargo.toml'
$t = [IO.File]::ReadAllText($cargo, [Text.Encoding]::UTF8)
$t = [Regex]::Replace($t, '(?m)^version\s*=\s*"[^"]*"', "version = `"$Version`"", 1)
[IO.File]::WriteAllText($cargo, $t, (New-Object Text.UTF8Encoding($false)))

$rc = Join-Path $root 'app.rc'
$r = [IO.File]::ReadAllText($rc, [Text.Encoding]::UTF8)
$r = [Regex]::Replace($r, '(?m)^\s*FILEVERSION\s+[\d, ]+', " FILEVERSION $quad")
$r = [Regex]::Replace($r, '(?m)^\s*PRODUCTVERSION\s+[\d, ]+', " PRODUCTVERSION $quad")
$r = $r.Replace('"ProductVersion", "0.1.0"', "`"ProductVersion`", `"$Version`"")
$r = $r.Replace('"FileVersion", "0.1.0"', "`"FileVersion`", `"$Version`"")
$r = [Regex]::Replace($r, 'VALUE "ProductVersion", "[^"]*"', "VALUE `"ProductVersion`", `"$Version`"")
$r = [Regex]::Replace($r, 'VALUE "FileVersion", "[^"]*"', "VALUE `"FileVersion`", `"$Version`"")
[IO.File]::WriteAllText($rc, $r, (New-Object Text.UTF8Encoding($false)))

Write-Host "version set to $Version (quad $quad)"
