//! 汽水音乐 CDP 补丁自动修复。
//!
//! 汽水是 Electron 应用。给它打上 `remote-debugging-port=9222` 之后，MyIsle就能
//! 通过 CDP 读取渲染进程里的官方逐字歌词（见 `cdp_lyric.rs`）。但汽水每次
//! **自动更新都会用官方 app.asar 覆盖掉补丁**，于是歌词突然失效。
//!
//! 本模块负责「自动排查 + 自动打补丁」：
//!   1. 找到当前安装的 `...\resources\app.asar`（按版本号取最新目录）；
//!   2. 检查里面 `entry.js` 是否已含补丁标记，没有则就地重打包（纯 Rust，
//!      不依赖 node/@electron/asar）；
//!   3. 备份原文件到 `data\app.asar.bak`；
//!   4. 若汽水正在运行且没在播放（安全时机），自动重启汽水让补丁生效。
//!
//! asar 容器格式（Chromium Pickle）：文件头 16 字节后是 UTF-8 JSON 目录，
//! 每个文件记录 `size` 与相对数据区的 `offset`（字符串）。重打包时把改动后的
//! `entry.js` 写回、其余文件按原顺序复制，并重算所有 offset。

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn dlog(s: &str) {
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::paths::file("soda_patch.log"))
    {
        let _ = writeln!(f, "{}", s);
    }
}

const MARK: &str = "/*myisle-cdp2*/";
const INJECT: &str = "try{const{app}=require(\"electron\");app.commandLine.appendSwitch(\"remote-debugging-port\",\"9222\");app.commandLine.appendSwitch(\"remote-allow-origins\",\"*\");app.commandLine.appendSwitch(\"disable-background-timer-throttling\");app.commandLine.appendSwitch(\"disable-renderer-backgrounding\");app.commandLine.appendSwitch(\"disable-backgrounding-occluded-windows\");app.commandLine.appendSwitch(\"disable-features\",\"CalculateNativeWinOcclusion\");}catch(e){}\n/*myisle-cdp2*/\n";

/// 在常见安装位置里按版本号挑最新的 `app.asar`。
pub fn find_asar() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("SODA_HOME") {
        let p = Path::new(&home).join("resources").join("app.asar");
        if p.is_file() {
            return Some(p);
        }
    }
    let mut roots: Vec<PathBuf> = vec![PathBuf::from(r"D:\qishuiMusic\App\Soda Music")];
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        roots.push(Path::new(&la).join("Programs").join("SodaMusic"));
        roots.push(Path::new(&la).join("Programs").join("Soda Music"));
        roots.push(Path::new(&la).join("SodaMusic"));
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        roots.push(Path::new(&pf).join("Soda Music"));
        roots.push(Path::new(&pf).join("SodaMusic"));
    }
    if let Ok(pf) = std::env::var("ProgramFiles(x86)") {
        roots.push(Path::new(&pf).join("Soda Music"));
        roots.push(Path::new(&pf).join("SodaMusic"));
    }
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    for root in roots {
        // 目录本身就是一个版本目录（ProgramFiles 直接放 resources）
        let direct = root.join("resources").join("app.asar");
        if direct.is_file() {
            if best.as_ref().map_or(true, |b| vec![999999u64] > b.0) {
                best = Some((vec![999999u64], direct));
            }
        }
        let Ok(rd) = std::fs::read_dir(&root) else { continue };
        for e in rd.flatten() {
            let dir = e.path();
            let p = dir.join("resources").join("app.asar");
            if !p.is_file() {
                continue;
            }
            let ver: Vec<u64> = e
                .file_name()
                .to_string_lossy()
                .split('.')
                .map(|x| x.parse::<u64>().unwrap_or(0))
                .collect();
            if best.as_ref().map_or(true, |b| ver > b.0) {
                best = Some((ver, p));
            }
        }
    }
    best.map(|b| b.1)
}

struct Header {
    json: Value,
    data_start: usize,
}

fn read_header(bytes: &[u8]) -> Result<Header, String> {
    if bytes.len() < 16 {
        return Err("asar too short".into());
    }
    let u32at = |o: usize| -> usize {
        u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as usize
    };
    let hdr_total = u32at(4); // 头部 pickle 总长（含 4 字节 payload 长度）
    let json_len = u32at(12);
    let start = 16usize;
    let end = start + json_len;
    if end > bytes.len() || 8 + hdr_total > bytes.len() {
        return Err("asar header out of range".into());
    }
    let text = std::str::from_utf8(&bytes[start..end]).map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(Header { json, data_start: 8 + hdr_total })
}

/// 收集所有叶子（文件）节点的路径 / size / offset / unpacked。
fn collect_leaves(v: &Value, path: &mut Vec<String>, out: &mut Vec<(Vec<String>, u64, u64, bool)>) {
    let Some(files) = v.get("files").and_then(|f| f.as_object()) else { return };
    for (name, child) in files {
        path.push(name.clone());
        if child.get("files").is_some() {
            collect_leaves(child, path, out);
        } else {
            let size = child.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
            let offset = child
                .get("offset")
                .and_then(|o| o.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| child.get("offset").and_then(|o| o.as_u64()))
                .unwrap_or(0);
            let unpacked = child.get("unpacked").and_then(|b| b.as_bool()).unwrap_or(false);
            out.push((path.clone(), size, offset, unpacked));
        }
        path.pop();
    }
}

fn leaf_mut<'a>(v: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    let mut cur = v;
    for (i, seg) in path.iter().enumerate() {
        cur = cur.get_mut("files")?.get_mut(seg)?;
        let _ = i;
    }
    Some(cur)
}

fn read_file_range(bytes: &[u8], data_start: usize, offset: u64, size: u64) -> Result<Vec<u8>, String> {
    let a = data_start + offset as usize;
    let b = a + size as usize;
    if b > bytes.len() {
        return Err(format!("file range {a}..{b} out of {}", bytes.len()));
    }
    Ok(bytes[a..b].to_vec())
}

/// 返回 (是否已打补丁, entry.js 原文)。
fn inspect(asar: &Path) -> Result<(bool, String), String> {
    let bytes = std::fs::read(asar).map_err(|e| e.to_string())?;
    let h = read_header(&bytes)?;
    let mut leaves = Vec::new();
    let mut path = Vec::new();
    collect_leaves(&h.json, &mut path, &mut leaves);
    let (_, size, offset, unpacked) = leaves
        .iter()
        .find(|(p, ..)| p.last().map(|s| s.as_str()) == Some("entry.js"))
        .cloned()
        .ok_or("entry.js not found")?;
    if unpacked {
        return Err("entry.js unexpectedly unpacked".into());
    }
    let raw = read_file_range(&bytes, h.data_start, offset, size)?;
    let text = String::from_utf8(raw).map_err(|e| e.to_string())?;
    let ok = text.contains("remote-debugging-port")
        && text.contains("remote-allow-origins")
        && text.contains("disable-renderer-backgrounding");
    let _ = MARK;
    Ok((ok, text))
}

/// 把 asar 重打包，只替换 entry.js。返回新文件字节。
fn repack(asar: &Path, new_entry: &[u8]) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(asar).map_err(|e| e.to_string())?;
    let mut h = read_header(&bytes)?;
    let mut leaves = Vec::new();
    let mut path = Vec::new();
    collect_leaves(&h.json, &mut path, &mut leaves);

    // 按原 offset 顺序排列数据区里的文件（unpacked 的不占数据区）。
    let mut ordered: Vec<usize> = (0..leaves.len()).collect();
    ordered.sort_by_key(|&i| leaves[i].2);

    let mut data: Vec<u8> = Vec::new();
    let mut placed: Vec<(usize, u64, u64)> = Vec::new(); // (leaf idx, new offset, new size)
    for &i in &ordered {
        let (ref p, size, offset, unpacked) = leaves[i];
        if unpacked {
            continue;
        }
        let content = if p.last().map(|s| s.as_str()) == Some("entry.js") {
            new_entry.to_vec()
        } else {
            read_file_range(&bytes, h.data_start, offset, size)?
        };
        let new_off = data.len() as u64;
        data.extend_from_slice(&content);
        placed.push((i, new_off, content.len() as u64));
    }

    // 更新 header：删除 integrity（避免哈希过期），重写 size/offset。
    strip_integrity(&mut h.json);
    for (i, new_off, new_size) in placed {
        let p = leaves[i].0.clone();
        if let Some(leaf) = leaf_mut(&mut h.json, &p) {
            if let Some(obj) = leaf.as_object_mut() {
                obj.insert("size".into(), Value::from(new_size));
                obj.insert("offset".into(), Value::from(new_off.to_string()));
            }
        }
    }

    let json = serde_json::to_vec(&h.json).map_err(|e| e.to_string())?;
    let pad = (4 - (json.len() % 4)) % 4;
    let payload = 4 + json.len() + pad;
    let hdr_total = 4 + payload;
    let mut out = Vec::with_capacity(8 + hdr_total + data.len());
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&(hdr_total as u32).to_le_bytes());
    out.extend_from_slice(&(payload as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend(std::iter::repeat(0u8).take(pad));
    out.extend_from_slice(&data);
    Ok(out)
}

fn strip_integrity(v: &mut Value) {
    if let Some(obj) = v.as_object_mut() {
        obj.remove("integrity");
        if let Some(files) = obj.get_mut("files").and_then(|f| f.as_object_mut()) {
            for (_, child) in files.iter_mut() {
                strip_integrity(child);
            }
        }
    }
}

/// 当 app.asar 位于 Program Files 等无写权限目录时，借一次 UAC 提权完成替换。
fn elevated_copy_expect(src: &Path, dst: &Path, expect_patched: bool) -> Result<(), String> {
    let dir = crate::paths::data_dir();
    let ps1 = dir.join("myisle_elevate_copy.ps1");
    let script = format!(
        "$ErrorActionPreference='Stop'\r\nCopy-Item -LiteralPath '{}' -Destination '{}' -Force\r\n",
        src.display(),
        dst.display()
    );
    std::fs::write(&ps1, script.as_bytes()).map_err(|e| e.to_string())?;
    let inner = format!(
        "Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','{}'",
        ps1.display()
    );
    let st = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &inner])
        .status()
        .map_err(|e| e.to_string())?;
    if !st.success() {
        return Err("UAC 提权被取消或失败".into());
    }
    match inspect(dst) {
        Ok((p, _)) if p == expect_patched => Ok(()),
        _ => Err("提权后校验失败".into()),
    }
}

fn elevated_copy(src: &Path, dst: &Path) -> Result<(), String> {
    elevated_copy_expect(src, dst, true)
}

/// 用户是否主动「还原了汽水官方原版」——此时自动补丁必须停手，否则 15 秒后又会被打回去。
fn disabled_flag() -> PathBuf {
    crate::paths::file("soda_patch_disabled")
}
pub fn patch_disabled() -> bool {
    disabled_flag().is_file()
}
pub fn backup_available() -> bool {
    crate::paths::file("app.asar.bak").is_file()
}

/// 把汽水 app.asar 还原成官方原版，并写入「不要自动打补丁」开关。
pub fn spawn_restore() -> Result<(), String> {
    if !backup_available() {
        return Err("没有官方原版备份（data\\app.asar.bak）".into());
    }
    let _ = std::fs::write(disabled_flag(), b"1");
    std::thread::spawn(|| {
        dlog("manual restore: start");
        if soda_running() {
            let _ = std::process::Command::new("taskkill")
                .args(["/IM", "SodaMusic.exe", "/F"])
                .status();
            std::thread::sleep(Duration::from_millis(1500));
        }
        let Some(asar) = find_asar() else {
            dlog("manual restore: app.asar not found");
            return;
        };
        let bak = crate::paths::file("app.asar.bak");
        let data = match std::fs::read(&bak) {
            Ok(d) => d,
            Err(e) => {
                dlog(&format!("manual restore: read backup failed: {e}"));
                return;
            }
        };
        let tmp = asar.with_extension("asar.restore.tmp");
        let wrote = (|| -> Result<(), String> {
            std::fs::write(&tmp, &data).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &asar).map_err(|e| e.to_string())
        })();
        if let Err(e) = wrote {
            let _ = std::fs::remove_file(&tmp);
            if let Err(ue) = elevated_copy_expect(&bak, &asar, false) {
                dlog(&format!("manual restore: failed ({e}); elevate failed: {ue}"));
                return;
            }
        }
        dlog("manual restore: original restored");
        if !soda_running() {
            let _ = launch_soda();
        }
    });
    Ok(())
}

/// 重新启用自动补丁（撤销「还原官方原版」），并立即修复一次。
pub fn spawn_enable_patch() -> Result<(), String> {
    let _ = std::fs::remove_file(disabled_flag());
    spawn_repair()
}

/// 需要时自动打补丁。返回 Ok(true)=本次新打了补丁；Ok(false)=本来就有补丁；
/// Err=失败（例如文件被占用）。
pub fn ensure_patched() -> Result<bool, String> {
    if patch_disabled() {
        return Err("用户已选择还原汽水官方原版".into());
    }
    let Some(asar) = find_asar() else { return Err("找不到汽水 app.asar".into()) };
    let (patched, entry) = inspect(&asar)?;
    if patched {
        return Ok(false);
    }
    let base = if let Some(i) = entry.find("/*myisle-cdp*/") {
        &entry[i + "/*myisle-cdp*/".len()..]
    } else {
        entry.as_str()
    };
    let new_entry = format!("{INJECT}{base}");
    let rebuilt = repack(&asar, new_entry.as_bytes())?;

    // 备份「未打补丁的原版」（不是当前文件——当前可能已带旧版补丁）。
    let bak = crate::paths::file("app.asar.bak");
    if let Ok(pristine) = repack(&asar, base.as_bytes()) {
        let _ = std::fs::write(&bak, &pristine);
    }

    // 先写临时文件再替换，避免写坏。
    let tmp = asar.with_extension("asar.myisle.tmp");
    let wrote = (|| -> Result<(), String> {
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(&rebuilt).map_err(|e| e.to_string())?;
        f.flush().ok();
        Ok(())
    })();
    if let Err(e) = wrote {
        // 无写权限（Program Files 安装）：把新包放到用户目录，再借 UAC 提权复制。
        let _ = std::fs::remove_file(&tmp);
        let staged = crate::paths::file("app.asar.patched.tmp");
        std::fs::write(&staged, &rebuilt).map_err(|e| format!("写暂存文件失败: {e}"))?;
        elevated_copy(&staged, &asar)
            .map_err(|ue| format!("提权打补丁失败（原始错误 {e}）: {ue}"))?;
        return Ok(true);
    }
    if let Err(e) = std::fs::rename(&tmp, &asar) {
        let staged = crate::paths::file("app.asar.patched.tmp");
        let _ = std::fs::copy(&tmp, &staged);
        let _ = std::fs::remove_file(&tmp);
        elevated_copy(&staged, &asar)
            .map_err(|ue| format!("替换 app.asar 失败（可能被占用: {e}），提权也失败: {ue}"))?;
    }
    Ok(true)
}

/// CDP 是否可用（/json/version 能通）。
pub fn cdp_up() -> bool {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_millis(400))
        .timeout_read(Duration::from_millis(600))
        .build();
    match agent.get("http://127.0.0.1:9222/json/version").call() {
        Ok(r) => r.status() == 200,
        Err(_) => false,
    }
}

/// 供设置页展示的「汽水状态」缓存：返回上次结果，过期后在后台刷新，绝不阻塞 UI 线程。
pub fn status_cached() -> (bool, bool) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<Option<(Instant, (bool, bool))>>> = OnceLock::new();
    static REFRESHING: AtomicBool = AtomicBool::new(false);
    let m = CACHE.get_or_init(|| Mutex::new(None));
    let cached = m.lock().ok().and_then(|g| *g);
    if let Some((t, s)) = cached {
        if t.elapsed() < Duration::from_secs(3) {
            return s;
        }
    }
    if !REFRESHING.swap(true, Ordering::Relaxed) {
        std::thread::spawn(move || {
            let s = (soda_running(), cdp_up());
            if let Some(m) = CACHE.get() {
                if let Ok(mut g) = m.lock() {
                    *g = Some((Instant::now(), s));
                }
            }
            REFRESHING.store(false, Ordering::Relaxed);
        });
    }
    cached.map(|(_, s)| s).unwrap_or((false, false))
}
pub fn soda_running() -> bool {
    !crate::soda_lyrics::soda_pids().is_empty()
}

fn desktop_shortcut() -> PathBuf {
    let user = std::env::var("USERPROFILE").unwrap_or_else(|_| r"C:\Users\23048".into());
    Path::new(&user).join("Desktop").join("汽水音乐.lnk")
}

/// 经桌面快捷方式拉起汽水（走 launcher 的正常启动流程）。
fn launch_soda() -> Result<(), String> {
    let lnk = desktop_shortcut();
    if !lnk.is_file() {
        return Err(format!("找不到汽水快捷方式: {}", lnk.display()));
    }
    std::process::Command::new("explorer.exe")
        .arg(&lnk)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 重启汽水（先关闭再经快捷方式拉起）。
pub fn restart_soda() -> Result<(), String> {
    let _ = std::process::Command::new("taskkill")
        .args(["/IM", "SodaMusic.exe", "/F"])
        .status();
    std::thread::sleep(Duration::from_millis(1500));
    launch_soda()
}

/// 设置页「汽水连接」行的「一键修复」：后台线程里 关闭汽水 → 重打补丁 → 重新拉起。
/// 不阻塞 UI；状态缓存 3 秒后会自然刷新。
pub fn spawn_repair() -> Result<(), String> {
    std::thread::spawn(|| {
        dlog("manual repair: start");
        if soda_running() {
            let _ = std::process::Command::new("taskkill")
                .args(["/IM", "SodaMusic.exe", "/F"])
                .status();
            std::thread::sleep(Duration::from_millis(1500));
        }
        match ensure_patched() {
            Ok(true) => dlog("manual repair: patch applied"),
            Ok(false) => dlog("manual repair: already patched"),
            Err(e) => dlog(&format!("manual repair: patch failed: {e}")),
        }
        if !soda_running() {
            if let Err(e) = launch_soda() {
                dlog(&format!("manual repair: launch failed: {e}"));
            }
        }
        std::thread::sleep(Duration::from_secs(4));
        dlog(&format!(
            "manual repair: done running={} cdp={}",
            soda_running(),
            cdp_up()
        ));
    });
    Ok(())
}

/// 启动时调用一次：确保补丁可用。汽水在运行但补丁缺失时，自动重启一次把它补好；
/// 若补丁已在、只是 CDP 还没热起来，则不动它（留给 tick 处理）。
pub fn boot_check() {
    if patch_disabled() {
        dlog("boot: patch disabled by user (restored official asar)");
        return;
    }
    if !soda_running() {
        match ensure_patched() {
            Ok(true) => dlog("boot: soda patch applied (not running)"),
            _ => {}
        }
        return;
    }
    let patched = find_asar().map(|a| inspect(&a).map(|x| x.0).unwrap_or(false)).unwrap_or(false);
    if patched {
        return;
    }
    let _ = std::process::Command::new("taskkill").args(["/IM", "SodaMusic.exe", "/F"]).status();
    std::thread::sleep(Duration::from_millis(1500));
    match ensure_patched() {
        Ok(true) => dlog("boot: soda patch applied after stopping SodaMusic"),
        Ok(false) => dlog("boot: already patched after stop"),
        Err(e) => { dlog(&format!("boot: soda patch failed: {e}")); return; }
    }
    match launch_soda() {
        Ok(()) => dlog("boot: soda relaunched with CDP patch"),
        Err(e) => dlog(&format!("boot: soda relaunch failed: {e}")),
    }
}
/// 供主循环调用：节流执行「检查 + 补丁 + 必要时重启」。
///
/// 汽水运行时会**独占锁定 app.asar**，所以补丁只能在汽水关闭时写。因此策略是：
/// - 未打补丁：若汽水在跑 → 关掉它 → 打补丁 → 重新拉起（一次更新后最多打断这一次）；
/// - 已打补丁但 CDP 不通（通常是刚打补丁还没重启）→ 未在播放时重启让它生效；
/// 播放中则等暂停后再重启，避免正听着被打断。
pub fn tick(playing: bool) {
    if patch_disabled() {
        return;
    }
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    static LAST_RESTART: Mutex<Option<Instant>> = Mutex::new(None);
    let now = Instant::now();
    {
        let mut last = LAST.lock().unwrap();
        if let Some(t) = *last {
            if now.duration_since(t) < Duration::from_secs(15) {
                return;
            }
        }
        *last = Some(now);
    }
    if cdp_up() {
        return;
    }
    let patched = match find_asar() {
        Some(a) => inspect(&a).map(|x| x.0).unwrap_or(false),
        None => return,
    };

    // 冷却：避免反复重启。
    let mut cooldown_ok = || {
        let mut lr = LAST_RESTART.lock().unwrap();
        if let Some(t) = *lr {
            if now.duration_since(t) < Duration::from_secs(300) {
                return false;
            }
        }
        *lr = Some(now);
        true
    };

    if !patched {
        if soda_running() {
            if !cooldown_ok() {
                return;
            }
            // 必须先关汽水，asar 被独占锁时才能写入补丁。
            let _ = std::process::Command::new("taskkill")
                .args(["/IM", "SodaMusic.exe", "/F"])
                .status();
            std::thread::sleep(Duration::from_millis(1500));
            match ensure_patched() {
                Ok(true) => dlog("soda patch applied after stopping SodaMusic"),
                Ok(false) => dlog("soda patch: already patched after stop"),
                Err(e) => {
                    dlog(&format!("soda patch failed after stop: {e}"));
                    return;
                }
            }
            match launch_soda() {
                Ok(()) => dlog("soda launched to activate CDP patch"),
                Err(e) => dlog(&format!("soda relaunch failed: {e}")),
            }
        } else {
            match ensure_patched() {
                Ok(true) => dlog("soda patch applied (SodaMusic not running)"),
                Ok(false) => {}
                Err(e) => dlog(&format!("soda patch failed: {e}")),
            }
        }
        return;
    }

    // 已打补丁但 CDP 仍不通：多半是打补丁后还没重启汽水，或渲染进程崩了。
    if soda_running() {
        static DOWN_SINCE: Mutex<Option<Instant>> = Mutex::new(None);
        if playing {
            // 播放中一般先不打断；但如果 CDP 持续 60s 都不通，说明确实坏了，强制重启一次自愈。
            let mut d = DOWN_SINCE.lock().unwrap();
            let since = *d.get_or_insert(now);
            if now.duration_since(since) >= Duration::from_secs(60) && cooldown_ok() {
                let _ = restart_soda();
                *d = None;
                dlog("soda CDP down >60s while playing: force restarted");
            } else {
                dlog("soda patched; CDP down, will retry/restart after playback pauses");
            }
            return;
        }
        *DOWN_SINCE.lock().unwrap() = None;
        if !cooldown_ok() {
            return;
        }
        match restart_soda() {
            Ok(()) => dlog("soda restarted to activate CDP patch"),
            Err(e) => dlog(&format!("soda restart failed: {e}")),
        }
    }
}

/// 诊断/测试入口：打印找到的 asar、是否已打补丁。
pub fn probe_report() -> Result<String, String> {
    let Some(asar) = find_asar() else { return Err("找不到 app.asar".into()) };
    let (patched, entry) = inspect(&asar)?;
    Ok(format!(
        "asar={} patched={} entry_len={} cdp_up={} running={}",
        asar.display(),
        patched,
        entry.len(),
        cdp_up(),
        soda_running()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn probe_soda_patch_env() {
        match probe_report() {
            Ok(s) => println!("PATCHREPORT {s}"),
            Err(e) => println!("PATCHERR {e}"),
        }
    }

    /// 端到端演练「还原官方原版 → 重新启用补丁」（会真的重启汽水，慎用）。
    #[test]
    #[ignore]
    fn probe_restore_roundtrip() {
        let src_bak = std::path::PathBuf::from(r"D:\LingDao\data\app.asar.bak");
        let dst_bak = crate::paths::file("app.asar.bak");
        if src_bak.is_file() && src_bak != dst_bak {
            let _ = std::fs::create_dir_all(dst_bak.parent().unwrap());
            let _ = std::fs::copy(&src_bak, &dst_bak);
        }
        let status = || {
            (
                patch_disabled(),
                find_asar().map(|a| inspect(&a).map(|x| x.0).unwrap_or(false)),
            )
        };
        println!("BEFORE backup={} disabled={} patched={:?}", backup_available(), status().0, status().1);
        spawn_restore().expect("restore");
        std::thread::sleep(Duration::from_secs(12));
        println!("AFTER_RESTORE disabled={} patched={:?}", status().0, status().1);
        spawn_enable_patch().expect("enable");
        std::thread::sleep(Duration::from_secs(18));
        println!("AFTER_ENABLE disabled={} patched={:?}", status().0, status().1);
    }

    /// 从当前（已打补丁）asar 里剥掉注入，导出纯净原版到 data\app.asar.orig 并刷新备份。
    #[test]
    #[ignore]
    fn probe_export_pristine() {
        let asar = find_asar().expect("asar");
        let (_p, entry) = inspect(&asar).expect("inspect");
        let base = if let Some(i) = entry.find("/*myisle-cdp2*/") {
            &entry[i + "/*myisle-cdp2*/".len()..]
        } else if let Some(i) = entry.find("/*myisle-cdp*/") {
            &entry[i + "/*myisle-cdp*/".len()..]
        } else {
            entry.as_str()
        };
        let pristine = repack(&asar, base.as_bytes()).expect("repack");
        let out = std::path::PathBuf::from(r"D:\LingDao\data\app.asar.orig");
        std::fs::write(&out, &pristine).expect("write");
        std::fs::write(crate::paths::file("app.asar.bak"), &pristine).expect("bak");
        println!("pristine len={} patched_check={:?}", pristine.len(), inspect(&out).map(|x| x.0));
    }
    /// 端到端演练（会真的重启汽水，慎用）：恢复原版 asar → 启动汽水 → tick 自动修复。
    #[test]
    #[ignore]
    fn probe_live_heal_full() {
        let asar = find_asar().expect("asar");
        let bak = std::path::PathBuf::from(r"D:\LingDao\data\app.asar.bak");
        // 1) 关掉汽水，恢复未打补丁的原版
        let _ = std::process::Command::new("taskkill").args(["/IM", "SodaMusic.exe", "/F"]).status();
        std::thread::sleep(Duration::from_millis(1500));
        std::fs::copy(&bak, &asar).expect("restore original asar");
        println!("BEFORE patched={}", inspect(&asar).unwrap().0);
        // 2) 以未打补丁状态启动汽水
        launch_soda().expect("launch soda");
        std::thread::sleep(Duration::from_secs(5));
        println!("STARTED running={} cdp_up={}", soda_running(), cdp_up());
        // 3) 调用自检：应关汽水→打补丁→再拉起
        tick(false);
        std::thread::sleep(Duration::from_secs(6));
        println!(
            "AFTER patched={} cdp_up={} running={}",
            inspect(&asar).unwrap().0,
            cdp_up(),
            soda_running()
        );
    }
    #[test]
    #[ignore]
    fn probe_autopatch_copy() {
        let mut bak = crate::paths::file("app.asar.bak");
        if !bak.is_file() { bak = std::path::PathBuf::from(r"D:\LingDao\data\app.asar.bak"); }
        let base = std::env::temp_dir().join("myisle_patchsim");
        let res = base.join("resources");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&res).unwrap();
        std::fs::copy(&bak, res.join("app.asar")).unwrap();
        std::env::set_var("SODA_HOME", &base);
        let before = inspect(&res.join("app.asar")).unwrap().0;
        let patched = ensure_patched();
        let after = inspect(&res.join("app.asar")).unwrap();
        println!(
            "AUTOPATCH before_patched={} result={:?} after_patched={} entry_len={}",
            before,
            patched,
            after.0,
            after.1.len()
        );
        std::env::remove_var("SODA_HOME");
        let _ = std::fs::remove_dir_all(&base);
    }
    #[test]
    #[ignore]
    fn probe_repack_roundtrip() {
        let asar = find_asar().expect("asar");
        let (patched, entry) = inspect(&asar).expect("inspect");
        let base = if let Some(i) = entry.find("/*myisle-cdp*/") {
        &entry[i + "/*myisle-cdp*/".len()..]
    } else {
        entry.as_str()
    };
    let new_entry = format!("{INJECT}{base}");
        let out = repack(&asar, new_entry.as_bytes()).expect("repack");
        let tmp = std::path::Path::new("data").join("asar_roundtrip.asar");
        std::fs::write(&tmp, &out).expect("write tmp");
        let (p2, e2) = inspect(&tmp).expect("inspect tmp");
        // 校验所有文件范围合法
        let bytes = std::fs::read(&tmp).unwrap();
        let h = read_header(&bytes).expect("hdr");
        let mut leaves = Vec::new();
        let mut path = Vec::new();
        collect_leaves(&h.json, &mut path, &mut leaves);
        let mut bad = 0;
        for (_, size, off, unpacked) in &leaves {
            if *unpacked { continue; }
            if h.data_start + *off as usize + *size as usize > bytes.len() { bad += 1; }
        }
        println!(
            "ROUNDTRIP src_patched={} src_entry={} out_len={} tmp_patched={} tmp_entry={} leaves={} bad={}",
            patched, entry.len(), out.len(), p2, e2.len(), leaves.len(), bad
        );
        if std::env::var("KEEP_ROUNDTRIP").is_err() { let _ = std::fs::remove_file(&tmp); } else { println!("KEPT {}", tmp.display()); }
    }
}

static OFFSCREEN_SAVED: Mutex<Option<(isize, i32, i32, i32, i32, isize)>> = Mutex::new(None);

pub fn show_soda_offscreen() -> bool {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetWindowLongPtrW, SetWindowPos,
        ShowWindow, GWL_EXSTYLE, HWND_TOP, SM_CXVIRTUALSCREEN, SM_XVIRTUALSCREEN, SWP_NOACTIVATE,
        SWP_NOZORDER, SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    struct Ctx<'a> { pids: &'a [u32], found: Option<HWND> }
    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let ctx = &mut *(lp.0 as *mut Ctx);
        let mut pid = 0u32;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if ctx.pids.contains(&pid) && GetWindowTextLengthW(hwnd) > 0 {
            ctx.found = Some(hwnd);
            return BOOL(0);
        }
        BOOL(1)
    }
    let pids = crate::soda_lyrics::soda_pids();
    if pids.is_empty() { return false; }
    let mut ctx = Ctx { pids: &pids, found: None };
    unsafe { let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut _ as isize)); }
    let hwnd = match ctx.found { Some(h) => h, None => return false };
    unsafe {
        if IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() {
            return false;
        }
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let w = (r.right - r.left).max(1000);
        let h = (r.bottom - r.top).max(700);
        let old_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if let Ok(mut s) = OFFSCREEN_SAVED.lock() {
            *s = Some((hwnd.0 as isize, r.left, r.top, w, h, old_style));
        }
        // 从任务栏/Alt-Tab 隐藏，并禁止激活
        let new_style = old_style
            | (WS_EX_TOOLWINDOW.0 as isize)
            | (WS_EX_NOACTIVATE.0 as isize);
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
        // 放到所有显示器左侧之外的虚拟区域，确保任何多屏布局都不可见
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let _ = vw;
        let left_x = vx - w - 80;
        let _ = SetWindowPos(hwnd, HWND_TOP, left_x, 0, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    true
}

pub fn hide_soda_windows() {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, SetWindowLongPtrW, SetWindowPos,
        ShowWindow, GWL_EXSTYLE, HWND_TOP, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
    };
    struct Ctx<'a> { pids: &'a [u32] }
    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let ctx = &mut *(lp.0 as *mut Ctx);
        let mut pid = 0u32;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if ctx.pids.contains(&pid) && GetWindowTextLengthW(hwnd) > 0 {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        BOOL(1)
    }
    let pids = crate::soda_lyrics::soda_pids();
    if pids.is_empty() { return; }
    let mut ctx = Ctx { pids: &pids };
    unsafe { let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut _ as isize)); }
    if let Ok(mut s) = OFFSCREEN_SAVED.lock() {
        if let Some((hw, ox, oy, ow, oh, old_style)) = s.take() {
            let h = HWND(hw as *mut _);
            unsafe {
                let _ = SetWindowLongPtrW(h, GWL_EXSTYLE, old_style);
                let _ = SetWindowPos(h, HWND_TOP, ox, oy, ow, oh,
                    SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
    }
}