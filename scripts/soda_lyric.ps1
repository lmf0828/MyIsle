# SodaMusic lyric extractor: reads renderer process memory (no debug flags).
# Usage: powershell -ExecutionPolicy Bypass -File scripts\soda_lyric.ps1 [-All]
param([switch]$All)
$ErrorActionPreference = 'Stop'
$cs = @'
using System;
using System.Text;
using System.Text.RegularExpressions;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class SodaLyric {
  [StructLayout(LayoutKind.Sequential)]
  public struct MBI { public ulong BaseAddress; public ulong AllocationBase; public uint AllocationProtect; public uint a1; public ulong RegionSize; public uint State; public uint Protect; public uint Type; public uint a2; }
  [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint a, bool inh, int pid);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll")] static extern IntPtr VirtualQueryEx(IntPtr h, IntPtr addr, out MBI mbi, IntPtr len);
  [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr h, IntPtr addr, byte[] buf, IntPtr size, out IntPtr read);
  static readonly Regex lineRx = new Regex(@"\[\d{1,2}:\d{2}[\.:]\d{1,3}\][^\n\r]*");
  static double TimeOf(string s){ var m=Regex.Match(s,@"\[(\d{1,2}):(\d{2})[\.:](\d{1,3})\]"); if(!m.Success) return -1; double mm=int.Parse(m.Groups[1].Value); double ss=int.Parse(m.Groups[2].Value); double frac=int.Parse(m.Groups[3].Value)/1000.0; return mm*60+ss+frac; }
  static bool Allowed(char c){
    if(char.IsWhiteSpace(c)) return true; int u=(int)c;
    if(u>=0x20 && u<=0x7E) return true;
    if(u>=0x00A0 && u<=0x00FF) return true;
    if(u>=0x2000 && u<=0x206F) return true;
    if(u>=0x3000 && u<=0x303F) return true;
    if(u>=0x4E00 && u<=0x9FFF) return true;
    if(u>=0xF900 && u<=0xFAFF) return true;
    if(u>=0xFF00 && u<=0xFFEF) return true;
    return false;
  }
  static string Sanitize(string line){ var sb=new StringBuilder(); foreach(char c in line){ if(Allowed(c)) sb.Append(c); else break; } return sb.ToString().TrimEnd(); }
  static List<string> SplitBlocks(string s){
    var blocks=new List<string>(); var all=lineRx.Matches(s);
    var cur=new List<string>(); double prev=-1;
    foreach(Match m in all){ string ln=Sanitize(m.Value.Trim()); double t=TimeOf(ln); if(t<0) continue;
      if(prev>=0 && t < prev-2.0){ if(cur.Count>=8) blocks.Add(string.Join("\n",cur)); cur.Clear(); }
      cur.Add(ln); prev=t; }
    if(cur.Count>=8) blocks.Add(string.Join("\n",cur));
    return blocks;
  }
  public static List<string> ScanPid(int pid){
    var result=new List<string>(); IntPtr h=OpenProcess(0x0410,false,pid);
    if(h==IntPtr.Zero) return result; long addr=0; long total=0; int sz=Marshal.SizeOf(typeof(MBI));
    try{
      while(true){ MBI m; if(VirtualQueryEx(h,(IntPtr)addr,out m,(IntPtr)sz)==IntPtr.Zero) break;
        long baseA=(long)m.BaseAddress; long size=(long)m.RegionSize; if(size<=0) break;
        uint p=m.Protect;
        bool ok = (m.State==0x1000) && (m.Type==0x20000) && p!=0x01 && (p&0x100)==0 && (p==0x04||p==0x02||p==0x08||p==0x20||p==0x40);
        if(ok && size<=160L*1024*1024 && total<1500L*1024*1024){
          byte[] buf=new byte[size]; IntPtr rd;
          if(ReadProcessMemory(h,(IntPtr)baseA,buf,(IntPtr)size,out rd) && (long)rd>0){
            total+=(long)rd; int n=(int)rd;
            for(int off=0; off<2; off++){
              string s; try{ s=(off==0?Encoding.UTF8.GetString(buf,0,n):Encoding.Unicode.GetString(buf,0,n)); }catch{ continue; }
              if(s.IndexOf("[0")<0) continue;
              foreach(var b in SplitBlocks(s)) result.Add(b);
            }
          }
        }
        long next=baseA+size; if(next<=addr) break; addr=next;
      }
    } finally { CloseHandle(h); }
    return result;
  }
  public static string DecodeBest(int pid){
    var list=ScanPid(pid); string best=null; int bestN=0; var seen=new HashSet<string>();
    foreach(var b in list){ if(!seen.Add(b)) continue; int n=b.Split('\n').Length; if(n>bestN){bestN=n;best=b;} }
    return best;
  }
}
'@
Add-Type -TypeDefinition $cs -Language CSharp

$renderers = Get-CimInstance Win32_Process -Filter "Name='SodaMusic.exe'" | Where-Object { $_.CommandLine -like '*--type=renderer*' } | Select-Object -ExpandProperty ProcessId
if(-not $renderers){ Write-Output 'SodaMusic not running'; exit 1 }
$best = $null; $bestN = 0; $bestPid = 0
foreach($r in $renderers){
  $blocks = [SodaLyric]::ScanPid($r)
  if($All){
    $seen = @{}
    foreach($bb in $blocks){
      if($seen.ContainsKey($bb)){ continue }
      $seen[$bb] = $true
      $ls = $bb -split "`n"
      Write-Output ("[block] pid=$r lines=" + $ls.Count + " first=" + $ls[0] + "  last=" + $ls[-1])
    }
  }
  foreach($bb in $blocks){
    $n = ($bb -split "`n").Count
    if($n -gt $bestN){ $bestN=$n; $best=$bb; $bestPid=$r }
  }
}
if(-not $best){ Write-Output 'No lyrics found in memory (song may have no lyrics)'; exit 2 }
$out = 'D:\LingDao\data\soda_lyric.lrc'
[IO.File]::WriteAllText($out, $best, (New-Object System.Text.UTF8Encoding($false)))
$lines = $best -split "`n"
Write-Output ("pid=$bestPid lines=" + $lines.Count)
Write-Output ("first: " + $lines[0])
Write-Output ("last : " + $lines[-1])
Write-Output ("saved : $out")
