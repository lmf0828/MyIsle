param([switch]$Deploy = $true)
$ErrorActionPreference = 'Continue'
. 'D:\LingDao\scripts\genv.ps1'
Set-Location 'D:\LingDao'
$out = & cargo build --release 2>&1 | Out-String
$code = $LASTEXITCODE
($out -split "`n" | Select-String -Pattern 'error\[|error:|Finished') | ForEach-Object { $_.Line }
if ($code -ne 0) { throw "cargo build failed: $code" }
$exe = 'D:\LingDao\target\release\lingdao.exe'
"built: $((Get-Item $exe).Length) bytes"
if ($Deploy) {
    Get-Process LingDao -ErrorAction SilentlyContinue | Stop-Process -Force
    for ($i = 0; $i -lt 25 -and (Get-Process LingDao -ErrorAction SilentlyContinue); $i++) { Start-Sleep -Milliseconds 200 }
    if (Get-Process LingDao -ErrorAction SilentlyContinue) { throw "LingDao 进程未退出，无法覆盖 exe" }
    Copy-Item $exe 'D:\LingDao\LingDao.exe' -Force
    Start-Process 'D:\LingDao\LingDao.exe'
    "deployed & restarted"
}
