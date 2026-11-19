//! 从「汽水音乐 / SodaMusic」Electron 渲染进程的私有堆内存里读取当前歌曲的官方 LRC 歌词。
//!
//! 背景：汽水把 `mediaDetail.lyric.content`（LRC 文本，UTF-16）明文存在渲染进程内存里；
//! 官方接口 api.qishui.com 需签名且反调试（加 --remote-debugging-port 会直接退出），
//! 所以这里不碰网络、不开调试端口，直接 OpenProcess + VirtualQueryEx + ReadProcessMemory
//! 扫描渲染进程私有堆，按时间轴还原 LRC。仅在「网络源都拿不到正文」时作为兜底。
//!
//! 防串歌：只接受「末句时间 ≈ 当前歌曲时长（±12%，最少±12s）且首句 <=35s」的歌词块；
//! 时长未知(<=0)时不使用，避免把上一首残留歌词贴到当前歌上。

use crate::lyrics::LrcLine;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Memory::{VirtualQueryEx, MEMORY_BASIC_INFORMATION};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

const MEM_COMMIT: u32 = 0x1000;
const MEM_PRIVATE: u32 = 0x20000;
const MEM_MAPPED: u32 = 0x40000;
const PAGE_NOACCESS: u32 = 0x01;
const PAGE_GUARD: u32 = 0x100;
const MAX_REGION: usize = 64 * 1024 * 1024;
const MAX_TOTAL: usize = 1536 * 1024 * 1024;
const MIN_REGION: usize = 64 * 1024;

pub fn soda_pids() -> Vec<u32> {
    let mut all: Vec<(u32, u32)> = Vec::new();
    unsafe {
        let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return Vec::new(),
        };
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut pe).is_ok() {
            loop {
                let name: OsString = OsString::from_wide(&pe.szExeFile);
                let s = name.to_string_lossy().to_lowercase();
                if s.contains("sodamusic") {
                    all.push((pe.th32ProcessID, pe.th32ParentProcessID));
                }
                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    // 主进程（父进程不是 SodaMusic 的那个）通常持有当前 mediaDetail/歌词，优先扫描它。
    let set: std::collections::HashSet<u32> = all.iter().map(|(p, _)| *p).collect();
    let mut ordered: Vec<u32> = Vec::new();
    for (p, _) in all.iter().filter(|(_, par)| !set.contains(par)) {
        ordered.push(*p);
    }
    for (p, _) in all.iter() {
        if !ordered.contains(p) {
            ordered.push(*p);
        }
    }
    ordered
}

fn allowed(c: char) -> bool {
    if c.is_whitespace() {
        return true;
    }
    let u = c as u32;
    (0x20..=0x7E).contains(&u)
        || (0xA0..=0xFF).contains(&u)
        || (0x2000..=0x206F).contains(&u)
        || (0x3000..=0x303F).contains(&u)
        || (0x4E00..=0x9FFF).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0xFF00..=0xFFEF).contains(&u)
}

fn parse_ts(inside: &str) -> Option<f64> {
    let (mm, rest) = inside.split_once(':')?;
    let mm: f64 = mm.trim().parse().ok()?;
    let (ss, frac) = if let Some((a, b)) = rest.split_once('.') {
        (a, b)
    } else if let Some((a, b)) = rest.split_once(':') {
        (a, b)
    } else {
        (rest, "")
    };
    let ss: f64 = ss.trim().parse().ok()?;
    let fracv = if frac.trim().is_empty() {
        0.0
    } else {
        let f = frac.trim();
        let d = f.len().min(3);
        f[..d].parse::<f64>().ok()? / 10f64.powi(d as i32)
    };
    Some(mm * 60.0 + ss + fracv)
}

/// 解析一行（可能带多个时间标签 `[t1][t2]text`），返回 (时间, 文本) 列表。
fn parse_line(line: &str) -> Vec<(f64, String)> {
    // 跳过行首可能残留的二进制/垃圾，定位到第一个 '[' 再解析时间标签。
    let mut s = match line.find('[') {
        Some(i) => &line[i..],
        None => return Vec::new(),
    };
    let mut times: Vec<f64> = Vec::new();
    while s.starts_with('[') {
        if let Some(end) = s.find(']') {
            match parse_ts(&s[1..end]) {
                Some(t) => {
                    times.push(t);
                    s = &s[end + 1..];
                }
                None => break,
            }
        } else {
            break;
        }
    }
    if times.is_empty() {
        return Vec::new();
    }
    let mut text = String::new();
    for c in s.chars() {
        if allowed(c) {
            text.push(c);
        } else {
            break;
        }
    }
    let text = text.trim().to_string();
    if text.is_empty() {
        return Vec::new();
    }
    times.into_iter().map(|t| (t, text.clone())).collect()
}

/// 从一段解码后的文本中挑出最可信的歌词块（时长校验 + 行数最多）。
fn best_block(s: &str, expected: f64) -> Option<(f64, Vec<LrcLine>)> {
    let mut blocks: Vec<Vec<(f64, String)>> = Vec::new();
    let mut cur: Vec<(f64, String)> = Vec::new();
    let mut prev = -1.0f64;
    for raw in s.split(['\n', '\r']) {
        for (t, text) in parse_line(raw) {
            if prev >= 0.0 && t < prev - 2.0 {
                if cur.len() >= 8 {
                    blocks.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
            cur.push((t, text));
            prev = t;
        }
    }
    if cur.len() >= 8 {
        blocks.push(cur);
    }

    let mut best: Option<(f64, Vec<LrcLine>)> = None;
    for b in blocks {
        let n = b.len() as f64;
        let first = b.first().unwrap().0;
        let last = b.last().unwrap().0;
        let score = if expected > 0.0 {
            let tol = (expected * 0.03).max(4.0);
            if first <= 35.0 && (last - expected).abs() <= tol {
                100000.0 + n * 10.0 - (last - expected).abs()
            } else {
                continue;
            }
        } else {
            n
        };
        if best.as_ref().map(|(bs, _)| score > *bs).unwrap_or(true) {
            let lines: Vec<LrcLine> = b
                .into_iter()
                .map(|(t, text)| LrcLine {
                    t,
                    text,
                    segs: Vec::new(),
                })
                .collect();
            best = Some((score, lines));
        }
    }
    best
}

/// 只保留字母/数字并小写，用于歌名比较（忽略空格、标点、全半角差异外的符号）。
fn key_of(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 从 s 的 start 位置（紧跟左引号之后）读取一个 JSON 字符串字面量，处理转义。
fn read_json_str(s: &str, start: usize) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = start;
    let mut out = String::new();
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'"' {
            return Some(out);
        }
        if c == b'\\' {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'u' => {
                    if i + 4 < bytes.len() {
                        if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 5]) {
                            if let Ok(v) = u32::from_str_radix(hex, 16) {
                                if let Some(ch) = char::from_u32(v) {
                                    out.push(ch);
                                }
                            }
                        }
                        i += 4;
                    }
                }
                other => out.push(other as char),
            }
            i += 1;
        } else {
            if !s.is_char_boundary(i) {
                i += 1;
                continue;
            }
            let ch = s[i..].chars().next()?;
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    None
}

/// 在内存文本里找 mediaDetail JSON 的 `"content":"...LRC..."`，并只接受「同一对象里 name 等于当前歌名」的那条。
/// 这样即便一大块内存里混着很多首歌，也不会串歌。JSON 里的换行是 `\n` 转义，必须反转义后再解析成歌词行。
fn json_lyric(s: &str, title_key: &str, expected: f64) -> Option<(f64, Vec<LrcLine>)> {
    let mut search = 0usize;
    let mut best: Option<(f64, Vec<LrcLine>)> = None;
    while let Some(rel) = s[search..].find("\"content\":\"") {
        let cs = search + rel + "\"content\":\"".len();
        if let Some(sval) = read_json_str(s, cs) {
            let instr = crate::lyrics::looks_instrumental(&sval);
            if sval.contains("[00:") || sval.trim_start().starts_with('[') || instr {
                // 找到这条 lyric 所属的 track 名（优先 "\"track\":{" 之后的第一个 "\"name\":\""，
                // 这样拿到的就是「歌名」，而不是专辑名/歌手名）。
                let mut lo = cs.saturating_sub(500_000);
                while lo < cs && !s.is_char_boundary(lo) {
                    lo += 1;
                }
                let pre = &s[lo..cs];
                let name = if let Some(trel) = pre.rfind("\"track\":{") {
                    let ts = lo + trel + "\"track\":{".len();
                    s[ts..cs].find("\"name\":\"").and_then(|nrel| {
                        read_json_str(s, ts + nrel + "\"name\":\"".len())
                    })
                } else {
                    let mut lo2 = cs.saturating_sub(20_000);
                    while lo2 < cs && !s.is_char_boundary(lo2) {
                        lo2 += 1;
                    }
                    s[lo2..cs]
                        .rfind("\"name\":\"")
                        .and_then(|nrel| read_json_str(s, lo2 + nrel + "\"name\":\"".len()))
                };
                if let Some(name) = name {
                    if key_of(&name) == title_key {
                        if instr {
                            // 歌词正文只是“纯音乐，请欣赏”：直接作为占位歌词返回。
                            if best.as_ref().map(|(bs, _)| 100000.0f64 > *bs).unwrap_or(true) {
                                best = Some((100000.0, crate::lyrics::instrumental_line()));
                            }
                        } else if let Some((sc, l)) = best_block(&sval, expected) {
                            if std::env::var("SODA_DBG").is_ok() {
                                eprintln!(
                                    "DBG json hit name={:?} sc={} n={} first={:?}",
                                    name,
                                    sc,
                                    l.len(),
                                    l.first().map(|x| x.text.clone())
                                );
                            }
                            if best.as_ref().map(|(bs, _)| sc > *bs).unwrap_or(true) {
                                best = Some((sc, l));
                            }
                        }
                    } else if std::env::var("SODA_DBG").is_ok() {
                        eprintln!("DBG json skip name={:?} (want {:?})", name, title_key);
                    }
                }
            }
        }
        search = cs;
    }
    best
}

fn scan_pid(pid: u32, title: &str, expected: f64) -> Option<(f64, Vec<LrcLine>)> {    scan_pid_lim(pid, title, expected, MAX_TOTAL)
}

fn scan_pid_lim(pid: u32, title: &str, expected: f64, limit: usize) -> Option<(f64, Vec<LrcLine>)> {
    let tl_key = key_of(title);
    unsafe {
        let h: HANDLE = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let mut addr: usize = 0;
        let mut total: usize = 0;
        let mut best: Option<(f64, Vec<LrcLine>)> = None;
        loop {
            let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
            let r = VirtualQueryEx(
                h,
                Some(addr as *const core::ffi::c_void),
                &mut mbi,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            );
            if r == 0 {
                break;
            }
            let base = mbi.BaseAddress as usize;
            let size = mbi.RegionSize;
            if size == 0 {
                break;
            }
            let st = mbi.State.0;
            let ty = mbi.Type.0;
            let pr = mbi.Protect.0;
            let readable = st == MEM_COMMIT
                && (ty == MEM_PRIVATE || ty == MEM_MAPPED)
                && pr != PAGE_NOACCESS
                && (pr & PAGE_GUARD) == 0
                && matches!(pr, 0x04 | 0x08 | 0x40)
                && size >= MIN_REGION;
            if readable && size <= MAX_REGION && total < limit {
                let mut buf = vec![0u8; size];
                let mut read: usize = 0;
                if ReadProcessMemory(
                    h,
                    base as *const core::ffi::c_void,
                    buf.as_mut_ptr() as *mut core::ffi::c_void,
                    size,
                    Some(&mut read),
                )
                .is_ok()
                    && read > 0
                {
                    total += read;
                    let slice = &buf[..read];
                    if slice.contains(&b'[') {
                        // UTF-16LE（汽水把歌词以 UTF-16 存在内存里）
                        let u16s: Vec<u16> = slice
                            .chunks_exact(2)
                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                            .collect();
                        let s16 = String::from_utf16_lossy(&u16s);
                        let s8 = String::from_utf8_lossy(slice);
                        // 1) 首选：mediaDetail JSON 里的 content（同一对象 name==当前歌，杜绝串歌）。
                        let jb = json_lyric(&s16, &tl_key, expected)
                            .or_else(|| json_lyric(&s8, &tl_key, expected));
                        if let Some((sc, l)) = jb {
                            if best.as_ref().map(|(bs, _)| sc > *bs).unwrap_or(true) {
                                best = Some((sc, l));
                            }
                        }
                        // 说明：不再使用「区域内出现歌名」的纯 LRC 兜底——实测会把残留的
                        // 别的歌的 LRC 块（时长恰好接近）串到当前歌上，故只信 mediaDetail
                        // JSON 里 name 精确匹配的 content。
                    }
                }
            }
            let next = base.saturating_add(size);
            if next <= addr {
                break;
            }
            addr = next;
        }
        let _ = CloseHandle(h);
        best
    }
}

/// 读取当前汽水歌曲的官方歌词。`expected` 为当前歌曲时长（秒，<=0 时不使用）。
pub fn read_for(title: &str, artist: &str, expected: f64) -> Option<Vec<LrcLine>> {
    if title.trim().is_empty() || expected < 30.0 {
        return None;
    }
    let pids = soda_pids();
    if pids.is_empty() {
        return None;
    }
    let mut best: Option<(f64, Vec<LrcLine>)> = None;
    for (i, pid) in pids.iter().enumerate() {
        // 主进程（第 0 个）给足预算；其余进程（渲染/工具进程）只给较小预算，控制失败时的耗时。
        let limit = if i == 0 { MAX_TOTAL } else { 256 * 1024 * 1024 };
        if let Some((sc, l)) = scan_pid_lim(*pid, title, expected, limit) {
            if best.as_ref().map(|(bs, _)| sc > *bs).unwrap_or(true) {
                best = Some((sc, l));
            }
            // 命中当前歌（已通过时长+歌名双校验）即可停止，避免继续扫其它进程。
            if sc >= 100000.0 {
                break;
            }
        }
    }
    let _ = artist;
    best.map(|(_, l)| l)
}

/// 诊断：在全部 SodaMusic 进程里搜索「当前歌名」并报告其所在区域是否也含歌词块，
/// 用于判断「歌词到底是没加载，还是在被过滤掉的区域里」。
pub fn probe_wide(title: &str, expected: f64) -> Vec<String> {
    let mut out = Vec::new();
    let t16: Vec<u8> = title.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let t8: Vec<u8> = title.as_bytes().to_vec();
    let find = |hay: &[u8], needle: &[u8]| -> Option<usize> {
        if needle.is_empty() || hay.len() < needle.len() {
            return None;
        }
        hay.windows(needle.len()).position(|w| w == needle)
    };
    for pid in soda_pids() {
        unsafe {
            let h = match OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) {
                Ok(h) => h,
                Err(_) => continue,
            };
            let mut addr: usize = 0;
            let mut total: usize = 0;
            loop {
                let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
                let r = VirtualQueryEx(
                    h,
                    Some(addr as *const core::ffi::c_void),
                    &mut mbi,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                );
                if r == 0 {
                    break;
                }
                let base = mbi.BaseAddress as usize;
                let size = mbi.RegionSize;
                if size == 0 {
                    break;
                }
                let st = mbi.State.0;
                let pr = mbi.Protect.0;
                let ty = mbi.Type.0;
                let readable = st == MEM_COMMIT && pr != PAGE_NOACCESS && (pr & PAGE_GUARD) == 0;
                if readable && size <= 256 * 1024 * 1024 && total < 4096 * 1024 * 1024 {
                    let mut buf = vec![0u8; size];
                    let mut read: usize = 0;
                    if ReadProcessMemory(
                        h,
                        base as *const core::ffi::c_void,
                        buf.as_mut_ptr() as *mut core::ffi::c_void,
                        size,
                        Some(&mut read),
                    )
                    .is_ok()
                        && read > 0
                    {
                        total += read;
                        let slice = &buf[..read];
                        let hit16 = find(slice, &t16);
                        let hit8 = find(slice, &t8);
                        let u16s: Vec<u16> = slice
                            .chunks_exact(2)
                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                            .collect();
                        let s16 = String::from_utf16_lossy(&u16s);
                        if hit16.is_some() || hit8.is_some() {
                            out.push(format!(
                                "TITLE pid={} base=0x{:X} size={}KB type=0x{:X} prot=0x{:X} hit16={:?} hit8={:?}",
                                pid, base, size / 1024, ty, pr, hit16, hit8
                            ));
                            if let Some((sc, l)) = best_block(&s16, expected) {
                                out.push(format!(
                                    "  BLOCK(dur) lines={} last={:.1} score={:.0}",
                                    l.len(),
                                    l.last().map(|x| x.t).unwrap_or(0.0),
                                    sc
                                ));
                            } else if let Some((_sc, l)) = best_block(&s16, 0.0) {
                                out.push(format!(
                                    "  BLOCK(any) lines={} first={:?} last={:.1}",
                                    l.len(),
                                    l.first().map(|x| x.text.clone()).unwrap_or_default(),
                                    l.last().map(|x| x.t).unwrap_or(0.0)
                                ));
                            }
                        } else if slice.contains(&b'[') {
                            if let Some((_sc, l)) = best_block(&s16, expected) {
                                out.push(format!(
                                    "BLOCK-only pid={} base=0x{:X} lines={} last={:.1}",
                                    pid,
                                    base,
                                    l.len(),
                                    l.last().map(|x| x.t).unwrap_or(0.0)
                                ));
                            }
                        }
                    }
                }
                let next = base.saturating_add(size);
                if next <= addr {
                    break;
                }
                addr = next;
            }
            let _ = CloseHandle(h);
        }
    }
    out
}

/// 诊断：列出内存里所有 mediaDetail 歌词块（可关联到的 track name + 近似行数），
/// 用来判断汽水是否把「队列里其它歌」的歌词预取进了内存，从而评估纯内存覆盖率。
pub fn probe_json_names() -> Vec<String> {
    let mut out = Vec::new();
    for pid in soda_pids() {
        unsafe {
            let h = match OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) {
                Ok(h) => h,
                Err(_) => continue,
            };
            let mut addr: usize = 0;
            let mut total: usize = 0;
            loop {
                let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
                let r = VirtualQueryEx(
                    h,
                    Some(addr as *const core::ffi::c_void),
                    &mut mbi,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                );
                if r == 0 {
                    break;
                }
                let base = mbi.BaseAddress as usize;
                let size = mbi.RegionSize;
                if size == 0 {
                    break;
                }
                let st = mbi.State.0;
                let pr = mbi.Protect.0;
                let readable = st == MEM_COMMIT && pr != PAGE_NOACCESS && (pr & PAGE_GUARD) == 0;
                if readable && size <= 64 * 1024 * 1024 && total < 2048 * 1024 * 1024 {
                    let mut buf = vec![0u8; size];
                    let mut read: usize = 0;
                    if ReadProcessMemory(
                        h,
                        base as *const core::ffi::c_void,
                        buf.as_mut_ptr() as *mut core::ffi::c_void,
                        size,
                        Some(&mut read),
                    )
                    .is_ok()
                        && read > 0
                    {
                        total += read;
                        let u16s: Vec<u16> = buf[..read]
                            .chunks_exact(2)
                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                            .collect();
                        let texts = [String::from_utf16_lossy(&u16s), String::from_utf8_lossy(&buf[..read]).into_owned()];
                        for s in texts.iter() {
                            let snap = |i: usize| -> usize {
                                let mut i = i.min(s.len());
                                while i > 0 && !s.is_char_boundary(i) {
                                    i -= 1;
                                }
                                i
                            };
                            let mut from = 0usize;
                            while let Some(rel) = s[from..].find("\"content\":\"") {
                                let pos = from + rel;
                                let hs = snap(pos.saturating_sub(400_000));
                                let mut name = "?".to_string();
                                if let Some(nr) = s[hs..pos].rfind("\"name\":\"") {
                                    let ns = hs + nr + 8;
                                    if let Some(e) = s[ns..].find('"') {
                                        let v = &s[ns..ns + e];
                                        if v.len() < 120 {
                                            name = v.to_string();
                                        }
                                    }
                                }
                                let cs = pos + 11;
                                let ce = snap((cs + 400_000).min(s.len()));
                                let cnt = s[cs..ce].matches("[0").count();
                                if cnt > 3 {
                                    out.push(format!("pid={} name=\"{}\" lines~{}", pid, name, cnt));
                                }
                                from = pos + 11;
                            }
                        }
                    }
                }
                let next = base.saturating_add(size);
                if next <= addr {
                    break;
                }
                addr = next;
            }
            let _ = CloseHandle(h);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 诊断：打印「当前歌名」在内存中出现位置前后 900 字符的上下文，
/// 用来观察汽水把这首歌的名字与歌词到底以什么结构存放（是否相邻、是什么键名）。
pub fn dump_near(title: &str) -> Vec<String> {
    let mut out = Vec::new();
    let t16: Vec<u8> = title.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    for pid in soda_pids() {
        unsafe {
            let h = match OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) {
                Ok(h) => h,
                Err(_) => continue,
            };
            let mut addr: usize = 0;
            let mut total: usize = 0;
            loop {
                let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
                let r = VirtualQueryEx(
                    h,
                    Some(addr as *const core::ffi::c_void),
                    &mut mbi,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                );
                if r == 0 {
                    break;
                }
                let base = mbi.BaseAddress as usize;
                let size = mbi.RegionSize;
                if size == 0 {
                    break;
                }
                let st = mbi.State.0;
                let pr = mbi.Protect.0;
                let readable = st == MEM_COMMIT && pr != PAGE_NOACCESS && (pr & PAGE_GUARD) == 0;
                if readable && size <= 64 * 1024 * 1024 && total < 2048 * 1024 * 1024 {
                    let mut buf = vec![0u8; size];
                    let mut read: usize = 0;
                    if ReadProcessMemory(
                        h,
                        base as *const core::ffi::c_void,
                        buf.as_mut_ptr() as *mut core::ffi::c_void,
                        size,
                        Some(&mut read),
                    )
                    .is_ok()
                        && read > 0
                    {
                        total += read;
                        let slice = &buf[..read];
                        let mut off = 0usize;
                        let mut n = 0;
                        while let Some(rel) = slice[off..]
                            .windows(t16.len())
                            .position(|w| w == t16.as_slice())
                        {
                            let at = off + rel;
                            n += 1;
                            if n > 3 {
                                break;
                            }
                            let st2 = at.saturating_sub(1800);
                            let en2 = (at + 1800).min(read);
                            let mut u16s: Vec<u16> = slice[st2..en2]
                                .chunks_exact(2)
                                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                                .collect();
                            for u in u16s.iter_mut() {
                                if *u < 0x20 || *u > 0x7e {
                                    if *u > 0x2000 {
                                        // keep CJK
                                    } else {
                                        *u = b'.' as u16;
                                    }
                                }
                            }
                            let s = String::from_utf16_lossy(&u16s);
                            out.push(format!("pid={} base=0x{:X} @{}: {}", pid, base, at, s));
                            off = at + t16.len();
                        }
                    }
                }
                let next = base.saturating_add(size);
                if next <= addr {
                    break;
                }
                addr = next;
            }
            let _ = CloseHandle(h);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore]
    fn probe_near_ignored() {
        let title = std::env::var("SODA_TITLE").unwrap_or_default();
        if title.is_empty() {
            eprintln!("set SODA_TITLE");
            return;
        }
        for line in super::dump_near(&title).into_iter().take(6) {
            eprintln!("NEAR {}", line);
        }
    }

    #[test]
    #[ignore]
    fn probe_json_names_ignored() {
        for line in super::probe_json_names() {
            eprintln!("{}", line);
        }
    }

    // 手动验证用：确认能从当前汽水渲染进程内存解析出歌词块（expected=0 只验证解析，不做时长校验）。
    #[test]
    #[ignore]
    fn probes_soda_memory() {
        let pids = super::soda_pids();
        if pids.is_empty() {
            eprintln!("no SodaMusic process");
            return;
        }
        let mut any = false;
        for p in pids {
            if let Some((sc, l)) = super::scan_pid(p, "", 0.0) {
                any = true;
                eprintln!(
                    "pid={} score={} lines={} first_t={:.2} first={:?}",
                    p,
                    sc,
                    l.len(),
                    l.first().map(|x| x.t).unwrap_or(0.0),
                    l.first().map(|x| x.text.clone()).unwrap_or_default()
                );
            }
        }
        assert!(any, "expected a lyric block in SodaMusic memory");
    }

    // 端到端：用假歌名让网络源必然失败，验证 fetch_full 的 soda 兜底链路能拿到内存歌词。
    #[test]
    #[ignore]
    fn probes_soda_via_fetch() {
        let pids = super::soda_pids();
        let (_, block) = pids
            .iter()
            .filter_map(|p| super::scan_pid(*p, "", 0.0))
            .next()
            .expect("need a lyric block in memory");
        let dur = block.last().unwrap().t;
        let (got, _cov) = crate::lyrics::fetch_full("__probe_none__", "__none__", dur, true);
        let lines = got.expect("soda fallback should return lyrics");
        eprintln!("soda fetch ok: lines={} dur={:.1}", lines.len(), dur);
        assert!(lines.len() >= 8);
    }

    // 诊断：真实走一遍 read_for（含标题校验），打印耗时与结果。
    #[test]
    #[ignore]
    fn probe_read_env() {
        let title = std::env::var("SODA_TITLE").unwrap_or_default();
        let artist = std::env::var("SODA_ARTIST").unwrap_or_default();
        let dur: f64 = std::env::var("SODA_DUR")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        if title.is_empty() {
            eprintln!("set SODA_TITLE");
            return;
        }
        let t0 = std::time::Instant::now();
        let r = super::read_for(&title, &artist, dur);
        match r {
            Some(v) => eprintln!(
                "read_for OK lines={} first={:?} last={:.1} elapsed_ms={}",
                v.len(),
                v.first().map(|x| x.text.clone()).unwrap_or_default(),
                v.last().map(|x| x.t).unwrap_or(0.0),
                t0.elapsed().as_millis()
            ),
            None => eprintln!("read_for NONE elapsed_ms={}", t0.elapsed().as_millis()),
        }
    }

    // 诊断：从环境变量 SODA_TITLE / SODA_DUR 取当前歌，报告标题/歌词块在哪些进程区域。
    #[test]
    #[ignore]
    fn probe_wide_env() {
        let title = std::env::var("SODA_TITLE").unwrap_or_default();
        let dur: f64 = std::env::var("SODA_DUR")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        if title.is_empty() {
            eprintln!("set SODA_TITLE");
            return;
        }
        let out = super::probe_wide(&title, dur);
        for l in &out {
            eprintln!("{}", l);
        }
        eprintln!("done: {} diagnostic lines", out.len());
    }

    // 单元测试：mediaDetail JSON（content 里是 \n 转义的整段 LRC）能被正确反转义 + 关联歌名。
    #[test]
    fn json_lyric_synthetic() {
        // 造 12 行 LRC（best_block 要求 >=8 行才认块），时间标签用 \n 转义形式塞进 JSON。
        let mut body = String::new();
        for i in 1..=12 {
            if i > 1 {
                body.push_str("\\n");
            }
            body.push_str(&format!("[00:{:02}.00]第{}行", i, i));
        }
        let s = format!(
            r#"{{"track":{{"id":"1","name":"测试歌","artists":[{{"name":"某歌手"}}]}},"lyric":{{"content":"{}","translations":[]}}}}"#,
            body
        );
        let tk = super::key_of("测试歌");
        let got = super::json_lyric(&s, &tk, 12.0).expect("should parse lyric by name");
        assert_eq!(got.1.len(), 12);
        assert_eq!(got.1[0].text, "第1行");
        // 不同歌名不应命中
        let other = super::key_of("别的歌");
        assert!(super::json_lyric(&s, &other, 12.0).is_none());
    }
}
