param(
    [string]$Version = "",
    [string]$Repo = ""
)
# 本地/CI 通用打包：生成 dist/LingDao-<ver>-portable.zip（可选 Setup + latest.json）。
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$genv = Join-Path $PSScriptRoot 'genv.ps1'
if (Test-Path $genv) { . $genv }

if ($Version) { & "$PSScriptRoot\set_version.ps1" -Version $Version }
$v = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches.Groups[1].Value
Write-Host "packing v$v"

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item (Join-Path $root 'target\release\lingdao.exe') (Join-Path $dist 'LingDao.exe') -Force
$zip = Join-Path $dist "LingDao-$v-portable.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $dist 'LingDao.exe') -DestinationPath $zip -Force
Write-Host "portable -> $zip"

# Inno Setup（若已安装）
$iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (!(Test-Path $iscc)) { $iscc = "$env:ProgramFiles\Inno Setup 6\ISCC.exe" }
$setup = Join-Path $dist "LingDao-Setup-$v.exe"
if (Test-Path $iscc) {
    & $iscc "/DAppVersion=$v" (Join-Path $root 'installer\LingDao.iss')
    if ($LASTEXITCODE -ne 0) { throw "iscc failed" }
    Write-Host "installer -> $setup"
} else {
    Write-Warning "未找到 Inno Setup（ISCC.exe），跳过安装包。安装后重跑即可。"
}

if (Test-Path $setup) {
    $hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
    $size = (Get-Item $setup).Length
    if (!$Repo) { $Repo = '<owner>/<repo>' }
    $obj = [ordered]@{
        version   = $v
        notes     = "i岛 v$v"
        url       = "https://github.com/$Repo/releases/download/v$v/LingDao-Setup-$v.exe"
        sha256    = $hash
        size      = $size
        mandatory = $false
        pub_date  = (Get-Date -Format 'yyyy-MM-dd')
    }
    $json = $obj | ConvertTo-Json
    [IO.File]::WriteAllText((Join-Path $dist 'latest.json'), $json, (New-Object Text.UTF8Encoding($false)))
    Write-Host "latest.json -> $dist\latest.json"
}
