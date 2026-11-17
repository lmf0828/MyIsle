$ErrorActionPreference = 'Continue'
$log = 'D:\LingDao\data\wireshark_install.log'
$done = 'D:\LingDao\data\wireshark_install.done'
Remove-Item $done -ErrorAction SilentlyContinue
try {
    & 'C:\Users\23048\AppData\Local\Microsoft\WindowsApps\winget.exe' install --id WiresharkFoundation.Wireshark -e --silent --accept-package-agreements --accept-source-agreements --disable-interactivity *> $log
    $code = $LASTEXITCODE
} catch {
    $_ | Out-File $log -Append
    $code = -1
}
"EXIT=$code" | Out-File $done -Encoding ascii
