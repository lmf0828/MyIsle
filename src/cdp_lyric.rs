//! 通过 CDP（Chrome DevTools Protocol）直接从「汽水音乐」渲染进程的 Vue 组件
//! `LyricsView.props.lyric` 里读取当前歌曲的官方歌词（含逐字时间），
//! 不需要汽水显示歌词页、也不联网。
//!
//! 前提：汽水 app.asar 已打补丁，启动时带 `--remote-debugging-port=9222`（见 README 记载）。

use crate::lyrics::{LrcLine, Seg};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// 从本机 9222 端口找到主页面 target 的 WebSocket 调试地址。
fn page_ws_url() -> Option<String> {
    let body = ureq::get("http://127.0.0.1:9222/json/list")
        .timeout(Duration::from_millis(800))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let arr = v.as_array()?;
    for t in arr {
        if t.get("type").and_then(|x| x.as_str()) != Some("page") {
            continue;
        }
        let url = t.get("url").and_then(|x| x.as_str()).unwrap_or("");
        if url.contains("main.asar/main.html") {
            if let Some(ws) = t.get("webSocketDebuggerUrl").and_then(|x| x.as_str()) {
                return Some(ws.to_string());
            }
        }
    }
    None
}

fn parse_ws_url(u: &str) -> Option<(String, String)> {
    // ws://127.0.0.1:9222/devtools/page/XXXX
    let rest = u.strip_prefix("ws://")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    Some((hostport.to_string(), path.to_string()))
}

fn ws_key() -> String {
    // 固定的 16 字节做 base64（RFC6455 只要求可打印随机串，不要求真的随机）
    const B: [u8; 16] = [
        0x4c, 0x69, 0x6e, 0x67, 0x44, 0x61, 0x6f, 0x57, 0x53, 0x4b, 0x65, 0x79, 0x31, 0x32, 0x33,
        0x21,
    ];
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < B.len() {
        let b0 = B[i] as u32;
        let b1 = if i + 1 < B.len() { B[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < B.len() { B[i + 2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if i + 1 < B.len() {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if i + 2 < B.len() {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
        i += 3;
    }
    out
}

fn ws_connect(u: &str) -> Option<TcpStream> {
    let (hostport, path) = parse_ws_url(u)?;
    let mut s = TcpStream::connect(&hostport).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(2500))).ok()?;
    s.set_write_timeout(Some(Duration::from_millis(2500))).ok()?;
    // 固定的 base64 随机 key（RFC6455 只要求是 16 字节的 base64）
    let key = ws_key();
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
        path, hostport, key
    );
    s.write_all(req.as_bytes()).ok()?;
    // 读到头部结束
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = s.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 16384 {
            return None;
        }
    }
    if !buf.starts_with(b"HTTP/1.1 101") {
        return None;
    }
    Some(s)
}

fn ws_send_text(s: &mut TcpStream, text: &str) -> Option<()> {
    let payload = text.as_bytes();
    let len = payload.len();
    let mut frame = Vec::with_capacity(len + 14);
    frame.push(0x81u8); // FIN + text
    if len < 126 {
        frame.push(0x80 | (len as u8)); // masked
    } else if len <= 65535 {
        frame.push(0x80 | 126);
        frame.push((len >> 8) as u8);
        frame.push((len & 0xff) as u8);
    } else {
        frame.push(0x80 | 127);
        for i in (0..8).rev() {
            frame.push(((len as u64) >> (8 * i)) as u8);
        }
    }
    let mask = [0x12u8, 0x34, 0x56, 0x78];
    frame.extend_from_slice(&mask);
    for (i, b) in payload.iter().enumerate() {
        frame.push(b ^ mask[i % 4]);
    }
    s.write_all(&frame).ok()?;
    s.flush().ok()?;
    Some(())
}

/// 读取一个完整的文本帧（忽略 ping/pong），返回 JSON 文本。
fn ws_read_text(s: &mut TcpStream) -> Option<String> {
    for _ in 0..16 {
        let mut h = [0u8; 2];
        s.read_exact(&mut h).ok()?;
        let opcode = h[0] & 0x0f;
        let masked = h[1] & 0x80 != 0;
        let mut len = (h[1] & 0x7f) as usize;
        if len == 126 {
            let mut e = [0u8; 2];
            s.read_exact(&mut e).ok()?;
            len = ((e[0] as usize) << 8) | e[1] as usize;
        } else if len == 127 {
            let mut e = [0u8; 8];
            s.read_exact(&mut e).ok()?;
            let mut l = 0usize;
            for b in e {
                l = (l << 8) | b as usize;
            }
            len = l;
        }
        let mut mask = [0u8; 4];
        if masked {
            s.read_exact(&mut mask).ok()?;
        }
        let mut data = vec![0u8; len];
        s.read_exact(&mut data).ok()?;
        if masked {
            for i in 0..len {
                data[i] ^= mask[i % 4];
            }
        }
        match opcode {
            0x1 => {
                return String::from_utf8(data).ok();
            }
            0x9 => {
                // ping → 回 pong（简单忽略）
                continue;
            }
            0x8 => return None,
            _ => continue,
        }
    }
    None
}

/// 当前歌词视图对应的歌曲信息与原始歌词内容。
pub struct SodaLyric {
    pub name: String,
    pub artist: String,
    pub duration: f64, // 秒
    pub content: String,
    pub cn: String,
    pub cover: String,
}

fn evaluate(expr: &str) -> Option<serde_json::Value> {
    let wsurl = page_ws_url()?;
    let mut s = ws_connect(&wsurl)?;
    let msg = serde_json::json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": { "expression": expr, "returnByValue": true, "awaitPromise": true }
    });
    ws_send_text(&mut s, &msg.to_string())?;
    // 读到 id=1 的响应
    for _ in 0..32 {
        let txt = ws_read_text(&mut s)?;
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
            if v.get("id").and_then(|x| x.as_i64()) == Some(1) {
                return Some(v);
            }
        }
    }
    None
}

const EXPR: &str = r#"(()=>{
 const el=document.querySelector('#app');
 const app=el&&el.__vue_app__;
 if(!app) return {ok:false,why:'noapp'};
 const root=app._container&&app._container._vnode;
 const seen=new Set(); let t=null;
 (function walk(v,d){
   if(!v||d>600||seen.has(v)||t)return; seen.add(v);
   const c=v.component;
   if(c){ const n=(c.type&&(c.type.__name||c.type.name))||'';
     if(n==='LyricsView'&&c.props&&c.props.lyric&&c.props.lyric.content&&c.subTree&&c.subTree.el&&c.subTree.el.isConnected){t=c;}
     walk(c.subTree,d+1);
   }
   const ch=v.children;
   if(Array.isArray(ch))ch.forEach(x=>walk(x,d+1)); else if(ch&&typeof ch==='object')walk(ch,d+1);
   if(v.suspense)walk(v.suspense.activeBranch,d+1);
 })(root,0);
 if(!t) return {ok:false,why:'nolyricview'};
 const lyr=t.props.lyric, tr=t.props.track||{};
 const arts=(tr.artists||[]).map(a=>a&&a.name).filter(Boolean).join(', ');
 const uc=(tr.album&&tr.album.url_cover)||{};
 let cover='';
 if(uc&&uc.uri){ const base=(uc.urls&&uc.urls[0])||'https://p3-luna.douyinpic.com/img/'; const tpl=uc.template_prefix||'tplv-b829550vbb'; cover=base+uc.uri+'~'+tpl+'-crop-center:400:400.jpg'; }
 return {ok:true,name:tr.name||'',artist:arts,duration:(tr.duration||0)/1000,
   content:lyr.content||'',cn:(lyr.translations&&lyr.translations.cn)||'',cover:cover};
})()"#;

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 读取当前在播歌曲的官方歌词。`title`/`artist` 用于防止切歌瞬间读到上一首。
pub fn fetch_current(title: &str, artist: &str) -> Option<SodaLyric> {
    fetch_current_inner(title, artist, true)
}

/// 同 `fetch_current`，但绝不切换汽水内部路由（用于后台轮询“正在播放”，避免干扰用户浏览页面）。
pub fn fetch_current_no_nav(title: &str, artist: &str) -> Option<SodaLyric> {
    fetch_current_inner(title, artist, false)
}

/// 从底部播放条 `.bottom-player` 读取当前曲目（歌名/歌手/时长）。
/// 该播放条在「正在播放页」「歌单/场景模式」等几乎所有页面都常驻，因此即使
/// LyricsView 未挂载（用户切到歌单/场景模式）也能识别到当前歌曲，不依赖 SMTC。
pub fn fetch_bottom_player() -> Option<SodaLyric> {
    let expr = r#"(()=>{
      try{
        const bp=document.querySelector('.bottom-player');
        if(!bp) return {ok:false};
        const t=bp.querySelector('.track .title');
        const a=bp.querySelector('.track .artist-link');
        const name=t?(t.textContent||'').trim():'';
        if(!name) return {ok:false};
        const tip=bp.querySelector('.indicator')||bp.querySelector('.slider-tip');
        let dur=0;
        if(tip){const m=(tip.textContent||'').match(/(\d+):(\d+)\s*\/\s*(\d+):(\d+)/); if(m){dur=(+m[3])*60+(+m[4]);}}
        const ci=bp.querySelector('.track .cover img');
        const cover=ci?(ci.src||''):'';
        return {ok:true,name:name,artist:a?(a.textContent||'').trim():'',duration:dur,cover:cover};
      }catch(e){return {ok:false};}
    })()"#;
    let v = evaluate(expr)?;
    let o = v.get("result")?.get("result")?.get("value")?;
    if o.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        return None;
    }
    let name = o
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return None;
    }
    Some(SodaLyric {
        name,
        artist: o.get("artist").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        duration: o.get("duration").and_then(|x| x.as_f64()).unwrap_or(0.0),
        content: String::new(),
        cn: String::new(),
        cover: o.get("cover").and_then(|x| x.as_str()).unwrap_or("").to_string(),
    })
}

/// 只读底部播放条的「播放态 + 实时进度」，不要求有歌名（歌单/场景页底部条常无标题）。
/// 播放态用中间大按钮图标判定：暂停双竖条 `M9 7.8…` = 正在播放；播放三角 `M11 40.407…` = 已暂停。
/// 进度用 `.slider-fill` 的宽度百分比（亚秒级实时；`.slider-tip` 文本是滞后的悬停提示，不能用）。
pub fn fetch_play_state() -> Option<(bool, f64, f64)> {
    let expr = r#"(()=>{
      try{
        const bp=document.querySelector('.bottom-player');
        if(!bp) return {ok:false};
        const tip=bp.querySelector('.indicator')||bp.querySelector('.slider-tip');
        let dur=0;
        if(tip){const m=(tip.textContent||'').match(/(\d+):(\d+)\s*\/\s*(\d+):(\d+)/); if(m){dur=(+m[3])*60+(+m[4]);}}
        const fill=bp.querySelector('.slider-fill');
        let pct=-1;
        if(fill){const w=(fill.style.width||'').replace('%',''); const f=parseFloat(w); if(!isNaN(f)) pct=f;}
        const btn=bp.querySelector('.controls.center .button.large');
        let playing=false;
        if(btn){const p=btn.querySelector('path'); const d=p?(p.getAttribute('d')||''):''; playing=/^M9\s*7\.8/.test(d);}
        const cur = (dur>0 && pct>=0)? dur*pct/100.0 : -1;
        return {ok:true, dur:dur, cur:cur, playing:playing};
      }catch(e){return {ok:false};}
    })()"#;
    let v = evaluate(expr)?;
    let o = v.get("result")?.get("result")?.get("value")?;
    if o.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        return None;
    }
    let dur = o.get("duration").and_then(|x| x.as_f64())
        .or_else(|| o.get("dur").and_then(|x| x.as_f64()))
        .unwrap_or(0.0);
    let cur = o.get("cur").and_then(|x| x.as_f64()).unwrap_or(-1.0);
    let playing = o.get("playing").and_then(|x| x.as_bool()).unwrap_or(false);
    Some((playing, cur, dur))
}

fn fetch_current_inner(title: &str, artist: &str, allow_nav: bool) -> Option<SodaLyric> {
    let want_t = norm(title);
    let mut result = None;
    for attempt in 0..4 {
        let mut retry = false;
        if let Some(v) = evaluate(EXPR) {
            let r = v.get("result").and_then(|x| x.get("result"));
            let val = r.and_then(|x| x.get("value"));
            if let Some(o) = val {
                if o.get("ok").and_then(|x| x.as_bool()) == Some(true) {
                    let name = o.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let content = o.get("content").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let got_t = norm(&name);
                    let name_ok = want_t.is_empty()
                        || got_t == want_t
                        || got_t.contains(&want_t)
                        || want_t.contains(&got_t);
                    if name_ok && !content.trim().is_empty() {
                        result = Some(SodaLyric {
                            name,
                            artist: o.get("artist").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            duration: o.get("duration").and_then(|x| x.as_f64()).unwrap_or(0.0),
                            content,
                            cn: o.get("cn").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            cover: o.get("cover").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        });
                        break;
                    }
                    retry = true;
                } else {
                    dlog(&format!(
                        "cdp not ready: {}",
                        o.get("why").and_then(|x| x.as_str()).unwrap_or("?")
                    ));
                    retry = true;
                }
            }
        }
        // 绝不移动/隐藏/显示汽水窗口（任何窗口操作都可能让客户端异常）。
        // 仅在需要时把汽水内部路由切到播放页，让 Vue 的 LyricsView 挂载；窗口本身一动不动。
        if retry && allow_nav && attempt == 0 {
            let _ = evaluate(NAV_PLAYER_EXPR);
            std::thread::sleep(Duration::from_millis(900));
        } else if retry && attempt < 3 {
            std::thread::sleep(Duration::from_millis(350));
        }
    }
    let _ = artist;
    result
}

const NAV_PLAYER_EXPR: &str = r#"(()=>{try{const p={enter_method:'click_player_icon_below',scene_name:'radio'};const h='#/player?et_params='+encodeURIComponent(JSON.stringify(p));if(location.hash.indexOf('/player')<0){location.hash=h;}return location.hash;}catch(e){return 'err';}})()"#;

const PROG_EXPR: &str = r#"(()=>{
  const bp=document.querySelector('.bottom-player');
  if(!bp) return null;
  const tip=bp.querySelector('.indicator')||bp.querySelector('.slider-tip');
  let pct=null;
  // 进度优先取 `.slider-fill` 的 style width%（亚秒级实时推进；`.slider-thumb` 在歌单/场景页可能不存在）
  const fill=bp.querySelector('.slider-fill');
  if(fill){const w=(fill.getAttribute('style')||'').match(/width:\s*([\d.]+)%/); if(w){pct=parseFloat(w[1]);}}
  // 次选 `.slider-dot` 的 left%
  if(pct===null){const dot=bp.querySelector('.slider-dot'); if(dot){const l=(dot.getAttribute('style')||'').match(/left:\s*([\d.]+)%/); if(l){pct=parseFloat(l[1]);}}}
  // 再次退回 thumb/track 的位置比例
  if(pct===null){const th=bp.querySelector('.slider-thumb');const tr=bp.querySelector('.slider-track');
    if(th&&tr){const b=th.getBoundingClientRect();const t=tr.getBoundingClientRect();if(t.width>0){pct=(b.left+b.width*0.5-t.left)/t.width*100.0;}}}
  // 注意：`.slider-tip` 是隐藏的悬停提示，innerText 在 display:none 时为空，必须用 textContent
  return {pct:pct, text: tip?tip.textContent:''};
})()"#;

fn mmss(s: &str) -> Option<f64> {
    let mut it = s.split(':');
    let m: f64 = it.next()?.trim().parse().ok()?;
    let sec: f64 = it.next()?.trim().parse().ok()?;
    Some(m * 60.0 + sec)
}

/// 从汽水播放器界面读取「当前播放位置 / 总时长」（秒）。
/// 总时长读播放条 `.indicator` 的 `当前 / 总长` 文本（真实值）；
/// 当前秒数优先用进度条填充比例 `.slider-fill` 的 width%（亚秒精度，避免整秒跳变导致
/// 逐字高亮“唱过去了又退回一两个字”），失败再退回 `.indicator` 的整秒。
/// 通过 CDP 点击汽水播放器上的按钮，实现上一曲 / 播放暂停 / 下一曲。
/// `cmd`: "prev" | "play" | "next"（"play" 即切换播放/暂停）。返回是否点击成功。
pub fn control(cmd: &str) -> bool {
    let pick = match cmd {
        "prev" => "document.querySelectorAll('.bottom-player .controls.center .button')[0]",
        "next" => "document.querySelectorAll('.bottom-player .controls.center .button')[2]",
        _ => "document.querySelector('.bottom-player .controls.center .button.large')",
    };
    let expr = format!(
        "(()=>{{try{{const b={pick};if(!b)return false;b.click();return true;}}catch(e){{return false;}}}})()"
    );
    evaluate(&expr)
        .and_then(|v| v.get("result")?.get("result")?.get("value")?.as_bool())
        .unwrap_or(false)
}

pub fn fetch_progress(_expected_dur: f64) -> Option<(f64, f64)> {
    let v = match evaluate(PROG_EXPR) {
        Some(v) => v,
        None => {
            if std::env::var_os("WINISLE_SODAPROG_LOG").is_some() {
                dlog("prog eval none");
            }
            return None;
        }
    };
    let obj = v.get("result")?.get("result")?.get("value")?;
    if obj.is_null() {
        return None;
    }
    let pct = obj.get("pct").and_then(|x| x.as_f64());
    let text = obj.get("text").and_then(|x| x.as_str()).unwrap_or("");
    if std::env::var_os("WINISLE_SODAPROG_LOG").is_some() {
        dlog(&format!("prog pct={:?} text={:?}", pct, text));
    }
    let toks: Vec<&str> = text.split_whitespace().collect();
    let mut pair: Option<(f64, f64)> = None;
    for w in toks.windows(3) {
        if w[1] != "/" {
            continue;
        }
        if let (Some(cur), Some(total)) = (mmss(w[0]), mmss(w[2])) {
            if total > 1.0 && total >= cur {
                pair = Some((cur, total));
            }
        }
    }
    let (cur0, total) = pair?;
    // 进度以进度条填充比例 pct 为准（亚秒精度、随播放实时推进）。
    // 说明：`.slider-tip` 的“当前时间”文本常常是静态/滞后的（只在悬停时更新），
    // 之前拿它做“合理性校验”会把实时 pct 误判为异常、退回静止的文本秒数，
    // 导致歌词高亮与真实播放位置越差越远（“歌词跟不上”）。改为优先 pct×总时长，
    // pct 不可用时才退回文本秒数。
    // 当前秒数：以 `.indicator` 的整秒为基准（真实、每秒刷新），若 pct 折算出的亚秒位置与整秒
    // 相差 ≤1.5s（说明 pct 与显示秒一致、只是更精细）则用 pct 的亚秒值，令歌词逐字更贴合；
    // 若 pct 明显偏离（pct 只有 1% 粒度≈2~3s、偶尔滞后/在缓冲位），则退回整秒，避免被粗粒度带偏。
    let cur = match pct {
        Some(p) if p.is_finite() && p >= 0.0 && p <= 100.5 && total > 1.0 => {
            let pc = (total * p / 100.0).clamp(0.0, total);
            if (pc - cur0).abs() <= 1.5 {
                pc
            } else {
                cur0
            }
        }
        _ => cur0,
    };
    Some((cur, total))
}
fn dlog(s: &str) {
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::paths::file("lyrics_debug.log"))
    {
        let _ = writeln!(f, "{}", s);
    }
}

/// 把汽水官方歌词文本解析成 LrcLine：
///   `[start_ms,dur_ms]<off,dur,?>chars...`（逐字）或 `[mm:ss.xx]text`（整行）。
pub fn parse_content(content: &str) -> Vec<LrcLine> {
    let mut out = Vec::new();
    for raw in content.split('\n') {
        let line = raw.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        if !line.starts_with('[') {
            continue;
        }
        let close = match line.find(']') {
            Some(i) => i,
            None => continue,
        };
        let head = &line[1..close];
        let body = &line[close + 1..];
        // 整行 LRC：[mm:ss.xx]
        if head.contains(':') {
            let parts: Vec<&str> = head.split(':').collect();
            if parts.len() != 2 {
                continue;
            }
            let m: f64 = parts[0].trim().parse().unwrap_or(0.0);
            let sec: f64 = parts[1].trim().parse().unwrap_or(0.0);
            let text = body.trim().to_string();
            if !text.is_empty() {
                out.push(LrcLine { t: m * 60.0 + sec, text, segs: vec![] });
            }
            continue;
        }
        // 逐字：[start,dur] 后面跟 <off,dur,?>文字
        let hp: Vec<&str> = head.split(',').collect();
        if hp.len() < 1 {
            continue;
        }
        let start_ms: f64 = hp[0].trim().parse().unwrap_or(-1.0);
        if start_ms < 0.0 {
            continue;
        }
        let line_t = start_ms / 1000.0;
        let mut text = String::new();
        let mut segs: Vec<Seg> = Vec::new();
        let mut cur_off: Option<f64> = None;
        let mut cur_dur: f64 = 0.0;
        let mut cur_chars: u32 = 0;
        let mut i = 0;
        let b: Vec<char> = body.chars().collect();
        while i < b.len() {
            if b[i] == '<' {
                // 结算上一段
                if let Some(off) = cur_off {
                    if cur_chars > 0 {
                        segs.push(Seg { t: line_t + off / 1000.0, dur: cur_dur / 1000.0, n: cur_chars });
                    }
                }
                if let Some(gt) = b[i..].iter().position(|&c| c == '>') {
                    let inside: String = b[i + 1..i + gt].iter().collect();
                    let p: Vec<&str> = inside.split(',').collect();
                    cur_off = Some(p.get(0).and_then(|x| x.trim().parse().ok()).unwrap_or(0.0));
                    cur_dur = p.get(1).and_then(|x| x.trim().parse().ok()).unwrap_or(0.0);
                    cur_chars = 0;
                    i += gt + 1;
                    continue;
                } else {
                    break;
                }
            } else {
                text.push(b[i]);
                if !b[i].is_whitespace() {
                    cur_chars += 1;
                }
                i += 1;
            }
        }
        if let Some(off) = cur_off {
            if cur_chars > 0 {
                segs.push(Seg { t: line_t + off / 1000.0, dur: cur_dur / 1000.0, n: cur_chars });
            }
        }
        let text = text.trim().to_string();
        if !text.is_empty() {
            out.push(LrcLine { t: line_t, text, segs });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore]
    fn probe_cdp() {
        let sl = super::fetch_current("", "").expect("no lyric");
        let l = super::parse_content(&sl.content);
        println!(
            "name={} artist={} dur={:.1} lines={} cnlen={}",
            sl.name,
            sl.artist,
            sl.duration,
            l.len(),
            sl.cn.len()
        );
        for x in l.iter().take(5) {
            println!("{:7.2} segs={} {}", x.t, x.segs.len(), x.text);
        }
        for x in l.iter().rev().take(3) {
            println!("LAST {:7.2} {}", x.t, x.text);
        }
    }
}
