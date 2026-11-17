//! 读取 Windows 系统媒体传输控制 (SMTC) 的当前播放信息。
//! 通用方案：任何接入系统媒体控制的播放器（网易云/QQ音乐/汽水/Spotify/Edge 网页播放等）
//! 都能读到 歌名/歌手/专辑/播放状态/进度，并可下发 播放/暂停/上一首/下一首。
//!
//! 实现要点（参考岛仔 Daozai 的 smtc.rs 做法，本机实测更稳）：
//! - 用 MTA 套间：`CoIncrementMTAUsage()` 保活 MTA，而不是在后台线程 `CoInitializeEx`+`.get()`。
//! - **绝不阻塞等待**：`RequestAsync()` 只发起操作，之后轮询 `op.Status()`；未完成就下一轮再看，
//!   超时才丢弃重试（对应岛仔的 media_result / PENDING 队列）。这样即使系统媒体服务一时卡住，
//!   也不会把读取线程永久堵死。
//! - 播放位置用 `GetTimelineProperties()` 的真实 Position/EndTime（单位 100ns），因此**播放器最小化到
//!   托盘也能读到进度**——这正是窗口标题兜底做不到的。
//! - 会话通过 `GetSessions()` 枚举（最小化仍保留会话），按“正在播放 + 上次 app 稳定”挑选。

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Foundation::{AsyncStatus, IAsyncOperation};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Mgr,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PbStatus,
};

#[derive(Clone, Default)]
pub struct Track {
    pub has_session: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub position: f64, // 秒（真实时间轴位置）
    pub duration: f64, // 秒
    pub app_id: String,
    pub has_position: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Cmd {
    Prev,
    Play,
    Pause,
    Next,
    Seek(f64),
}

pub struct Tracker {
    shared: Arc<Mutex<Track>>,
    cmd_tx: Sender<Cmd>,
}

impl Tracker {
    pub fn new() -> Self {
        let shared = Arc::new(Mutex::new(Track::default()));
        let (cmd_tx, cmd_rx) = channel::<Cmd>();
        let sh = shared.clone();
        std::thread::spawn(move || worker(sh, cmd_rx));
        Self { shared, cmd_tx }
    }

    /// 每次 UI 帧调用；只读共享快照，绝不阻塞。
    pub fn snapshot(&mut self) -> Track {
        self.shared.lock().map(|t| t.clone()).unwrap_or_default()
    }

    /// 下发播放控制：发到工作线程，用工作线程已持有的有效会话执行。
    pub fn send(&mut self, c: Cmd) {
        let _ = self.cmd_tx.send(c);
    }
}

/// 轮询一个 WinRT 异步操作直到完成或超时（MTA 下无需消息泵）。
fn wait_op<T: windows::core::RuntimeType>(op: &IAsyncOperation<T>, timeout: Duration) -> Option<T> {
    let st = Instant::now();
    loop {
        match op.Status() {
            Ok(s) => {
                if s == AsyncStatus::Completed {
                    return op.GetResults().ok();
                }
                if s == AsyncStatus::Error || s == AsyncStatus::Canceled {
                    return None;
                }
            }
            Err(_) => return None,
        }
        if st.elapsed() >= timeout {
            return None;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn worker(shared: Arc<Mutex<Track>>, cmd_rx: Receiver<Cmd>) {
    // 保活 MTA：整个线程生命周期内有效
    let _apt = unsafe { windows::Win32::System::Com::CoIncrementMTAUsage().ok() };
    dlog(&format!("worker MTA init ok={}", _apt.is_some()));

    let mut mgr: Option<Mgr> = None;
    let mut prefer = String::new();
    let mut last_session: Option<Session> = None;
    let mut last_title = String::new();
    let mut meta_title = String::new();
    let mut meta_artist = String::new();
    let mut meta_album = String::new();
    let mut last_position = -1.0f64;
    let mut last_pos_at = Instant::now();
    let mut tick: u64 = 0;

    loop {
        // 1) 获取 manager（发起 + 轮询，不永久阻塞）
        if mgr.is_none() {
            match Mgr::RequestAsync() {
                Ok(op) => {
                    if let Some(m) = wait_op(&op, Duration::from_millis(1500)) {
                        dlog("manager OK");
                        mgr = Some(m);
                    } else {
                        dlog("manager timeout, retry later");
                        std::thread::sleep(Duration::from_millis(600));
                        continue;
                    }
                }
                Err(e) => {
                    dlog(&format!("RequestAsync ERR: {}", e.message()));
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            }
        }

        let m = mgr.as_ref().unwrap();

        // 2) 选当前该展示的会话
        let session = pick_session(m, &prefer);
        if let Some(s) = &session {
            last_session = Some(s.clone());
        }
        let sess_eff = session.as_ref().or(last_session.as_ref());

        // 3) 处理控制命令（并在这里用短等待"挂着"等命令，命令一到达立刻执行，零延迟）
        while let Ok(c) = cmd_rx.try_recv() {
            exec_cmd(c, sess_eff);
        }

        // 4) 读取快照
        tick += 1;
        let fetch_props = (tick % 3 == 1) || last_title.is_empty();
        let mut t = match sess_eff {
            Some(s) => read_track(s, fetch_props),
            None => Track::default(),
        };
        if t.has_session && !t.app_id.is_empty() {
            prefer = t.app_id.clone();
        }
        // 未取属性的轮次用上次已知的歌名/歌手/专辑补齐，避免 title 在快照里忽有忽无导致 UI 闪。
        if fetch_props {
            if !t.title.is_empty() {
                meta_title = t.title.clone();
                meta_artist = t.artist.clone();
                meta_album = t.album.clone();
            }
        } else {
            t.title = meta_title.clone();
            t.artist = meta_artist.clone();
            t.album = meta_album.clone();
        }
        // 播放位置采样去重：同一首歌同一位置反复上报时，保留首次采样时刻，避免进度“冻结”
        if t.has_position && t.playing && t.title == last_title && (t.position - last_position).abs() < 0.05 {
            // 保持 last_pos_at 不变（不刷新），UI 侧据此继续外推
        } else {
            last_position = t.position;
            last_pos_at = Instant::now();
            last_title = t.title.clone();
        }
        let _ = last_pos_at;
        if tick % 5 == 0 {
            dlog(&format!(
                "snap title={:?} app={:?} haspos={} pos={:.2} dur={:.1} playing={}",
                t.title, t.app_id, t.has_position, t.position, t.duration, t.playing
            ));
        }

        if let Ok(mut g) = shared.lock() {
            *g = t;
        }

        match cmd_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(c) => {
                // 命令一到立刻执行（不必等下一轮轮询），随后马上进入下一轮刷新快照
                exec_cmd(c, sess_eff);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn exec_cmd(c: Cmd, sess: Option<&Session>) {
    dlog(&format!("got cmd {:?}", c));
    match sess {
        Some(s) => {
            let op = match c {
                Cmd::Play => s.TryPlayAsync().ok(),
                Cmd::Pause => s.TryPauseAsync().ok(),
                Cmd::Next => s.TrySkipNextAsync().ok(),
                Cmd::Prev => s.TrySkipPreviousAsync().ok(),
                Cmd::Seek(sec) => s.TryChangePlaybackPositionAsync((sec.max(0.0) * 1.0e7) as i64).ok(),
            };
            match op {
                Some(op) => match wait_op(&op, Duration::from_millis(1200)) {
                    Some(b) => dlog(&format!("cmd {:?} ok={}", c, b)),
                    None => {
                        dlog(&format!("cmd {:?} timeout -> media key", c));
                        media_fallback(c);
                    }
                },
                None => {
                    dlog("cmd no-op(no op) -> media key");
                    media_fallback(c);
                }
            }
        }
        None => {
            media_fallback(c);
            dlog(&format!("cmd {:?} via media key", c));
        }
    }
}

fn media_fallback(c: Cmd) {
    let vk = match c {
        Cmd::Play | Cmd::Pause => 0xB3u8, // VK_MEDIA_PLAY_PAUSE（无会话时只能切换）
        Cmd::Next => 0xB0u8,
        Cmd::Prev => 0xB1u8,
        Cmd::Seek(_) => return, // 无会话时无法定位
    };
    crate::platform::media_key(vk);
}

fn read_track(s: &Session, fetch_props: bool) -> Track {
    let mut t = Track::default();
    t.has_session = true;
    t.app_id = s
        .SourceAppUserModelId()
        .map(|x| x.to_string())
        .unwrap_or_default();
    t.playing = s
        .GetPlaybackInfo()
        .ok()
        .and_then(|i| i.PlaybackStatus().ok())
        .map(|st| st == PbStatus::Playing)
        .unwrap_or(false);

    // 真实时间轴（最小化到托盘也有）
    if let Ok(tl) = s.GetTimelineProperties() {
        if let Ok(p) = tl.Position() {
            t.position = p.Duration as f64 / 1.0e7;
            t.has_position = true;
        }
        if let Ok(e) = tl.EndTime() {
            t.duration = e.Duration as f64 / 1.0e7;
        }
    }

    // 媒体属性：不必每轮都拉（每次会阻塞等待一下），每 3 轮或标题为空时刷新一次，保证控制/位置低延迟
    if fetch_props {
        if let Ok(op) = s.TryGetMediaPropertiesAsync() {
            if let Some(p) = wait_op(&op, Duration::from_millis(500)) {
                t.title = p.Title().map(|x| x.to_string()).unwrap_or_default();
                t.artist = p.Artist().map(|x| x.to_string()).unwrap_or_default();
                t.album = p.AlbumTitle().map(|x| x.to_string()).unwrap_or_default();
            }
        }
    }
    t
}

/// 在所有媒体会话里挑一个“当前该展示”的：
/// 优先正在播放的；多路在播时优先保持上次的播放器；都不在播则取第一个非空会话；最后回退 GetCurrentSession。
fn pick_session(m: &Mgr, prefer_app: &str) -> Option<Session> {
    let mut first_any: Option<Session> = None;
    let mut first_playing: Option<Session> = None;
    match m.GetSessions() {
        Ok(list) => {
            let n = list.Size().unwrap_or(0);
            dlog(&format!("sessions n={}", n));
            for i in 0..n {
                if let Ok(s) = list.GetAt(i) {
                    let playing = s
                        .GetPlaybackInfo()
                        .ok()
                        .and_then(|x| x.PlaybackStatus().ok())
                        .map(|st| st == PbStatus::Playing)
                        .unwrap_or(false);
                    let app = s
                        .SourceAppUserModelId()
                        .map(|x| x.to_string())
                        .unwrap_or_default();
                    if playing {
                        if !prefer_app.is_empty() && app == prefer_app {
                            return Some(s);
                        }
                        if first_playing.is_none() {
                            first_playing = Some(s);
                        }
                    } else if first_any.is_none() {
                        first_any = Some(s);
                    }
                }
            }
        }
        Err(e) => dlog(&format!("GetSessions ERR {}", e.message())),
    }
    let r = first_playing.or(first_any);
    if r.is_none() {
        if let Ok(s) = m.GetCurrentSession() {
            return Some(s);
        }
    }
    r
}

fn dlog(s: &str) {
    use std::io::Write;
    {
        let p = crate::paths::file("smtc_debug.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
            let _ = writeln!(f, "{}", s);
        }
    }
}
