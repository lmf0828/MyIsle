//! 汽水音乐「正在播放」的后备数据源（走 CDP，不依赖系统 SMTC）。
//!
//! 背景：本机 SMTC 的 GlobalSystemMediaTransportControlsSessionManager::RequestAsync
//! 有时会整体卡死（即使汽水在放歌也拿不到任何会话），导致岛上识别不到音乐。
//! 汽水已被打了 CDP 调试端口补丁，可以直接从渲染进程读取当前曲目：
//! - 优先读底部播放条 `.bottom-player`（歌单/场景模式/播放页都常驻，最稳）；
//! - 读不到再退回「正在播放页」的 Vue LyricsView（可拿到逐字歌词）。
//! 这里在后台低频采样并缓存，主线程在 SMTC 拿不到标题时用它兜底。

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

struct Now {
    name: String,
    artist: String,
    duration: f64,
    position: f64,
    playing: bool,
    cover: Option<Arc<Vec<u8>>>,
    at: Instant,
}

static NEED: AtomicBool = AtomicBool::new(false);
static NOW: OnceLock<Mutex<Option<Now>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<Now>> {
    NOW.get_or_init(|| Mutex::new(None))
}

/// 主线程每帧告知：现在是否真的需要汽水后备数据（SMTC 没有标题且播放器在跑）。
pub fn request(need: bool) {
    NEED.store(need, Ordering::Relaxed);
}

/// 读取最近一次采样（较久未更新则失效）。
/// 返回 (歌名, 歌手, 时长, 位置, 是否在播, 封面图字节)。
/// 后台轮询为“不打扰用户”不会主动把汽水切回播放页，故用户浏览其它页面时可能短暂取不到新值；
/// 这里对“正在播放”的曲目按经过时间顺延进度，避免进度停在原地。
pub fn snapshot() -> Option<(String, String, f64, f64, bool, Option<Arc<Vec<u8>>>)> {
    let g = cell().lock().ok()?;
    let n = g.as_ref()?;
    let age = n.at.elapsed();
    // 仅在采样仍「新鲜」时使用：正常情况下后台每 ~1.1s 刷新一次；若汽水切到歌单/场景等
    // 读不到歌名的页面，刷新会一直失败，此时不能再拿几分钟前的陈旧标题冒充当前歌曲
    // （否则会显示上一条的歌名、封面/歌词全对不上）。超过 8s 视为失效，交回上层用 SMTC。
    if age > Duration::from_secs(8) {
        return None;
    }
    let mut pos = n.position;
    if n.playing {
        pos += age.as_secs_f64();
        if n.duration > 1.0 && pos > n.duration {
            pos = n.duration;
        }
    }
    Some((
        n.name.clone(),
        n.artist.clone(),
        n.duration,
        pos,
        n.playing,
        n.cover.clone(),
    ))
}

fn fetch_cover_bytes(url: &str) -> Option<Vec<u8>> {
    if url.is_empty() {
        return None;
    }
    let resp = ureq::get(url)
        .timeout(Duration::from_millis(1800))
        .call()
        .ok()?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(6 * 1024 * 1024)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.is_empty() {
        None
    } else {
        Some(buf)
    }
}

/// 启动后台采样线程（进程内只调一次）。
pub fn spawn() {
    if std::env::var_os("WINISLE_NOSODA").is_some() {
        return;
    }
    std::thread::Builder::new()
        .name("winisle-soda-now".into())
        .spawn(|| {
            let mut last_cover_url = String::new();
            let mut cover: Option<Arc<Vec<u8>>> = None;
            let mut last_cur: f64 = f64::NAN;
            let mut last_change = Instant::now();
            let mut last_name = String::new();
            loop {
                if !NEED.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(300));
                    continue;
                }
                if !crate::soda_patch::cdp_up() {
                    // CDP 不可用：清掉旧值，等待下次
                    if let Ok(mut g) = cell().lock() {
                        *g = None;
                    }
                    cover = None;
                    last_cover_url.clear();
                    std::thread::sleep(Duration::from_millis(1200));
                    continue;
                }
                // 优先从底部播放条读（歌单/场景模式也常驻，不依赖 LyricsView）；读不到再退回 LyricsView。
                let sl_opt = crate::cdp_lyric::fetch_bottom_player()
                    .or_else(|| crate::cdp_lyric::fetch_current_no_nav("", ""));
                if let Some(sl) = sl_opt {
                    if !sl.name.trim().is_empty() {
                        let prog = crate::cdp_lyric::fetch_progress(sl.duration);
                        let (pos, total) = match prog {
                            Some((c, t)) => (c, if t > 1.0 { t } else { sl.duration }),
                            None => (0.0, sl.duration),
                        };
                        let dur = if total > 1.0 { total } else { sl.duration };
                        // 判断是否在播：真实进度（pct 亚秒级）在推进 → 播放中；
                        // 连续 2.5s 不动 → 视为暂停。不再用整数秒文本做比较（1.1s 轮询下
                        // 会把“同一秒”频繁误判为暂停，进而冻结歌词时钟 → 歌词跟不上/自己暂停）。
                        if sl.name != last_name {
                            last_name = sl.name.clone();
                            last_cur = pos;
                            last_change = Instant::now();
                        }
                        if !last_cur.is_nan() && (pos - last_cur).abs() > 0.12 {
                            last_change = Instant::now();
                        }
                        last_cur = pos;
                        let playing = last_change.elapsed() < Duration::from_millis(2500);
                        // 封面：切换歌曲/换 URL 时才下载，避免每轮重复拉图
                        if !sl.cover.is_empty() && sl.cover != last_cover_url {
                            if let Some(bytes) = fetch_cover_bytes(&sl.cover) {
                                cover = Some(Arc::new(bytes));
                                last_cover_url = sl.cover.clone();
                            }
                        }
                        if let Ok(mut g) = cell().lock() {
                            *g = Some(Now {
                                name: sl.name,
                                artist: sl.artist,
                                duration: dur,
                                position: pos,
                                playing,
                                cover: cover.clone(),
                                at: Instant::now(),
                            });
                        }
                        std::thread::sleep(Duration::from_millis(1100));
                        continue;
                    }
                }
                std::thread::sleep(Duration::from_millis(1400));
            }
        })
        .ok();
}
