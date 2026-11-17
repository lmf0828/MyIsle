//! 歌词获取（网易云音乐公开接口，按 歌名+歌手 搜索取 LRC 并解析成时间轴），
//! 附带专辑封面下载。网络/查询均在后台线程执行，带磁盘缓存与重试，避免重复请求被限流(405)。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone)]
pub struct Seg {
    pub t: f64,   // 该字/词开始（秒）
    pub dur: f64, // 该字/词持续（秒）
    pub n: u32,   // 字数（不含空格）
}

#[derive(Clone)]
pub struct LrcLine {
    pub t: f64, // 秒（已按 [offset] 校正）
    pub text: String,
    pub segs: Vec<Seg>, // 逐字时间（来自 yrc）；为空表示只有整行时间
}

fn dlog(s: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::paths::file("lyrics_debug.log"))
    {
        let _ = writeln!(f, "{}", s);
    }
}

fn cache_dir() -> Option<PathBuf> {
    let d = crate::paths::ensure().join("cache");
    let _ = std::fs::create_dir_all(&d);
    Some(d)
}

fn key_hash(title: &str, artist: &str, dur_sec: f64) -> String {
    let mut h = DefaultHasher::new();
    title.hash(&mut h);
    artist.hash(&mut h);
    // 时长分桶（5 秒一档）：不同版本（录音室/现场）时长不同 → 歌词/时间轴也不同，必须分开缓存
    (((dur_sec / 5.0).round()) as i64).hash(&mut h);
    format!("{:016x}", h.finish())
}

fn pct_encode(s: &str) -> String {
    let mut o = String::new();
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{:02X}", b));
        }
    }
    o
}

fn get_json_with(url: &str, referer: &str) -> Option<serde_json::Value> {
    for attempt in 0..3 {
        let r = ureq::get(url)
            .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .set("Referer", referer)
            .timeout(Duration::from_secs(8))
            .call();
        match r {
            Ok(resp) => return resp.into_json::<serde_json::Value>().ok(),
            Err(e) => {
                dlog(&format!("GET err attempt{} {} -> {}", attempt, url, e));
                std::thread::sleep(Duration::from_millis(400));
            }
        }
    }
    None
}

fn get_json(url: &str) -> Option<serde_json::Value> {
    get_json_with(url, "https://music.163.com/")
}

fn get_bytes(url: &str) -> Option<Vec<u8>> {
    for attempt in 0..3 {
        let r = ureq::get(url)
            .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .set("Referer", "https://music.163.com/")
            .timeout(Duration::from_secs(10))
            .call();
        match r {
            Ok(resp) => {
                let mut buf = Vec::new();
                if resp.into_reader().read_to_end(&mut buf).is_ok() {
                    return Some(buf);
                }
            }
            Err(e) => {
                dlog(&format!("GETBYTES err attempt{} -> {}", attempt, e));
                std::thread::sleep(Duration::from_millis(400));
            }
        }
    }
    None
}

use std::io::Read;

fn parse_ts(s: &str) -> Option<f64> {
    let mut it = s.split(':');
    let m: f64 = it.next()?.trim().parse().ok()?;
    let sec: f64 = it.next()?.trim().parse().ok()?;
    Some(m * 60.0 + sec)
}

fn is_noise(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    // 作词/作曲/编曲/制作等署名信息保留在歌词列表中展示（不再过滤）
    if t.contains("纯音乐") || t.contains("此歌曲为没有填词的纯音乐") {
        return true;
    }
    false
}

fn filt(v: Vec<LrcLine>) -> Vec<LrcLine> {
    v.into_iter().filter(|x| !is_noise(&x.text)).collect()
}

/// 解析 LRC；支持一行多时间标签，并处理 [offset:±N]（毫秒）。
/// 网易云约定：offset 正值表示歌词整体提前，因此校正为 t - offset/1000。
pub fn parse_lrc(s: &str) -> Vec<LrcLine> {
    let mut out: Vec<LrcLine> = Vec::new();
    let mut offset_ms: f64 = 0.0;
    for raw in s.lines() {
        let mut rest = raw.trim();
        let mut times: Vec<f64> = Vec::new();
        while let Some(strip) = rest.strip_prefix('[') {
            if let Some(end) = strip.find(']') {
                let inner = &strip[..end];
                if let Some(off) = inner.strip_prefix("offset:") {
                    if let Ok(v) = off.trim().parse::<f64>() {
                        offset_ms = v;
                    }
                } else if let Some(t) = parse_ts(inner) {
                    times.push(t);
                }
                rest = &strip[end + 1..];
            } else {
                break;
            }
        }
        let text = rest.trim();
        if times.is_empty() || text.is_empty() {
            continue;
        }
        for t in times {
            out.push(LrcLine { t, text: text.to_string(), segs: Vec::new() });
        }
    }
    let off = offset_ms / 1000.0;
    if off != 0.0 {
        for l in out.iter_mut() {
            l.t -= off;
        }
    }
    out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// 解析网易云逐字歌词 yrc。格式：
/// `[行开始ms,行时长ms](字开始ms,字时长ms,0)字(字开始ms,字时长ms,0)字…`
/// 时间是相对整首歌的毫秒数。返回带 segs 的 LrcLine。
pub fn parse_yrc(s: &str) -> Vec<LrcLine> {
    let mut out: Vec<LrcLine> = Vec::new();
    for raw in s.lines() {
        let line = raw.trim();
        if !line.starts_with('[') {
            continue;
        }
        let close = match line.find(']') {
            Some(i) => i,
            None => continue,
        };
        let head = &line[1..close];
        let mut hp = head.split(',');
        let start_ms: f64 = match hp.next().and_then(|x| x.trim().parse().ok()) {
            Some(v) => v,
            None => continue,
        };
        let mut rest = &line[close + 1..];
        let mut segs: Vec<Seg> = Vec::new();
        let mut text = String::new();
        while rest.starts_with('(') {
            let end = match rest.find(')') {
                Some(i) => i,
                None => break,
            };
            let inner = &rest[1..end];
            let mut ip = inner.split(',');
            let ws: f64 = ip.next().and_then(|x| x.trim().parse().ok()).unwrap_or(start_ms);
            let wd: f64 = ip.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0.0);
            rest = &rest[end + 1..];
            let next_paren = rest.find('(').unwrap_or(rest.len());
            let wtext = rest[..next_paren].trim_end_matches(['\r', '\n']);
            let n = wtext.chars().filter(|c| !c.is_whitespace()).count() as u32;
            text.push_str(wtext);
            if n > 0 {
                segs.push(Seg { t: ws / 1000.0, dur: wd / 1000.0, n });
            }
            rest = &rest[next_paren..];
        }
        let text = text.trim().to_string();
        if segs.is_empty() || text.is_empty() {
            continue;
        }
        out.push(LrcLine { t: start_ms / 1000.0, text, segs });
    }
    out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn clean_title(t: &str) -> String {
    // 归一化各种空白（含全角空格、不换行空格、零宽字符），避免窗口标题里的特殊空格导致搜索失配
    let norm: String = t
        .chars()
        .map(|c| {
            if c.is_whitespace() || c == '\u{3000}' || c == '\u{00A0}' || c == '\u{200B}' || c == '\u{FEFF}' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut s = norm.split_whitespace().collect::<Vec<_>>().join(" ");
    for (op, cl) in [('(', ')'), ('（', '）'), ('[', ']'), ('【', '】')] {
        if let (Some(a), Some(b)) = (s.find(op), s.rfind(cl)) {
            if b > a {
                s = format!("{}{}", &s[..a], &s[b + cl.len_utf8()..]);
            }
        }
    }
    if let Some(i) = s.find(" - ") {
        s = s[..i].to_string();
    }
    s.trim().to_string()
}

/// 返回按匹配度降序的 (score, 歌曲 id, 时长秒) 候选列表（同名多版本极常见，需要逐个试并用时长区分版本）。
fn find_song_candidates(title: &str, artist: &str) -> Vec<(i32, i64, f64)> {
    let ctitle = clean_title(title);
    let q = format!("{} {}", ctitle, artist);
    let q = q.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let surl = format!(
        "https://music.163.com/api/search/get/web?s={}&type=1&limit=30&offset=0",
        pct_encode(q)
    );
    let sj = match get_json(&surl) {
        Some(x) => x,
        None => return Vec::new(),
    };
    let songs = match sj.get("result").and_then(|v| v.get("songs")).and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    let mut scored: Vec<(i32, i64, f64)> = Vec::new();
    for s in songs {
        let id = match s.get("id").and_then(|v| v.as_i64()) {
            Some(x) => x,
            None => continue,
        };
        let dur_ms = s.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let dur_s = dur_ms / 1000.0;
        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
        // 归一化比较：只保留字母数字（含中文），忽略空格/标点/大小写；
        // 歌名必须“完全相同”才算命中（候选带 (Live)/(cover) 等后缀会被排除）。
        let norm = |x: &str| -> String {
            x.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect::<String>()
        };
        let nt = norm(&ctitle);
        let nm = norm(name);
        let na = norm(artist);
        let mut sc = 0i32;
        if !nt.is_empty() && nm == nt {
            sc += 10; // 歌名完全一致
        } else if !nt.is_empty() && (nm.contains(&nt) || nt.contains(&nm)) {
            sc += 4; // 仅包含（可能是 cover/remix，低权重）
        }
        if let Some(ars) = s.get("artists").and_then(|v| v.as_array()) {
            let artist_ok = ars.iter().any(|a| {
                a.get("name").and_then(|v| v.as_str()).map(|n| {
                    let nn = norm(n);
                    !na.is_empty() && !nn.is_empty() && (nn == na || nn.contains(&na) || na.contains(&nn))
                }).unwrap_or(false)
            });
            if artist_ok {
                sc += 6; // 歌手一致
            }
        }
        scored.push((sc, id, dur_s));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
}

/// 归一化：只保留字母数字（含中文），忽略空格/标点/大小写；用于歌名/歌手比较。
fn norm_alpha(x: &str) -> String {
    x.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect::<String>()
}

// ===== 备用歌词源 1：QQ 音乐（smartbox 搜索 mid → 歌词接口），中文/抖音热歌覆盖好 =====
fn qq_search_mid(title: &str, artist: &str) -> Option<String> {
    let q = format!("{} {}", clean_title(title), artist.trim());
    let q = q.trim();
    if q.is_empty() {
        return None;
    }
    let url = format!(
        "https://c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg?key={}&format=json&g_tk=5381",
        pct_encode(q)
    );
    let v = get_json_with(&url, "https://y.qq.com/")?;
    let items = v
        .get("data")?
        .get("song")?
        .get("itemlist")?
        .as_array()?;
    let nt = norm_alpha(&clean_title(title));
    let na = norm_alpha(artist);
    let mut best: Option<(i32, String)> = None;
    for it in items {
        let name = it.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let singer = it.get("singer").and_then(|x| x.as_str()).unwrap_or("");
        let mid = it.get("mid").and_then(|x| x.as_str()).unwrap_or("");
        if mid.is_empty() {
            continue;
        }
        let nm = norm_alpha(name);
        let mut sc = 0i32;
        if !nt.is_empty() && nm == nt {
            sc += 10;
        } else if !nt.is_empty() && (nm.contains(&nt) || nt.contains(&nm)) {
            sc += 4;
        } else {
            continue; // 歌名必须相关
        }
        let nn = norm_alpha(singer);
        if !na.is_empty() && !nn.is_empty() && (nn == na || nn.contains(&na) || na.contains(&nn)) {
            sc += 6;
        }
        if best.as_ref().map(|(bs, _)| sc > *bs).unwrap_or(true) {
            best = Some((sc, mid.to_string()));
        }
    }
    best.map(|(_, mid)| mid)
}

fn qq_fetch(title: &str, artist: &str) -> Option<Vec<LrcLine>> {
    let mid = qq_search_mid(title, artist)?;
    let url = format!(
        "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg?songmid={}&format=json&nobase64=1",
        mid
    );
    let v = get_json_with(&url, "https://y.qq.com/")?;
    let lyr = v.get("lyric").and_then(|x| x.as_str()).unwrap_or("");
    if lyr.trim().is_empty() {
        return None;
    }
    let l = filt(parse_lrc(lyr));
    if l.len() >= 2 {
        dlog(&format!("qq lyric mid={} lines={}", mid, l.len()));
        Some(l)
    } else {
        None
    }
}

// ===== 备用歌词源 2：LRCLIB（https://lrclib.net，免登录，返回 LRC 同步歌词）=====
fn lrclib_fetch(title: &str, artist: &str, expected_dur: f64) -> Option<Vec<LrcLine>> {
    let ctitle = clean_title(title);
    if ctitle.is_empty() {
        return None;
    }
    let mut url = format!("https://lrclib.net/api/search?track_name={}", pct_encode(&ctitle));
    if !artist.trim().is_empty() {
        url.push_str(&format!("&artist_name={}", pct_encode(artist.trim())));
    }
    let v = get_json_with(&url, "https://lrclib.net/")?;
    let arr = v.as_array()?;
    let nt = norm_alpha(&ctitle);
    let na = norm_alpha(artist);
    let mut best: Option<(f64, &serde_json::Value)> = None;
    for it in arr {
        if it.get("instrumental").and_then(|x| x.as_bool()).unwrap_or(false) {
            continue;
        }
        let synced = it.get("syncedLyrics").and_then(|x| x.as_str()).unwrap_or("");
        if synced.trim().is_empty() {
            continue;
        }
        let tn = it.get("trackName").and_then(|x| x.as_str()).unwrap_or("");
        let an = it.get("artistName").and_then(|x| x.as_str()).unwrap_or("");
        let nm = norm_alpha(tn);
        let mut sc = 0.0f64;
        if !nt.is_empty() && nm == nt {
            sc += 10.0;
        } else if !nt.is_empty() && (nm.contains(&nt) || nt.contains(&nm)) {
            sc += 4.0;
        } else {
            continue;
        }
        let nn = norm_alpha(an);
        if !na.is_empty() && !nn.is_empty() && (nn == na || nn.contains(&na) || na.contains(&nn)) {
            sc += 6.0;
        }
        if expected_dur >= 20.0 {
            if let Some(d) = it.get("duration").and_then(|x| x.as_f64()) {
                sc += 10.0 - (d - expected_dur).abs().min(10.0); // 时长越接近分越高
            }
        }
        if best.as_ref().map(|(bs, _)| sc > *bs).unwrap_or(true) {
            best = Some((sc, it));
        }
    }
    let (_, it) = best?;
    let synced = it.get("syncedLyrics").and_then(|x| x.as_str()).unwrap_or("");
    let l = filt(parse_lrc(synced));
    if l.len() >= 2 {
        dlog(&format!("lrclib pick lines={}", l.len()));
        Some(l)
    } else {
        None
    }
}

/// 依次尝试备用源：QQ 音乐 → LRCLIB。返回带时间轴的歌词。
fn fetch_alternates(title: &str, artist: &str, expected_dur: f64) -> Option<Vec<LrcLine>> {
    if let Some(l) = qq_fetch(title, artist) {
        return Some(l);
    }
    lrclib_fetch(title, artist, expected_dur)
}

/// 拉取歌词 + 专辑封面图（JPEG/PNG 原始字节）。任一失败返回 None。
/// 带磁盘缓存：同一首（歌名+歌手哈希）只联网一次。
pub fn fetch_full(
    title: &str,
    artist: &str,
    expected_dur: f64,
    soda: bool,
) -> (Option<Vec<LrcLine>>, Option<Vec<u8>>) {
    // 传进来的可能是可疑试听片段时长（<5s/0 表示未知）：若已知整首时长就用它，避免同一首歌
    // 在“片段桶”和“整首桶”里各缓存一份、且选错版本。
    let mut expected_dur = expected_dur;
    if expected_dur < 5.0 {
        if let Some(kd) = known_duration(title, artist) {
            if kd >= 20.0 {
                expected_dur = kd;
            }
        }
    }
    let key = key_hash(title, artist, expected_dur);
    let dir = cache_dir();
    if let Some(d) = &dir {
        let lp = d.join(format!("{}.lrc", key));
        let yp = d.join(format!("{}.yrc", key));
        let cp = d.join(format!("{}.cover", key));
        let np = d.join(format!("{}.nolyric", key));
        // 负缓存：之前确认过“确实没有歌词”（网易云/QQ/LRCLIB 都没有），6 小时内不再重复联网。
        let no_recent = std::fs::metadata(&np)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .map(|e| e.as_secs() < 6 * 3600)
            .unwrap_or(false);
        // 旧版缓存可能存过“被试听片段时长压扁”的歌词：整首时长远大于缓存最后一句时间 → 判为坏缓存，重新联网取。
        let known_full = known_duration(title, artist).unwrap_or(0.0);
        let stale = |v: &Vec<LrcLine>| {
            let last = v.last().map(|l| l.t).unwrap_or(0.0);
            // 旧版缓存可能存过“被试听片段时长压扁”的歌词
            let squashed = known_full >= 120.0 && last < known_full * 0.4;
            // 缓存歌词末句时间与当前实际时长差太多 → 多半是“同名不同歌”的错误匹配缓存，判为坏缓存重新取。
            let wrong_song =
                expected_dur >= 60.0 && last >= 1.0 && (last - expected_dur).abs() > (expected_dur * 0.15).max(15.0);
            squashed || wrong_song
        };
        // 汽水歌曲：歌词只认「汽水内存读到的官方 LRC」；把它单独缓存成 {key}.soda.lrc，
        // 记住这首读成功过，下次直接命中（不必每次都让汽水重新渲染歌词页）。
        // 绝不读取任何网络来源的 .lrc/.yrc 缓存（避免历史错配又被读出来）。
        let soda_cache = d.join(format!("{}.soda.lrc", key));
        let cached_lines = if soda {
            std::fs::read_to_string(&soda_cache)
                .ok()
                .map(|s| filt(parse_lrc(&s)))
                .filter(|v| !v.is_empty() && !stale(v))
        } else {
            std::fs::read_to_string(&yp)
                .ok()
                .map(|s| filt(parse_yrc(&s)))
                .filter(|v| !v.is_empty() && !stale(v))
                .or_else(|| {
                    std::fs::read_to_string(&lp)
                        .ok()
                        .map(|s| filt(parse_lrc(&s)))
                        .filter(|v| !v.is_empty() && !stale(v))
                })
        };
        let cached_cover = std::fs::read(&cp).ok().filter(|v| !v.is_empty());
        if cached_lines.is_some() || cached_cover.is_some() {
            dlog(&format!("cache hit {} lines={} cover={}B", title, cached_lines.as_ref().map(|v| v.len()).unwrap_or(0), cached_cover.as_ref().map(|v| v.len()).unwrap_or(0)));
            if cached_lines.is_some() && cached_cover.is_some() {
                return (cached_lines, cached_cover);
            }
            if cached_lines.is_none() && no_recent {
                dlog(&format!("nolyric cached {}", title));
                return (None, cached_cover);
            }
            // 缓存不全则继续联网补齐（保留已有部分）
            let (l2, c2) = fetch_remote(title, artist, expected_dur, &key, dir.as_ref(), soda);
            if l2.is_none() && !soda {
                let _ = std::fs::write(&np, b"1");
            }
            return (l2.or(cached_lines), c2.or(cached_cover));
        }
    }
    let (l, c) = fetch_remote(title, artist, expected_dur, &key, dir.as_ref(), soda);
    if l.is_none() {
        // 汽水不写“无歌词”负缓存：歌词可能只是还没加载，下次播放应重新读内存。
        if let Some(d) = &dir {
            if !soda {
                let _ = std::fs::write(d.join(format!("{}.nolyric", key)), b"1");
            }
        }
    }
    (l, c)
}

/// 汽水音乐在播时：歌词只认汽水渲染进程内存里的官方 LRC（不跨源拼凑，避免错配）；
/// 封面仍尽量从网易云按歌名取（仅封面，不用其歌词）。取不到歌词就返回空（显示“暂无歌词”）。
fn fetch_remote(
    title: &str,
    artist: &str,
    expected_dur: f64,
    key: &str,
    dir: Option<&PathBuf>,
    soda: bool,
) -> (Option<Vec<LrcLine>>, Option<Vec<u8>>) {
    if soda {
        // 汽水歌曲：歌词只认汽水自己的官方数据，绝不跨源适配，从根上杜绝“把别首歌贴上来”。
        // 首选：通过 CDP 读渲染进程里 LyricsView 的官方歌词（含逐字时间，不需要显示歌词页）。
        if let Some(sl) = crate::cdp_lyric::fetch_current(title, artist) {
            let l = crate::cdp_lyric::parse_content(&sl.content);
            if l.len() >= 2 {
                dlog(&format!(
                    "soda cdp {} - {} dur={:.1} lines={}",
                    sl.name,
                    sl.artist,
                    sl.duration,
                    l.len()
                ));
                if let Some(d) = dir {
                    let body: String = l
                        .iter()
                        .map(|x| format!("[{:02}:{:05.2}]{}\n", (x.t as u32) / 60, x.t % 60.0, x.text))
                        .collect();
                    let _ = std::fs::write(d.join(format!("{}.soda.lrc", key)), body);
                }
                let cover = netease_cover_only(title, artist, expected_dur);
                return (Some(l), cover);
            }
        }
        // 兜底：直接从汽水进程内存里扫官方 LRC（name 精确匹配）。
        if let Some(l) = soda_read_with_retry(title, artist, expected_dur) {
            dlog(&format!("soda lyric {} - {} lines={}", title, artist, l.len()));
            // 记住「这首从汽水内存成功读到过官方歌词」，写成 {key}.soda.lrc，下次直接命中。
            if let Some(d) = dir {
                let body: String = l
                    .iter()
                    .map(|x| format!("[{:02}:{:05.2}]{}\n", (x.t as u32) / 60, x.t % 60.0, x.text))
                    .collect();
                let _ = std::fs::write(d.join(format!("{}.soda.lrc", key)), body);
            }
            let cover = netease_cover_only(title, artist, expected_dur);
            return (Some(l), cover);
        }
        dlog(&format!("soda none {} - {} (pure memory, no network)", title, artist));
        let cover = netease_cover_only(title, artist, expected_dur);
        return (None, cover);
    }
    fetch_remote_net(title, artist, expected_dur, key, dir)
}

/// 汽水歌词刚开播可能还没写进内存：短轮询几次（最多约 6 秒），让 汽水 有时间加载歌词。
fn soda_read_with_retry(title: &str, artist: &str, expected_dur: f64) -> Option<Vec<LrcLine>> {
    if expected_dur < 30.0 {
        return crate::soda_lyrics::read_for(title, artist, expected_dur);
    }
    for attempt in 0..3 {
        if let Some(l) = crate::soda_lyrics::read_for(title, artist, expected_dur) {
            return Some(l);
        }
        if attempt < 2 {
            std::thread::sleep(std::time::Duration::from_millis(1200));
        }
    }
    None
}

/// 仅取封面（网易云按歌名找一个时长接近的候选，只用它的 album.picUrl），不返回歌词。
fn netease_cover_only(title: &str, artist: &str, expected_dur: f64) -> Option<Vec<u8>> {
    let cands = find_song_candidates(title, artist);
    if cands.is_empty() {
        return None;
    }
    let mut idxs: Vec<usize> = (0..cands.len()).filter(|&i| cands[i].0 >= 10).collect();
    if idxs.is_empty() {
        idxs = (0..cands.len()).filter(|&i| cands[i].0 >= 4).collect();
    }
    if idxs.is_empty() {
        return None;
    }
    idxs.sort_by(|&i, &j| {
        let ci = (cands[i].2 - expected_dur).abs();
        let cj = (cands[j].2 - expected_dur).abs();
        ci.partial_cmp(&cj)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(cands[j].0.cmp(&cands[i].0))
    });
    for &ci in idxs.iter().take(5) {
        let (_sc, id, cd) = cands[ci];
        if expected_dur >= 30.0 && cd >= 5.0 && (cd - expected_dur).abs() > (expected_dur * 0.10).max(10.0) {
            continue;
        }
        let durl = format!("https://music.163.com/api/song/detail?ids=[{}]", id);
        if let Some(dj) = get_json(&durl) {
            if let Some(url) = dj
                .get("songs")
                .and_then(|v| v.as_array())
                .and_then(|a| a.get(0))
                .and_then(|s0| s0.get("album"))
                .and_then(|al| al.get("picUrl"))
                .and_then(|u| u.as_str())
            {
                if !url.is_empty() {
                    let u2 = if url.contains('?') {
                        url.to_string()
                    } else {
                        format!("{}?param=400y400", url)
                    };
                    if let Some(b) = get_bytes(&u2) {
                        return Some(b);
                    }
                }
            }
        }
    }
    None
}

fn fetch_remote_net(
    title: &str,
    artist: &str,
    expected_dur: f64,
    key: &str,
    dir: Option<&PathBuf>,
) -> (Option<Vec<LrcLine>>, Option<Vec<u8>>) {
    // 优先从网易云客户端本地播放队列里解析出「确切的 song id」，再按 id 取歌词，
    // 从根上避免「靠歌名搜索 → 同名歌曲/翻唱/伴奏错配」（如 Keep the Roses→roses）。
    let mem_id = crate::netease_mem::current_song_id(title, artist, expected_dur);
    let cands: Vec<(i32, i64, f64)> = match mem_id {
        Some(id) => {
            dlog(&format!("netease mem id={} for {} - {}", id, title, artist));
            vec![(100, id, expected_dur)]
        }
        None => find_song_candidates(title, artist),
    };
    if cands.is_empty() {
        dlog(&format!("no song id for {} - {}", title, artist));
        return (None, None);
    }
    // 精确度优先：先用"歌名完全一致 + 歌手一致"(>=16) 的候选；
    // 没有再用"歌名完全一致"(>=10)；再没有才退到"仅包含"(>=4)。绝不接受只靠歌手匹配的其它歌。
    let mut pool: Vec<usize> = (0..cands.len()).filter(|&i| cands[i].0 >= 16).collect();
    if pool.is_empty() {
        pool = (0..cands.len()).filter(|&i| cands[i].0 >= 10).collect();
    }
    if pool.is_empty() {
        pool = (0..cands.len()).filter(|&i| cands[i].0 >= 4).collect();
    }
    // 版本优先：同一池内，时长与"当前播放器实际时长"最接近的排前面。
    // 但若 expected_dur 疑似 VIP 试听片段（<60s，且有明显更长的完整版本），
    // 就不能按“接近片段时长”选（会挑到 30s 试听版歌词）——改为按匹配分、再按时长从长到短。
    let trial = expected_dur < 5.0
        || (expected_dur < 60.0 && cands.iter().any(|c| c.2 > expected_dur * 1.5));
    let mut idxs: Vec<usize> = pool;
    idxs.sort_by(|&i, &j| {
        if trial {
            // 只按匹配分排序（稳定排序保留搜索原始相关度顺序），不按时长挑——
            // 否则可能挑到更长的现场/重制版。完整版搜索结果的 dt 本来就是整首时长。
            return cands[j].0.cmp(&cands[i].0);
        }
        let ci = (cands[i].2 - expected_dur).abs();
        let cj = (cands[j].2 - expected_dur).abs();
        let class_i = if expected_dur > 5.0 && ci > 8.0 { 1 } else { 0 };
        let class_j = if expected_dur > 5.0 && cj > 8.0 { 1 } else { 0 };
        class_i
            .cmp(&class_j)
            .then(ci.partial_cmp(&cj).unwrap_or(std::cmp::Ordering::Equal))
            .then(cands[j].0.cmp(&cands[i].0))
    });

    // 逐个候选尝试取歌词：只试“强匹配”候选（歌名精确或歌手命中），
    // 跳过“暂无歌词/纯音乐/空”的版本，取第一个有真实歌词的；都无则视为无歌词。
    let mut lines: Option<Vec<LrcLine>> = None;
    let mut chosen_id: Option<i64> = None;
    let mut chosen_yrc: Option<String> = None;
    let mut chosen_dur: f64 = 0.0;
    // 第一轮：在强匹配候选里优先找“带逐字(yrc)”的版本（同名歌曲不同上传，有的有 yrc）
    for &ci in idxs.iter().take(12) {
        let (sc, id, _cd) = cands[ci];
        // 时长闸门：只在“非精确（歌名+歌手）匹配”(sc<16) 时启用；精确匹配直接信它，
        // 避免误杀（网易云候选的时长元数据有时不准）。Keep the Roses(sc=4) 这类才会被拦。
        if sc < 16 && expected_dur >= 30.0 && _cd >= 5.0 && (_cd - expected_dur).abs() > (expected_dur * 0.10).max(10.0) {
            dlog(&format!("skip yrc dur id={} cd={:.1} exp={:.1}", id, _cd, expected_dur));
            continue;
        }
        let lurl = format!("https://music.163.com/api/song/lyric?id={}&lv=-1&kv=-1&tv=-1&yv=-1&yrv=-1", id);
        if let Some(lj) = get_json(&lurl) {
            let yrc_txt = lj.get("yrc").and_then(|v| v.get("lyric")).and_then(|v| v.as_str()).unwrap_or("");
            if !yrc_txt.trim().is_empty() {
                let l = filt(parse_yrc(yrc_txt));
                let bad = l.iter().any(|x| x.text.contains("暂无歌词") || x.text.contains("纯音乐"));
                if l.len() >= 2 && !bad {
                    dlog(&format!("yrc pick id={} sc={} lines={}", id, sc, l.len()));
                    lines = Some(l);
                    chosen_id = Some(id);
                    chosen_yrc = Some(yrc_txt.to_string());
                    chosen_dur = _cd;
                    break;
                }
            }
        }
    }
    // 第二轮：没有逐字版本时，退回整行 lrc
    if lines.is_none() {
        for &ci in idxs.iter().take(12) {
            let (sc, id, _cd) = cands[ci];
            // 时长闸门：同上，只在非精确匹配(sc<16)时启用。
            if sc < 16 && expected_dur >= 30.0 && _cd >= 5.0 && (_cd - expected_dur).abs() > (expected_dur * 0.10).max(10.0) {
                dlog(&format!("skip lrc dur id={} cd={:.1} exp={:.1}", id, _cd, expected_dur));
                continue;
            }
            let lurl = format!("https://music.163.com/api/song/lyric?id={}&lv=-1&kv=-1&tv=-1", id);
            if let Some(lj) = get_json(&lurl) {
                if let Some(lrc) = lj.get("lrc").and_then(|v| v.get("lyric")).and_then(|v| v.as_str()) {
                    let l = filt(parse_lrc(lrc));
                    let bad = l.iter().any(|x| x.text.contains("暂无歌词") || x.text.contains("纯音乐"));
                    if l.len() >= 2 && !bad {
                        dlog(&format!("lyric pick id={} sc={} lines={}", id, sc, l.len()));
                        lines = Some(l);
                        chosen_id = Some(id);
                        chosen_dur = _cd;
                        break;
                    }
                }
            }
        }
    }

    // 网易云没有可用歌词时，尝试备用源（QQ 音乐 → LRCLIB），提高汽水/抖音等曲目覆盖。
    if lines.is_none() {
        if let Some(l) = fetch_alternates(title, artist, expected_dur) {
            dlog(&format!("alt lyric {} - {} lines={}", title, artist, l.len()));
            lines = Some(l);
            chosen_dur = 0.0; // 备用源时长未知，跳过下面的比例缩放
        }
    }

    // 同名不同版本（现场/录音室/重新编曲）时长不同：若最接近版本的时长仍与实际播放时长差 >2.5s，
    // 按比例缩放歌词时间轴，避免整首歌词系统性快/慢（比直接错位好得多）。
    // 但只在两边都是“整首时长”（>=60s）且比例接近 1（0.7~1.4）时才缩放：
    // VIP 试听片段（30~40s）绝不能把完整歌词按比例压扁，否则整首歌词会挤成一团。
    let trial = expected_dur < 5.0
        || (expected_dur < 60.0 && cands.iter().any(|c| c.2 > expected_dur * 1.5));
    if !trial && expected_dur >= 60.0 && chosen_dur >= 60.0 {
        let ratio = expected_dur / chosen_dur;
        if (ratio - 1.0).abs() > 0.01 && ratio > 0.7 && ratio < 1.4 {
            if let Some(v) = lines.as_mut() {
                for l in v.iter_mut() {
                    l.t *= ratio;
                    for s in l.segs.iter_mut() {
                        s.t *= ratio;
                        s.dur *= ratio;
                    }
                }
            }
            dlog(&format!("time-scale {:.3} expected={:.1} chosen={:.1}", ratio, expected_dur, chosen_dur));
        }
    }

    // 封面 + 时长：优先用选中歌曲，否则用第一个候选
    let mut cover: Option<Vec<u8>> = None;
    let mut dur_ms: Option<i64> = None;
    if let Some(id) = chosen_id.or_else(|| cands.first().map(|(_, id, _)| *id)) {
        let durl = format!("https://music.163.com/api/song/detail?ids=[{}]", id);
        if let Some(dj) = get_json(&durl) {
            if let Some(s0) = dj.get("songs").and_then(|v| v.as_array()).and_then(|a| a.get(0)) {
                dur_ms = s0
                    .get("dt")
                    .and_then(|v| v.as_i64())
                    .or_else(|| s0.get("duration").and_then(|v| v.as_i64()));
                if let Some(url) = s0
                    .get("album")
                    .and_then(|al| al.get("picUrl"))
                    .and_then(|u| u.as_str())
                {
                    if !url.is_empty() {
                        // 取 400x400 缩略图，避免 6MB 大图
                        let u2 = if url.contains('?') { url.to_string() } else { format!("{}?param=400y400", url) };
                        cover = get_bytes(&u2);
                    }
                }
            }
        }
    }

    dlog(&format!("net {} - {} => lines={} cover={}B", title, artist, lines.as_ref().map(|v| v.len()).unwrap_or(0), cover.as_ref().map(|v| v.len()).unwrap_or(0)));

    // 写缓存
    if let Some(d) = dir {
        if let Some(y) = &chosen_yrc {
            let _ = std::fs::write(d.join(format!("{}.yrc", key)), y);
        }
        if let Some(l) = &lines {
            let body: String = l.iter().map(|x| format!("[{:02}:{:05.2}]{}\n", (x.t as u32) / 60, x.t % 60.0, x.text)).collect();
            let _ = std::fs::write(d.join(format!("{}.lrc", key)), body);
        }
        if let Some(c) = &cover {
            if c.len() > 1000 {
                let _ = std::fs::write(d.join(format!("{}.cover", key)), c);
            }
        }
        if let Some(ms) = dur_ms {
            let _ = std::fs::write(d.join(format!("{}.dur", key)), ms.to_string());
            // 另存“歌名/歌手 -> 时长”索引（key_hash 含时长分桶，无法靠它反查时长）
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(d.join("durations.txt"))
            {
                use std::io::Write as _;
                let _ = writeln!(f, "{}\t{}\t{}", title, artist, ms);
            }
        }
    }
    (lines, cover)
}

/// 仅取歌词（保留旧接口）。
pub fn fetch(title: &str, artist: &str) -> Option<Vec<LrcLine>> {
    fetch_full(title, artist, 0.0, false).0
}

/// 已知歌曲总时长（秒）：读缓存（联网时随歌词一起写入）。
pub fn known_duration(title: &str, artist: &str) -> Option<f64> {
    let dir = cache_dir()?;
    let txt = std::fs::read_to_string(dir.join("durations.txt")).ok()?;
    for line in txt.lines() {
        let mut it = line.split('\t');
        let (t, a, d) = (it.next()?, it.next()?, it.next()?);
        if t == title && a == artist {
            if let Ok(ms) = d.trim().parse::<f64>() {
                if ms > 5000.0 {
                    return Some(ms / 1000.0);
                }
            }
        }
    }
    None
}

#[allow(dead_code)]
fn known_duration_old(title: &str, artist: &str) -> Option<f64> {
    let dir = cache_dir()?;
    let key = key_hash(title, artist, 0.0);
    let txt = std::fs::read_to_string(dir.join(format!("{}.dur", key))).ok()?;
    let ms: f64 = txt.trim().parse().ok()?;
    if ms > 5000.0 {
        Some(ms / 1000.0)
    } else {
        None
    }
}

#[cfg(test)]
mod probes {
    // 验证“同名不同歌”闸门：Keep the Roses - KAVO 实际时长 156.9s，
    // 网易云旧逻辑会把 Laleh《roses》(~264s) 贴上来；修复后应被时长闸门拒掉。
    #[test]
    #[ignore]
    fn probe_keep_roses() {
        let (l, _c) = super::fetch_full("Keep the Roses", "KAVO", 156.9, false);
        match l {
            Some(v) => eprintln!(
                "lines={} first={:?} last_t={:.1}",
                v.len(),
                v.first().map(|x| x.text.clone()).unwrap_or_default(),
                v.last().map(|x| x.t).unwrap_or(0.0)
            ),
            None => eprintln!("none"),
        }
    }

    // 验证「网易云按 song id 取词」：清掉缓存后走一遍 fetch_full（非汽水），
    // 期望日志出现 `netease mem id=` 且拿到正确整首歌词。
    #[test]
    #[ignore]
    fn probe_fetch_netease_env() {
        let title = std::env::var("NE_TITLE").unwrap_or_default();
        let artist = std::env::var("NE_ARTIST").unwrap_or_default();
        let dur: f64 = std::env::var("NE_DUR")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        if title.is_empty() {
            eprintln!("set NE_TITLE");
            return;
        }
        if let Some(d) = super::cache_dir() {
            let key = super::key_hash(&title, &artist, dur);
            for ext in ["lrc", "yrc", "cover", "nolyric"] {
                let _ = std::fs::remove_file(d.join(format!("{}.{}", key, ext)));
            }
        }
        let id = crate::netease_mem::current_song_id(&title, &artist, dur);
        let (l, c) = super::fetch_full(&title, &artist, dur, false);
        eprintln!(
            "NE id={:?} lines={} cover={}B first={:?} last={:.1}",
            id,
            l.as_ref().map(|v| v.len()).unwrap_or(0),
            c.as_ref().map(|v| v.len()).unwrap_or(0),
            l.as_ref().and_then(|v| v.first().map(|x| x.text.clone())).unwrap_or_default(),
            l.as_ref().and_then(|v| v.last().map(|x| x.t)).unwrap_or(0.0)
        );
    }

    // 验证「汽水 CDP 取词」整条链路：清掉汽水缓存后 fetch_full(soda=true)。
    #[test]
    #[ignore]
    fn probe_fetch_soda_env() {
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
        if let Some(d) = super::cache_dir() {
            let key = super::key_hash(&title, &artist, dur);
            let _ = std::fs::remove_file(d.join(format!("{}.soda.lrc", key)));
        }
        let (l, c) = super::fetch_full(&title, &artist, dur, true);
        eprintln!(
            "SODA lines={} cover={}B first={:?} last={:.1}",
            l.as_ref().map(|v| v.len()).unwrap_or(0),
            c.as_ref().map(|v| v.len()).unwrap_or(0),
            l.as_ref().and_then(|v| v.first().map(|x| x.text.clone())).unwrap_or_default(),
            l.as_ref().and_then(|v| v.last().map(|x| x.t)).unwrap_or(0.0)
        );
    }
}
