$ErrorActionPreference = 'Continue'
$dir = 'D:\LingDao\data\cap'
New-Item -ItemType Directory -Force $dir | Out-Null
$etl = Join-Path $dir 'cap.etl'
$pcap = Join-Path $dir 'cap.pcapng'
$done = Join-Path $dir 'cap.done'
Remove-Item $etl, $pcap, $done -ErrorAction SilentlyContinue
& pktmon filter remove *> $null
& pktmon filter add -p 443 *> $null
& pktmon start --capture --pkt-size 0 --file $etl *> $null
Start-Sleep -Seconds 30
& pktmon stop *> $null
& pktmon etl2pcap $etl -o $pcap *> $null
"done" | Out-File $done -Encoding ascii
