#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! WinIsle —??Windows 灵动岛?
//! 技术栈：Rust + eframe/egui(glow) + 原始 Win32/DWM FFI + 系统托盘(tray-icon)?
//! 交互：托盘小图标 ?弹出设置面板；关闭面??隐藏到托?/ 彻底退出?

mod platform;
mod tray;
mod smtc;
mod lyrics;
mod netease_mem;
mod soda_lyrics;
mod cdp_lyric;
mod soda_patch;
mod update;
mod paths;
mod weather;
mod audio_meter;
mod soda_now;

use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tray::{Tray, TrayAction};

// 收起?/ 展开态的窗口尺寸（逻辑点）
const W_COLLAPSED: f32 = 150.0;
const W_COLLAPSED_MUSIC: f32 = 356.0;
/// 收起态「待机表情」模式（无音乐）时的胶囊最小宽度（贴合表情画面，两端圆角）
const W_COLLAPSED_FACE: f32 = 72.0;
/// 待机表情名称（与 draw_robot_face 的 expr 编号一一对应）
const FACES: [&str; 14] = [
    "平静", "开心", "难过", "生气", "哭泣", "爱心眼", "害羞",
    "思考", "眨眼", "酷", "亲亲", "自信", "困倦", "困惑",
];
/// 已删除（不再展示/可选）的表情，按 FACES 资产下标标记：哭泣=4、思考=7、困倦=12、困惑=13。
const FACE_REMOVED: [bool; 14] = [
    false, false, false, false, true, false, false,
    true, false, false, false, false, true, true,
];
/// 当前保留可选的表情资产下标（保持原资产编号，避免与表情帧数据错位）。
fn face_kept_indices() -> Vec<usize> {
    (0..FACES.len()).filter(|i| !FACE_REMOVED[*i]).collect()
}
const UPD_BADGE_W: f32 = 40.0;
const UPD_BADGE_H: f32 = 15.3;
const UPD_BADGE_GAP: f32 = 8.0;
// —— 收起态可组合组件 ——
/// 收起态各组件“实际可视宽度”（逻辑像素），用于让组件之间真正留出 CB_GAP 的间隙。
/// 频谱仅 3 根竖条（约 13px）、时钟是文字自身宽度，故槽宽取贴合内容，而非拍脑袋留白。
const CW_SPECTRUM: f32 = 13.0;
const CW_LYRICS: f32 = 200.0;
const CW_FACE: f32 = 48.0;
const CW_CLOCK: f32 = 34.0; // HH:MM 文字宽；显示秒时另加
const CW_CLOCK_SECS_EXTRA: f32 = 20.0;
const CW_WEATHER: f32 = 90.0;
/// 收起态表情“两眼间距”的收拢量（逻辑像素）：左右眼各向中间靠拢其一半。
const FACE_EYE_GAP_REDUCE: f32 = 2.0;
/// 收起条两端内边距与组件间距（每个模块左右各留 5px，模块之间即统一 10px）
const CB_EDGE: f32 = 5.0;
/// 首位模块左侧 / 末尾模块右侧额外再加的内边距：使最外缘也是 5(基础)+5 = 10，
/// 与相邻模块之间 5+5=10 的间距保持一致。
const CB_OUTER_EXTRA: f32 = 5.0;
const CB_GAP: f32 = 10.0;
/// 收起态歌词字号（逻辑像素）。收起态固定此值、不读配置（配置 lyric_font 仅作用于展开态）。
const LYRIC_COLLAPSED_PT: f32 = 13.0;
/// 收起条最大宽度 = 目前展开后的宽度（用户指定）
const CW_MAX: f32 = W_EXPANDED;
const H_COLLAPSED: f32 = 30.0;
const W_EXPANDED: f32 = 520.0;
const H_EXPANDED: f32 = 300.0;
/// 窗口比岛体最大尺寸四周各多出的透明外框（逻辑像素）。这样窗口区域(岛体)永远
/// 不触到窗口最外沿，窗口自带的边框/阴影始终被裁掉，不会在顶部或两侧闪出白色。
const MARGIN: f32 = 6.0;
// 岛体在屏幕顶端再多“压”进屏幕几个逻辑像素，确保顶部永远是实心填充，
// 任何 1px 的位置取整/描边抖动都不会在屏顶露出一条白边。
const TOP_OVERLAP: f32 = 3.0;
/// 展开态右侧内容区四周边距（逻辑像素）：左右上 16、底部 14。
/// 音乐页 / 设置页 / 占位页共用同一套边距，保证各页排版规整、互相对齐。
const CONTENT_PAD_X: f32 = 16.0;
const CONTENT_PAD_T: f32 = 16.0;
const CONTENT_PAD_B: f32 = 14.0;

// ============================ 停靠位置 ============================
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DockPosition {
    TopCenter,
    TopLeft,
    TopRight,
    LeftCenter,
    RightCenter,
    BottomCenter,
}

impl DockPosition {
    const ALL: [DockPosition; 6] = [
        DockPosition::TopCenter,
        DockPosition::TopLeft,
        DockPosition::TopRight,
        DockPosition::LeftCenter,
        DockPosition::RightCenter,
        DockPosition::BottomCenter,
    ];

    fn label(&self) -> &'static str {
        match self {
            DockPosition::TopCenter => "顶部居中",
            DockPosition::TopLeft => "左上",
            DockPosition::TopRight => "右上",
            DockPosition::LeftCenter => "左侧居中",
            DockPosition::RightCenter => "右侧居中",
            DockPosition::BottomCenter => "底部居中",
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            DockPosition::TopCenter => "top_center",
            DockPosition::TopLeft => "top_left",
            DockPosition::TopRight => "top_right",
            DockPosition::LeftCenter => "left_center",
            DockPosition::RightCenter => "right_center",
            DockPosition::BottomCenter => "bottom_center",
        }
    }

    fn from_str(s: &str) -> Self {
        match s.trim() {
            "top_left" => DockPosition::TopLeft,
            "top_right" => DockPosition::TopRight,
            "left_center" => DockPosition::LeftCenter,
            "right_center" => DockPosition::RightCenter,
            "bottom_center" => DockPosition::BottomCenter,
            _ => DockPosition::TopCenter,
        }
    }

    /// 贴边的两条边不做圆角，其余两条边?r 圆角——形?刘海"观感?
    fn corner_radius(&self, r: f32) -> egui::CornerRadius {
        let r = r.max(0.0) as u8;
        match self {
            DockPosition::TopCenter | DockPosition::TopLeft | DockPosition::TopRight => {
                egui::CornerRadius { nw: 0, ne: 0, sw: r, se: r }
            }
            DockPosition::BottomCenter => egui::CornerRadius { nw: r, ne: r, sw: 0, se: 0 },
            DockPosition::LeftCenter => egui::CornerRadius { nw: 0, ne: r, sw: 0, se: r },
            DockPosition::RightCenter => egui::CornerRadius { nw: r, ne: 0, sw: r, se: 0 },
        }
    }
}

/// 给定窗口尺寸，计算其在屏?逻辑?上的停靠原点?
fn dock_origin(pos: DockPosition, mx: f32, my: f32, w: f32, h: f32) -> egui::Pos2 {
    let (x, y) = match pos {
        DockPosition::TopCenter => ((mx - w) * 0.5, 0.0),
        DockPosition::TopLeft => (0.0, 0.0),
        DockPosition::TopRight => (mx - w, 0.0),
        DockPosition::BottomCenter => ((mx - w) * 0.5, my - h),
        DockPosition::LeftCenter => (0.0, (my - h) * 0.5),
        DockPosition::RightCenter => (mx - w, (my - h) * 0.5),
    };
    egui::pos2(x.max(0.0), y.max(0.0))
}

// ============================ 主题 ============================
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Theme {
    Dark,
    Light,
    System,
}

impl Theme {
    fn label(&self) -> &'static str {
        match self {
            Theme::Dark => "深色",
            Theme::Light => "浅色",
            Theme::System => "跟随系统",
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::System => "system",
        }
    }
    fn from_str(s: &str) -> Self {
        match s.trim() {
            "light" => Theme::Light,
            "system" => Theme::System,
            _ => Theme::Dark,
        }
    }
    fn is_light(&self) -> bool {
        match self {
            Theme::Light => true,
            Theme::Dark => false,
            Theme::System => system_prefers_light(),
        }
    }
}

/// 读取 Windows 是否使用浅色应用主题（默认深色）?
fn system_prefers_light() -> bool {
    let out = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "AppsUseLightTheme",
        ])
        .output();
    if let Ok(o) = out {
        let s = String::from_utf8_lossy(&o.stdout);
        if let Some(idx) = s.find("REG_DWORD") {
            let tail = &s[idx + "REG_DWORD".len()..];
            if let Some(tok) = tail.split_whitespace().next() {
                return tok.trim() == "0x1";
            }
        }
    }
    false
}

// ============================ 调色?============================
#[derive(Clone, Copy)]
struct Colors {
    bg: egui::Color32,
    card: egui::Color32,
    card2: egui::Color32,
    line: egui::Color32,
    line_soft: egui::Color32,
    text: egui::Color32,
    muted: egui::Color32,
    muted2: egui::Color32,
    accent: egui::Color32,
    accent_strong: egui::Color32,
    accent_dim: egui::Color32,
    danger: egui::Color32,
    danger_dim: egui::Color32,
}

fn palette(light: bool) -> Colors {
    let rgb = egui::Color32::from_rgb;
    let rgba = egui::Color32::from_rgba_unmultiplied;
    if light {
        Colors {
            bg: rgb(246, 247, 249),
            card: rgb(255, 255, 255),
            card2: rgb(240, 241, 244),
            line: rgb(223, 226, 232),
            line_soft: rgb(233, 235, 240),
            text: rgb(28, 30, 36),
            muted: rgb(91, 98, 115),
            muted2: rgb(139, 145, 161),
            accent: rgb(16, 185, 129),
            accent_strong: rgb(16, 185, 129),
            accent_dim: rgba(16, 185, 129, 28),
            danger: rgb(217, 75, 75),
            danger_dim: rgba(217, 75, 75, 24),
        }
    } else {
        Colors {
            bg: rgb(23, 24, 29),
            card: rgb(30, 32, 39),
            card2: rgb(35, 38, 46),
            line: rgb(42, 45, 54),
            line_soft: rgb(34, 36, 43),
            text: rgb(233, 234, 238),
            muted: rgb(154, 161, 176),
            muted2: rgb(107, 113, 128),
            accent: rgb(52, 211, 153),
            accent_strong: rgb(16, 185, 129),
            accent_dim: rgba(52, 211, 153, 40),
            danger: rgb(244, 114, 114),
            danger_dim: rgba(244, 114, 114, 30),
        }
    }
}

fn apply_style(ctx: &egui::Context, light: bool) {
    let c = palette(light);
    let mut style = (*ctx.style()).clone();
    style.visuals = if light { egui::Visuals::light() } else { egui::Visuals::dark() };
    style.visuals.panel_fill = c.bg;
    style.visuals.window_fill = c.bg;
    style.visuals.extreme_bg_color = c.card2;
    style.visuals.faint_bg_color = c.card;
    style.visuals.widgets.noninteractive.bg_fill = c.card;
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, c.line_soft);
    style.visuals.widgets.inactive.bg_fill = c.card2;
    style.visuals.widgets.inactive.weak_bg_fill = c.card2;
    style.visuals.widgets.hovered.bg_fill = c.card2;
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, c.line);
    style.visuals.widgets.active.bg_fill = c.accent_dim;
    style.visuals.selection.bg_fill = c.accent_dim;
    style.visuals.selection.stroke = egui::Stroke::new(1.0, c.accent);
    style.visuals.hyperlink_color = c.accent;
    style.visuals.override_text_color = Some(c.text);
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.slider_width = 150.0;
    ctx.set_style(style);
    // 提升形状抗锯齿：岛体是圆角胶囊，默认 1px feathering 在圆角/贴边处容易出现台阶状“锯齿”。
    // 加大 feathering 宽度，并关闭“把矩形对齐到整像素”，让圆角走亚像素抗锯齿，边缘更平滑。
    ctx.tessellation_options_mut(|o| {
        o.feathering = true;
        o.feathering_size_in_pixels = 1.5;
        o.round_rects_to_pixels = false;
    });
}

// ============================ 配置 ============================
#[derive(Clone, PartialEq, Debug)]
struct Config {
    position: DockPosition,
    /// 岛体中心在屏幕上的水平位置比?0..1)。顶端吸附时用它决定左右位置?
    x_frac: f32,
    /// 岛体中心在屏幕上的垂直位置比（0..1）。侧边（左/右）吸附时用它决定上下位置。
    y_frac: f32,
    /// 岛体所在显示器的物理左上角 X（虚拟屏坐标）。区分多显示器时使用。
    mon_x: f32,
    /// 岛体所在显示器的物理左上角 Y（虚拟屏坐标）。
    mon_y: f32,
    font: f32,
    delay_ms: u32,
    bg: [u8; 3],
    alpha: u8,
    radius: f32,
    show_seconds: bool,
    show_date: bool,
    autostart: bool,
    theme: Theme,
    /// 歌词时间校准（毫秒，正值=歌词提前）
    lyric_shift_ms: f32,
    /// 收起态歌词“换行上滚”动画开关
    roll_anim: bool,
    /// 上滚动画时长（毫秒）
    roll_ms: f32,
    /// 逐字渐绿开关
    char_green: bool,
    /// 超长歌词跑马灯开关
    marquee: bool,
    /// 跑马灯边缘渐隐开关（左侧淡出 / 右侧渐显）
    edge_fade: bool,
    /// 左右吸附开关：开启时靠近屏幕左右边缘会吸附到侧边；关闭时只吸附顶部
    side_snap: bool,
    /// 展开态歌词字号
    lyric_font: f32,
    /// 收起态频谱动画开关
    spectrum: bool,
    /// 展开态音乐面板底部「波纹频谱（水波反射）」开关
    water_spectrum: bool,
    /// 收起态且无音乐时的展示：0=时间，1=待机表情（旧版字段，仅用于迁移）
    collapsed_mode: u8,
    /// 待机表情编号（0..FACES.len()-1）
    idle_face: u8,
    /// 收起态组件布局：顺序即拼接顺序（左→右），每项前缀 `+` 启用 / `-` 关闭，逗号分隔。
    /// 例："+spectrum,+lyrics,-face,+clock,-weather"。空字符串表示按旧 collapsed_mode 迁移。
    collapsed_layout: String,
    /// 用户已在设置菜单看过更新的版本号（看过则收起态/设置角标不再出现）
    upd_seen_rail: String,
    /// 用户已点击过“软件更新”行的版本号（点过则行角标不再出现）
    upd_seen_row: String,
    /// 最近一次「已是最新版本」检查的本地日期（yyyy-mm-dd）；当天不再允许重复检查
    upd_checked_day: String,
    /// 天气：当前城市（中国天气网城市代码；"geo:纬度,经度" 表示全球地点�?
    weather_city: String,
    /// 天气：当前城市显示名（搜索选择的城市；�?自动定位�?
    weather_city_name: String,
    /// 天气温度单位�?=摄氏�?=华氏
    weather_unit: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            position: DockPosition::TopCenter,
            x_frac: 0.5,
            y_frac: 0.5,
            mon_x: 0.0,
            mon_y: 0.0,
            font: 21.0,
            delay_ms: 200,
            bg: [28, 28, 34],
            alpha: 255,
            radius: 16.0,
            show_seconds: false,
            show_date: false,
            autostart: false,
            theme: Theme::Dark,
            lyric_shift_ms: 0.0,
            roll_anim: true,
            roll_ms: 500.0,
            char_green: true,
            marquee: true,
            edge_fade: true,
            side_snap: true,
            lyric_font: 13.0,
            spectrum: true,
            water_spectrum: true,
            collapsed_mode: 0,
            idle_face: 1,
            collapsed_layout: String::new(),
            upd_seen_rail: String::new(),
            upd_seen_row: String::new(),
            upd_checked_day: String::new(),
            weather_city: String::new(), // �?= 自动定位（首次拉取时探测所在城市）
            weather_city_name: String::new(),
            weather_unit: 0,
        }
    }
}

fn config_path() -> PathBuf {
    crate::paths::ensure().join("config.ini")
}

/// 唤醒信号：重复启动时第二个进程写它，已运行实例监测到后打开设置面板?
fn trigger_path() -> PathBuf {
    config_path().with_file_name(".show_trigger")
}

fn trigger_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(trigger_path())
        .ok()
        .and_then(|m| m.modified().ok())
}

impl Config {
    fn load() -> Self {
        let mut cfg = Config::default();
        if let Ok(text) = std::fs::read_to_string(config_path()) {
            let text = text.trim_start_matches('\u{feff}');
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let Some((k, v)) = line.split_once('=') else { continue };
                let k = k.trim();
                let v = v.trim();
                match k {
                    "position" => cfg.position = DockPosition::from_str(v),
                    "x_frac" => cfg.x_frac = v.parse().unwrap_or(cfg.x_frac),
                    "y_frac" => cfg.y_frac = v.parse().unwrap_or(cfg.y_frac),
                    "mon_x" => cfg.mon_x = v.parse().unwrap_or(cfg.mon_x),
                    "mon_y" => cfg.mon_y = v.parse().unwrap_or(cfg.mon_y),
                    "font" => cfg.font = v.parse().unwrap_or(cfg.font),
                    "delay_ms" => cfg.delay_ms = v.parse().unwrap_or(cfg.delay_ms),
                    "bg" => {
                        let p: Vec<u8> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                        if p.len() == 3 {
                            cfg.bg = [p[0], p[1], p[2]];
                        }
                    }
                    "alpha" => cfg.alpha = v.parse().unwrap_or(cfg.alpha),
                    "radius" => cfg.radius = v.parse().unwrap_or(cfg.radius),
                    "show_seconds" => cfg.show_seconds = v == "1" || v == "true",
                    "show_date" => cfg.show_date = v == "1" || v == "true",
                    "collapsed_mode" => cfg.collapsed_mode = v.parse().unwrap_or(cfg.collapsed_mode),
                    "idle_face" => cfg.idle_face = v.parse().unwrap_or(cfg.idle_face),
                    "collapsed_layout" => cfg.collapsed_layout = v.to_string(),
                    "autostart" => cfg.autostart = v == "1" || v == "true",
                    "theme" => cfg.theme = Theme::from_str(v),
                    "lyric_shift_ms" => cfg.lyric_shift_ms = v.parse().unwrap_or(cfg.lyric_shift_ms),
                    "roll_anim" => cfg.roll_anim = v == "1" || v == "true",
                    "marquee" => cfg.marquee = v == "1" || v == "true",
                    "edge_fade" => cfg.edge_fade = v == "1" || v == "true",
                    "side_snap" => cfg.side_snap = v == "1" || v == "true",
                    "lyric_font" => cfg.lyric_font = v.parse().unwrap_or(cfg.lyric_font),
                    "spectrum" => cfg.spectrum = v == "1" || v == "true",
                    "water_spectrum" => cfg.water_spectrum = v == "1" || v == "true",
                    "upd_seen_rail" => cfg.upd_seen_rail = v.to_string(),
                    "upd_seen_row" => cfg.upd_seen_row = v.to_string(),
                    "upd_checked_day" => cfg.upd_checked_day = v.to_string(),
                    "weather_city" => cfg.weather_city = v.to_string(),
                    "weather_city_name" => cfg.weather_city_name = v.to_string(),
                    "weather_unit" => cfg.weather_unit = v.parse().unwrap_or(cfg.weather_unit),
                    _ => {}
                }
            }
        }
        // 支持 顶部 / 左侧 / 右侧 三种停靠：保留左/右，其余（含旧版 TopLeft/TopRight/BottomCenter）统一按顶部处理。
        cfg.position = match cfg.position {
            DockPosition::LeftCenter => DockPosition::LeftCenter,
            DockPosition::RightCenter => DockPosition::RightCenter,
            _ => DockPosition::TopCenter,
        };
        cfg.x_frac = cfg.x_frac.clamp(0.0, 1.0);
        cfg.y_frac = cfg.y_frac.clamp(0.0, 1.0);
        // 若待机表情指向已删除的表情，回退到第一个保留项。
        if (cfg.idle_face as usize) >= FACES.len() || FACE_REMOVED[cfg.idle_face as usize] {
            cfg.idle_face = face_kept_indices().first().copied().unwrap_or(0) as u8;
        }
        // 收起态组件布局缺失时，按旧 collapsed_mode 迁移
        if cfg.collapsed_layout.trim().is_empty() {
            cfg.collapsed_layout = if cfg.collapsed_mode == 1 {
                "-spectrum,-lyrics,+face,-clock,-weather".to_string()
            } else {
                "+spectrum,+lyrics,-face,+clock,-weather".to_string()
            };
        }
        // 天气固定跟随当前位置：忽略旧版本可能保存的手动城市。
        cfg.weather_city.clear();
        cfg.weather_city_name.clear();
        cfg
    }

    fn save(&self) {
        let text = format!(
            "# WinIsle 配置\nposition={}\nx_frac={}\ny_frac={}\nfont={}\ndelay_ms={}\nbg={},{},{}\nalpha={}\nradius={}\nshow_seconds={}\nshow_date={}\nautostart={}\ntheme={}\nlyric_shift_ms={}\nroll_anim={}\nroll_ms={}\nchar_green={}\nmarquee={}\nedge_fade={}\nside_snap={}\nlyric_font={}\nspectrum={}\nwater_spectrum={}\ncollapsed_mode={}\nidle_face={}\ncollapsed_layout={}\nupd_seen_rail={}\nupd_seen_row={}\nupd_checked_day={}\nweather_city={}\nweather_city_name={}\nweather_unit={}\nmon_x={}\nmon_y={}\n",
            self.position.as_str(),
            self.x_frac,
            self.y_frac,
            self.font,
            self.delay_ms,
            self.bg[0],
            self.bg[1],
            self.bg[2],
            self.alpha,
            self.radius,
            self.show_seconds as u8,
            self.show_date as u8,
            self.autostart as u8,
            self.theme.as_str(),
            self.lyric_shift_ms,
            self.roll_anim as u8,
            self.roll_ms,
            self.char_green as u8,
            self.marquee as u8,
            self.edge_fade as u8,
            self.side_snap as u8,
            self.lyric_font,
            self.spectrum as u8,
            self.water_spectrum as u8,
            self.collapsed_mode,
            self.idle_face,
            self.collapsed_layout,
            self.upd_seen_rail,
            self.upd_seen_row,
            self.upd_checked_day,
            self.weather_city,
            self.weather_city_name,
            self.weather_unit,
            self.mon_x,
            self.mon_y,
        );
        let _ = std::fs::write(config_path(), text);
    }
}

/// 开机自启：写入/删除 HKCU\...\Run（无需管理员）。自启时?--autostart（静默，不弹面板）?
fn set_autostart(on: bool) {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    // 清理旧品牌遗留的自启项，避免指向已不存在的旧 exe�?
    let _ = std::process::Command::new("reg")
        .args(["delete", key, "/v", "MyIsle", "/f"])
        .status();
    if on {
        let val = format!("\"{}\" --autostart", exe);
        let _ = std::process::Command::new("reg")
            .args(["add", key, "/v", "WinIsle", "/t", "REG_SZ", "/d", &val, "/f"])
            .status();
    } else {
        let _ = std::process::Command::new("reg")
            .args(["delete", key, "/v", "WinIsle", "/f"])
            .status();
    }
}

// ============================ 通用控件 ============================
fn bold_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(Arc::from("bold")))
}

/// 系统图标字体（Segoe MDL2 Assets）——用于菜单/按钮图标，呈现原生 Windows 风格。
fn icon_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(Arc::from("icons")))
}

/// HSV -> Color32（用于生成近似专辑封面的渐变色）
fn hsv(h: f32, s: f32, v: f32, a: u8) -> egui::Color32 {
    let h = ((h % 360.0) + 360.0) % 360.0;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let to = |f: f32| (((f + m) * 255.0).clamp(0.0, 255.0)) as u8;
    egui::Color32::from_rgba_unmultiplied(to(r), to(g), to(b), a)
}

fn lerp_color(a: egui::Color32, b: egui::Color32, f: f32) -> egui::Color32 {
    let f = f.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f) as u8;
    egui::Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// 把一张贴图画在 rect 内，并绕 pivot 旋转 ang 弧度（用于唱臂抬落动画）。
fn draw_tex_rotated(painter: &egui::Painter, id: egui::TextureId, rect: egui::Rect, pivot: egui::Pos2, ang: f32, a: u8) {
    let (s, c) = (ang.sin(), ang.cos());
    let corners = [
        (0.0f32, 0.0f32, 0.0f32, 0.0f32),
        (1.0, 0.0, 1.0, 0.0),
        (0.0, 1.0, 0.0, 1.0),
        (1.0, 1.0, 1.0, 1.0),
    ];
    let mut mesh = egui::Mesh::with_texture(id);
    for (fx, fy, u, v) in corners {
        let p = egui::pos2(rect.left() + fx * rect.width(), rect.top() + fy * rect.height());
        let (dx, dy) = (p.x - pivot.x, p.y - pivot.y);
        let np = egui::pos2(pivot.x + dx * c - dy * s, pivot.y + dx * s + dy * c);
        mesh.vertices.push(egui::epaint::Vertex {
            pos: np,
            uv: egui::pos2(u, v),
            color: egui::Color32::from_rgba_unmultiplied(255, 255, 255, a),
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 2, 1, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// 把一张正方形贴图的内切圆按半径 radius 画成圆盘，并绕圆心旋转 ang 弧度（用于唱片封面旋转）。
/// 与真实封面共用的同一套圆扇 mesh：贴图点 base 处会出现在屏幕角度 base+ang，从而随 ang 旋转。
fn draw_rotating_circle_tex(
    painter: &egui::Painter,
    id: egui::TextureId,
    dc: egui::Pos2,
    radius: f32,
    ang: f32,
    a: u8,
) {
    let col = egui::Color32::from_rgba_unmultiplied(255, 255, 255, a);
    let mut mesh = egui::Mesh::with_texture(id);
    mesh.vertices.push(egui::epaint::Vertex { pos: dc, uv: egui::pos2(0.5, 0.5), color: col });
    let n = 160usize;
    for i in 0..n {
        let base = (i as f32) / (n as f32) * std::f32::consts::TAU;
        let th = base + ang;
        let (s, c) = (th.sin(), th.cos());
        let (sb, cb) = (base.sin(), base.cos());
        mesh.vertices.push(egui::epaint::Vertex {
            pos: egui::pos2(dc.x + radius * c, dc.y + radius * s),
            uv: egui::pos2(0.5 + 0.5 * cb, 0.5 + 0.5 * sb),
            color: col,
        });
    }
    for i in 0..n {
        let a1 = 1 + i as u32;
        let a2 = 1 + ((i + 1) % n) as u32;
        mesh.indices.extend_from_slice(&[0, a1, a2]);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// 按音源 app_id（可能是 exe 名或 WinRT AUMID）-> 友好名称
fn src_label(app_id: &str) -> String {
    let s = app_id.to_ascii_lowercase();
    if s.contains("cloudmusic") || app_id.contains("网易") {
        "网易云音乐".to_string()
    } else if s.contains("soda") || app_id.contains("汽水") {
        "汽水音乐".to_string()
    } else if s.contains("qqmusic") {
        "QQ音乐".to_string()
    } else if s.contains("spotify") {
        "Spotify".to_string()
    } else if s.contains("kugou") {
        "酷狗音乐".to_string()
    } else if s.contains("kuwo") || s.contains("kwmusic") {
        "酷我音乐".to_string()
    } else if s.contains("applemusic") || s.contains("appleinc.applemusicwin") {
        "Apple Music".to_string()
    } else if s.contains("bodian") {
        "波点音乐".to_string()
    } else {
        app_id.to_string()
    }
}

fn card_begin(ui: &mut egui::Ui, c: &Colors, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(c.card)
        .stroke(egui::Stroke::new(1.0, c.line_soft))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, add);
}

fn section_label(ui: &mut egui::Ui, c: &Colors, text: &str) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(text).size(11.0).color(c.muted2).strong());
    ui.add_space(4.0);
}

/// 自定义开关（iOS 风格拨动）?
fn switch(ui: &mut egui::Ui, on: &mut bool, c: &Colors) -> egui::Response {
    let desired = egui::vec2(42.0, 24.0);
    let (rect, mut resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let p = ui.painter();
    let r = rect.height() * 0.5;
    let bg = if *on { c.accent_strong } else { c.card2 };
    p.rect_filled(rect, egui::CornerRadius::same(r as u8), bg);
    if !*on {
        p.rect_stroke(rect, egui::CornerRadius::same(r as u8), egui::Stroke::new(1.0, c.line), egui::StrokeKind::Inside);
    }
    let knob_x = if *on { rect.right() - r } else { rect.left() + r };
    p.circle_filled(egui::pos2(knob_x, rect.center().y), r - 3.0, egui::Color32::WHITE);
    resp
}

/// 一行：左侧标题+说明，右侧控件?
fn kv_row(ui: &mut egui::Ui, c: &Colors, title: &str, desc: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.add_space(1.0);
            ui.label(egui::RichText::new(title).size(13.5).color(c.text));
            if !desc.is_empty() {
                ui.label(egui::RichText::new(desc).size(11.0).color(c.muted2));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            control(ui);
        });
    });
}

fn hairline(ui: &mut egui::Ui, c: &Colors) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, c.line_soft);
}

/// 六宫格停靠位置选择器?
fn position_grid(ui: &mut egui::Ui, sel: &mut DockPosition, c: &Colors) {
    let gap = 8.0;
    let avail = ui.available_width();
    let tile_w = ((avail - 2.0 * gap) / 3.0).max(60.0);
    let tile_h = 54.0;
    egui::Grid::new("pos_grid")
        .num_columns(3)
        .spacing(egui::vec2(gap, gap))
        .show(ui, |ui| {
            for (i, &p) in DockPosition::ALL.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(tile_w, tile_h), egui::Sense::click());
                let is_sel = *sel == p;
                let painter = ui.painter_at(rect);
                let fill = if is_sel { c.accent_dim } else { c.card2 };
                painter.rect_filled(rect, egui::CornerRadius::same(10), fill);
                painter.rect_stroke(
                    rect,
                    egui::CornerRadius::same(10),
                    egui::Stroke::new(if is_sel { 1.6 } else { 1.0 }, if is_sel { c.accent } else { c.line }),
                    egui::StrokeKind::Inside,
                );
                // 迷你屏幕
                let screen = egui::Rect::from_min_size(
                    rect.min + egui::vec2(12.0, 9.0),
                    egui::vec2(rect.width() - 24.0, 24.0),
                );
                painter.rect_filled(screen, egui::CornerRadius::same(4), egui::Color32::from_rgb(18, 19, 23));
                // 迷你?
                let pw = 18.0;
                let ph = 3.6;
                let (px, py) = match p {
                    DockPosition::TopCenter => (screen.center().x - pw / 2.0, screen.top() + 2.0),
                    DockPosition::TopLeft => (screen.left() + 3.0, screen.top() + 2.0),
                    DockPosition::TopRight => (screen.right() - pw - 3.0, screen.top() + 2.0),
                    DockPosition::LeftCenter => (screen.left() + 2.0, screen.center().y - ph / 2.0),
                    DockPosition::RightCenter => (screen.right() - pw - 2.0, screen.center().y - ph / 2.0),
                    DockPosition::BottomCenter => (screen.center().x - pw / 2.0, screen.bottom() - ph - 2.0),
                };
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::pos2(px, py), egui::vec2(pw, ph)),
                    egui::CornerRadius::same(2),
                    c.accent,
                );
                // 文字
                painter.text(
                    egui::pos2(rect.center().x, rect.bottom() - 7.0),
                    egui::Align2::CENTER_BOTTOM,
                    p.label(),
                    egui::FontId::proportional(10.5),
                    if is_sel { c.accent } else { c.muted },
                );
                if resp.clicked() {
                    *sel = p;
                }
                if (i + 1) % 3 == 0 {
                    ui.end_row();
                }
            }
        });
}

// ============================ App ============================
/// 左侧导航对应的独立模块（互不覆盖；音乐在后台持续运行，切到别的模块不影响它）�?
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Module {
    Music,
    Schedule,
    Weather,
}

/// 仅用于调试钩�?WINISLE_PAGE：启动后直接定位到某个视图�?
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DbgView {
    Settings,
    Schedule,
    Weather,
}

struct IslandApp {
    hwnd: isize,
    initialized: bool,
    expanded: f32,
    target: f32,
    collapse_at: Option<Instant>,
    monitor: egui::Vec2,
    /// 岛体所在显示器的逻辑矩形（虚拟屏坐标，已除以 pixels_per_point）。
    monitor_rect: egui::Rect,
    last_geom: (f32, f32, f32, f32),
    last_tick: std::time::Instant,
    cfg: Config,
    settings_open: bool,
    confirm_open: bool,
    settings_hwnd: isize,
    settings_shown: bool,
    island_visible: bool,
    last_trigger: Option<std::time::SystemTime>,
    tray: Option<Tray>,
    focus_settings: bool,
    // 拖动移动
    dragging: bool,
    drag_off: egui::Vec2,
    /// 抓取点相对岛体宽高的比例（0..1）。拖动时按当前岛体尺寸重新折算抓取点，
    /// 这样岛体在拖动中即使展开度变化（如开始拖动时自动收起），被抓的那个点也会始终咬在指针处，
    /// 而不会随塌收漂移到指针下方（尺寸不变时等价于固定偏移，1:1 跟手）。
    drag_frac: egui::Vec2,
    drag_last_btn: bool,
    /// 松手后的"归位"动画：从松手位置平滑滑到顶部吸附点。存 (起点原点, 终点原点, 开始时??
    settle: Option<(egui::Pos2, egui::Pos2, Instant)>,
    /// 窗口当前是否为大窗口(展开态足?。收起静止时?false —?窗口实际缩到岛体大小?
    /// 不占用桌面。仅展开?悬停时才是大窗口?
    full: bool,
    /// 刚拖放完成后的一小段"悬停抑制"，避免松手瞬间因光标仍在岛上而立刻又展开?
    hover_hold: Option<Instant>,
    /// 当前是否已对窗口启用"鼠标穿?(WS_EX_TRANSPARENT)?
    click_through: bool,
    /// 当前已设置的窗口区域 (l,t,r,b,rad)（物理像素，窗口本地）。避免每帧重?SetWindowRgn?
    region: (i32, i32, i32, i32, i32),
    /// 上一帧 SetWindowRgn 耗时（毫秒），仅用于性能诊断日志
    perf_region_ms: f32,
    /// 上一帧网易云内存读取耗时（毫秒），仅用于诊断
    perf_netease_ms: f32,
    /// 音乐模块：最近一次 SMTC 读取到的播放信息
    music: smtc::Track,
    /// SMTC 轮询器（主线程非阻塞状态机，见 smtc.rs）
    tracker: smtc::Tracker,
    /// 歌词后台工作线程
    lyr_tx: std::sync::mpsc::Sender<(String, String, String, f64, bool, bool)>,
    lyr_rx: std::sync::mpsc::Receiver<(String, Option<Vec<lyrics::LrcLine>>, Option<Vec<u8>>)>,
    lyr_key: String,
    lyr_dur: f64, // 当初请求歌词时采用的“期望时长”，用于发现同名不同版本(时长不符)后重�?
    lyr_lines: Vec<lyrics::LrcLine>,
    /// 当前曲目的歌词请求是否已完成（用于区分“正在加载”与“确实没有歌词/纯音乐”）
    lyr_done: bool,
    /// 播放进度估算时钟（SMTC 的 position 有些播放器一直给 0，故自维护）
    clock_base: f64,
    clock_start: Instant,
    clock_playing: bool,
    prev_playing: bool,
    from_smtc: bool,
    ui_playing: bool,
    // 点击控制后的乐观状态，短时间内优先于 SMTC 快照，避免图标"闪回"或感觉迟钝
    override_play: Option<(bool, std::time::Instant)>,
    /// 用户刚点了“暂停”、但播放器（汽水）尚未确认的时刻。用于在确认前冻结进度校正，
    /// 防止暂停瞬间进度继续前冲、随后又弹回原位（表现为歌词“滚一下又回去”）。
    pause_pending_at: Option<std::time::Instant>,
    /// 以 SMTC 真实 Position 为准的时间基准（播放器不给 position 时回退到 clock）
    spos: f64,
    spos_at: Instant,
    spos_valid: bool,
    /// 上一次从 SMTC 收到的原始 position（用于判断播放器是否真的在推进位置）；
    /// 像汽水这类播放器会持续上报同一个静止 position，不能拿它反复把进度往回拉。
    smtc_rep_pos: f64,
    /// 上一次网易云内存上报的原始 position（秒）与“最后一次真正变化”的时刻。
    /// 网易云内存位置更新是量化的（约 0.5~1s 一跳），有时还会在两个近邻值间抖
    /// （如 2.30 / 2.31）。不能每 450ms 拿原始值直接覆盖自维护时钟，否则进度会来回跳，
    /// 歌词就会在两行之间闪。这里只把它当参考，主时钟保持单调。
    netease_prev_raw: f64,
    netease_raw_move_at: Instant,
    /// 上次执行「汽水 CDP 补丁自检」的时间（节流用）。
    last_soda_heal: Instant,
    /// 启动后是否已做过一次「汽水补丁」检查（未运行时直接补好）。
    soda_boot_checked: bool,
    /// 汽水补丁自检是否正在后台执行，避免重复进入（该检查含网络/进程操作，放后台防卡顿）。
    soda_tick_busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 汽水「在运行且 CDP 可用」的缓存（soda_running+cdp_up 有进程枚举/网络开销，需节流）。
    soda_live_cached: bool,
    /// 汽水当前播放态（由底部播放条按钮图标实时判定）：Some(true)=正在播放，Some(false)=已暂停，
    /// None=未知。用于纠正 SMTC 播放标志偶发失真/闪烁导致的“自己暂停”。
    soda_playing: Option<bool>,
    soda_live_at: Instant,
    /// 汽水可用性探测放后台线程，避免每秒一次 20–35ms 卡顿（丝滑度关键）。
    soda_live_rx: Option<std::sync::mpsc::Receiver<bool>>,
    /// 上次从汽水界面读取真实播放位置的时间（节流用）。
    soda_pos_at: Instant,
    /// 汽水进度查询放在后台线程，避免 CDP 网络抖动阻塞 UI 渲染（丝滑度关键）。
    soda_prog_rx: Option<
        std::sync::mpsc::Receiver<(
            Option<(f64, f64, f64)>,
            Option<bool>,
            Option<crate::cdp_lyric::SodaTrialMeta>,
        )>,
    >,
    /// 从汽水播放条读到的真实总时长（秒），用于纠正 SMTC 给错的时长。
    soda_dur: f64,
    /// 汽水当前曲目整首歌的完整时长（秒，来自 track.duration）。
    soda_full_dur: f64,
    /// 汽水 VIP 试听片段在整首歌里的起点（秒）。
    soda_clip_start: f64,
    /// 汽水 VIP 试听片段长度（秒）。
    soda_clip_dur: f64,
    /// 汽水当前是否在播放 30 秒试听片段（底部条总时长明显短于整首歌）。
    soda_trial_active: bool,
    /// 汽水播放器当前是否处于“试听片段”模式（来自 DOM 的试听区间标记）。
    soda_trial_hint: bool,
    /// 上一次从汽水播放条读到的进度值；用于识别「DOM 进度条约 1 秒才跳一次」
    /// 造成的重复旧值，避免把重复旧值当成落后而把歌词时间轴往回拉。
    soda_last_p: f64,
    /// 汽水歌词「还没拿到」时的自动重试节流与次数。
    lyr_retry_at: Instant,
    lyr_retry_n: u8,
    lyr_inflight: bool,
    disc_angle: f32,
    arm_angle: f32,
    /// 顶部条按下后“待决拖动”：移动超过阈值才真正拖动，避免单击误收起
    drag_pending: Option<egui::Pos2>,
    /// 原型烘焙的唱机贴图（播放/暂停两态），与 HTML 原型一致
    tt_play: Option<egui::TextureHandle>,
    tt_pause: Option<egui::TextureHandle>,
    tt_base: Option<egui::TextureHandle>,
    tt_arm0: Option<egui::TextureHandle>,
    cover_tex: Option<egui::TextureHandle>,
    /// 无真实封面时的兜底封面贴图（按歌名哈希缓存，逐像素抗锯齿的圆形渐变）
    fallback_cover_tex: Option<(u64, egui::TextureHandle)>,
    soda_cover_hash: u64,
    upd_badge_tex: Option<egui::TextureHandle>,
    /// 收起态待机表情动画帧贴图（1:1 复刻 Otto 原版逐帧动画，按 FACES 顺序懒加载）
    face_frames: Vec<Option<Vec<egui::TextureHandle>>>,
    /// 每个待机表情“实际不透明内容”的 UV 包围盒（用于收起条裁掉透明留白，使模块间距=实际内容间距）
    face_content_uv: Vec<Option<(f32, f32, f32, f32)>>,
    /// 兜底：SMTC 不可用时，从播放器窗口标题抓 “歌名 - 歌手”
    media_title_at: Instant,
    media_title_cache: Option<(String, String, String)>,
    /// 窗口标题兜底放到后台线程计算（EnumWindows 很慢，放主线程会造成周期性掉帧）。
    media_title_rx: Option<std::sync::mpsc::Receiver<Option<(String, String, String)>>>,
    /// 播放器进程检测结果缓存（枚举进程较慢，最多每秒查一次，避免无音乐时每帧 50ms+ 卡顿）
    player_running_cached: bool,
    player_running_at: Instant,
    /// 从“有歌”变成“没有歌”的起始时刻，用于延迟清除残留的音乐状态
    music_lost_since: Option<Instant>,
    /// 上一帧 music_present 的值：只有“此前确实有音乐”时才做 400ms 丢失去抖。
    /// 冷启动时它为 false，因此不会凭空把 music_present 置真——否则收起条会先展开、再缩回、再展开。
    music_was_present: bool,
    /// 最近一次拿到的好歌名（用于播放器短时读不到标题时保持稳定，避免闪烁/来回切换）
    last_title: Option<(String, String, String)>,
    last_title_at: Instant,
    /// 音乐模块显示程度 0..1：用于宽胶囊/内容/空态的平滑过渡动画
    music_amt: f32,
    /// 音乐「正在运动」：正在播放，或 music_amt 仍在淡入淡出（需 60fps）；
    /// 暂停且已稳定时为 false → 收起态允许降到 16.7fps，避免挂机时空烧 CPU。
    music_motion: bool,
    /// 最近一次显示过的歌词行号（滞回，避免边界处来回跳）
    lyr_idx_last: i32,
    /// 收起态歌词“上滚”动画：换行瞬间的时刻，以及上一句（滚出）的索引
    lyr_anim_at: Option<Instant>,
    lyr_anim_from: Option<usize>,
    /// 展开态歌词滚轮浏览：相对“当前句居中”的像素偏移（0=跟随当前进度）
    lyr_drag_off: f32,
    /// 滚轮目标偏移（`lyr_drag_off` 平滑趋近它，形成滚动过渡）
    lyr_scroll_target: f32,
    /// 频谱动画相位累加（支持按歌曲快慢自适应变速，切换无跳变）
    spec_phase: f64,
    /// 频谱当前平滑速度（弧度/秒）
    spec_speed: f32,
    /// 频谱上一帧时间戳（用于按帧推进相位）
    spec_last_t: f64,
    /// 水波频谱「未播放」形态的逐柱平滑高度（按归一化位置 0..127 索引）。
    /// 与 water-reflection-test.html 的 fakeBins 一样做时间缓动，避免柱子直接跳目标值造成“掉帧感”。
    wave_idle: [f32; 128],
    /// 水波频谱「播放」形态的逐柱平滑显示高度（同上索引），对真实频段再做一次柔和。
    wave_play: [f32; 128],
    wave_disp: [f32; 128],
    /// 播放/未播放两种形态的交叉淡入比例：0=未播放，1=播放，按时间平滑过渡，切换时不突然卡过去。
    wave_mix: f32,
    /// 上一帧“提交结束”的时刻，用于把渲染循环限速到 60fps（该虚拟显示器 200Hz，vsync 会顶到 200fps）
    frame_last: Instant,
    /// 上一帧向事件循环申请的休眠毫秒数；用于自适应估算“每帧实际耗时(含绘制)”，
    /// 从而把 request_repaint_after 的时长扣掉工作耗时，让帧周期精确落到目标（60fps）。
    prev_req_ms: f32,
    /// 最近一次“需要高帧率”的时刻（悬停/拖动/播放/展开动画/入场等）。用于在完全静止 600ms 后
    /// 把常驻重绘降到低帧率（含展开面板静止时），省 CPU 又能在交互后保持丝滑。
    last_busy: Instant,
    /// 启动入场动画的起始时刻（窗口首次真正显示那一刻才置为 Some；此前画 0 不透明度，避免提前走完动画）。
    intro_start: Option<Instant>,
    /// 进程内启动基准时刻（用于窗口可见性检测失败时的兜底启动淡入）。
    boot_at: Instant,
    /// 启动淡入/上浮是否已完成。
    intro_done: bool,
    /// 启动阶段用 DWM cloak 把窗口从合成器摘除，直到窗口被移到最终位置（避免先在左上角露一帧胶囊）。
    cloaked: bool,
    /// cloak 持续的帧计数（等 apply_geometry 把窗口移到最终贴边位置后再解除）。
    cloak_frames: u8,
    /// 收起态内容（频谱/歌词/时钟/表情）的可见度 0..1：由「是否已完全收起」驱动、
    /// 用时间平滑，避免收缩动画收尾时内容突然整块出现造成的末段卡顿。
    cb_anim: f32,
    /// 退出退场动画的起始时刻（托盘「退出」后先淡出+上滑再真正结束进程）。
    exit_start: Option<Instant>,
    /// 当前句索引的平滑值（换行时整列表平滑上移，而不是直接跳一行）
    lyr_ci_anim: f32,
    lyr_release_at: Option<Instant>,
    /// 自动回到当前句的动画：起始偏移与起始时刻（定时后按固定时长缓出）
    lyr_return_from: f32,
    lyr_return_start: Option<Instant>,
    /// VIP 试听片段在整首歌里的起点（秒）。0 = 非试听或从 0 开始；歌词按此偏移对齐。
    trial_start: f32,
    /// 试听起点探测：(上次尝试时刻, 已尝试次数)。切歌后若检测到片段时长 <60s 才开始探测。
    trial_probe: Option<(Instant, u32)>,
    /// 切歌时的本地时间戳，只采纳这之后网易云内存日志里的 freeTrialInfo。
    trial_since: String,
    /// 待确认的切歌标题（防抖：标题稳定 300ms 才真正切歌/请求歌词）
    pending_title: Option<(String, Instant)>,
    netease_at: Instant,
    /// 网易云内存时间轴读到的真实总时长（秒）；>=20 时优先用于进度条
    netease_dur: f64,
    /// 当前帧里可点的控制按钮矩形（拖动时避开这些区域，让点击落到按钮上）
    ctrl_hit: Vec<egui::Rect>,
    /// 左侧导航当前选中的独立模块（音乐/日程/天气）
    active: Module,
    /// 音乐模块内的设置页是否展开（仅音乐模块有效）
    show_settings: bool,
    settings_scroll: f32,
    /// 设置页滚动目标值（滚轮累加），实际 settings_scroll 每帧向它平滑逼近，实现丝滑滚动
    settings_scroll_target: f32,
    /// 设置页滚动条可见度（0..1）：滚动时淡入，停止�?1 秒后自动淡出
    settings_scroll_vis: f32,
    /// 设置页最近一次滚动活动时间（用于滚动条自动隐藏判定）
    settings_scroll_act: std::time::Instant,
    /// 调试：WINISLE_PAGE=settings/schedule/weather 时启动即定位对应视图（用于截图验证）
    dbg_page: Option<DbgView>,
    /// 左侧菜单选中项滑动动画：开始时刻 / 上一选中索引 / 上个模块
    rail_anim_at: Option<std::time::Instant>,
    rail_from: f32,
    last_active: Module,
    /// 检查到的新版本（None=已是最新/未检查）
    upd_ver: Option<update::Manifest>,
    /// 设置页「软件更新」行的状态文案
    upd_msg: String,
    upd_checking: bool,
    upd_busy: bool,
    upd_last_check: Instant,
    upd_rx: Option<std::sync::mpsc::Receiver<Result<Option<update::Manifest>, String>>>,
    upd_inst_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    /// 下载进度百分比（后台线程写、UI 只读）
    upd_dl_pct: std::sync::Arc<std::sync::atomic::AtomicU8>,
    // ===== 天气模块 =====
    /// 天气快照（线上成功或缓存兜底�?
    w_data: Option<weather::Weather>,
    /// 天气后台拉取线程的接收端
    w_rx: Option<std::sync::mpsc::Receiver<Result<weather::Weather, String>>>,
    /// 是否正在拉取（避免重复起线程�?
    w_inflight: bool,
    /// 最近一次尝试拉取的时刻（节流自动刷新）
    w_last_fetch: Instant,
    /// 最近一次拉取失败的错误文案（UI 可提示）
    w_err: Option<String>,
    /// 刷新结果提示（“已更新 / 更新失败”），连同颜色与过期时刻
    w_notice: String,
    w_notice_err: bool,
    w_notice_until: Option<Instant>,
    /// 本机最近一次成功拉取的时刻文案（HH:MM）；缓存启动时为“缓存�?
    w_fetch_label: String,
    /// 天气页滚动偏移（当前�?/ 目标值，平滑逼近�?
    w_scroll: f32,
    w_scroll_target: f32,
    /// 收起态天气温度文本的实测宽度：天气槽宽 = 图标 20 + 间距 6 + 温度文字宽度，
    /// 模块左右各 5px 由外层统一间距提供，避免槽内留白把相邻模块推远。
    w_temp_w: f32,
    /// 天气页滚动条可见度（0..1）：滚动时淡入，停止约 1.2 秒后自动淡出
    w_scroll_vis: f32,
    /// 天气页最近一次滚动活动时间（用于滚动条自动隐藏判定）
    w_scroll_act: std::time::Instant,
    /// 城市搜索页开�?
    w_city_search: bool,
    /// 搜索页是从天气页打开的（返回时回天气页，而不是设置页�?
    w_city_search_from_weather: bool,
    /// 搜索页打开后是否还需要把键盘焦点交给输入�?
    w_search_need_focus: bool,
    /// 聚焦输入框的重试次数（窗口刚允许激活时，首帧请求可能还没生效）
    w_search_focus_tries: u8,
    /// 上一帧搜索页开合状态（用于临时切换窗口可激活属性）
    w_activatable_last: bool,
    /// 城市搜索页入�?退场动画进度（0..1�?
    w_city_search_anim: f32,
    /// 城市搜索输入与结�?
    w_search_query: String,
    w_search_results: Vec<weather::CityHit>,
    /// 结果列表出现时刻（用于逐条淡入过渡�?
    w_search_results_at: Option<Instant>,
    w_search_rx: Option<std::sync::mpsc::Receiver<Vec<weather::CityHit>>>,
    w_search_inflight: bool,
    w_search_last_edit: Instant,
    w_search_last_query: String,
}

impl IslandApp {
    fn new(tray: Option<Tray>, open_panel: bool) -> Self {
        let cfg = Config::load();
        let (lyr_tx, lyr_req_rx) =
            std::sync::mpsc::channel::<(String, String, String, f64, bool, bool)>();
        let (lyr_res_tx, lyr_rx) =
            std::sync::mpsc::channel::<(String, Option<Vec<lyrics::LrcLine>>, Option<Vec<u8>>)>();
        std::thread::spawn(move || {
            while let Ok((key, title, artist, dur, soda, light)) = lyr_req_rx.recv() {
                // light=true：仅走 CDP 廉价探测（不扫内存、不碰磁盘），用于歌词页迟到时补取；
                // light=false：完整链路（CDP + 内存兜底 + 缓存 + 封面）。
                let (r, cov) = if light {
                    (lyrics::soda_probe_cdp(&title, &artist), None)
                } else {
                    lyrics::fetch_full(&title, &artist, dur, soda)
                };
                // 迷你?
                {
                    use std::io::Write;
                    let p = crate::paths::file("lyrics_debug.log");
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
                        let n = r.as_ref().map(|v| v.len()).unwrap_or(0);
                        let cb = cov.as_ref().map(|v| v.len()).unwrap_or(0);
                        let _ = writeln!(f, "{} - {} => lines={} cover={}B", title, artist, n, cb);
                    }
                }
                if lyr_res_tx.send((key, r, cov)).is_err() {
                    break;
                }
            }
        });
        // 注：此前为“移到屏幕外取歌词”遗留窗口加过的自动看门狗已移除（它会误伤正常窗口）�?
        // 仅在启动时用保守�?ensure_soda_reachable 还原 WinIsle 自己留下的精确句柄�?
        // SMTC 采用主线程非阻塞状态机（见 smtc.rs），无需后台线程�?
        let dbg_page = match std::env::var("WINISLE_PAGE").ok().as_deref() {
            Some("settings") => Some(DbgView::Settings),
            Some("schedule") => Some(DbgView::Schedule),
            Some("weather") => Some(DbgView::Weather),
            _ => None,
        };
        // 启动时先读缓存，保证断网/首次拉取前也有数据可显示
        let w_data = weather::load_cache(&cfg.weather_city);
        let w_fetch_label = if w_data.is_some() { "缓存".to_string() } else { String::new() };
        let mut app = Self {
            hwnd: 0,
            initialized: false,
            expanded: 0.0,
            target: 0.0,
            collapse_at: None,
            monitor: egui::vec2(1920.0, 1080.0),
            monitor_rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1920.0, 1080.0)),
            last_geom: (0.0, 0.0, -9999.0, -9999.0),
            last_tick: std::time::Instant::now(),
            cfg,
            settings_open: open_panel,
            confirm_open: false,
            settings_hwnd: 0,
            settings_shown: false,
            island_visible: true,
            last_trigger: trigger_mtime(),
            tray,
            focus_settings: open_panel,
            dragging: false,
            drag_off: egui::vec2(0.0, 0.0),
            drag_frac: egui::vec2(0.5, 0.5),
            drag_last_btn: false,
            settle: None,
            full: false,
            hover_hold: None,
            region: (i32::MIN, 0, 1, 1, 0),
            perf_region_ms: 0.0,
            perf_netease_ms: 0.0,
            click_through: false,
            music: smtc::Track::default(),
            tracker: smtc::Tracker::new(),
            lyr_tx,
            lyr_rx,
            lyr_key: String::new(),
            lyr_dur: 0.0,
            lyr_lines: Vec::new(),
            lyr_done: false,
            clock_base: 0.0,
            clock_start: Instant::now(),
            clock_playing: false,
            prev_playing: false,
            from_smtc: false,
            ui_playing: true,
            override_play: None,
            pause_pending_at: None,
            spos: 0.0,
            spos_at: Instant::now(),
            spos_valid: false,
            smtc_rep_pos: -1.0,
            netease_prev_raw: -1.0,
            netease_raw_move_at: Instant::now(),
            last_soda_heal: Instant::now() - Duration::from_secs(3600),
            soda_boot_checked: false,
            soda_tick_busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            soda_live_cached: false,
            soda_playing: None,
            soda_live_at: Instant::now() - Duration::from_secs(3600),
            soda_live_rx: None,
            soda_pos_at: Instant::now() - Duration::from_secs(3600),
            soda_prog_rx: None,
            soda_dur: 0.0,
            soda_full_dur: 0.0,
            soda_clip_start: 0.0,
            soda_clip_dur: 0.0,
            soda_trial_active: false,
            soda_trial_hint: false,
            soda_last_p: -1.0,
            lyr_retry_at: Instant::now() - Duration::from_secs(3600),
            lyr_retry_n: 0,
            lyr_inflight: false,
            disc_angle: 0.0,
            arm_angle: -22.0,
            drag_pending: None,
            tt_play: None,
            tt_pause: None,
            tt_base: None,
            tt_arm0: None,
            cover_tex: None,
            fallback_cover_tex: None,
            soda_cover_hash: 0,
        upd_badge_tex: None,
            face_frames: (0..FACES.len()).map(|_| None).collect(),
            face_content_uv: (0..FACES.len()).map(|_| None).collect(),
            media_title_at: Instant::now() - Duration::from_secs(10),
            media_title_cache: None,
            media_title_rx: None,
            player_running_cached: false,
            player_running_at: Instant::now() - Duration::from_secs(10),
            music_lost_since: None,
            music_was_present: false,
            last_title: None,
            last_title_at: Instant::now() - Duration::from_secs(60),
            music_amt: 0.0,
            music_motion: false,
            lyr_idx_last: -1,
            lyr_anim_at: None,
            lyr_anim_from: None,
            lyr_drag_off: 0.0,
            lyr_scroll_target: 0.0,
            spec_phase: 0.0,
            spec_speed: 5.2,
            spec_last_t: 0.0,
            wave_idle: [0.52; 128],
            wave_play: [0.0; 128],
        wave_disp: [0.52; 128],
            wave_mix: 0.0,
            frame_last: Instant::now() - Duration::from_millis(20),
            prev_req_ms: 16.0,
            last_busy: Instant::now(),
            intro_start: None,
            boot_at: Instant::now(),
            // 入场动画默认关闭：改为和以前一样「双击后直接出现在顶部」（不再有从上方落下的黑边一闪）。
            // 如需重新启用淡入/上浮入场，把这里改成 false，并把 intro_dy 的 14.0、draw 里的 0.16/0.34 调好即可。
            intro_done: true,
            cloaked: false,
            cloak_frames: 0,
            cb_anim: 0.0,
            exit_start: None,
            lyr_ci_anim: -1.0,
            lyr_release_at: None,
            lyr_return_from: 0.0,
            lyr_return_start: None,
            trial_start: 0.0,
            trial_probe: None,
            trial_since: String::new(),
            pending_title: None,
            netease_at: Instant::now(),
        netease_dur: 0.0,
            // netease reader started below
            ctrl_hit: Vec::new(),
            active: Module::Music,
            show_settings: false,
            settings_scroll: 0.0,
            settings_scroll_target: 0.0,
            settings_scroll_vis: 0.0,
            settings_scroll_act: std::time::Instant::now() - std::time::Duration::from_secs(10),
            dbg_page,
            rail_anim_at: None,
            rail_from: 0.0,
            last_active: Module::Music,
            upd_ver: None,
            upd_msg: String::new(),
            upd_checking: false,
            upd_busy: false,
            upd_last_check: Instant::now() - Duration::from_secs(5 * 3600),
            upd_rx: None,
            upd_inst_rx: None,
            upd_dl_pct: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0)),
            w_data,
            w_rx: None,
            w_inflight: false,
            w_last_fetch: Instant::now() - Duration::from_secs(3600),
            w_err: None,
            w_notice: String::new(),
            w_notice_err: false,
            w_notice_until: None,
            w_fetch_label,
            w_scroll: 0.0,
            w_scroll_target: 0.0,
            w_temp_w: 18.0,
            w_scroll_vis: 0.0,
            w_scroll_act: std::time::Instant::now() - std::time::Duration::from_secs(10),
            w_city_search: false,
            w_city_search_from_weather: false,
            w_search_need_focus: false,
            w_search_focus_tries: 0,
            w_activatable_last: false,
            w_city_search_anim: 0.0,
            w_search_query: String::new(),
            w_search_results: Vec::new(),
            w_search_results_at: None,
            w_search_rx: None,
            w_search_inflight: false,
            w_search_last_edit: Instant::now(),
            w_search_last_query: String::new(),
        };
        // 不依赖上一帧可能滞后的窗口原点，保证能精确贴到最?最右角?
        if let Ok(v) = std::env::var("WINISLE_CITY_SEARCH") {
            app.show_settings = true;
            app.w_city_search = true;
            app.w_search_need_focus = true;
            app.w_search_focus_tries = 5;
            app.w_city_search_anim = 1.0;
            if v == "weather" {
                app.active = Module::Weather;
                app.last_active = Module::Weather;
                app.rail_from = 1.0;
                app.w_city_search_from_weather = true;
            }
        }
        // 仅调试：预填搜索词（配合 WINISLE_CITY_SEARCH 使用），不产生任何真实键盘事件。
        if let Ok(q) = std::env::var("WINISLE_SEARCH_QUERY") {
            app.w_search_query = q;
            app.w_search_last_edit = Instant::now() - Duration::from_millis(500);
        }
        app
    }

    fn ensure_window(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.initialized {
            return;
        }
        if let Ok(handle) = frame.window_handle() {
            if let RawWindowHandle::Win32(w) = handle.as_raw() {
                self.hwnd = w.hwnd.get();
                platform::set_main_hwnd(self.hwnd);
                platform::make_island(self.hwnd);
                platform::install_erase_guard(self.hwnd);
                platform::start_click_through_poller(self.hwnd);
                // 关键：让窗口成为“分层窗口(WS_EX_LAYERED)”。只有分层窗口上的 WS_EX_TRANSPARENT 才能真正把鼠标/点击
                // 穿透到下层其它进程的窗口；普通窗口仅设 WS_EX_TRANSPARENT 时对本进程以外无效（这正是“点了没反应”的原因）。
                // SetLayeredWindowAttributes(alpha=255) 保持整窗不透明，逐像素透明仍由 GL/DWM 的 alpha 负责。
                if std::env::var_os("WINISLE_NO_LAYERED").is_none() {
                    platform::set_layered(self.hwnd);
                }
                // 首次创建后先把客户区预填充成深色（背景刷已置空，避免首帧任何未定义底色）
                platform::fill_dark_now(self.hwnd);
                // 先把窗口按最终贴边位置精确摆好，再交给 eframe 显示首帧。
                // 这里**不手动 ShowWindow、也不整窗 alpha=0/cloak**：eframe 自身就是「隐藏创建、等首帧画好后再
                // set_visible(true)」（eframe-0.31.1/src/native/epi_integration.rs:308-310 与 glow_integration.rs:158）。
                // 之前额外的手动 show/cloak 反而会在首帧尚未绘制时把白底原生窗口露出来→闪白（回归）。回到自带路径。
                self.refresh_monitor(ctx);
                self.apply_geometry(ctx);
                // 关键：先把窗口从合成器摘除（DWM cloak）。窗口刚创建时还在系统默认位置（左上角），
                // 若此时被 eframe 显示，就会在左上角闪一帧“胶囊”，随后才跳到顶部中间。
                // cloak 期间完全不显示，等 apply_geometry 连续几帧把窗口移到最终贴边位置后再解除。
                platform::set_cloak(self.hwnd, true);
                self.cloaked = true;
                self.cloak_frames = 0;
                self.initialized = true;
            }
        }
    }

    fn drive_animation(&mut self, ctx: &egui::Context) {
        if !self.island_visible {
            self.target = 0.0;
            self.expanded = 0.0;
            self.collapse_at = None;
            return;
        }
        let now = Instant::now();
        if std::env::var_os("WINISLE_FORCE_EXPAND").is_some() {
            // 仅调试：无鼠标环境下强制展开（用于截�?自动化）
            self.target = 1.0;
            self.collapse_at = None;
            let dt = ctx.input(|i| i.stable_dt).min(0.05);
            let k = 1.0 - (-15.0 * dt).exp();
            self.expanded += (1.0 - self.expanded) * k;
            if (1.0 - self.expanded).abs() > 0.002 {
                ctx.request_repaint();
            }
            return;
        }
        if std::env::var_os("WINISLE_CYCLE").is_some() {
            // 仅调试：按时间自动“展开→收起”循环，用于无鼠标环境下复现展开/收起后的穿透状态。
            let t = ctx.input(|i| i.time);
            self.target = if (t % 4.0) < 2.0 { 1.0 } else { 0.0 };
            let dt = ctx.input(|i| i.stable_dt).min(0.05);
            let k = 1.0 - (-15.0 * dt).exp();
            self.expanded += (self.target - self.expanded) * k;
            if (self.target - self.expanded).abs() > 0.002 {
                ctx.request_repaint();
            }
            return;
        }
        if self.dragging || self.settle.is_some() {
            self.target = 0.0;
            self.collapse_at = None;
        }
        let hover = if self.dragging || self.settle.is_some() {
            false
        } else if let Some(t) = self.hover_hold {
            if now < t {
                false
            } else {
                self.hover_hold = None;
                self.hover(ctx, self.hwnd)
            }
        } else {
            self.hover(ctx, self.hwnd)
        };
        if hover {
            self.target = 1.0;
            self.collapse_at = None;
        } else if self.target > 0.5 {
            match self.collapse_at {
                None => self.collapse_at = Some(now + Duration::from_millis(self.cfg.delay_ms as u64)),
                Some(t) if now >= t => {
                    self.target = 0.0;
                    self.collapse_at = None;
                }
                _ => {}
            }
        }
        let dt = ctx.input(|i| i.stable_dt).min(0.05);
        let k = 1.0 - (-15.0 * dt).exp();
        self.expanded += (self.target - self.expanded) * k;
        if (self.expanded - self.target).abs() < 0.002 {
            self.expanded = self.target;
        } else {
            ctx.request_repaint();
        }
    }

    /// 展开态的尺寸。以后按内容动态大小时，只需改这里（只要窗口画布 >= 该尺寸即可）?
    fn expanded_size(&self) -> egui::Vec2 {
        egui::vec2(W_EXPANDED, H_EXPANDED)
    }

    /// 启动入场动画：从最终停靠位上方约 18 逻辑像素处，ease-out 滑入到停靠位。
    /// 返回当前竖直偏移（逻辑像素，负值=在上方；0=已到位）。
    fn intro_dy(&self) -> f32 {
        if !self.island_visible {
            return 0.0;
        }
        let t0 = match self.intro_start {
            Some(t) => t,
            None => return 0.0,
        };
        let el = t0.elapsed().as_secs_f32();
        let dur = 0.34_f32;
        if el >= dur {
            return 0.0;
        }
        let p = (el / dur).clamp(0.0, 1.0);
        // ease-out-cubic
        let e = 1.0 - (1.0 - p).powi(3);
        // 负值=在上方：从屏幕上方 10px 处「向下滑入」到位（用户要求从上往下落）。
        // 配合 0.26s 淡入，首帧近乎透明，不会出现过去那种“黑东西闪一下”。
        -10.0 * (1.0 - e)
    }

    /// 退出退场动画：整体向上滑出（负=向上），配合整窗淡出，约 0.18s。
    fn exit_dy(&self) -> f32 {
        if let Some(t0) = self.exit_start {
            let p = (t0.elapsed().as_secs_f32() / 0.18).clamp(0.0, 1.0);
            // ease-in（加速上滑）
            -24.0 * (p * p)
        } else {
            0.0
        }
    }

    /// 收起态内容可见度平滑：完全收起（且不是在展开过程）时→1，否则→0。
    /// 用时间常数平滑，收尾时内容淡入而不是突然出现，消除收缩动画末段的卡顿/跳变。
    fn step_cb_anim(&mut self, ctx: &egui::Context) {
        let target = if self.island_visible && self.expanded < 0.03 && self.target < 0.5 {
            1.0
        } else {
            0.0
        };
        let dt = ctx.input(|i| i.stable_dt).min(0.05);
        let k = 1.0 - (-dt / 0.09).exp();
        self.cb_anim += (target - self.cb_anim) * k;
        if (self.cb_anim - target).abs() < 0.002 {
            self.cb_anim = target;
        } else {
            // [FIX] 同上：改用 16ms 延迟重绘，避免启动入场 / 收起内容淡入时以 ~200fps 空转。
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    /// 解析收起态组件顺序：返回 (id, enabled) 列表（包含全部已知组件，缺失的按默认顺序补齐）。
    fn comp_order(&self) -> Vec<(String, bool)> {
        const KNOWN: [&str; 5] = ["spectrum", "lyrics", "face", "clock", "weather"];
        let mut out: Vec<(String, bool)> = Vec::new();
        for tok in self.cfg.collapsed_layout.split(',') {
            let t = tok.trim();
            if t.is_empty() {
                continue;
            }
            let (en, id) = if let Some(r) = t.strip_prefix('+') {
                (true, r.trim())
            } else if let Some(r) = t.strip_prefix('-') {
                (false, r.trim())
            } else {
                (true, t)
            };
            if KNOWN.contains(&id) && !out.iter().any(|(x, _)| x == id) {
                out.push((id.to_string(), en));
            }
        }
        for k in KNOWN {
            if !out.iter().any(|(x, _)| x == k) {
                out.push((k.to_string(), false));
            }
        }
        out
    }

    fn set_comp_order(&mut self, order: &[(String, bool)]) {
        let s = order
            .iter()
            .map(|(id, on)| format!("{}{}", if *on { "+" } else { "-" }, id))
            .collect::<Vec<_>>()
            .join(",");
        self.cfg.collapsed_layout = s;
    }

    fn comp_label(id: &str) -> &'static str {
        match id {
            "spectrum" => "频谱",
            "lyrics" => "歌词",
            "face" => "表情",
            "clock" => "时间",
            "weather" => "天气",
            _ => "?",
        }
    }

    /// 组件在其启用且（音乐组件需有音乐）时的基准宽度（逻辑像素）。
    fn comp_base_w(&self, id: &str) -> f32 {
        match id {
            "spectrum" => CW_SPECTRUM,
            "lyrics" => CW_LYRICS,
            "face" => self.face_slot_w(),
            "clock" => {
                let mut w = CW_CLOCK;
                if self.cfg.show_seconds {
                    w += CW_CLOCK_SECS_EXTRA;
                }
                if self.show_time_update() {
                    w += UPD_BADGE_W + UPD_BADGE_GAP;
                }
                w
            }
            "weather" => self.weather_slot_w(),
            _ => 0.0,
        }
    }

    /// 该组件当前实际占用宽度（音乐组件按 music_amt 缩放；不需要显示的返回 0）。
    fn comp_eff_w(&self, id: &str) -> f32 {
        let base = self.comp_base_w(id);
        match id {
            "spectrum" => base * self.music_amt.clamp(0.0, 1.0),
            "lyrics" => base * self.music_amt.clamp(0.0, 1.0),
            "weather" => {
                if self.w_data.is_none() {
                    0.0
                } else {
                    base
                }
            }
            _ => base,
        }
    }

    /// 收起态相邻组件之间的间距：涉及音乐组件（频谱/歌词）的间距随 music_amt 平滑收拢，
    /// 使音乐模块淡出时胶囊宽度连续收敛到「无音乐」布局，避免最后 10px 的突跳/“卡一下”。
    fn comp_gap(&self, prev: &str, cur: &str) -> f32 {
        let amt = self.music_amt.clamp(0.0, 1.0);
        let m = |id: &str| id == "spectrum" || id == "lyrics";
        if m(prev) && m(cur) {
            CB_GAP * amt
        } else if m(prev) || m(cur) {
            CB_GAP * (1.0 + amt) * 0.5
        } else {
            CB_GAP
        }
    }

    /// 收起态当前应显示的组件（按顺序、已启用、宽度>0），返回 (id, 宽度)。
    fn active_comps(&self) -> Vec<(String, f32)> {
        let mut out = Vec::new();
        for (id, on) in self.comp_order() {
            if !on {
                continue;
            }
            let w = self.comp_eff_w(&id);
            // 音乐组件在淡出过程中即使宽度已很小也保留（宽度随 music_amt 收缩），
            // 让间距一起平滑收拢，避免“宽度先归零、间距突消失”的最后一帧台阶。
            let is_music = id == "spectrum" || id == "lyrics";
            if w > 0.5 || (is_music && self.music_amt.clamp(0.0, 1.0) > 0.002) {
                out.push((id, w));
            }
        }
        out
    }

    /// 收起态总宽度（含两端内边距与组件间距），夹到 [72, CW_MAX]。
    fn collapsed_w(&self) -> f32 {
        let comps = self.active_comps();
        if comps.is_empty() {
            return W_COLLAPSED_FACE;
        }
        // 仅「表情」一项时：同样遵循「左右各 5px」的紧凑胶囊宽度
        if comps.len() == 1 && comps[0].0 == "face" {
            return self.face_slot_w() + CB_EDGE * 2.0 + CB_OUTER_EXTRA * 2.0;
        }
        let mut w = CB_EDGE * 2.0;
        let mut prev: Option<&str> = None;
        for (id, cw) in comps.iter() {
            if let Some(p) = prev {
                w += self.comp_gap(p, id);
            }
            w += *cw;
            prev = Some(id.as_str());
        }
        // 首位左侧、末尾右侧各再加 5px，让最外缘也与模块间 10px 的间隔统一
        w += CB_OUTER_EXTRA * 2.0;
        w.clamp(W_COLLAPSED_FACE, CW_MAX)
    }

    /// 给定布局按“基准宽度”（不受音乐淡入缩放）计算的总宽，用于 520 上限判定与占宽提示。
    fn order_base_width(&self, order: &[(String, bool)]) -> f32 {
        let widths: Vec<f32> = order
            .iter()
            .filter(|(_, on)| *on)
            .map(|(id, _)| self.comp_base_w(id))
            .collect();
        if widths.is_empty() {
            return 0.0;
        }
        let mut w = CB_EDGE * 2.0;
        for (i, cw) in widths.iter().enumerate() {
            if i > 0 {
                w += CB_GAP;
            }
            w += *cw;
        }
        w += CB_OUTER_EXTRA * 2.0;
        w
    }

    /// 收起态是否启用「表情」组件（用于设置页展开表情网格、加载帧）。
    fn face_mode_active(&self) -> bool {
        self.comp_order().iter().any(|(id, on)| *on && id == "face")
    }

    /// 解析内嵌的表情动画二进制（MFAN）：返回每个表情 (w, h, 帧数, 像素起始偏移)。
    /// 结果与静态资源一一对应、永不变化 → 用 OnceLock 缓存，避免每帧重复解析+分配。
    fn face_anim_meta() -> &'static Vec<(u16, u16, u16, usize)> {
        use std::sync::OnceLock;
        static CACHE: OnceLock<Vec<(u16, u16, u16, usize)>> = OnceLock::new();
        CACHE.get_or_init(|| {
            const ANIM: &[u8] = include_bytes!("assets/faces_anim.bin");
            let mut out = Vec::new();
            if ANIM.len() < 8 || &ANIM[0..4] != b"MFAN" {
                return out;
            }
            let count = u16::from_le_bytes([ANIM[6], ANIM[7]]) as usize;
            let mut p = 8usize;
            for _ in 0..count {
                if p + 6 > ANIM.len() {
                    break;
                }
                let w = u16::from_le_bytes([ANIM[p], ANIM[p + 1]]);
                let h = u16::from_le_bytes([ANIM[p + 2], ANIM[p + 3]]);
                let fc = u16::from_le_bytes([ANIM[p + 4], ANIM[p + 5]]);
                p += 6;
                let off = p;
                p += (w as usize) * (h as usize) * (fc as usize);
                out.push((w, h, fc, off));
            }
            out
        })
    }

    /// 计算某表情「所有帧的不透明内容」并集 UV 包围盒 (u0,v0,u1,v1)，用于裁掉透明留白。
    /// 纯函数（不依赖 self），供布局与绘制共用。
    fn face_content_uv_compute(idx: usize) -> (f32, f32, f32, f32) {
        let metas = Self::face_anim_meta();
        let full = (0.0, 0.0, 1.0, 1.0);
        let (w, h, fc, off) = match metas.get(idx) {
            Some(m) => *m,
            None => return full,
        };
        const ANIM: &[u8] = include_bytes!("assets/faces_anim.bin");
        let (wu, hu, fcu) = (w as usize, h as usize, fc as usize);
        let stride = wu * hu;
        let (mut minx, mut miny, mut maxx, mut maxy) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for f in 0..fcu {
            let base = off + f * stride;
            if base + stride > ANIM.len() {
                break;
            }
            for y in 0..hu {
                let row = base + y * wu;
                for x in 0..wu {
                    if ANIM[row + x] > 16 {
                        if x < minx { minx = x; }
                        if x > maxx { maxx = x; }
                        if y < miny { miny = y; }
                        if y > maxy { maxy = y; }
                    }
                }
            }
        }
        if minx == usize::MAX {
            full
        } else {
            (
                minx as f32 / wu as f32,
                miny as f32 / hu as f32,
                (maxx + 1) as f32 / wu as f32,
                (maxy + 1) as f32 / hu as f32,
            )
        }
    }

    /// 收起态「表情」模块的紧凑槽宽（贴合不透明内容，避免居中留白把间距撑大）。
    fn face_slot_w(&self) -> f32 {
        let aspect = Self::face_content_aspect(self.cfg.idle_face as usize);
        let max_h = (H_COLLAPSED * 1.2).max(10.0);
        (max_h * aspect).min(CW_FACE).max(8.0)
    }

    /// 收起态「天气」模块的紧凑槽宽：图标 20 + 间距 6 + 温度文字实测宽度。
    /// 槽宽只包住内容，模块左右各 5px 由统一的组件间距/首尾外边距提供。
    fn weather_slot_w(&self) -> f32 {
        if self.w_data.is_none() {
            return CW_WEATHER;
        }
        20.0 + 6.0 + self.w_temp_w.max(10.0)
    }

    /// 某表情内容包围盒对应的“可视宽高比”。
    /// 纯函数（只依赖静态资源 + idx），每帧会被布局多次调用 → 用缓存避免重复的全像素包围盒扫描。
    fn face_content_aspect(idx: usize) -> f32 {
        use std::collections::HashMap;
        use std::sync::{Mutex, OnceLock};
        static CACHE: OnceLock<Mutex<HashMap<usize, f32>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(v) = cache.lock().unwrap().get(&idx) {
            return *v;
        }
        let metas = Self::face_anim_meta();
        let (w, h) = metas
            .get(idx)
            .map(|m| (m.0 as f32, m.1 as f32))
            .unwrap_or((77.0, 80.0));
        let (u0, v0, u1, v1) = Self::face_content_uv_compute(idx);
        let aspect = ((w * (u1 - u0)) / (h * (v1 - v0)).max(1.0)).max(0.2);
        cache.lock().unwrap().insert(idx, aspect);
        aspect
    }

    fn face_content_uv(&mut self, idx: usize) -> (f32, f32, f32, f32) {
        let metas = Self::face_anim_meta();
        if self.face_content_uv.len() != metas.len() {
            self.face_content_uv = (0..metas.len()).map(|_| None).collect();
        }
        if let Some(uv) = self.face_content_uv.get(idx).and_then(|o| *o) {
            return uv;
        }
        let uv = Self::face_content_uv_compute(idx);
        if let Some(slot) = self.face_content_uv.get_mut(idx) {
            *slot = Some(uv);
        }
        uv
    }

    /// 懒加载指定表情的整段动画帧贴图（Otto 原版逐帧，绿字 + 透明底）。
    fn ensure_face_frames(&mut self, ctx: &egui::Context, idx: usize) {
        const ANIM: &[u8] = include_bytes!("assets/faces_anim.bin");
        let metas = Self::face_anim_meta();
        if self.face_frames.len() != metas.len() {
            self.face_frames = (0..metas.len()).map(|_| None).collect();
        }
        if idx >= metas.len() || self.face_frames[idx].is_some() {
            return;
        }
        let (w, h, fc, off) = metas[idx];
        let (w, h, fc) = (w as usize, h as usize, fc as usize);
        let stride = w * h;
        let mut frames: Vec<egui::TextureHandle> = Vec::with_capacity(fc);
        for f in 0..fc {
            let base = off + f * stride;
            if base + stride > ANIM.len() {
                break;
            }
            let src = &ANIM[base..base + stride];
            let mut rgba = Vec::with_capacity(stride * 4);
            for &a in src {
                rgba.push(126);
                rgba.push(234);
                rgba.push(182);
                rgba.push(a);
            }
            let ci = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            let tex = ctx.load_texture(
                format!("face_fr_{}_{}", idx, f),
                ci,
                egui::TextureOptions::LINEAR,
            );
            frames.push(tex);
        }
        if !frames.is_empty() {
            self.face_frames[idx] = Some(frames);
        }
    }


    /// 有效播放状态：SMTC 可用时以真实状态为准；否则用本地乐观状态（媒体键控制无法回读）。
    /// 播放器进程是否在运行（结果缓存 1 秒；进程枚举在无音乐时很贵，不能每帧做）。
    fn player_is_running(&mut self) -> bool {
        if self.player_running_at.elapsed() >= Duration::from_secs(1) {
            self.player_running_cached = platform::player_running();
            self.player_running_at = Instant::now();
        }
        self.player_running_cached
    }

    fn is_playing(&self) -> bool {
        if let Some((v, t)) = self.override_play {
            if std::time::Instant::now() < t {
                return v;
            }
        }
        // 汽水：优先用底部播放条按钮图标判定的真实播放态，避免 SMTC 播放标志偶发失真导致“自己暂停”。
        if self.app_is_soda() {
            if let Some(p) = self.soda_playing {
                return p;
            }
        }
        if self.from_smtc { self.music.playing } else { self.ui_playing }
    }

    /// 与当前音乐客户端的「链路是否正常」：绿灯=已识别且有歌词（或仍在加载），红灯=有问题。
    /// 呼吸灯（连接状态灯）状态：
    /// 0 = 绿灯（已连上播放器，歌词正常或仍在加载）
    /// 1 = 黄灯（已连上播放器，但当前拿不到歌词，例如汽水停在歌单/场景模式）
    /// 2 = 红灯（未连上播放器：汽水 CDP 断开或无可播放信息）
    fn music_link_state(&self) -> u8 {
        if std::env::var_os("WINISLE_LINK_BAD").is_some() {
            return 2;
        }
        if self.music.title.is_empty() {
            return 2;
        }
        let a = self.music.app_id.to_ascii_lowercase();
        let is_soda = a.contains("soda") || a.contains("qishui") || self.music.app_id.contains("汽水");
        // 汽水走 CDP 读取，CDP 端口不通即视为未连接；用后台缓存的探测结果，避免每帧网络探测。
        // 但若当前标题来自 SMTC（from_smtc，例如汽水在歌单页时 CDP 读不到标题、改由 SMTC 提供），
        // 则说明信息真实可用，不应再因 CDP 未就绪而误判为红灯。
        let conn_ok = if is_soda { self.soda_live_cached || self.from_smtc } else { true };
        if !conn_ok {
            return 2;
        }
        // 歌词已拿到，或仍在加载（未判定为“无歌词”），都算正常；判定拿不到歌词降为黄灯（不是异常）。
        let lyrics_ok = !self.lyr_lines.is_empty() || !self.lyr_done;
        if lyrics_ok { 0 } else { 1 }
    }

    /// 当前播放进度（秒）：自维护时钟（SMTC 的 position 部分播放器恒为 0）。
    fn current_pos(&self) -> f64 {
        if self.spos_valid {
            if self.is_playing() {
                self.spos + self.spos_at.elapsed().as_secs_f64()
            } else {
                self.spos
            }
        } else if self.clock_playing {
            self.clock_base + self.clock_start.elapsed().as_secs_f64()
        } else {
            self.clock_base
        }
    }

    /// 歌词时间轴用的播放位置：VIP 试听片段是“片段内相对时间”，
    /// 需加上片段在整首歌中的起点，才能对上完整歌词的绝对时间。
    fn lyric_time(&self) -> f64 {
        self.current_pos() + self.trial_start as f64
    }

    /// 进度条显示用的绝对位置：汽水试听片段要把片段起点补回整首歌时间轴。
    /// （网易云沿用原来的相对位置显示，避免改变已验收的行为。）
    fn display_pos(&self) -> f64 {
        if self.app_is_soda() {
            self.lyric_time()
        } else {
            self.current_pos()
        }
    }

    /// 总时长估计：取 SMTC 时长与「歌词最后一句+6s」的较大者
    /// （汽水非会员常只给 30s 试听时长，用歌词长度兜底，进度条才不会被提前走完）。
    fn app_is_soda(&self) -> bool {
        let a = self.music.app_id.to_ascii_lowercase();
        a.contains("汽水") || a.contains("soda") || a.contains("qishui")
    }

    fn upd_version(&self) -> Option<String> {
        self.upd_ver.as_ref().map(|m| m.version.clone())
    }

    /// 收起态时间旁的升级胶囊：设置菜单点过一次后就不再出现（按版本记录）。
    fn show_time_update(&self) -> bool {
        match self.upd_version() {
            Some(v) => v != self.cfg.upd_seen_rail,
            None => false,
        }
    }

    fn show_rail_update(&self) -> bool {
        self.show_time_update()
    }

    /// 设置页“软件更新”行角标：只有用户真的点过这一行才消失（按版本记录）。
    fn show_row_update(&self) -> bool {
        match self.upd_version() {
            Some(v) => v != self.cfg.upd_seen_row,
            None => false,
        }
    }

    fn mark_upd_seen_rail(&mut self) {
        if let Some(v) = self.upd_version() {
            if self.cfg.upd_seen_rail != v {
                self.cfg.upd_seen_rail = v;
                self.cfg.save();
            }
        }
    }

    fn mark_upd_seen_row(&mut self) {
        if let Some(v) = self.upd_version() {
            if self.cfg.upd_seen_row != v {
                self.cfg.upd_seen_row = v;
                self.cfg.save();
            }
        }
    }

    fn duration_est(&self) -> f64 {
        // 汽水：只用汽水自己播放条读到的真实总时长；读不到就�?SMTC 上报值。不借用任何网易云缓存�?
        if self.app_is_soda() {
            // 试听片段：进度条要显示整首歌的总时长，而不是片段自身的 30s。
            if self.soda_trial_active && self.soda_full_dur > self.soda_dur + 20.0 {
                return self.soda_full_dur;
            }
            return if self.soda_dur >= 1.0 { self.soda_dur } else { self.music.duration };
        }
        // 优先用播放器上报的真实总时长；仅当缺失/明显异常才用歌词兜底。
        if self.music.duration >= 20.0 {
            return self.music.duration;
        }
        // 网易云内存时间轴读到的真实总时长
        if self.netease_dur >= 20.0 {
            return self.netease_dur;
        }
        // 后台读取线程缓存的网易云真实时长（主线程字段可能还没轮到刷新）
        if let Some((_, d)) = platform::netease_posdur_cached() {
            if d >= 20.0 {
                return d;
            }
        }
        // 其次用网易云接口拿到的真实时长（随歌词一起缓存）
        if !self.music.title.is_empty() {
            if let Some(d) = lyrics::known_duration(&self.music.title, &self.music.artist) {
                if d >= 20.0 {
                    return d;
                }
            }
        }
        let mut d = if self.music.duration > 1.0 { self.music.duration } else { 0.0 };
        if let Some(l) = self.lyr_lines.last() {
            // 纯音乐占位只有一�?t=0，不能拿来推算总时长（否则会误把时长算�?6s）�?
            if !lyrics::is_instrumental(&self.lyr_lines) {
                d = d.max(l.t + 6.0);
            }
        }
        d
    }

    /// 取歌词时用的“整首歌曲时长”。VIP 试听只给 30~40s 片段，绝不能拿它去缩放/选版歌词：
    /// 已知完整时长就用它；否则若时长不足 60s 视为可疑试听片段，返回 0（交给歌词接口按歌名/歌手选完整版）。
    fn lyric_duration(&self) -> f64 {
        if !self.music.title.is_empty() {
            if let Some(d) = lyrics::known_duration(&self.music.title, &self.music.artist) {
                if d >= 20.0 {
                    return d;
                }
            }
        }
        let d = self.duration_est();
        if d >= 60.0 {
            d
        } else {
            0.0
        }
    }

    /// 当前歌词行索引（最后一句 <= pos 的行）。
    fn cur_lyric_idx(&self) -> Option<usize> {
        if self.lyr_lines.is_empty() {
            return None;
        }
        let pos = self.lyric_time() + self.cfg.lyric_shift_ms as f64 / 1000.0;
        let mut idx = 0usize;
        for (i, l) in self.lyr_lines.iter().enumerate() {
            if l.t <= pos + 0.05 {
                idx = i;
            } else {
                break;
            }
        }
        Some(idx)
    }

    /// 当前行内进度 0..1（用于逐字渐绿）。
    fn line_progress(&self, idx: usize) -> f32 {
        if idx >= self.lyr_lines.len() {
            return 0.0;
        }
        let pos = self.lyric_time() + self.cfg.lyric_shift_ms as f64 / 1000.0;
        let cur = self.lyr_lines[idx].t;
        // 有逐字时间（yrc）时：按“唱到第几个字”精确渲染，间奏不参与。
        let segs = &self.lyr_lines[idx].segs;
        if !segs.is_empty() {
            let total: f32 = segs.iter().map(|s| s.n as f32).sum();
            if total <= 0.0 {
                return 0.0;
            }
            let mut sung = 0.0f32;
            for s in segs {
                let st = s.t;
                let ed = s.t + s.dur;
                if pos >= ed {
                    sung += s.n as f32;
                } else if pos > st {
                    let frac = if s.dur > 0.0 { ((pos - st) / s.dur).clamp(0.0, 1.0) } else { 0.0 };
                    sung += s.n as f32 * frac as f32;
                }
            }
            return (sung / total).clamp(0.0, 1.0);
        }
        // 无逐字时间：用“到下一行的时间”铺满本句，空格按短暂停顿估，
        // 避免只按字数估算导致整句提前变绿、和实际人声对不上。
        let mut est = 0.30_f64;
        for ch in self.lyr_lines[idx].text.chars() {
            if ch.is_whitespace() {
                est += 0.16;
            } else {
                est += if (ch as u32) < 0x2E80 { 0.12 } else { 0.28 };
            }
        }
        let next = self.lyr_lines.get(idx + 1).map(|l| l.t).unwrap_or(cur + est);
        let gap = (next - cur).max(0.0);
        // 正常句用行间隔的 94% 铺满；遇到超长间奏时用字数估算的两倍兜底，
        // 防止绿色在一个很长的空白段里慢慢爬。
        let dur = if gap > 0.55 {
            (gap * 0.94).min(est * 2.2).max(0.55)
        } else {
            est.max(0.55)
        };
        ((pos - cur) / dur).clamp(0.0, 1.0) as f32
    }

    /// 按给定的展开?e 计算岛体矩形（相对固定窗口左上角）。e=1 时尺寸为 expanded_size()?
    /// 侧边吸附时展开面板的竖向生长方向：朝“竖向空间更大的一侧”生长。
    /// true=向上生长（岛体锚在底部，面板往上撑满）；false=向下生长（锚在顶部，往下撑满）。
    fn side_grow_up(&self) -> bool {
        let mt = self.monitor_rect.min.y;
        let mh = self.monitor.y.max(1.0);
        let bh2 = (H_COLLAPSED + TOP_OVERLAP) * 0.5;
        let cy = (mt + self.cfg.y_frac * mh).clamp(mt + bh2, (mt + mh - bh2).max(mt + bh2));
        let room_up = (cy - bh2) - mt;
        let room_down = (mt + mh) - (cy + bh2);
        // 展开时岛体需要额外向下/向上再长出的高度（= 展开面板高度 - 收起条高）。
        let need = H_EXPANDED - H_COLLAPSED;
        // 优先“向下展开”：只要下方空间够展开，就向下撑；不够再向上；两边都不够则朝空间大的一侧。
        if room_down >= need {
            false
        } else if room_up >= need {
            true
        } else {
            room_up > room_down
        }
    }

    /// 顶部吸附时展开面板的水平生长方向：中间够宽就左右对称撑；靠边则锚住该侧朝内撑。
    /// -1=锚左(向右撑)，0=居中对称，1=锚右(向左撑)。
    fn top_h_anchor(&self) -> i32 {
        let mx = self.monitor_rect.min.x;
        let mw = self.monitor.x.max(1.0);
        let cw = self.collapsed_w();
        let bh = cw * 0.5;
        let bx = (mx + self.cfg.x_frac * mw).clamp(mx + bh, (mx + mw - bh).max(mx + bh));
        let half_extra = (W_EXPANDED - cw) * 0.5;
        let room_l = (bx - bh) - mx;
        let room_r = (mx + mw) - (bx + bh);
        if room_l >= half_extra && room_r >= half_extra {
            0
        } else if room_r >= room_l {
            -1
        } else {
            1
        }
    }

    fn pill_rect_e(&self, full: egui::Rect, e: f32) -> egui::Rect {
        let target = self.expanded_size();
        let cw = self.collapsed_w();
        let w = cw + (target.x - cw) * e;
        let h = H_COLLAPSED + (target.y - H_COLLAPSED) * e;
        let fh = h + TOP_OVERLAP;
        // 岛体在“固定窗口”内的本地位置，按停靠边锚定；窗口 origin 与展开度 e 无关（恒定），
        // 展开/收缩时窗口不移动，只有岛体在窗口内朝“空的一侧”生长，收起后回到原位置。
        //  顶部吸附——水平居中、贴顶（向下生长，水平对称）。
        //  左右吸附——水平锚到贴边侧；竖向朝空间更大的一侧生长（靠上则往下撑，靠下则往上撑）。
        let (x, y) = match self.cfg.position {
            DockPosition::LeftCenter => {
                let y = if self.side_grow_up() {
                    full.top() + (full.height() - (MARGIN - TOP_OVERLAP)) - fh
                } else {
                    full.top() + MARGIN - TOP_OVERLAP
                };
                (full.left(), y)
            }
            DockPosition::RightCenter => {
                let y = if self.side_grow_up() {
                    full.top() + (full.height() - (MARGIN - TOP_OVERLAP)) - fh
                } else {
                    full.top() + MARGIN - TOP_OVERLAP
                };
                (full.right() - w, y)
            }
            _ => {
                let x = match self.top_h_anchor() {
                    -1 => full.left() + MARGIN,
                    1 => full.right() - MARGIN - w,
                    _ => full.left() + (full.width() - w) * 0.5,
                };
                (x, full.top() + MARGIN - TOP_OVERLAP)
            }
        };
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, fh))
    }

    fn pill_rect(&self, full: egui::Rect) -> egui::Rect {
        self.pill_rect_e(full, self.expanded)
    }

    /// 更新命中测试矩形：只有岛体区域接收鼠标（可抓?拖动），其余透明区域穿透?
    fn set_hit_region(&self, ctx: &egui::Context) {
        if self.hwnd == 0 {
            return;
        }
        if !self.island_visible {
            platform::set_hit_pill(0, 0, 0, 0);
            return;
        }
        let ppp = ctx.pixels_per_point();
        let (win, pill) = self.layout_screen();
        let local = egui::Rect::from_min_max(
            egui::pos2(pill.left() - win.left(), pill.top() - win.top()),
            egui::pos2(pill.right() - win.left(), pill.bottom() - win.top()),
        );
        platform::set_hit_pill(
            (local.left() * ppp) as i32,
            (local.top() * ppp) as i32,
            (local.right() * ppp) as i32,
            (local.bottom() * ppp) as i32,
        );
    }

    /// 刷新“当前显示器”：以配置里记录的物理左上角为准，找到该显示器当前的物理矩形，
    /// 再换算成逻辑矩形（除以 pixels_per_point）。显示器被拔掉/改分辨率时会自动退化。
    /// 拖动过程中不会被调用改变目标（cfg.mon_* 只在松手落定时更新），因此不会干扰拖拽。
    fn refresh_monitor(&mut self, ctx: &egui::Context) {
        let ppp = ctx.pixels_per_point().max(0.5);
        let (mx, my) = (self.cfg.mon_x, self.cfg.mon_y);
        // 用记录点 (+2,+2) 探测该显示器（避免正好落在多屏接缝上）。
        if let Some((l, t, r, b)) = platform::monitor_rect_at(mx as i32 + 2, my as i32 + 2) {
            let mw = (r - l) as f32 / ppp;
            let mh = (b - t) as f32 / ppp;
            if mw > 1.0 && mh > 1.0 {
                self.monitor_rect =
                    egui::Rect::from_min_size(egui::pos2(l as f32 / ppp, t as f32 / ppp), egui::vec2(mw, mh));
                self.monitor = egui::vec2(mw, mh);
                return;
            }
        }
        // 兜底：egui 上报的当前显示器尺寸（原点按 0,0）。
        if let Some(ms) = ctx.input(|i| i.viewport().monitor_size) {
            if ms.x > 1.0 && ms.y > 1.0 {
                self.monitor = ms;
                self.monitor_rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), ms);
            }
        }
    }

    /// 窗口固定为大窗口(W_EXPANDED×H_EXPANDED)；收起时?SetWindowRgn 把窗口裁剪到岛体?
    /// 因此这里始终按大窗口计算 origin 与岛体屏幕矩形（岛体相对窗口的本地位置由停靠位决定）?
    fn layout_screen(&self) -> (egui::Rect, egui::Rect) {
        let w = W_EXPANDED + 2.0 * MARGIN;
        let h = H_EXPANDED + 2.0 * MARGIN;
        let cw = self.collapsed_w();
        // 当前显示器在虚拟屏中的逻辑左上角（多显示器时不是 0,0）。
        let mx = self.monitor_rect.min.x;
        let my = self.monitor_rect.min.y;
        let mw = self.monitor.x.max(1.0);
        let mh = self.monitor.y.max(1.0);
        let full_size = egui::vec2(w, h);
        // 收起态岛体（条）高度、及其一半，用于确定沿边中心/锚点。
        let bar_h = H_COLLAPSED + TOP_OVERLAP;
        let bh2 = bar_h * 0.5;
        let origin = match self.cfg.position {
            DockPosition::LeftCenter | DockPosition::RightCenter => {
                // 沿边中心（用收起的条半高夹紧，保证条本身完整在屏内）。
                let cy = (my + self.cfg.y_frac * mh).clamp(my + bh2, (my + mh - bh2).max(my + bh2));
                // 让“收起态的条”竖向上正好落在 cy 处；生长方向决定条在窗口的顶部还是底部。
                // local0 与展开度 e 无关 → 窗口 origin 恒定、静止不移动（避免展开/收缩时窗口移动导致抖动/缝隙）。
                let local0 = if self.side_grow_up() {
                    h - (MARGIN - TOP_OVERLAP) - bar_h
                } else {
                    MARGIN - TOP_OVERLAP
                };
                let oy = (cy - bh2) - local0;
                let ox = if matches!(self.cfg.position, DockPosition::LeftCenter) {
                    mx
                } else {
                    mx + mw - w
                };
                egui::pos2(ox, oy)
            }
            _ => {
                // 顶部吸附：水平按锚点生长（中间对称、靠边锚住朝内撑），窗口 origin 与 e 无关。
                let bh = cw * 0.5;
                let bx = (mx + self.cfg.x_frac * mw).clamp(mx + bh, (mx + mw - bh).max(mx + bh));
                let ox = match self.top_h_anchor() {
                    -1 => (bx - bh) - MARGIN,
                    1 => (bx + bh) - (w - MARGIN),
                    _ => bx - w * 0.5,
                };
                egui::pos2(ox, my - MARGIN)
            }
        };
        // 入场动画：把窗口整体从略上方滑入（只改窗口挂载位置，岛体本地绘制不变）。
        // 退场动画：整体向上滑出，配合整窗淡出。
        let origin = egui::pos2(origin.x, origin.y + self.intro_dy() + self.exit_dy());
        let full = egui::Rect::from_min_size(origin, full_size);
        let pill = self.pill_rect_e(full, self.expanded);
        (full, pill)
    }

    /// 轮询全局光标判断是否悬停在岛体上（用“实际窗口矩形 + 本地岛体位置”换算到屏幕物理坐标，
    /// 不依赖会漂移的假设原点，保证任何时候都准确）。
    fn hover(&self, ctx: &egui::Context, hwnd: platform::HWND) -> bool {
        let ppp = ctx.pixels_per_point();
        let (cx, cy) = platform::cursor_pos();
        let (wl, wt, _wr, _wb) = platform::window_rect(hwnd);
        let local = egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(W_EXPANDED + 2.0 * MARGIN, H_EXPANDED + 2.0 * MARGIN),
        );
        let pill = self.pill_rect(local);
        let scr = egui::Rect::from_min_max(
            egui::pos2(wl as f32 + pill.left() * ppp, wt as f32 + pill.top() * ppp),
            egui::pos2(wl as f32 + pill.right() * ppp, wt as f32 + pill.bottom() * ppp),
        );
        // 悬停判定贴近岛体本身：只留极小容差（物理像素），避免鼠标还没真正碰到岛就提前展开。
        scr.expand(2.0).contains(egui::pos2(cx as f32, cy as f32))
    }

    /// 轮询全局左键+光标：在岛体上按下即可拖动窗口，松开时吸附到最近停靠位?
    fn handle_drag(&mut self, ctx: &egui::Context) {
        if self.hwnd == 0 {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let down = platform::lbutton_down();
        let (cx, cy) = platform::cursor_pos();
        let cur = egui::pos2(cx as f32 / ppp, cy as f32 / ppp);
        let (_win, pill) = self.layout_screen();
        let inside = pill.expand(3.0).contains(cur);
        // 只有“岛顶部条”可以发起拖动；内容区点击不劫持（留给模块内容点击）
        // 拖动热区（屏幕逻辑坐标）：水平＝整条宽度；竖直＝收起时整条（更好点中），展开时只顶部 16px 作拖动条。
        let handle_h = if self.expanded < 0.05 {
            pill.height()
        } else {
            16.0_f32.min(pill.height())
        };
        let handle = egui::Rect::from_min_max(
            egui::pos2(pill.left(), pill.top()),
            egui::pos2(pill.right(), pill.top() + handle_h),
        )
        .expand(2.0);
        let on_ctrl = self.ctrl_hit.iter().any(|r| r.contains(cur));

        // 按下：落在顶部条且不在控制按钮上 → 记为“待决拖动”（不立刻拖）
        if down && !self.drag_last_btn {
            if inside && handle.contains(cur) && !on_ctrl && !self.dragging {
                self.drag_pending = Some(cur);
            }
        }
        // 只有移动超过阈值才真正进入拖动；否则松手=普通单击，不收起、不吸附
        if let Some(start) = self.drag_pending {
            if down {
                if !self.dragging && (cur - start).length() > 4.0 {
                    // 用“窗口真实矩形”来算抓手偏移，而不是用 layout 目标：若此刻窗口正处在
                    // 上一次归位动画中途或与目标有偏差，用目标算会让岛体瞬间跳到指针下方。
                    // 同时清掉未完成的归位动画，避免它每帧继续改窗口位置、和拖动打架。
                    let (wl_px, wt_px, _wr_px, _wb_px) = platform::window_rect(self.hwnd);
                    let win0 = egui::pos2(wl_px as f32 / ppp, wt_px as f32 / ppp);
                    let local_full = egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(W_EXPANDED + 2.0 * MARGIN, H_EXPANDED + 2.0 * MARGIN),
                    );
                    let lp0 = self.pill_rect_e(local_full, self.expanded);
                    // 记录“指针相对岛体左上角”的偏移（逻辑坐标），并折算成相对岛体宽高的比例 drag_frac。
                    // 拖动定位时用 drag_frac 按“当前岛体尺寸”重新折算抓取点——这样即使拖动开始后
                    // 岛体自动塌收、或宽度随音乐变化，被抓的那个点也始终咬在指针处，不会漂到指针下方。
                    self.dragging = true;
                    self.drag_off = cur - (win0 + lp0.min.to_vec2());
                    let pw = lp0.width().max(1.0);
                    let ph = lp0.height().max(1.0);
                    self.drag_frac = egui::vec2(
                        (self.drag_off.x / pw).clamp(0.0, 1.0),
                        (self.drag_off.y / ph).clamp(0.0, 1.0),
                    );
                    self.settle = None;
                    self.hover_hold = None;
                    platform::capture_mouse(self.hwnd);
                }
            } else {
                self.drag_pending = None;
            }
        }
        if !down && self.dragging {
            self.dragging = false;
            self.drag_pending = None;
            platform::release_mouse();
            // 松手归位：先记录“岛体（胶囊）此刻的真实屏幕位置”，再让 snap 改写停靠边/位置，
            // 随后把窗口原点换算成“让岛体仍从原屏幕位置出发”的值。这样即使这次吸附把
            // 停靠边从 上 切成 左/右（岛体在窗口内的本地偏移从顶部变到底部），归位动画也
            // 只平滑移动、不会先“跳到角落”再弹回来。
            let local_full = egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(W_EXPANDED + 2.0 * MARGIN, H_EXPANDED + 2.0 * MARGIN),
            );
            let lp_old = self.pill_rect_e(local_full, self.expanded);
            let (wl0, wt0, _wr0, _wb0) = platform::window_rect(self.hwnd);
            let bar_before = egui::pos2(wl0 as f32 / ppp, wt0 as f32 / ppp) + lp_old.min.to_vec2();
            self.snap_to_nearest(cur, ppp);
            // 松手后 300ms 内不响应悬停，避免光标仍停在岛上立刻又展开
            self.hover_hold = Some(Instant::now() + Duration::from_millis(300));
            // 用新的本地偏移，反推窗口原点，使岛体在 from 帧恰好停在 bar_before（视觉连续）。
            let lp_new = self.pill_rect_e(local_full, self.expanded);
            let (full, _pill) = self.layout_screen();
            let to = full.min;
            let from = bar_before - lp_new.min.to_vec2();
            self.settle = Some((from, to, Instant::now()));
        }
        self.drag_last_btn = down;
        platform::set_dragging(self.dragging);
        if self.dragging {
            // 拖动期间保持置顶，避免落到别的窗口后面
            platform::keep_on_top(self.hwnd);
            // 跨显示器（不同 DPI 缩放）时，egui 的逻辑坐标 × ppp 会因 ppp 切换而抖动，
            // 表现为窗口在屏幕间“卡住/闪边框”。这里改用**物理像素**直接定位：
            // 记录按下时指针相对岛体左上角的物理偏移，之后窗口原点 = 指针物理位置 − 该偏移。
            platform::ensure_island_style(self.hwnd);
            let win_w = W_EXPANDED + 2.0 * MARGIN;
            let win_h = H_EXPANDED + 2.0 * MARGIN;
            let placeholder = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(win_w, win_h));
            // 用「当前展开度」下的岛体本地矩形计算抓取点位置，并把按下时抓到的那个相对点
            //（drag_frac，按岛体宽高比例记录）始终咬在指针下。
            // 关键：拖动一开始岛体会自动塌收回收起条（drive_animation 里 target=0），岛体在窗口内的
            // 本地位置随之变化；若仍用按下瞬间的固定像素偏移，岛体会随塌收一路漂到指针下方（用户反馈）。
            // 改用 drag_frac 每帧按当前岛体尺寸折算，抓取点在任何展开度/宽度变化下都精确贴在指针处。
            let lp = self.pill_rect_e(placeholder, self.expanded);
            // 岛体在窗口内的本地偏移/尺寸（物理像素），用于边界夹紧
            let pill_l = (lp.min.x * ppp).round() as i32;
            let pill_t = (lp.min.y * ppp).round() as i32;
            let pill_w = (lp.width() * ppp).round() as i32;
            let pill_h = (lp.height() * ppp).round() as i32;
            let (cxp, cyp) = platform::cursor_pos();
            // 抓取点（岛体本地逻辑坐标 → 物理像素）：左上角 + 比例 × 当前尺寸。
            let grab_x = (lp.min.x + self.drag_frac.x * lp.width()) * ppp;
            let grab_y = (lp.min.y + self.drag_frac.y * lp.height()) * ppp;
            let mut nx = cxp - grab_x.round() as i32;
            let mut ny = cyp - grab_y.round() as i32;
            // 边界按“整个虚拟桌面”物理像素计算：岛体整块留在桌面内即可拖到其它显示器。
            let (vl, vt, vr, vb) = platform::virtual_screen_rect();
            let min_x = vl - pill_l;
            let max_x = (vr - pill_l - pill_w).max(min_x);
            let min_y = vt - pill_t;
            let max_y = (vb - pill_t - pill_h).max(min_y);
            nx = nx.clamp(min_x, max_x);
            ny = ny.clamp(min_y, max_y);
            platform::set_outer_position_px(self.hwnd, nx, ny);
            self.last_geom.2 = nx as f32 / ppp;
            self.last_geom.3 = ny as f32 / ppp;
        }
    }

    /// 松手吸附：先判定岛体中心落在哪块显示器上，把停靠目标切换到该显示器，
    /// 再判断岛体中心离“上 / 左 / 右”哪条边最近，贴到该边；沿边位置按拖放点换算并持久化。
    fn snap_to_nearest(&mut self, _cur: egui::Pos2, ppp: f32) {
        let win_w = W_EXPANDED + 2.0 * MARGIN;
        let win_h = H_EXPANDED + 2.0 * MARGIN;
        // 岛体（胶囊）在窗口内的本地矩形 → 换算成屏幕（逻辑）矩形。
        // 用窗口“真实”矩形（GetWindowRect）来换算岛体当前实际屏幕位置：拖动时窗口会被边界夹紧，
        // 此时 cur-drag_off 与真实窗口位置不一致，若用它折算落位会“跳到别处”。用真实矩形才等于“当前位置”。
        let local = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(win_w, win_h));
        let lp = self.pill_rect_e(local, self.expanded);
        let (wl_px, wt_px, _wr_px, _wb_px) = platform::window_rect(self.hwnd);
        let wl = wl_px as f32 / ppp;
        let wt = wt_px as f32 / ppp;
        let pill_min = egui::pos2(wl + lp.min.x, wt + lp.min.y);
        let pill_max = egui::pos2(wl + lp.max.x, wt + lp.max.y);
        let cx_log = (pill_min.x + pill_max.x) * 0.5;
        let cy_log = (pill_min.y + pill_max.y) * 0.5;
        // 换算成物理点，找到它所在的显示器（不在任何屏上也返回最近的一块）。
        let px = (cx_log * ppp) as i32;
        let py = (cy_log * ppp) as i32;
        let mon = platform::monitor_rect_at(px, py).or_else(|| platform::monitor_rect_of_window(self.hwnd));
        let (l, t, r, b) = match mon {
            Some(v) => v,
            None => (0, 0, self.monitor.x as i32, self.monitor.y as i32),
        };
        let ml = l as f32 / ppp;
        let mt = t as f32 / ppp;
        let mw = ((r - l) as f32 / ppp).max(1.0);
        let mh = ((b - t) as f32 / ppp).max(1.0);
        // 更新“当前显示器”的逻辑矩形与尺寸，后续 layout_screen/apply_geometry 以它为准。
        self.monitor_rect = egui::Rect::from_min_size(egui::pos2(ml, mt), egui::vec2(mw, mh));
        self.monitor = egui::vec2(mw, mh);
        // 持久化：记录该显示器的物理左上角。
        self.cfg.mon_x = l as f32;
        self.cfg.mon_y = t as f32;
        // 判断岛体“离上 / 左 / 右 哪条边的空隙最小”——用岛体真实边缘算空隙，而不是用中心距离。
        // 胶囊是很宽的长条：当它靠向右上角时中心离右边还很远，用中心会误判为上吸附。
        let gap_top = (pill_min.y - mt).max(0.0);
        let gap_left = (pill_min.x - ml).max(0.0);
        let gap_right = ((ml + mw) - pill_max.x).max(0.0);
        // 左右吸附：靠近屏幕左右边缘（阈值内）或侧边空隙比顶部更小时，吸附到该侧边；
        // 开关关闭时只吸附顶部（不会贴到左右两边）。
        let side_magnet = 160.0_f32; // 逻辑像素：距左右边缘该范围内即触发侧边吸附
        let gap_side = gap_left.min(gap_right);
        let pick_side = self.cfg.side_snap && (gap_side <= side_magnet || gap_side < gap_top);
        let cw = self.collapsed_w();
        if !pick_side {
            // 顶部吸附：水平位置由拖放点换算 x_frac。
            self.cfg.position = DockPosition::TopCenter;
            let margin = (cw * 0.5) / mw;
            self.cfg.x_frac = ((cx_log - ml) / mw).clamp(margin, 1.0 - margin);
        } else {
            // 侧边吸附：左右取空隙更小的一侧，纵向位置由拖放点换算 y_frac。
            self.cfg.position = if gap_left <= gap_right {
                DockPosition::LeftCenter
            } else {
                DockPosition::RightCenter
            };
            let margin = ((H_COLLAPSED + TOP_OVERLAP) * 0.5) / mh;
            self.cfg.y_frac = ((cy_log - mt) / mh).clamp(margin, 1.0 - margin);
        }
        self.cfg.save();
    }

    /// 松手归位动画：从松手时的窗口原点平滑滑到吸附点，ease-out 缓动（轻微回弹手感）。
    fn step_settle(&mut self, ctx: &egui::Context) {
        let dur = Duration::from_millis(260);
        if let Some((from, to, start)) = self.settle {
            let t = (start.elapsed().as_secs_f32() / dur.as_secs_f32()).clamp(0.0, 1.0);
            // ease-out-back：末端轻微超过目标再回落，形成“贴上去再弹一下”的手感。
            let c1 = 0.55_f32;
            let c3 = c1 + 1.0;
            let u = t - 1.0;
            let mut e = 1.0 + c3 * u * u * u + c1 * u * u;
            // 限制“越过目标”的量（沿 from→to 直线方向，最多约 8 逻辑像素），避免超调飞出屏幕。
            let dist = (to - from).length();
            if e > 1.0 && dist > 0.5 {
                let tol = 8.0_f32;
                e = e.min(1.0 + tol / dist);
            }
            let p = from + (to - from) * e;
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(p));
            self.last_geom.2 = p.x;
            self.last_geom.3 = p.y;
            if t >= 1.0 {
                self.settle = None;
                // 归位结束后再请求一帧：让 apply_geometry 用 SetWindowPos 精确落到贴边位置。
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(Duration::from_millis(8));
            }
        }
    }

    fn apply_geometry(&mut self, ctx: &egui::Context) {
        let (full, pill) = self.layout_screen();
        let (w, h) = (full.width(), full.height());
        let ppp = ctx.pixels_per_point();
        if (w - self.last_geom.0).abs() > 0.5 || (h - self.last_geom.1).abs() > 0.5 {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
        }
        // 直接用 Win32 SetWindowPos 以“物理像素整数”精确定位窗口外框，绕开 winit
        // OuterPosition 的取整/锚点偏差——那正是展开↔塌收大幅移动后窗口少走约 4px、
        // 导致贴边留缝的根因。每帧按当前目标精确落位，位移连续，不使用反馈，绝不抖振。
        let target_x = (full.min.x * ppp).round() as i32;
        let target_y = (full.min.y * ppp).round() as i32;
        if self.hwnd != 0 {
            let (wl, wt, _wr, _wb) = platform::window_rect(self.hwnd);
            if wl != target_x || wt != target_y {
                platform::set_outer_position_px(self.hwnd, target_x, target_y);
            }
        }
        self.last_geom = (w, h, full.min.x, full.min.y);
        if std::env::var_os("WINISLE_GEOM_DEBUG").is_some() && self.hwnd != 0 {
            let local_left = pill.left() - full.left();
            let local_top = pill.top() - full.top();
            let local_bot = pill.bottom() - full.top();
            let (wl, wt, _wr, _wb) = platform::window_rect(self.hwnd);
            let pill_left_phys = wl as f32 + local_left * ppp;
            let pill_top_phys = wt as f32 + local_top * ppp;
            let pill_bot_phys = wt as f32 + local_bot * ppp;
            let target_left_phys = self.monitor_rect.min.x * ppp;
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("D:\\LingDao\\_geom_debug.txt")
            {
                let _ = writeln!(
                    f,
                    "e={:.3} pos={:?} ppp={:.2} win=(L{} T{}) target=({},{}) pillL_phys={:.1} pillT_phys={:.1} pillB_phys={:.1} gapLeft={:.1}",
                    self.expanded,
                    self.cfg.position.as_str(),
                    ppp,
                    wl,
                    wt,
                    target_x,
                    target_y,
                    pill_left_phys,
                    pill_top_phys,
                    pill_bot_phys,
                    pill_left_phys - target_left_phys
                );
            }
        }
    }

    /// ?圆角区域"把窗口裁剪成当前岛体形状（随动画逐帧跟随，圆角半径与绘制一致）?
    /// 关键的圆角：区域必须是圆角矩形，不能是外接矩形——否则圆角外侧那一小块会留在窗口里
    /// 却是透明的，展开时沿两个圆角闪出白色缺口。这里用圆角区域把圆角也裁掉，形?绘制形状?
    /// 仅当区域(l,t,r,b,rad)变化时才调用 SetWindowRgn?
    fn apply_region(&mut self, _ctx: &egui::Context) {
        // 不再用 1-bit 的窗口区域（HRGN）裁剪岛体形状。
        // 原因：SetWindowRgn 的圆角边界是像素级阶梯，且会切断岛体圆角边缘约 0.75px 的
        // 亚像素抗锯齿羽化带，使圆角/贴边处出现明显“锯齿”（A/B 实测：区域开=硬边锯齿，
        // 区域关=平滑抗锯齿）。因此改用 DWM 的逐像素透明（with_transparent + blur-behind）
        // 来呈现岛体，圆角交给 GPU 抗锯齿，边缘平滑。
        // 鼠标穿透：由 WM_NCHITTEST 子类负责（set_hit_region 每帧更新命中矩形，透明边距
        // 返回 HTTRANSPARENT），不再依赖窗口区域。
        if self.hwnd == 0 {
            return;
        }
        if self.region != (0, 0, 0, 0, 0) {
            platform::clear_window_region(self.hwnd);
            self.region = (0, 0, 0, 0, 0);
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        if !self.island_visible {
            return;
        }
        // 启动淡入：岛体整块内容以 0→1 不透明度渐显（配合 intro_dy 上浮），
        // 避免第一帧岛体“突然蹦出/黑块一闪”。窗口首次真正显示前（intro_start=None）画 0（完全透明）。
        let mut op = if let Some(v) = std::env::var_os("WINISLE_OP") {
            v.to_string_lossy().parse::<f32>().map(|f| f.clamp(0.0, 1.0)).unwrap_or(1.0)
        } else if self.intro_done {
            1.0
        } else if let Some(t0) = self.intro_start {
            let ip = (t0.elapsed().as_secs_f32() / 0.26).clamp(0.0, 1.0);
            1.0 - (1.0 - ip).powi(2)
        } else {
            0.0
        };
        // 退场：整体不透明度 1→0 淡出（ui.set_opacity 对 GL 逐像素窗口有效，配合 exit_dy 上滑）。
        if let Some(t0) = self.exit_start {
            let pp = (t0.elapsed().as_secs_f32() / 0.18).clamp(0.0, 1.0);
            op *= (1.0 - pp) * (1.0 - pp);
        }
        ui.set_opacity(op);
        self.ctrl_hit.clear();
        let pill = self.pill_rect(ui.max_rect());
        let e = self.expanded;
        let r = self.cfg.radius.min(pill.height() * 0.5).min(pill.width() * 0.5);
        let cr = self.cfg.position.corner_radius(r);
        let bg = egui::Color32::from_rgba_unmultiplied(
            self.cfg.bg[0], self.cfg.bg[1], self.cfg.bg[2], self.cfg.alpha,
        );
        let t = ui.input(|i| i.time);
        let st = platform::local_time();
        let painter = ui.painter().clone();
        painter.rect_filled(pill, cr, bg);
        // 展开/收起动画进行中（岛体尺寸在变化）：只画背景，不画文字/歌词/控件，
        // 避免逐帧重排+裁剪造成的“一闪一闪”，动画结束（完全收起或完全展开）再画内容。
        if self.expanded > 0.02 && self.expanded < 0.98 {
            return;
        }
        if e < 0.5 {
            self.draw_collapsed_bar(ui, &painter, pill, e, t);
        } else if false {
            let has = self.music_amt > 0.01;
            let a = ((0.5 - e) * 2.0 * 255.0) as u8;
            let amt = self.music_amt.clamp(0.0, 1.0);
            let am = (a as f32 * amt) as u8; // 音乐内容的淡入淡�?
            let mut time_rect: Option<egui::Rect> = None;
            if has {
                let fg = egui::Color32::from_rgba_unmultiplied(238, 240, 246, am);
                let green = egui::Color32::from_rgba_unmultiplied(64, 214, 160, am);
                let cy = pill.center().y;
                let sx = pill.left() + 15.0;
                // 右侧时间
                let mut time_s = format!("{:02}:{:02}", st.wHour, st.wMinute);
                if self.cfg.show_seconds {
                    time_s = format!("{}:{:02}", time_s, st.wSecond);
                }
                let time_font = bold_font(14.0);
                // 右侧内容宽度按「升级胶�?+ 时间」动态计算，岛体宽度也同步变�?
                let time_w = painter
                    .layout_no_wrap(time_s.clone(), time_font.clone(), fg)
                    .size()
                    .x;
                let time_left = pill.right() - 13.0 - time_w;
                let badge_left = if self.show_time_update() {
                    time_left - UPD_BADGE_GAP - UPD_BADGE_W
                } else {
                    time_left
                };
                // 左侧跳动频谱：底部与歌词文字底部对齐，只有上半部分随动画长高/回落
                let lyric_font = egui::FontId::proportional(14.0);
                // 行框底部会低于汉字实际墨迹底部约 3pt，这里减掉，使频谱底边与歌词/时间文字底边齐平
                let base_y = cy + painter
                    .layout_no_wrap("音".to_owned(), lyric_font, fg)
                    .size()
                    .y
                    * 0.5
                    - 3.0;

                // 频谱速度自适应：按歌曲「歌词密度 / 节奏」估算快慢（纯音乐 / 无歌词回退基准速度）
                {
                    let base_speed = 5.2f32;
                    let fac = {
                        let n = self.lyr_lines.len();
                        if n >= 4 {
                            let dur = (self.lyr_lines[n - 1].t - self.lyr_lines[0].t).max(1.0);
                            let chars: usize = self.lyr_lines.iter().map(|l| l.text.chars().count()).sum();
                            let cps = chars as f64 / dur; // 每秒字数：慢歌 ~2，快歌 ~6+
                            let f_cps = ((cps - 2.0) / 4.0).clamp(0.0, 1.0) as f32;
                            let gap = dur / (n - 1) as f64; // 平均每句时长（秒）
                            let f_gap = ((8.0 - gap) / 6.0).clamp(0.0, 1.0) as f32;
                            let mix = f_cps * 0.6 + f_gap * 0.4;
                            0.78 + mix * 0.62 // 0.78× ~ 1.40×
                        } else {
                            1.0
                        }
                    };
                    let target = base_speed * fac;
                    let dt = ((t - self.spec_last_t) as f32).clamp(0.0, 0.1);
                    self.spec_last_t = t;
                    let ksm = 1.0 - (-dt / 0.4).exp();
                    self.spec_speed += (target - self.spec_speed) * ksm;
                    self.spec_phase += (dt * self.spec_speed) as f64;
                    for k in 0..3 {
                        let ph = (self.spec_phase + (k as f64) * 1.4).sin() as f32 * 0.5 + 0.5;
                        let hgt = 4.0 + ph * 9.0;
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(sx + (k as f32) * 5.0, base_y - hgt),
                                egui::pos2(sx + (k as f32) * 5.0 + 3.0, base_y),
                            ),
                            1.5,
                            green,
                        );
                    }
                }
                // 中间滚动：优先显示当前歌词（逐字渐绿），无歌词则显示 歌名 · 歌手
                let base = egui::Color32::from_rgba_unmultiplied(200, 205, 218, am);
                // 逐字渐绿开关：关闭时当前句不做绿色填充
                let fill_green = if self.cfg.char_green { green } else { base };
                let text_rect = egui::Rect::from_min_size(
                    egui::pos2(sx + 22.0, pill.top()),
                    egui::vec2((badge_left - 10.0) - (sx + 22.0), pill.height()),
                );
                let instrumental = lyrics::is_instrumental(&self.lyr_lines);
                let cur = if instrumental { None } else { self.cur_lyric_idx() };
                if instrumental {
                    // 纯音乐：收起态直接显示“纯音乐，请欣赏”，与展开态一致�?
                    self.draw_marquee(&painter, text_rect, lyrics::INSTRUMENTAL_TEXT, egui::FontId::proportional(14.0), green, t, false, self.cfg.edge_fade);
                } else if let Some(i) = cur {
                    let line = self.lyr_lines[i].text.clone();
                    let prog = self.line_progress(i);
                    // �?这一句已经唱了多�?作为滚动相位：换到新的一句时跑马灯从最左重新开�?
                    let lt = (self.lyric_time() - self.lyr_lines[i].t).max(0.0);
                    // 换行上滚动画：上一句向上滚出并淡出，当前句从下方滚入（ease-out-cubic）
                    let roll_dur = self.cfg.roll_ms.clamp(50.0, 2000.0);
                    let mut roll = 1.0f32;
                    let mut old: Option<(String, f64)> = None;
                    if self.cfg.roll_anim {
                        if let Some(at) = self.lyr_anim_at {
                            let el = at.elapsed().as_secs_f32() * 1000.0;
                            if el < roll_dur {
                                let p = (el / roll_dur).clamp(0.0, 1.0);
                                roll = 1.0 - (1.0 - p).powi(3);
                                if let Some(pf) = self.lyr_anim_from {
                                    if pf != i && pf < self.lyr_lines.len() {
                                        let olt = (self.lyric_time() - self.lyr_lines[pf].t).max(0.0);
                                        old = Some((self.lyr_lines[pf].text.clone(), olt));
                                    }
                                }
                            } else {
                                self.lyr_anim_at = None;
                                self.lyr_anim_from = None;
                            }
                        }
                    }
                    if roll < 1.0 {
                        let lh = text_rect.height().max(1.0);
                        if let Some((ot, olt)) = &old {
                            let ody = -lh * roll;
                            let oa = ((am as f32) * (1.0 - roll)).clamp(0.0, 255.0) as u8;
                            let obase = egui::Color32::from_rgba_unmultiplied(200, 205, 218, oa);
                            self.draw_lyric_offset(
                                &painter, text_rect, text_rect, ody, ot,
                                egui::FontId::proportional(14.0), obase, obase, 0.0, *olt, false, self.cfg.edge_fade,
                            );
                        }
                        let ndy = lh * (1.0 - roll);
                        self.draw_lyric_offset(
                            &painter, text_rect, text_rect, ndy, &line,
                            egui::FontId::proportional(14.0), base, fill_green, prog, lt, false, self.cfg.edge_fade,
                        );
                    } else {
                        self.draw_lyric_line(&painter, text_rect, &line, egui::FontId::proportional(14.0), base, fill_green, prog, lt, false, self.cfg.edge_fade);
                    }
                } else {
                    let text = if self.music.title.is_empty() {
                        "正在播放".to_owned()
                    } else if self.music.artist.is_empty() {
                        self.music.title.clone()
                    } else {
                        format!("{} · {}", self.music.title, self.music.artist)
                    };
                    self.draw_marquee(&painter, text_rect, &text, egui::FontId::proportional(14.0), fg, t, false, self.cfg.edge_fade);
                }
                // 右侧时间（升级胶囊在它左边）
                time_rect = Some(painter.text(
                    egui::pos2(pill.right() - 13.0, cy),
                    egui::Align2::RIGHT_CENTER,
                    &time_s,
                    time_font,
                    fg,
                ));
                if self.show_time_update() {
                    let br = egui::Rect::from_min_size(
                        egui::pos2(badge_left, cy - UPD_BADGE_H * 0.5),
                        egui::vec2(UPD_BADGE_W, UPD_BADGE_H),
                    );
                    self.paint_update_pill(&painter, br, am);
                }
            } else if self.face_mode_active() {
                // 无音乐 + 表情模式：播放 Otto 原版逐帧动画
                let centered_a = (a as f32 * (1.0 - amt)) as u8;
                let idx = self.cfg.idle_face as usize;
                self.ensure_face_frames(ui.ctx(), idx);
                let metas = Self::face_anim_meta();
                let (aspect, fc) = metas
                    .get(idx)
                    .map(|(w, h, fc, _)| (*w as f32 / (*h).max(1) as f32, *fc as usize))
                    .unwrap_or((1.0, 1));
                let frame = (((t * 1000.0) / 80.0) as usize) % fc.max(1);
                let tex = self
                    .face_frames
                    .get(idx)
                    .and_then(|o| o.as_ref())
                    .and_then(|v| v.get(frame));
                let max_h = (pill.height() * 1.5).max(10.0);
                let fh = max_h.min((pill.width() - 12.0).max(8.0) / aspect.max(0.2));
                draw_robot_face(
                    &painter,
                    egui::pos2(pill.center().x, pill.center().y),
                    fh,
                    centered_a,
                    self.cfg.idle_face,
                    t,
                    tex,
                );
                // 表情是逐帧动画：始终按 60ms 平滑刷新（≈16.7fps，高于原生 80ms 帧间隔，绝不会丢帧）。
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(60));
            } else {
                let mut s = format!("{:02}:{:02}", st.wHour, st.wMinute);
                if self.cfg.show_seconds {
                    s = format!("{}:{:02}", s, st.wSecond);
                }
                let centered_a = (a as f32 * (1.0 - amt)) as u8; // 无音乐时居中时间，随音乐淡入而淡�?
                let col = egui::Color32::from_rgba_unmultiplied(238, 240, 246, centered_a);
                let font = bold_font(self.cfg.font);
                if self.show_time_update() {
                    // 「↑ 升级�? 时间整体居中
                    let time_w = painter
                        .layout_no_wrap(s.clone(), font.clone(), col)
                        .size()
                        .x;
                    let group_w = UPD_BADGE_W + UPD_BADGE_GAP + time_w;
                    let start = pill.center().x - group_w * 0.5;
                    let br = egui::Rect::from_min_size(
                        egui::pos2(start, pill.center().y - UPD_BADGE_H * 0.5),
                        egui::vec2(UPD_BADGE_W, UPD_BADGE_H),
                    );
                    self.paint_update_pill(&painter, br, centered_a);
                    time_rect = Some(painter.text(
                        egui::pos2(start + UPD_BADGE_W + UPD_BADGE_GAP, pill.center().y - self.cfg.font * 0.06),
                        egui::Align2::LEFT_CENTER,
                        &s,
                        font,
                        col,
                    ));
                } else {
                    // 无音乐、无升级：左侧常显天气（图标 + 温度 + 天气），右侧时间�?
                    if let Some(w) = self.w_data.as_ref() {
                        let f = self.cfg.weather_unit == 1;
                        let sky = weather::sky_of(&w.today_type, st.wHour >= 18 || st.wHour < 6);
                        let cy = pill.center().y;
                        let tcol = egui::Color32::from_rgba_unmultiplied(238, 240, 246, centered_a);
                        let dim = egui::Color32::from_rgba_unmultiplied(150, 158, 175, centered_a);
                        let bgm = egui::Color32::from_rgb(self.cfg.bg[0], self.cfg.bg[1], self.cfg.bg[2]);
                        draw_weather_icon(&painter, sky, egui::pos2(pill.left() + 18.0, cy), 20.0, centered_a, t, bgm);
                        let temp = weather::conv_temp(w.temp, f);
                        painter.text(egui::pos2(pill.left() + 34.0, cy - 0.5), egui::Align2::LEFT_CENTER, format!("{:.0}°", temp), bold_font(13.5), tcol);
                        if !w.today_type.is_empty() {
                            painter.text(egui::pos2(pill.left() + 66.0, cy), egui::Align2::LEFT_CENTER, &w.today_type, egui::FontId::proportional(11.0), dim);
                        }
                        time_rect = Some(painter.text(
                            egui::pos2(pill.right() - 13.0, cy - self.cfg.font * 0.06),
                            egui::Align2::RIGHT_CENTER,
                            &s,
                            bold_font(13.5),
                            tcol,
                        ));
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                    } else {
                        // 无天气数据时只居中显示时�?
                        time_rect = Some(painter.text(
                            egui::pos2(pill.center().x, pill.center().y - self.cfg.font * 0.06),
                            egui::Align2::CENTER_CENTER,
                            &s,
                            font,
                            col,
                        ));
                    }
                }
            }
        } else {
            self.draw_expanded_panel(ui, &painter, pill, e, t);
        }
    }

    /// 收起态：按用户配置的组件顺序横向拼接绘制（频谱/歌词/表情/时间/天气）。
    fn draw_collapsed_bar(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, pill: egui::Rect, e: f32, t: f64) {
        let vis = self.cb_anim.clamp(0.0, 1.0);
        let a = (((0.5 - e) * 2.0 * 255.0).clamp(0.0, 255.0) * vis) as u8;
        let am = (a as f32 * self.music_amt.clamp(0.0, 1.0)) as u8;
        let st = platform::local_time();
        let comps = self.active_comps();
        let clip_p = painter.with_clip_rect(pill);
        let painter: &egui::Painter = &clip_p;
        let cy = pill.center().y;
        let fg = egui::Color32::from_rgba_unmultiplied(238, 240, 246, a);
        let green = egui::Color32::from_rgba_unmultiplied(64, 214, 160, am);
        let lyric_font = egui::FontId::proportional(LYRIC_COLLAPSED_PT);
        let base_y = cy + painter.layout_no_wrap("音".to_owned(), lyric_font, fg).size().y * 0.5 - 3.0;
        let total: f32 = comps.iter().map(|(_, w)| *w).sum::<f32>()
            + {
                let mut g = 0.0f32;
                let mut prev: Option<&str> = None;
                for (id, _) in comps.iter() {
                    if let Some(p) = prev {
                        g += self.comp_gap(p, id);
                    }
                    prev = Some(id.as_str());
                }
                g
            };
        let mut x = pill.left() + ((pill.width() - total).max(0.0)) * 0.5;
        for (i, (id, cw)) in comps.iter().enumerate() {
            if i > 0 {
                let prev = comps[i - 1].0.as_str();
                x += self.comp_gap(prev, id);
            }
            if x > pill.right() - 2.0 {
                break;
            }
            let x1 = (x + *cw).min(pill.right() - 2.0);
            match id.as_str() {
                "spectrum" => {
                    self.draw_spectrum(painter, x, base_y, green, t);
                }
                "lyrics" => {
                    let tr = egui::Rect::from_min_size(egui::pos2(x, pill.top()), egui::vec2((x1 - x).max(1.0), pill.height()));
                    let base = egui::Color32::from_rgba_unmultiplied(200, 205, 218, am);
                    self.draw_collapsed_lyrics(painter, tr, fg, base, green, am, t);
                }
                "clock" => {
                    self.draw_collapsed_clock(painter, x, x1, cy, a, st);
                }
                "weather" => {
                    self.draw_collapsed_weather(painter, x, x1, cy, a, t, st);
                }
                "face" => {
                    let idx = self.cfg.idle_face as usize;
                    self.ensure_face_frames(ui.ctx(), idx);
                    let metas = Self::face_anim_meta();
                    let fc = metas
                        .get(idx)
                        .map(|(_, _, fc, _)| *fc as usize)
                        .unwrap_or(1);
                    // 先取 UV 裁剪盒（&mut self），再借 texture，避免借用冲突。
                    let (u0, v0, u1, v1) = self.face_content_uv(idx);
                    let frame = (((t * 1000.0) / 80.0) as usize) % fc.max(1);
                    let tex = self
                        .face_frames
                        .get(idx)
                        .and_then(|o| o.as_ref())
                        .and_then(|v| v.get(frame));
                    let cx = (x + x1) * 0.5;
                    let slot_w = (x1 - x).max(8.0);
                    // 按“不透明内容”包围盒裁剪并铺满槽宽，使表情模块的可视宽度≈槽宽，
                    // 这样它与相邻模块之间的实际间隙才等于 CB_GAP(10px)。
                    let (pw, ph) = metas
                        .get(idx)
                        .map(|(w, h, _, _)| (*w as f32, *h as f32))
                        .unwrap_or((77.0, 80.0));
                    let content_aspect =
                        ((pw * (u1 - u0)) / (ph * (v1 - v0)).max(1.0)).max(0.2);
                    let max_h = (pill.height() * 1.2).max(10.0);
                    let fw = slot_w.min(max_h * content_aspect);
                    let fh = fw / content_aspect;
                    if let Some(tx) = tex {
                        draw_face_img_cropped(
                            painter,
                            egui::pos2(cx, cy),
                            fw,
                            fh,
                            a,
                            tx,
                            (u0, v0, u1, v1),
                            FACE_EYE_GAP_REDUCE,
                        );
                    } else {
                        draw_robot_face(painter, egui::pos2(cx, cy), fh, a, self.cfg.idle_face, t, tex);
                    }
                    // 表情是逐帧动画：始终按 60ms 平滑刷新，绝不被空闲降帧拖慢。
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                }
                _ => {}
            }
            x = x1;
        }
        if self.music_motion {
            // 播放中 / 淡入淡出中：歌词逐字渐绿/跑马灯 + 频谱都要 60fps，保证会动的模块绝不掉帧。
            // 暂停且静止时 music_motion=false，这里不再强求 16ms，交由表情/天气的 60ms 兜底（≈16.7fps）。
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
        }
        if comps.iter().any(|(id, _)| id.as_str() == "weather") {
            // 天气图标是动画：始终 60ms 平滑刷新。
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
        }
    }

    /// 收起态频谱：3 根随歌词密度自适应速度跳动的绿色竖条。
    fn draw_spectrum(&mut self, painter: &egui::Painter, sx: f32, base_y: f32, green: egui::Color32, t: f64) {
        // 收起态的频谱只是一个「正在播放」的动画标识：固定匀速循环，不跟随音频电平/节奏。
        let base_speed = 3.2f32;
        let dt = ((t - self.spec_last_t) as f32).clamp(0.0, 0.1);
        self.spec_last_t = t;
        // 仅在「音乐在运动」时推进相位；暂停/静止时冻结竖条，避免低帧率下逐帧跳动。
        if self.music_motion {
            self.spec_phase += (dt * base_speed) as f64;
        }
        for k in 0..3 {
            let ph = (self.spec_phase + (k as f64) * 1.4).sin() as f32 * 0.5 + 0.5;
            let hgt = 4.0 + ph * 8.0;
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(sx + (k as f32) * 5.0, base_y - hgt),
                    egui::pos2(sx + (k as f32) * 5.0 + 3.0, base_y),
                ),
                1.5,
                green,
            );
        }
    }

    /// 歌词底部「水面反射」频谱：上方一组频谱灯柱，下方是随波纹横向荡开的倒影，
    /// 中间一条发光地平线。无真实 FFT 时用多条正弦叠加出有呼吸感的伪频谱（低频更强 + 拍点脉冲）。
    fn draw_water_reflection(&mut self, painter: &egui::Painter, rect: egui::Rect, fill_rect: egui::Rect, br: f32, accent: egui::Color32, a: u8, t: f64) {
        if rect.width() < 30.0 || rect.height() < 18.0 || a == 0 {
            return;
        }
        let zero_r = egui::CornerRadius { nw: 0, ne: 0, sw: 0, se: 0 };
        // 允许柱子向上探出频谱带一点点（进入歌词与频谱之间的空隙），但仍安全停在第五行歌词之下。
        let over = 4.0f32;
        let clip_rect = egui::Rect::from_min_max(
            egui::pos2(fill_rect.left(), rect.top() - 10.0),
            fill_rect.max,
        );
        let clip = painter.with_clip_rect(clip_rect);
        let uv = egui::epaint::WHITE_UV;
        let grad_rect = |p: &egui::Painter, r: egui::Rect, top: egui::Color32, bot: egui::Color32| {
            if r.width() <= 0.0 || r.height() <= 0.0 {
                return;
            }
            let mut m = egui::Mesh::default();
            m.vertices.push(egui::epaint::Vertex { pos: r.left_top(), uv, color: top });
            m.vertices.push(egui::epaint::Vertex { pos: r.right_top(), uv, color: top });
            m.vertices.push(egui::epaint::Vertex { pos: r.right_bottom(), uv, color: bot });
            m.vertices.push(egui::epaint::Vertex { pos: r.left_bottom(), uv, color: bot });
            m.indices = vec![0, 1, 2, 0, 2, 3];
            p.add(egui::Shape::mesh(m));
        };

        // 底部水波区域沿用音乐内容区本身的底色；水面渐变在右下角按岛体圆角收边，
        // 避免矩形填充把胶囊右下角涂成直角。
        let _ = accent;
        let br = br.max(0.0);
        let right = fill_rect.right();
        let bottom_y = fill_rect.bottom();
        // 给定 y，返回圆角内允许的最右 x（只处理右下角 se 圆角）。
        let max_x_at = |y: f32| -> f32 {
            if br <= 0.5 || y <= bottom_y - br {
                right
            } else {
                let dy = (y - (bottom_y - br)).clamp(0.0, br);
                right - br + (br * br - dy * dy).max(0.0).sqrt()
            }
        };
        // 给定 x，返回圆角内允许的最低 y。
        let bottom_at = |x: f32| -> f32 {
            if br <= 0.5 || x <= right - br {
                bottom_y
            } else {
                let dx = (x - (right - br)).clamp(0.0, br);
                bottom_y - br + (br * br - dx * dx).max(0.0).sqrt()
            }
        };

        let pad_x = 2.0f32;
        let x0 = fill_rect.left() + pad_x;
        // 右端不留内边距：让最后一根柱子贴住面板右边缘，和左边起始位置视觉更平衡。
        let x1 = fill_rect.right();
        // 地平线略下移（0.55→0.60），给上方灯柱更多生长空间；倒影仍在下方。
        let horizon = rect.top() + rect.height() * 0.60;
        let max_h = (horizon - (rect.top() - over) - 2.0).max(2.0);
        let refl_depth = (rect.bottom() - horizon - 1.0).max(2.0);
        let n = (((x1 - x0) / 7.0).floor() as usize).clamp(20, 72);
        let gap = 2.0f32;
        let bw = (((x1 - x0) - gap * (n as f32 - 1.0)) / n as f32).max(1.0);

        let playing = self.is_playing();
        let tt = t as f32;
        // 与 water-reflection-test.html 一致的时间缓动：未播放柱子逐帧向目标缓动（原来直接取目标值，
        // 显示器上像素取整会让缓慢起伏变成“一顿一顿”的掉帧感）；播放/未播放两种形态再交叉淡入，
        // 避免暂停↔播放切换时柱形突然卡过去。
        let dt = painter.ctx().input(|i| i.stable_dt).clamp(0.001, 0.05);
        let ease_idle = 1.0 - (-dt / 0.16).exp();
        let ease_mix = 1.0 - (-dt / 0.24).exp();
        self.wave_mix += ((if playing { 1.0 } else { 0.0 }) - self.wave_mix) * ease_mix;
        let wave_mix = self.wave_mix.clamp(0.0, 1.0);
        // 稳定的整数哈希→[0,1)，用于每根柱子的随机相位/幅度与逐帧小抖动
        let hash01 = |mut x: u32| -> f32 {
            x ^= x >> 16;
            x = x.wrapping_mul(0x7feb_352d);
            x ^= x >> 15;
            x = x.wrapping_mul(0x846c_a68b);
            x ^= x >> 16;
            (x & 0xffff) as f32 / 65535.0
        };
        let framef = (tt * 30.0) as u32;
        let smoothstep = |a: f32, b: f32, x: f32| -> f32 {
            let x = ((x - a) / (b - a)).clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        // HSL→RGB：与 water-reflection-test.html 一致的配色（青 195° → 品红 330°）
        let hsl = |h: f32, s: f32, l: f32, alpha: u8| -> egui::Color32 {
            let h = h.rem_euclid(360.0);
            let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
            let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
            let m = l - c * 0.5;
            let (r, g, b) = if h < 60.0 {
                (c, x, 0.0)
            } else if h < 120.0 {
                (x, c, 0.0)
            } else if h < 180.0 {
                (0.0, c, x)
            } else if h < 240.0 {
                (0.0, x, c)
            } else if h < 300.0 {
                (x, 0.0, c)
            } else {
                (c, 0.0, x)
            };
            egui::Color32::from_rgba_unmultiplied(
                ((r + m) * 255.0).clamp(0.0, 255.0) as u8,
                ((g + m) * 255.0).clamp(0.0, 255.0) as u8,
                ((b + m) * 255.0).clamp(0.0, 255.0) as u8,
                alpha,
            )
        };

        // 天空背光 + 水面底色：与 HTML 单效果版一致的蓝调氛围。
        grad_rect(
            &clip,
            egui::Rect::from_min_max(egui::pos2(fill_rect.left(), rect.top()), egui::pos2(fill_rect.right(), horizon)),
            egui::Color32::from_rgba_unmultiplied(8, 16, 34, 0),
            egui::Color32::from_rgba_unmultiplied(14, 34, 64, (a as f32 * 0.32) as u8),
        );
        // 水面渐变：按列构造网格，底边沿右下圆角收口（右侧最后 br 像素逐步抬高）。
        {
            let top_c = egui::Color32::from_rgba_unmultiplied(10, 42, 74, (a as f32 * 0.58) as u8);
            let bot_c = egui::Color32::from_rgba_unmultiplied(3, 10, 24, (a as f32 * 0.88) as u8);
            let cols = ((fill_rect.width() / 6.0).ceil() as usize).max(2);
            let mut m = egui::Mesh::default();
            for k in 0..=cols {
                let x = fill_rect.left() + fill_rect.width() * (k as f32 / cols as f32);
                let yb = bottom_at(x);
                m.vertices.push(egui::epaint::Vertex { pos: egui::pos2(x, horizon), uv, color: top_c });
                m.vertices.push(egui::epaint::Vertex { pos: egui::pos2(x, yb.max(horizon)), uv, color: bot_c });
            }
            for k in 0..cols {
                let i0 = (k * 2) as u32;
                let i1 = i0 + 1;
                let i2 = i0 + 2;
                let i3 = i0 + 3;
                m.indices.extend_from_slice(&[i0, i1, i2, i1, i3, i2]);
            }
            clip.add(egui::Shape::mesh(m));
        }

        for i in 0..n {
            let f = i as f32 / (n as f32 - 1.0).max(1.0);
            let d = ((f - 0.5).abs() * 2.0).clamp(0.0, 1.0);
            // “盆地”包络：中间压平、两端抬升。用幂函数 d^1.7 而非 smoothstep，
            // 让最外缘仍保留斜率、越靠边越能继续长高，不在两端形成一条齐平的顶线。
            let bowl = 0.18 + 1.06 * d.powf(1.7);
            let beat = ((tt * std::f32::consts::PI * (128.0 / 60.0) / 2.0).sin() * 0.5 + 0.5).powi(4);
            // 「未播放」的基础柱高（低频平滑起伏 + 极小固定随机 + 轻微拍点）
            let shape = 0.12 * (f * std::f32::consts::TAU * 1.0 + tt * 0.9).sin()
                + 0.07 * (f * std::f32::consts::TAU * 2.0 - tt * 0.6).sin()
                + 0.04 * (f * std::f32::consts::TAU * 3.5 + tt * 1.3).sin();
            let rnd = hash01((i as u32).wrapping_mul(2_654_435_761) ^ 0x9e37_79b9) * 2.0 - 1.0;
            let idle_target = (0.50 + shape + 0.05 * rnd + 0.09 * beat).clamp(0.30, 0.80);
            // 按横向归一化位置索引平滑状态，展开动画里柱数变化也不会打乱状态。
            let si = ((f * 127.0).round() as usize).min(127);
            let prev_idle = self.wave_idle[si];
            let idle_s = prev_idle + (idle_target - prev_idle) * ease_idle;
            self.wave_idle[si] = idle_s;
            let idle_v = (idle_s * bowl).clamp(0.0, 1.0);
            let play_v = if playing {
                // 以「未播放」的柱子为基底，叠加真实分频段能量做“泵动”：
                // 不同歌曲/时刻高低不同；中间始终受盆形约束，不会向上盖到第五行歌词
                let bands_now = crate::audio_meter::bands();
                let bpos = f * (crate::audio_meter::NBANDS - 1) as f32;
                let bi = bpos.floor() as usize;
                let bf = bpos - bi as f32;
                let b0 = bands_now[bi.min(crate::audio_meter::NBANDS - 1)];
                let b1 = bands_now[(bi + 1).min(crate::audio_meter::NBANDS - 1)];
                let band = b0 + (b1 - b0) * bf; // 0..1 真实能量
                // 鼓点/瞬态更突出：轻度 gamma 拉高低能量差，flick 只保留极小随机
                let bandv = band.powf(0.72);
                let flick = 0.97 + 0.06 * hash01((i as u32) ^ framef.wrapping_mul(2_654_435_761));
                // 以真实频段为主（78%）、未播放柱形纹理为辅（22%），让重鼓能明显顶起来
                let idle_n = ((idle_target - 0.30) / 0.50).clamp(0.0, 1.0);
                let mix = 0.22 * idle_n + 0.78 * bandv;
                let drive = 0.14 + 1.10 * mix; // 静音≈0.14，重鼓≈1.24
                // 底鼓包络整体“泵动”：鼓点瞬间所有柱子一起抬升，鼓点间回落，抖动更明显
                let kick = crate::audio_meter::kick();
                let vr = bowl * drive * flick * (0.55 + 0.90 * kick);
                let knee = 0.70f32;
                let ceil = 0.96f32;
                let target = if vr <= knee {
                    vr
                } else {
                    knee + (ceil - knee) * (1.0 - (-(vr - knee) / (ceil - knee) * 1.8).exp())
                };
                // 播放态直接取目标值：鼓点起音要“炸”，不能像空闲态那样慢慢追。
                // 平滑只用于未播放形态和两态交叉淡入，避免把底鼓峰值削平。
                self.wave_play[si] = target;
                target
            } else {
                // 暂停时让播放态内部值归到空闲形态，恢复播放时从当前空闲轮廓平滑起跳。
                self.wave_play[si] = idle_v;
                idle_v
            };
            // 未播放↔播放交叉淡入：切换过程中柱形连续形变，而不是瞬间跳到另一套高度。
            let vv = idle_v * (1.0 - wave_mix) + play_v * wave_mix;
            // 再对每根柱子的显示高度做一次攻击/释放平滑：保证播放↔暂停切换（尤其播放→暂停）也连续，不会直接跳短。
            let prev_disp = self.wave_disp[si];
            let tau_disp = if vv > prev_disp { 0.05_f32 } else { 0.11_f32 };
            let ease_disp = 1.0 - (-dt / tau_disp).exp();
            let disp = prev_disp + (vv - prev_disp) * ease_disp;
            self.wave_disp[si] = disp;
            let v = disp.clamp(0.05, 0.99);
            let hgt = (v * max_h).max(1.5) + 5.0;
            let bx = x0 + i as f32 * (bw + gap);

            // 配色与 HTML 单效果版一致：按横向位置 195°→330°（青→品红）
            let hue = 195.0 + f * 135.0;
            let base = hsl(hue, 0.92, 0.62, 255);
            let top_c = hsl(hue, 0.96, 0.78, (a as f32 * 0.98) as u8);
            let bot_c = hsl(hue, 0.90, 0.52, (a as f32 * 0.26) as u8);
            grad_rect(
                &clip,
                egui::Rect::from_min_max(egui::pos2(bx, horizon - hgt), egui::pos2(bx + bw, horizon)),
                top_c,
                bot_c,
            );
            // 顶部高光：不再单独叠一条 1.5px 的 cap（会在柱顶形成一小块异色块）
            // 直接把渐变顶端调亮，让柱顶与柱身同一个渐变平滑过渡。

            // 倒影：分段绘制 + 横向波纹抖动，越往下越淡（与 HTML 单效果版一致的算法）
            let refl_h = (hgt * 0.85).min(refl_depth);
            let segs = 14usize;
            for s in 0..segs {
                let depth = (s + 1) as f32 / segs as f32;
                let yy = horizon + depth * refl_h;
                let amp = 0.6 + depth * 4.2;
                let off = (yy * 0.09 + tt * 2.6 + i as f32 * 0.4).sin() * amp;
                let ra = ((1.0 - depth) * 0.50 * (0.35 + 0.65 * v) * a as f32) as u8;
                let rc = egui::Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), ra.max(2));
                let seg_h = refl_h / segs as f32 + 0.7;
                let rx0 = bx + off;
                let rx1 = (rx0 + bw).min(max_x_at((yy + seg_h).min(bottom_y)));
                if rx1 > rx0 {
                    clip.rect_filled(
                        egui::Rect::from_min_size(egui::pos2(rx0, yy), egui::vec2(rx1 - rx0, seg_h)),
                        zero_r,
                        rc,
                    );
                }
            }
        }

        // 横向水面波纹光：与 HTML 一样在水面上叠几条极淡的起伏线，增加水的质感。
        for s in 0..4 {
            let yy = horizon + 5.0 + s as f32 * (refl_depth / 5.0);
            let steps = (((x1 - x0) / 8.0).max(2.0)) as i32;
            let mut pts = Vec::with_capacity((steps + 1) as usize);
            for k in 0..=steps {
                let x = x0 + k as f32 * 8.0;
                let wy = yy + ((x * 0.03 + tt * 2.0 + s as f32).sin()) * 1.2;
                if x <= max_x_at(wy) {
                    pts.push(egui::pos2(x, wy));
                }
            }
            let ca = ((a as f32) * (0.06 - s as f32 * 0.008)).max(0.0) as u8;
            clip.add(egui::Shape::line(
                pts,
                egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(140, 225, 255, ca)),
            ));
        }

        // 地平线：一条均匀的发光细线（不再突出中间某一段，避免中间显得更亮一条）
        clip.rect_filled(
            egui::Rect::from_min_max(egui::pos2(fill_rect.left(), horizon - 0.5), egui::pos2(fill_rect.right(), horizon + 0.5)),
            zero_r,
            egui::Color32::from_rgba_unmultiplied(150, 235, 255, (a as f32 * 0.20) as u8),
        );
    }

    /// 收起态歌词槽：当前歌词（逐字渐绿 + 换行上滚）或无歌词时歌名·歌手。
    /// 槽宽固定 250；文字放得下就居中显示，放不下就从头部只滚一遍、滚到能完整显示即停。
    fn draw_collapsed_lyrics(&mut self, painter: &egui::Painter, text_rect: egui::Rect, fg: egui::Color32, base: egui::Color32, green: egui::Color32, am: u8, t: f64) {
        let fill_green = if self.cfg.char_green { green } else { base };
        let instrumental = lyrics::is_instrumental(&self.lyr_lines);
        let cur = if instrumental { None } else { self.cur_lyric_idx() };
        if instrumental {
            self.draw_marquee(painter, text_rect, lyrics::INSTRUMENTAL_TEXT, egui::FontId::proportional(LYRIC_COLLAPSED_PT), green, t, false, self.cfg.edge_fade);
        } else if let Some(i) = cur {
            let line = self.lyr_lines[i].text.clone();
            let prog = self.line_progress(i);
            let lt = (self.lyric_time() - self.lyr_lines[i].t).max(0.0);
            let roll_dur = self.cfg.roll_ms.clamp(50.0, 2000.0);
            let mut roll = 1.0f32;
            let mut old: Option<(String, f64)> = None;
            if self.cfg.roll_anim {
                if let Some(at) = self.lyr_anim_at {
                    let el = at.elapsed().as_secs_f32() * 1000.0;
                    if el < roll_dur {
                        let p = (el / roll_dur).clamp(0.0, 1.0);
                        roll = 1.0 - (1.0 - p).powi(3);
                        if let Some(pf) = self.lyr_anim_from {
                            if pf != i && pf < self.lyr_lines.len() {
                                let olt = (self.lyric_time() - self.lyr_lines[pf].t).max(0.0);
                                old = Some((self.lyr_lines[pf].text.clone(), olt));
                            }
                        }
                    } else {
                        self.lyr_anim_at = None;
                        self.lyr_anim_from = None;
                    }
                }
            }
            if roll < 1.0 {
                let lh = text_rect.height().max(1.0);
                if let Some((ot, olt)) = &old {
                    let ody = -lh * roll;
                    let oa = ((am as f32) * (1.0 - roll)).clamp(0.0, 255.0) as u8;
                    let obase = egui::Color32::from_rgba_unmultiplied(200, 205, 218, oa);
                    self.draw_lyric_offset(painter, text_rect, text_rect, ody, ot, egui::FontId::proportional(LYRIC_COLLAPSED_PT), obase, obase, 0.0, *olt, false, self.cfg.edge_fade);
                }
                let ndy = lh * (1.0 - roll);
                self.draw_lyric_offset(painter, text_rect, text_rect, ndy, &line, egui::FontId::proportional(LYRIC_COLLAPSED_PT), base, fill_green, prog, lt, false, self.cfg.edge_fade);
            } else {
                self.draw_lyric_line(painter, text_rect, &line, egui::FontId::proportional(LYRIC_COLLAPSED_PT), base, fill_green, prog, lt, false, self.cfg.edge_fade);
            }
        } else {
            let text = if self.music.title.is_empty() {
                "正在播放".to_owned()
            } else if self.music.artist.is_empty() {
                self.music.title.clone()
            } else {
                format!("{} · {}", self.music.title, self.music.artist)
            };
            self.draw_marquee(painter, text_rect, &text, egui::FontId::proportional(LYRIC_COLLAPSED_PT), fg, t, false, self.cfg.edge_fade);
        }
    }

    /// 收起态时钟槽：居中/右对齐显示时间；需要时在左侧画「升级」胶囊。
    fn draw_collapsed_clock(&mut self, painter: &egui::Painter, x: f32, x1: f32, cy: f32, a: u8, st: platform::SYSTEMTIME) {
        let mut s = format!("{:02}:{:02}", st.wHour, st.wMinute);
        if self.cfg.show_seconds {
            s = format!("{}:{:02}", s, st.wSecond);
        }
        let tcol = egui::Color32::from_rgba_unmultiplied(238, 240, 246, a);
        let right = if self.show_time_update() { x1 - 2.0 } else { x1 };
        if self.show_time_update() {
            let br = egui::Rect::from_min_size(egui::pos2(x, cy - UPD_BADGE_H * 0.5), egui::vec2(UPD_BADGE_W, UPD_BADGE_H));
            self.paint_update_pill(painter, br, a);
            painter.text(egui::pos2(right, cy - 0.5), egui::Align2::RIGHT_CENTER, &s, bold_font(13.5), tcol);
        } else {
            painter.text(egui::pos2((x + right) * 0.5, cy - 0.5), egui::Align2::CENTER_CENTER, &s, bold_font(13.5), tcol);
        }
    }

    /// 收起态天气槽：天气图标 + 温度（不显示天气类型文字，保持胶囊紧凑）。
    fn draw_collapsed_weather(&mut self, painter: &egui::Painter, x: f32, _x1: f32, cy: f32, a: u8, t: f64, st: platform::SYSTEMTIME) {
        let (sky, temp) = match self.w_data.as_ref() {
            Some(w) => (
                weather::sky_of(&w.today_type, st.wHour >= 18 || st.wHour < 6),
                weather::conv_temp(w.temp, self.cfg.weather_unit == 1),
            ),
            None => return,
        };
        let tcol = egui::Color32::from_rgba_unmultiplied(238, 240, 246, a);
        let bgm = egui::Color32::from_rgb(self.cfg.bg[0], self.cfg.bg[1], self.cfg.bg[2]);
        let text = format!("{:.0}°", temp);
        let fid = bold_font(13.5);
        // 实测温度文字宽度，供下一帧的天气槽宽精确贴合内容（避免槽内右侧留白）。
        let tw = painter.layout_no_wrap(text.clone(), fid.clone(), tcol).size().x;
        self.w_temp_w = tw;
        draw_weather_icon(painter, sky, egui::pos2(x + 10.0, cy), 20.0, a, t, bgm);
        painter.text(egui::pos2(x + 26.0, cy - 0.5), egui::Align2::LEFT_CENTER, text, fid, tcol);
    }

    /// 展开态但当前没有音乐会话时，显示友好的空状态，而不是一块空白深色面板。
    fn draw_expanded_idle(&self, painter: &egui::Painter, rect: egui::Rect, e: f32) {
        let a = (((e - 0.5) * 2.0) * 255.0).clamp(0.0, 255.0) as u8;
        painter.text(
            egui::pos2(rect.center().x, rect.center().y - 10.0),
            egui::Align2::CENTER_CENTER,
            "未在播放音乐",
            egui::FontId::proportional(15.0),
            egui::Color32::from_rgba_unmultiplied(150, 158, 175, a),
        );
        painter.text(
            egui::pos2(rect.center().x, rect.center().y + 12.0),
            egui::Align2::CENTER_CENTER,
            "打开音乐客户端播放后自动显示歌词",
            egui::FontId::proportional(12.0),
            egui::Color32::from_rgba_unmultiplied(118, 126, 142, a),
        );
    }

    /// 在给定矩形内绘制一行文本：放得下就居中，放不下就横向循环滚动（跑马灯）。
    fn draw_marquee(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        text: &str,
        font: egui::FontId,
        color: egui::Color32,
        t: f64,
        left_align: bool,
        left_fade: bool,
    ) {
        let galley = painter.layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER);
        let tw = galley.size().x;
        let y = rect.center().y - galley.size().y * 0.5;
        if tw <= rect.width() {
            let lx = if left_align { rect.left() } else { rect.center().x - tw * 0.5 };
            painter.galley(egui::pos2(lx, y), galley, color);
        } else {
            // 只滚一遍：从头部开始，滚到句尾与右边界对齐即停住，不再循环重复。
            let max_off = (tw - rect.width()).max(0.0);
            let off = ((t * 28.0) as f32).min(max_off);
            // 文本超出槽宽时（无论滚到哪一帧），左边缘都做 alpha 渐隐，避免文字与左侧图标之间出现生硬竖切；
            // 两侧渐隐：文本溢出槽宽时生效（左缘淡出、右缘让新进字渐显）
            let fade_left = left_fade;
            let fade_right = left_fade && (tw - (off + rect.width()) > 0.5);
            let fw = (rect.width() * 0.18).clamp(18.0, 42.0);
            self.galley_left_fade(painter, rect, egui::pos2(rect.left() - off, y), &galley, color, fw, fade_left, fade_right);
        }
    }

    /// 在 clip 内绘制一行 galley；支持左缘与右缘的 alpha 渐隐：
    /// - fade_left：左侧渐隐带，让已滚走的文字柔和淡出，而不是硬切；
    /// - fade_right：右侧渐隐带，让从右边新进入的文字“渐显”，而不是硬切。
    /// 两带之外的中段保持清晰。
    fn galley_left_fade(
        &self,
        painter: &egui::Painter,
        clip: egui::Rect,
        pos: egui::Pos2,
        galley: &std::sync::Arc<egui::Galley>,
        color: egui::Color32,
        fade_w: f32,
        fade_left: bool,
        fade_right: bool,
    ) {
        if (!fade_left && !fade_right) || color.a() == 0 {
            painter.with_clip_rect(clip).galley(pos, galley.clone(), color);
            return;
        }
        let fw = fade_w.min(clip.width() * 0.5).max(1.0);
        let lf = if fade_left { fw } else { 0.0 };
        let rf = if fade_right { fw } else { 0.0 };
        // 清晰段：两侧渐隐带之间，一次绘制
        let crisp = egui::Rect::from_min_max(
            egui::pos2(clip.left() + lf, clip.top()),
            egui::pos2(clip.right() - rf, clip.bottom()),
        );
        if crisp.width() > 0.5 {
            painter.with_clip_rect(crisp).galley(pos, galley.clone(), color);
        }
        let bands = 20usize;
        // 左缘：alpha 由 0 平滑升到满
        if fade_left {
            let bw = fw / bands as f32;
            for i in 0..bands {
                let tt = (i as f32 + 0.5) / bands as f32;
                // smoothstep，让透明度过渡更柔和自然（不是线性生硬）
                let ee = tt * tt * (3.0 - 2.0 * tt);
                let a = (color.a() as f32 * ee).round().clamp(0.0, 255.0) as u8;
                if a == 0 {
                    continue;
                }
                let c = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a);
                let band = egui::Rect::from_min_max(
                    egui::pos2(clip.left() + i as f32 * bw, clip.top()),
                    egui::pos2(clip.left() + (i as f32 + 1.0) * bw, clip.bottom()),
                )
                .intersect(clip);
                if band.width() <= 0.0 {
                    continue;
                }
                painter.with_clip_rect(band).galley(pos, galley.clone(), c);
            }
        }
        // 右缘：alpha 由满平滑降到 0（新字从右进入时渐显）
        if fade_right {
            let bw = fw / bands as f32;
            for i in 0..bands {
                let t_in = (i as f32 + 0.5) / bands as f32; // 0=靠近中段, 1=最右边缘
                let ee = 1.0 - t_in * t_in * (3.0 - 2.0 * t_in);
                let a = (color.a() as f32 * ee).round().clamp(0.0, 255.0) as u8;
                if a == 0 {
                    continue;
                }
                let c = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a);
                let x0 = clip.right() - fw + i as f32 * bw;
                let band = egui::Rect::from_min_max(
                    egui::pos2(x0, clip.top()),
                    egui::pos2(x0 + bw, clip.bottom()),
                )
                .intersect(clip);
                if band.width() <= 0.0 {
                    continue;
                }
                painter.with_clip_rect(band).galley(pos, galley.clone(), c);
            }
        }
    }

    /// 画一行歌词：base 色打底，按 prog 从左到右用 fill 色覆盖（逐字渐绿）；过长则横向滚动。
    /// 按“已唱字数比例”算出应有的像素边界（按每个字形实际宽度累加），
    /// 比 `prog * 整行宽度` 精确，避免中英文混排时渐绿位置漂移。
    fn lyric_fill_dx(&self, galley: &egui::Galley, prog: f32) -> f32 {
        let target = (prog.clamp(0.0, 1.0) as f64)
            * galley
                .rows
                .iter()
                .flat_map(|r| r.glyphs.iter())
                .filter(|g| !g.chr.is_whitespace())
                .count() as f64;
        let mut n = 0usize;
        for g in galley.rows.iter().flat_map(|r| r.glyphs.iter()) {
            if g.chr.is_whitespace() {
                continue;
            }
            let lo = n as f64;
            let hi = lo + 1.0;
            if target <= lo {
                return g.pos.x;
            }
            if target < hi {
                return g.pos.x + (target - lo) as f32 * g.advance_width;
            }
            n += 1;
        }
        galley.size().x
    }

    /// 有新版本时的小徽标：薄荷绿描边圆角小方块 + 向上箭头（不要文字，避免像状态灯）。
    /// 收起态「↑ 升级」胶囊：按设计稿 1:1 还原（深绿底、薄荷描边、实心箭头 + 升级）。
    fn paint_update_pill(&mut self, painter: &egui::Painter, rect: egui::Rect, alpha: u8) {
        if self.upd_badge_tex.is_none() {
            let bytes: &[u8] = include_bytes!("../assets/update_badge.png");
            if let Ok(img) = image::load_from_memory(bytes) {
                let img = img.to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                self.upd_badge_tex = Some(painter.ctx().load_texture(
                    "update_badge",
                    ci,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        if let Some(tex) = self.upd_badge_tex.as_ref() {
            painter.image(
                tex.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::from_white_alpha(alpha),
            );
        }
    }

    fn paint_update_mark(
        &self,
        painter: &egui::Painter,
        center: egui::Pos2,
        size: f32,
        animate: bool,
    ) {
        // 图标字形来自系统图标组件库（Segoe MDL2 Assets，E74A = Up），这里只提供圆形徽标底座。
        let t = painter.ctx().input(|i| i.time) as f32;
        let wave = 0.5 + 0.5 * (t * 2.6).sin();
        let (bg_a, bob) = if animate {
            ((205.0 + 50.0 * wave) as u8, -1.2 * wave)
        } else {
            (245, 0.0)
        };
        let c = egui::pos2(center.x, center.y + bob);
        painter.circle_filled(
            c,
            size * 0.5,
            egui::Color32::from_rgba_unmultiplied(74, 222, 128, bg_a),
        );
        painter.text(
            c,
            egui::Align2::CENTER_CENTER,
            "\u{E74A}",
            icon_font(size * 0.56),
            egui::Color32::from_rgba_unmultiplied(6, 46, 35, 255),
        );
    }

    /// 歌词上下边缘柔和淡出：按“整行盒子”到上下边界的距离衰减。
    /// 行只要有一部分越过边界，就几乎透明（alpha≈0），所以不会被硬生生切出半行；
    /// 完全落在边界内的行保持正常亮度。纯文字 alpha，不叠加任何底色矩形，避免色差。
    fn fade_lyric_color(
        &self,
        base: egui::Color32,
        row_top: f32,
        row_bottom: f32,
        rect: egui::Rect,
    ) -> egui::Color32 {
        // 以整行盒到歌词区上/下边缘的实际像素距离做渐隐：最上一行/最下一行明显虚化，中间行清晰。
        let fade = 42.0f32;
        let d = (row_top - rect.top())
            .min(rect.bottom() - row_bottom)
            .max(0.0);
        let t = (d / fade).clamp(0.0, 1.0);
        let s = t * t * (3.0 - 2.0 * t);
        egui::Color32::from_rgba_unmultiplied(
            base.r(),
            base.g(),
            base.b(),
            (base.a() as f32 * s) as u8,
        )
    }

    fn draw_lyric_line(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        text: &str,
        font: egui::FontId,
        base: egui::Color32,
        fill: egui::Color32,
        prog: f32,
        t: f64,
        left_align: bool,
        left_fade: bool,
    ) {
        let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER);
        let tw = galley.size().x;
        let y = rect.center().y - galley.size().y * 0.5;
        let scroll = tw > rect.width();
        // 只滚一遍：滚到句尾与右边对齐就停住，不再循环重复展�?
        let off = if scroll {
            let max_off = (tw - rect.width()).max(0.0);
            ((t * 30.0) as f32).min(max_off)
        } else {
            0.0
        };
        // 需要渐绿时，预生成一份 fill 色 galley
        let galf = if prog > 0.0 {
            Some(painter.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER))
        } else {
            None
        };
        // 左侧渐隐（已滚走部分淡出）+ 右侧渐隐（新进字渐显）
        let fade_on = left_fade && scroll;
        let more_right = scroll && (tw - (off + rect.width()) > 0.5);
        let fade_right = left_fade && more_right;
        let fw = (rect.width() * 0.18).clamp(18.0, 42.0);
        for k in 0..1 {
            let left = if scroll {
                rect.left() - off + (k as f32) * tw
            } else if left_align {
                rect.left()
            } else {
                rect.center().x - tw * 0.5
            };
            self.galley_left_fade(painter, rect, egui::pos2(left, y), &galley, base, fw, fade_on, fade_right);
            // 逐字渐绿：两段（含滚动时的第二份）都按同一进度填充，保证滚动时也持续变绿
            if let Some(gf) = &galf {
                let right = left + self.lyric_fill_dx(&galley, prog);
                let gcl = egui::Rect::from_min_max(
                    egui::pos2(left, rect.top()),
                    egui::pos2(right, rect.bottom()),
                )
                .intersect(rect);
                let fill_right_fade = fade_right && (right >= rect.right() - 0.5);
                if gcl.width() > 0.5 {
                    self.galley_left_fade(painter, gcl, egui::pos2(left, y), gf, fill, fw, fade_on, fill_right_fade);
                }
            }
        }
    }

    /// 展开态「纯音乐，请欣赏」艺术字：逐字横向渐变（薄荷绿→近白）+ 柔和外发光，居中显示。
    /// 仅整首无歌词的纯音乐使用；收起态仍用普通跑马灯（用户要求“即可”）。
    fn draw_instrumental_art(&self, painter: &egui::Painter, rect: egui::Rect, cy: f32, a: u8) {
        if a == 0 {
            return;
        }
        // 朴素纯灰：与汽水一致——单色、无渐变、无发光，居中一行。
        let gray = egui::Color32::from_rgba_unmultiplied(150, 150, 150, a);
        painter.text(
            egui::pos2(rect.center().x, cy),
            egui::Align2::CENTER_CENTER,
            lyrics::INSTRUMENTAL_TEXT,
            egui::FontId::proportional(15.0),
            gray,
        );
    }

    /// 画一行歌词（支持竖直偏移 dy 与自定义裁剪 clip），供收起态“上滚”动画使用：
    /// base 打底，prog>0 时从左到右用 fill 渐绿；过长则横向滚动（只滚一遍，滚到句尾停住）�?
    #[allow(clippy::too_many_arguments)]
    fn draw_lyric_offset(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        clip: egui::Rect,
        dy: f32,
        text: &str,
        font: egui::FontId,
        base: egui::Color32,
        fill: egui::Color32,
        prog: f32,
        t: f64,
        left_align: bool,
        left_fade: bool,
    ) {
        let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER);
        let tw = galley.size().x;
        let gh = galley.size().y;
        let y = rect.center().y - gh * 0.5 + dy;
        let scroll = tw > rect.width();
        let off = if scroll {
            let max_off = (tw - rect.width()).max(0.0);
            ((t * 30.0) as f32).min(max_off)
        } else {
            0.0
        };
        let left = if scroll {
            rect.left() - off
        } else if left_align {
            rect.left()
        } else {
            rect.center().x - tw * 0.5
        };
        // 左侧渐隐（已滚走部分淡出）+ 右侧渐隐（新进字渐显）
        let fade_on = left_fade && scroll;
        let more_right = scroll && (tw - (off + rect.width()) > 0.5);
        let fade_right = left_fade && more_right;
        let fw = (rect.width() * 0.18).clamp(18.0, 42.0);
        self.galley_left_fade(painter, clip, egui::pos2(left, y), &galley, base, fw, fade_on, fade_right);
        if prog > 0.0 {
            let galf = painter.layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER);
            let right = left + self.lyric_fill_dx(&galf, prog);
            let gcl = egui::Rect::from_min_max(
                egui::pos2(left, y),
                egui::pos2(right, y + gh),
            )
            .intersect(clip);
            let fill_right_fade = fade_right && (right >= clip.right() - 0.5);
            if gcl.width() > 0.5 && gcl.height() > 0.5 {
                self.galley_left_fade(painter, gcl, egui::pos2(left, y), &galf, fill, fw, fade_on, fill_right_fade);
            }
        }
    }

    /// 展开态音乐面板：左侧旋转唱片 + 右侧 歌名/歌手/五行歌词/进度/控制。
    /// 展开态音乐面板：左竖排模块菜单 + 右侧音乐内容（唱片/歌名/进度/控制 + 五行歌词）
    fn draw_music_expanded(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, rect: egui::Rect, e: f32, t: f64) {
        let a = (((e - 0.5) * 2.0) * 255.0 * self.music_amt.clamp(0.0, 1.0)).clamp(0.0, 255.0) as u8;
        let ink = egui::Color32::from_rgba_unmultiplied(233, 236, 242, a);
        let dim = egui::Color32::from_rgba_unmultiplied(150, 158, 175, a);
        let faint = egui::Color32::from_rgba_unmultiplied(110, 117, 133, a);
        let green = egui::Color32::from_rgba_unmultiplied(56, 214, 150, a);
        let green_dim = egui::Color32::from_rgba_unmultiplied(26, 84, 64, a);

        // ---------- 仅保留音乐模块（不绘制左侧菜单栏）----------
        // 与外壳统一的内容边距：左右上 16、底部 14
        let cx0 = rect.left() + CONTENT_PAD_X;
        let cx1 = rect.right() - CONTENT_PAD_X;

        // 动画：唱片旋转 + 唱臂播放/暂停摆动（参考原型：PLAY=-15°、PAUSE=-41°）
        let dt = ui.ctx().input(|i| i.stable_dt).clamp(0.0, 0.1);
        let playing_now = self.is_playing();
        if playing_now {
            self.disc_angle = (self.disc_angle + dt * 0.45) % std::f32::consts::TAU; // �?14s/圈（放慢�?
        }
        let arm_target = if playing_now { -22.0 } else { -40.0 };
        let k = (1.0 - (-dt * 9.0).exp()).clamp(0.0, 1.0);
        self.arm_angle += (arm_target - self.arm_angle) * k;

        // 圆盘三层：最外灰环 / 中间黑胶（带细音纹）/ 最中间圆形封面
        let tt_size = 150.0;
        let outer_r = 68.0 / 180.0 * tt_size; // 最外层灰环外径
        // 唱片区 150x150 的方框里有透明留白（外环半径 < 半个方框）。要让「看得见的圆盘边缘」
        // 与内容区左/上边距完全相等，就把方框整体向 左/上 挪掉这圈留白。
        let disc_pad = tt_size * 0.5 - outer_r;
        // 视觉圆盘与唱臂整体略往下放，别顶着面板上沿
        let disc_top_nudge = 11.0f32;
        // 圆盘（含唱臂、底座）整体向右移动 4 逻辑像素
        let disc_dx = 4.0f32;
        let tt_rect = egui::Rect::from_min_size(
            egui::pos2(
                cx0 - disc_pad + disc_dx,
                rect.top() + CONTENT_PAD_T - disc_pad + disc_top_nudge,
            ),
            egui::vec2(tt_size, tt_size),
        );
        let dc = tt_rect.center();
        let vinyl_r = 64.2 / 180.0 * tt_size; // 黑胶半径（灰环再细约2px�?
        let cover_r = 46.6 / 180.0 * tt_size; // 中心封面半径（再放大 ~3px 物理�?
        // 1) 最外灰色环
        painter.circle_filled(dc, outer_r, egui::Color32::from_rgba_unmultiplied(54, 58, 66, a));
        
        // 2) 中间黑胶
        painter.circle_filled(dc, vinyl_r, egui::Color32::from_rgba_unmultiplied(8, 9, 13, a));
        for k in 0..9 {
            let rr = cover_r + (vinyl_r - cover_r) * (k as f32 + 0.5) / 9.0;
            painter.circle_stroke(dc, rr, egui::Stroke::new(0.5, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 9)));
        }
        if let Some(ct) = self.cover_tex.as_ref() {
            // 3) 中心圆形封面（随盘旋转）
            draw_rotating_circle_tex(painter, ct.id(), dc, cover_r, self.disc_angle, a);
        } else {
            // 无真实封面：中心用按歌名哈希的渐变圆 + 音符兜底
            let mut h: u64 = 1469598103934665603;
            for b in self.music.title.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(1099511628211);
            }
            let hue = (h % 360) as f32;
            // 逐像素抗锯齿的圆形渐变贴图（按歌名哈希缓存），彻底消除锯齿与边缘缺像素
            let need_rebuild = match &self.fallback_cover_tex {
                Some((hk, _)) => *hk != h,
                None => true,
            };
            if need_rebuild {
                let ppp = ui.ctx().pixels_per_point().max(1.0);
                let s = (((cover_r * 2.0 * ppp).ceil() as usize) + 2).clamp(24, 512);
                let top = hsv(hue, 0.55, 0.62, 255);
                let bot = hsv((hue + 40.0) % 360.0, 0.62, 0.34, 255);
                let (tr, tg, tb) = (top.r() as f32, top.g() as f32, top.b() as f32);
                let (br, bg, bb) = (bot.r() as f32, bot.g() as f32, bot.b() as f32);
                let mut rgba = vec![0u8; s * s * 4];
                let c = (s as f32 - 1.0) * 0.5;
                let rad = s as f32 * 0.5 - 1.0;
                for y in 0..s {
                    let t = y as f32 / (s as f32 - 1.0).max(1.0);
                    let cr = (tr + (br - tr) * t).round() as u8;
                    let cg = (tg + (bg - tg) * t).round() as u8;
                    let cb = (tb + (bb - tb) * t).round() as u8;
                    for x in 0..s {
                        let dx = x as f32 - c;
                        let dy = y as f32 - c;
                        let dist = (dx * dx + dy * dy).sqrt();
                        let cov = (rad + 0.5 - dist).clamp(0.0, 1.0);
                        let idx = (y * s + x) * 4;
                        rgba[idx] = cr;
                        rgba[idx + 1] = cg;
                        rgba[idx + 2] = cb;
                        rgba[idx + 3] = (cov * 255.0).round() as u8;
                    }
                }
                let ci = egui::ColorImage::from_rgba_unmultiplied([s, s], &rgba);
                let tex = ui.ctx().load_texture("fallback_cover", ci, egui::TextureOptions::LINEAR);
                self.fallback_cover_tex = Some((h, tex));
            }
            if let Some((_, tex)) = self.fallback_cover_tex.as_ref() {
                // 兜底封面也像真实封面一样，用圆扇 mesh 随 disc_angle 旋转
                draw_rotating_circle_tex(painter, tex.id(), dc, cover_r, self.disc_angle, a);
            }
            // 中心音符：用 TextShape 绕盘心旋转，保证与封面同向同步转动（不再钉死在中心）
            let note_color = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 210);
            let ng = painter.layout_no_wrap("♪".to_owned(), egui::FontId::proportional(cover_r * 0.9), note_color);
            // TextShape 的旋转支点是 pos（左上角）：令 pos = dc - R(angle)*字形中心偏移，
            // 旋转后字形视觉中心正好落在盘心，实现绕盘心旋转。
            let c0 = ng.mesh_bounds.center().to_vec2();
            let ang = self.disc_angle;
            let (ns, nc) = (ang.sin(), ang.cos());
            let rot_c0 = egui::vec2(nc * c0.x - ns * c0.y, ns * c0.x + nc * c0.y);
            let note_pos = egui::pos2(dc.x - rot_c0.x, dc.y - rot_c0.y);
            painter.add(egui::epaint::TextShape::new(note_pos, ng, note_color).with_angle(ang));
        }
        // 唱臂：底座静止 + 臂绕固定支点旋转（支点在贴图内 (146,20)，因整体 translate(-12,0)）
        if let Some(base) = self.tt_base.as_ref() {
            painter.image(base.id(), tt_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::from_rgba_unmultiplied(255, 255, 255, a));
        }
        if let Some(tex) = self.tt_arm0.as_ref() {
            let pivot = egui::pos2(tt_rect.left() + 146.0 / 180.0 * tt_size, tt_rect.top() + 20.0 / 180.0 * tt_size);
            let delta = self.arm_angle.to_radians();
            draw_tex_rotated(painter, tex.id(), tt_rect, pivot, delta, a);
        }

        // 右侧文字区：头部（歌名 / 进度条起点 / 控制按钮）在原先整体左移 10px 的基础上回退 3px（净左移 7px）；
        // 尾部（进度条终点 / 时长 / 来源标签）保持不动（仍为原先的 -10px）。
        // 以「圆盘未右移时」的右边缘为基准，避免下面圆盘右移把文字区一起带走。
        let head_dx = -7.0f32;
        let tt_right_ref = tt_rect.right() - disc_dx;
        let tx = tt_right_ref + 18.0 + head_dx;
        let title = if self.music.title.is_empty() { "未知曲目".to_string() } else { self.music.title.clone() };
        let artist = self.music.artist.clone();
        let avail = (cx1 - tx).max(40.0);
        // 单行省略：用 LayoutJob 限制为 1 行 + 溢出字符「…」，保证文字完整可读、绝不越界被裁切
        let one_line = |text: &str, font: egui::FontId, color: egui::Color32, maxw: f32| -> std::sync::Arc<egui::Galley> {
            let mut job = egui::text::LayoutJob::single_section(
                text.to_string(),
                egui::TextFormat { font_id: font, color, ..Default::default() },
            );
            job.wrap.max_width = maxw;
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            ui.fonts(|f| f.layout_job(job))
        };
        let af = egui::FontId::proportional(12.0);
        let gap = if artist.is_empty() { 0.0 } else { 16.0 };
        let aw = if artist.is_empty() {
            0.0
        } else {
            ui.fonts(|f| f.layout_no_wrap(artist.clone(), af.clone(), dim).size().x)
        };
        // 优先单行「歌名 - 歌手」：只要在可读字号（>=12.5）下能完整放下，就保持一行
        let mut fit_tsz: Option<f32> = None;
        let mut s = 15.5f32;
        while s >= 12.5 - 1e-3 {
            let w = ui.fonts(|f| f.layout_no_wrap(title.clone(), bold_font(s), ink).size().x);
            if w + gap + aw <= avail {
                fit_tsz = Some(s);
                break;
            }
            s -= 0.5;
        }
        let title_cy_single = rect.top() + CONTENT_PAD_T + 31.0;
        if let Some(ts) = fit_tsz {
            // —— 一行完整显示 ——
            let tf = bold_font(ts);
            let tw = ui.fonts(|f| f.layout_no_wrap(title.clone(), tf.clone(), ink).size().x);
            painter.text(egui::pos2(tx, title_cy_single), egui::Align2::LEFT_CENTER, &title, tf, ink);
            if !artist.is_empty() {
                let mid_x = tx + tw + gap * 0.5;
                let ax = tx + tw + gap;
                painter.text(egui::pos2(mid_x, title_cy_single), egui::Align2::CENTER_CENTER, "-", af.clone(), dim);
                painter.text(egui::pos2(ax, title_cy_single), egui::Align2::LEFT_CENTER, &artist, af, dim);
            }
        } else {
            // —— 一行放不下：两行显示（歌名自适应/必要时省略，歌手独占第二行完整显示）——
            let mut tsz = 15.5f32;
            loop {
                let w = ui.fonts(|f| f.layout_no_wrap(title.clone(), bold_font(tsz), ink).size().x);
                if w <= avail || tsz <= 12.5 { break; }
                tsz -= 0.5;
            }
            let tf = bold_font(tsz);
            let af2 = egui::FontId::proportional(11.5);
            let (title_cy, artist_cy) = if artist.is_empty() {
                (rect.top() + CONTENT_PAD_T + 31.0, 0.0)
            } else {
                (rect.top() + CONTENT_PAD_T + 23.0, rect.top() + CONTENT_PAD_T + 39.0)
            };
            let tg = one_line(&title, tf, ink, avail);
            painter.galley(egui::pos2(tx, title_cy - tg.size().y * 0.5), tg, ink);
            if !artist.is_empty() {
                let prefix = "— ";
                let pw = ui.fonts(|f| f.layout_no_wrap(prefix.to_string(), af2.clone(), dim).size().x);
                let ag = one_line(&artist, af2.clone(), dim, (avail - pw).max(10.0));
                let ay = artist_cy - ag.size().y * 0.5;
                painter.text(egui::pos2(tx, artist_cy), egui::Align2::LEFT_CENTER, prefix, af2.clone(), dim);
                painter.galley(egui::pos2(tx + pw, ay), ag, dim);
            }
        }

        // 进度条 + 时间（滑块式）
        let dur = self.duration_est();
        let pos = self.display_pos();
        let prog = if dur > 1.0 { (pos / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
        let bar_y = rect.top() + CONTENT_PAD_T + 69.0 - 10.0;
        let bar = egui::Rect::from_min_max(egui::pos2(tx, bar_y), egui::pos2(cx1, bar_y + 3.0));
        painter.rect_filled(bar, 1.5, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30));
        painter.rect_filled(egui::Rect::from_min_max(bar.min, egui::pos2(tx + bar.width() * prog, bar_y + 3.0)), 1.5, green);
        // 进度条只做展示、不可拖动（拖动交给播放器本身；去掉白色圆点避免误以为可拖）
        let fmt = |s: f64| {
            let s = s.max(0.0) as u32;
            format!("{:02}:{:02}", s / 60, s % 60)
        };
        painter.text(egui::pos2(tx, bar_y + 6.0), egui::Align2::LEFT_TOP, fmt(pos), egui::FontId::proportional(10.0), faint);
        painter.text(egui::pos2(cx1, bar_y + 6.0), egui::Align2::RIGHT_TOP, fmt(dur), egui::FontId::proportional(10.0), faint);

        // 控制按钮：上一首 / 播放(绿色大圆) / 下一首
        let btn_y = rect.top() + CONTENT_PAD_T + 108.0 - 10.0;
        let mut clicked: Option<&str> = None;
        let prev_c = egui::pos2(tx + 12.0, btn_y);
        let rprev = egui::Rect::from_center_size(prev_c, egui::vec2(22.0, 22.0));
        self.ctrl_hit.push(rprev);
        let resp_prev = ui.interact(rprev, ui.id().with("mctrl_prev"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        painter.circle_filled(prev_c, 11.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 18));
        let c_dn = egui::Color32::from_rgba_unmultiplied(200, 210, 220, 200);
        painter.text(prev_c, egui::Align2::CENTER_CENTER, "\u{E892}", icon_font(12.0), c_dn); // Previous
        if resp_prev.clicked() {
            clicked = Some("prev");
        }
        let play_c = egui::pos2(tx + 58.0, btn_y);
        let rplay = egui::Rect::from_center_size(play_c, egui::vec2(36.0, 36.0));
        self.ctrl_hit.push(rplay);
        let resp_play = ui.interact(rplay, ui.id().with("mctrl_play"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        painter.circle_filled(play_c, 17.0, green);
        let ic = egui::Color32::from_rgba_unmultiplied(10, 20, 16, a);
        let play_glyph = if self.is_playing() { "\u{E769}" } else { "\u{E768}" }; // Pause / Play
        painter.text(play_c, egui::Align2::CENTER_CENTER, play_glyph, icon_font(14.0), ic);
        if resp_play.clicked() {
            clicked = Some("play");
        }
        let next_c = egui::pos2(tx + 104.0, btn_y);
        let rnext = egui::Rect::from_center_size(next_c, egui::vec2(22.0, 22.0));
        self.ctrl_hit.push(rnext);
        let resp_next = ui.interact(rnext, ui.id().with("mctrl_next"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        painter.circle_filled(next_c, 11.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 18));
        painter.text(next_c, egui::Align2::CENTER_CENTER, "\u{E893}", icon_font(12.0), c_dn); // Next
        if resp_next.clicked() {
            clicked = Some("next");
        }
        if !self.music.app_id.is_empty() {
            // 呼吸灯（连接状态灯）+ 客户端名：整体贴齐进度条右端。
            // 呼吸灯右缘与进度条末尾（cx1＝布局右边缘）平齐，客户端名放在其左侧，不再溢出到右侧留白里。
            let state = self.music_link_state();
            let (cr, cg, cb) = match state {
                0 => (46u8, 204u8, 113u8),  // 绿：已连上、正常
                1 => (245u8, 190u8, 40u8),  // 黄：已连上但当前无歌词（如歌单模式）
                _ => (235u8, 64u8, 52u8),   // 红：未连上播放器
            };
            let fade = (a as f32 / 255.0).clamp(0.0, 1.0);
            let pulse = 0.5 + 0.5 * (t as f32 * 2.0).sin();
            let dot_r = 4.6f32;
            let dot = egui::pos2(cx1 - dot_r, btn_y - 6.0);
            painter.text(
                egui::pos2(dot.x - dot_r - 5.0, btn_y - 6.0),
                egui::Align2::RIGHT_CENTER,
                src_label(&self.music.app_id),
                egui::FontId::proportional(10.5),
                faint,
            );
            painter.circle_filled(dot, dot_r, egui::Color32::from_rgba_unmultiplied(cr, cg, cb, (40.0 * (0.35 + 0.65 * pulse) * fade) as u8));
            painter.circle_filled(dot, 2.5, egui::Color32::from_rgba_unmultiplied(cr, cg, cb, ((0.45 + 0.55 * pulse) * fade * 255.0) as u8));
        }
        if let Some(id) = clicked {
            {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(crate::paths::file("ctrl_debug.log")) {
                    let _ = writeln!(f, "click={}", id);
                }
            }
            let was_playing = self.is_playing();
            let cmd = match id {
                "prev" => smtc::Cmd::Prev,
                "play" => {
                    if was_playing {
                        smtc::Cmd::Pause
                    } else {
                        smtc::Cmd::Play
                    }
                }
                _ => smtc::Cmd::Next,
            };
            // 乐观更新图标/进度（真正状态稍后由 SMTC 快照校正）
            match id {
                "play" => {
                    let want_play = !was_playing;
                    self.ui_playing = want_play;
                    let cp = self.current_pos();
                    self.clock_base = cp;
                    self.clock_start = Instant::now();
                    self.clock_playing = want_play;
                    self.override_play = Some((want_play, Instant::now() + Duration::from_millis(1200)));
                    // 把自维护时钟钉在当前显示位置：暂停时立即冻结，不会因 spos 采样滞后先回跳
                    self.spos = cp;
                    self.spos_at = Instant::now();
                    self.spos_valid = true;
                    // 记录“暂停请求待确认”，确认前不采纳仍在推进的播放器进度
                    self.pause_pending_at = if want_play { None } else { Some(Instant::now()) };
                }
                _ => {
                    self.ui_playing = true;
                    self.override_play = Some((true, Instant::now() + Duration::from_millis(1200)));
                    self.pause_pending_at = None;
                }
            }
            // 指令分发：汽水优先用 CDP 直接点它的播放器按钮；SMTC 不可用时用系统多媒体键兜底。
            let app_l = self.music.app_id.to_ascii_lowercase();
            let is_soda = app_l.contains("汽水") || app_l.contains("soda") || app_l.contains("qishui");
            let mut sent = false;
            if is_soda && crate::soda_patch::cdp_up() {
                let c = id.to_string();
                std::thread::spawn(move || {
                    let _ = crate::cdp_lyric::control(&c);
                });
                sent = true;
            }
            if !sent {
                if is_soda || !self.from_smtc {
                    let vk = match id {
                        "prev" => 0xB1u8, // VK_MEDIA_PREV_TRACK
                        "play" => 0xB3u8, // VK_MEDIA_PLAY_PAUSE
                        _ => 0xB0u8,      // VK_MEDIA_NEXT_TRACK
                    };
                    platform::media_key(vk);
                } else {
                    self.tracker.send(cmd);
                }
            }
        }

        // ---------- 歌词区（可上下拖动浏览；松手后自动回到当前进度） ----------
        let instrumental = lyrics::is_instrumental(&self.lyr_lines);
        let cur = if instrumental { None } else { self.cur_lyric_idx() };
        let lfont = self.cfg.lyric_font.clamp(10.0, 20.0);
        let line_font = egui::FontId::proportional(lfont);
        let row_h = 22.0f32;
        let lx0 = cx0 - disc_pad;
        // 歌词底部叠加一条「水波反射」频谱（仅叠加绘制，不改变歌词布局）
        let wf_h = 46.0f32;
        let wf_rect = egui::Rect::from_min_max(
            egui::pos2(lx0, rect.bottom() - wf_h),
            egui::pos2(cx1, rect.bottom()),
        );
        // 歌词区域完全沿用原有布局（位置、行数、滚动逻辑均不动）
        let vtop = rect.top() + CONTENT_PAD_T + 146.0 - 10.0 - 12.5;
        let vbot = rect.bottom() - CONTENT_PAD_B - 10.0 - 12.5;
        let lyric_rect = egui::Rect::from_min_max(egui::pos2(lx0, vtop), egui::pos2(cx1, vbot));
        let view_h = (vbot - vtop).max(row_h);
        let rows = ((view_h / row_h).floor() as i32).max(1);
        let grid_top = vtop + (view_h - rows as f32 * row_h) * 0.5;
        let anchor = (rows - 1) as f32 * 0.5;
        let center_y = grid_top + anchor * row_h;
        // 歌词底部：波纹频谱（水波反射）——先画在歌词之下，歌词完全不受影响。
        // 开关是纯插拔式：设置里关闭后，再次打开音乐面板时直接不绘制，不再先做一次渐隐。
        if self.cfg.water_spectrum {
            let wf_fill = egui::Rect::from_min_max(
                egui::pos2(wf_rect.left(), wf_rect.top()),
                egui::pos2(rect.right(), rect.bottom()),
            );
            let wf_r = self.cfg.radius.min(rect.height() * 0.5).min(rect.width() * 0.5);
            let wf_br = self.cfg.position.corner_radius(wf_r).se as f32;
            self.draw_water_reflection(painter, wf_rect, wf_fill, wf_br, green, a, t);
        }
        if let Some(ci) = cur {
            // 仅用鼠标滚轮滚动歌词列表；悬停时显示手指光标
            let resp = ui
                .interact(lyric_rect, ui.id().with("lyric_scroll"), egui::Sense::hover())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let n = self.lyr_lines.len();
            let min_off = -((n.saturating_sub(1) - ci) as f32) * row_h;
            let max_off = ci as f32 * row_h;
            let dt = ui.input(|i| i.stable_dt).clamp(0.001, 0.05);
            if resp.hovered() {
                let d = ui.input(|i| i.raw_scroll_delta.y);
                if d != 0.0 {
                    self.lyr_scroll_target += d;
                    self.lyr_release_at = Some(Instant::now());
                    self.lyr_return_start = None;
                }
            }
            self.lyr_scroll_target = self.lyr_scroll_target.clamp(min_off, max_off);

            // 停止滚动 2s 后自动回到当前句：用 0.25s 缓出动画快速回位
            if self.lyr_return_start.is_none() {
                if let Some(rel) = self.lyr_release_at {
                    if rel.elapsed().as_secs_f32() >= 2.0
                        && (self.lyr_drag_off.abs() > 0.5 || self.lyr_scroll_target.abs() > 0.5)
                    {
                        self.lyr_return_start = Some(Instant::now());
                        self.lyr_return_from = self.lyr_drag_off;
                        self.lyr_scroll_target = 0.0;
                    }
                }
            }
            if let Some(st) = self.lyr_return_start {
                let p = (st.elapsed().as_secs_f32() / 0.25).clamp(0.0, 1.0);
                let e = 1.0 - (1.0 - p).powi(3);
                self.lyr_drag_off = self.lyr_return_from * (1.0 - e);
                if p >= 1.0 {
                    self.lyr_drag_off = 0.0;
                    self.lyr_scroll_target = 0.0;
                    self.lyr_return_start = None;
                    self.lyr_release_at = None;
                }
            } else {
                // 滚轮输入的偏移平滑趋近目标：滚动过程本身也是动画
                let ks = 1.0 - (-dt / 0.05).exp();
                self.lyr_drag_off += (self.lyr_scroll_target - self.lyr_drag_off) * ks;
                if (self.lyr_scroll_target - self.lyr_drag_off).abs() < 0.05 {
                    self.lyr_drag_off = self.lyr_scroll_target;
                }
            }
            self.lyr_drag_off = self.lyr_drag_off.clamp(min_off, max_off);

            // 当前句索引也做平滑：换行时整列表缓缓上移，而不是瞬间跳一行
            if self.lyr_ci_anim < 0.0 {
                self.lyr_ci_anim = ci as f32;
            }
            let kc = 1.0 - (-dt / 0.075).exp();
            self.lyr_ci_anim += (ci as f32 - self.lyr_ci_anim) * kc;
            if (ci as f32 - self.lyr_ci_anim).abs() < 0.002 {
                self.lyr_ci_anim = ci as f32;
            }

            // 绘制歌词列表：位置用连续的 ci_anim + drag_off，滚动是平滑的
            let clip = painter.with_clip_rect(lyric_rect);
            for i in 0..n {
                let y = grid_top + (i as f32 - self.lyr_ci_anim + self.lyr_drag_off / row_h + anchor) * row_h;
                if y >= vbot - 0.5 {
                    break;
                }
                if y + row_h <= vtop + 0.5 {
                    continue;
                }
                let lrect = egui::Rect::from_min_size(egui::pos2(lx0, y), egui::vec2(cx1 - lx0, row_h));
                // 当前句的逐字渐绿/进度始终随歌正常渲染，不受滚动/回位动画影响
                let at_current = i == ci;
                if at_current {
                    let pr = self.line_progress(i);
                    let lt = (self.lyric_time() - self.lyr_lines[i].t).max(0.0);
                    let gdim = egui::Color32::from_rgba_unmultiplied(120, 170, 150, a);
                    let fill = if self.cfg.char_green { green } else { gdim };
                    let gdim = self.fade_lyric_color(gdim, y, y + row_h, lyric_rect);
                    let fill = self.fade_lyric_color(fill, y, y + row_h, lyric_rect);
                    self.draw_lyric_line(&clip, lrect, &self.lyr_lines[i].text, line_font.clone(), gdim, fill, pr, lt, false, false);
                } else {
                    let fc = self.fade_lyric_color(faint, y, y + row_h, lyric_rect);
                    // 非当前句的横向跑马灯也按“歌曲进度”驱动，而不是墙钟：暂停时自然停住，
                    // 不会出现暂停后长句仍在横向滚动（“歌词自己又滚起来”）。
                    let ilt = (self.lyric_time() - self.lyr_lines[i].t).max(0.0);
                    self.draw_lyric_line(&clip, lrect, &self.lyr_lines[i].text, line_font.clone(), fc, fc, 0.0, ilt, false, false);
                }
            }
        } else if instrumental {
            // 纯音乐：歌词区居中显示「纯音乐，请欣赏」（朴素纯灰，与汽水一致）。
            self.draw_instrumental_art(painter, lyric_rect, center_y, a);
        } else {
            let hint = if self.lyr_done { "暂无歌词" } else { "歌词加载中…" };
            painter.text(egui::pos2((lx0 + cx1) * 0.5, center_y), egui::Align2::CENTER_CENTER, hint, egui::FontId::proportional(13.0), egui::Color32::from_rgba_unmultiplied(120, 196, 160, a));
        }
    }

    // ---------------- 展开态外壳：左侧菜单 + 页面栈 ----------------
    /// 展开态外壳：左侧竖排模块菜单 + 右侧内容区（音乐底页 / 设置页 / 占位页，可层层覆盖）。
    fn draw_expanded_panel(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, pill: egui::Rect, e: f32, t: f64) {
        let a = (((e - 0.5) * 2.0) * 255.0).clamp(0.0, 255.0) as u8;
        let rail_w = 66.0f32;
        let muted = egui::Color32::from_rgba_unmultiplied(150, 158, 175, a);
        let accent = egui::Color32::from_rgba_unmultiplied(52, 211, 153, a);

        // ---- 左侧菜单栏背景（弱化：只比岛体略深的柔和底色，去掉硬分割线）----
        let rail = egui::Rect::from_min_max(pill.left_top(), egui::pos2(pill.left() + rail_w, pill.bottom()));
        let rail_bg = egui::Color32::from_rgba_unmultiplied(22, 23, 28, a);
        // 左侧菜单栏的左边缘两角必须跟随岛体当前停靠方向的实际圆角：
        // TopCenter 左上=直角/左下=圆角，LeftCenter 左侧贴边=直角，RightCenter 左侧为内侧=圆角。
        // 之前写死 nw:0 会在右吸附展开时把本该圆润的左上角画成直角。
        let rr_pill = self.cfg.radius.min(pill.height() * 0.5).min(pill.width() * 0.5);
        let cr = self.cfg.position.corner_radius(rr_pill);
        painter.rect_filled(
            rail,
            egui::CornerRadius { nw: cr.nw, ne: 0, sw: cr.sw, se: 0 },
            rail_bg,
        );

        let active = match self.active { Module::Music => 0usize, _ => 1usize };
        let items: [(&str, u8); 2] = [("音乐", 0), ("天气", 2)];
        let item_h = 48.0f32;
        let top_y = pill.top() + 34.0; // 让出顶部时间

        // 顶部时间（左侧菜单“音乐”上方）
        {
            let stt = platform::local_time();
            let mut ts = format!("{:02}:{:02}", stt.wHour, stt.wMinute);
            let fsz = if self.cfg.show_seconds { ts = format!("{}:{:02}", ts, stt.wSecond); 13.0 } else { 17.0 };
            painter.text(egui::pos2(pill.left() + rail_w * 0.5, pill.top() + 17.5), egui::Align2::CENTER_CENTER, &ts, bold_font(fsz), egui::Color32::from_rgba_unmultiplied(233, 236, 242, a));
        }

        // 选中项滑动动画（菜单名之间切换）
        if self.active != self.last_active {
            self.rail_from = match self.last_active { Module::Music => 0.0, _ => 1.0 };
            self.rail_anim_at = Some(std::time::Instant::now());
            self.last_active = self.active;
        }
        let mut rail_anim_done = false;
        let shown_idx = if let Some(at) = self.rail_anim_at {
            let p = (at.elapsed().as_secs_f32() / 0.26).clamp(0.0, 1.0);
            let ease = 1.0 - (1.0 - p).powi(3);
            if p >= 1.0 { rail_anim_done = true; }
            self.rail_from + (active as f32 - self.rail_from) * ease
        } else { active as f32 };

        // 搜索页从天气页打开时，左栏仍应高亮“天气”，而不是底部的“设置”。
        let settings_active = self.show_settings && !(self.w_city_search && self.w_city_search_from_weather);
        let eff_shown = if settings_active { -2.0 } else { shown_idx };
        // 顶部两个模块的选中高亮（停留在设置页时不显示）
        if !settings_active {
            let hl_y = top_y + shown_idx * item_h;
            let hl_rect = egui::Rect::from_min_size(egui::pos2(pill.left() + 9.0, hl_y), egui::vec2(rail_w - 18.0, item_h - 8.0));
            painter.rect_filled(hl_rect, 10.0, egui::Color32::from_rgba_unmultiplied(52, 211, 153, (a as f32 * 0.14) as u8));
        }

        let mut iy = top_y;
        for (idx, (label, icon_kind)) in items.iter().enumerate() {
            let item = egui::Rect::from_min_size(egui::pos2(pill.left() + 9.0, iy), egui::vec2(rail_w - 18.0, item_h - 8.0));
            let w = (1.0 - (idx as f32 - eff_shown).abs()).clamp(0.0, 1.0);
            let icol = lerp_color(muted, accent, w);
            let cx = item.center().x;
            draw_nav_icon(painter, *icon_kind, egui::pos2(cx, iy + 13.0), 18.0, icol);
            painter.text(egui::pos2(cx, iy + 31.5), egui::Align2::CENTER_CENTER, *label, egui::FontId::proportional(10.5), icol);
            let resp = ui.interact(item, ui.id().with(("rail", idx)), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                self.show_settings = false;
                self.w_city_search = false;
                self.w_city_search_from_weather = false;
                self.w_search_need_focus = false;
                self.w_search_focus_tries = 0;
                let ni = match idx { 0 => Module::Music, _ => Module::Weather };
                if ni != self.active {
                    self.rail_from = match self.active { Module::Music => 0.0, _ => 1.0 };
                    self.rail_anim_at = Some(std::time::Instant::now());
                    self.active = ni;
                    self.last_active = ni;
                }
            }
            iy += item_h;
        }
        if rail_anim_done { self.rail_anim_at = None; }

        // 底部「设置」入口（所有模块的公共设置都在这里�?
        {
            let set_y = pill.bottom() - item_h;
            let item = egui::Rect::from_min_size(egui::pos2(pill.left() + 9.0, set_y), egui::vec2(rail_w - 18.0, item_h - 8.0));
            if settings_active {
                painter.rect_filled(item, 10.0, egui::Color32::from_rgba_unmultiplied(52, 211, 153, (a as f32 * 0.14) as u8));
            }
            let icol = if settings_active { accent } else { muted };
            let cx = item.center().x;
            painter.text(egui::pos2(cx, set_y + 13.0), egui::Align2::CENTER_CENTER, "\u{E713}", icon_font(17.0), icol);
            painter.text(egui::pos2(cx, set_y + 31.5), egui::Align2::CENTER_CENTER, "设置", egui::FontId::proportional(10.5), icol);
            let resp = ui.interact(item, ui.id().with("rail_settings"), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                // 从搜索页点“设置”：先关闭搜索遮罩，露出设置页。
                if self.w_city_search {
                    self.w_city_search = false;
                    self.w_city_search_from_weather = false;
                    self.w_search_need_focus = false;
                    self.w_search_focus_tries = 0;
                }
                if !self.show_settings {
                    self.settings_scroll = 0.0;
                    self.settings_scroll_target = 0.0;
                    self.settings_scroll_act = std::time::Instant::now() - std::time::Duration::from_secs(10);
                    self.show_settings = true;
                    self.mark_upd_seen_rail();
                    if self.upd_ver.is_none() && !self.upd_checking {
                        self.start_update_check();
                    }
                }
            }
            // 有新版本：设置菜单右上角一个圆点角标，提示里面有更新（看过一次后不再出现）
            if self.show_rail_update() {
                let bc = egui::pos2(item.right() - 8.0, item.top() + 8.0);
                painter.circle_filled(bc, 4.8, egui::Color32::from_rgba_unmultiplied(18, 20, 25, a));
                painter.circle_filled(bc, 3.0, accent);
            }
        }

        // 内容区过渡进度：与左侧菜单选中动画共用同一时钟
        let (switch_p, switch_dir) = if let Some(at) = self.rail_anim_at {
            let p = (at.elapsed().as_secs_f32() / 0.26).clamp(0.0, 1.0);
            let new_idx = match self.active { Module::Music => 0.0f32, _ => 1.0f32 };
            (p, (new_idx - self.rail_from).signum())
        } else {
            (1.0f32, 0.0f32)
        };

        // ---- 右侧内容�?----
        let content = egui::Rect::from_min_max(
            egui::pos2(pill.left() + rail_w, pill.top()),
            egui::pos2(pill.right(), pill.bottom()),
        );
        let svis = ui.ctx().animate_bool_with_time(egui::Id::new("settings_vis"), self.show_settings, 0.22);
        let switch_ease = 1.0 - (1.0 - switch_p).powi(3);
        let dx = switch_dir * (1.0 - switch_ease) * 26.0;
        let cslide = egui::Rect::from_min_max(
            egui::pos2(content.left() + dx, content.top()),
            egui::pos2(content.right() + dx, content.bottom()),
        );
        let cclip = painter.with_clip_rect(content);
        if svis < 0.999 {
            match self.active {
                Module::Music => {
                    if self.music_amt > 0.5 {
                        self.draw_music_expanded(ui, &cclip, cslide, e, t);
                    } else {
                        let st = platform::local_time();
                        if self.cfg.show_date {
                            let wd = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"][(st.wDayOfWeek as usize).min(6)];
                            let date = format!("{:04}-{:02}-{:02} {}", st.wYear, st.wMonth, st.wDay, wd);
                            cclip.text(egui::pos2(cslide.right() - 16.0, cslide.top() + 16.0), egui::Align2::RIGHT_TOP, &date, egui::FontId::proportional(13.0), muted);
                        }
                        self.draw_expanded_idle(&cclip, cslide, e);
                    }
                }
                Module::Schedule => self.draw_placeholder_page(&cclip, cslide, e, "日程", "日程模块即将上线"),
                Module::Weather => self.draw_weather_page(ui, &cclip, cslide, e),
            }
        }
        if self.show_settings || svis > 0.001 {
            self.draw_settings_page(ui, painter, content, e, svis);
        }        // 模块切换过渡：用岛体底色做淡入遮罩，让新页面“浮现�?
        if switch_p < 0.999 {
            let ma = ((1.0 - switch_p).powf(1.5) * self.cfg.alpha as f32) as u8;
            let mask = egui::Color32::from_rgba_unmultiplied(self.cfg.bg[0], self.cfg.bg[1], self.cfg.bg[2], ma);
            let rr = self.cfg.radius.min(content.height() * 0.5);
            painter.with_clip_rect(content).rect_filled(content, rr, mask);
        }
    }

    /// 独立模块占位页（日程 / 天气）：标题 + “敬请期待”。各模块独立，无需“返回音乐”。
    fn draw_placeholder_page(&self, painter: &egui::Painter, content: egui::Rect, e: f32, title: &str, sub: &str) {
        let a = (((e - 0.5) * 2.0) * 255.0).clamp(0.0, 255.0) as u8;
        let ink = egui::Color32::from_rgba_unmultiplied(233, 236, 242, a);
        let muted = egui::Color32::from_rgba_unmultiplied(150, 158, 175, a);
        let muted2 = egui::Color32::from_rgba_unmultiplied(107, 113, 128, a);
        painter.text(egui::pos2(content.left() + CONTENT_PAD_X, content.top() + CONTENT_PAD_T), egui::Align2::LEFT_TOP, title, bold_font(17.0), ink);
        painter.text(egui::pos2(content.center().x, content.center().y - 6.0), egui::Align2::CENTER_CENTER, sub, egui::FontId::proportional(13.0), muted);
        painter.text(egui::pos2(content.center().x, content.center().y + 18.0), egui::Align2::CENTER_CENTER, "敬请期待", egui::FontId::proportional(11.0), muted2);
    }

    /// 天气页：城市�?+ 刷新、hero（动态图�?大温�?最高最�?AQI）、未�?7 天温度区间条、详情�?
    /// 数据全部来自 `self.w_data`（无 key，itboy 主源 + 本地缓存兜底）�?
    fn draw_weather_page(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, rect: egui::Rect, e: f32) {
        let a = (((e - 0.5) * 2.0) * 255.0).clamp(0.0, 255.0) as u8;
        let ink = egui::Color32::from_rgba_unmultiplied(233, 236, 242, a);
        let muted = egui::Color32::from_rgba_unmultiplied(150, 158, 175, a);
        let muted2 = egui::Color32::from_rgba_unmultiplied(107, 113, 128, a);
        let accent = egui::Color32::from_rgba_unmultiplied(52, 211, 153, a);
        let bg_col = egui::Color32::from_rgba_unmultiplied(self.cfg.bg[0], self.cfg.bg[1], self.cfg.bg[2], 255);

        let x0 = rect.left() + CONTENT_PAD_X;
        let x1 = rect.right() - CONTENT_PAD_X;
        let y0 = rect.top() + CONTENT_PAD_T;

        // 城市标题（固定显示当前位置，不提供搜索/切换入口）
        let city = match &self.w_data {
            Some(w) if !w.city.is_empty() => w.city.clone(),
            _ => {
                if !self.cfg.weather_city_name.is_empty() {
                    self.cfg.weather_city_name.clone()
                } else if self.cfg.weather_city.is_empty() {
                    "定位中…".to_string()
                } else {
                    "天气".to_string()
                }
            }
        };
        painter.text(egui::pos2(x0, y0 + 8.0), egui::Align2::LEFT_CENTER, &city, bold_font(16.0), ink);

        // 更新时间 + 刷新按钮（右侧）
        let refresh_hit = egui::Rect::from_min_max(egui::pos2(x1 - 80.0, y0 - 3.0), egui::pos2(x1, y0 + 21.0));
        let refresh_resp = ui
            .interact(refresh_hit, ui.id().with("w_refresh"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if let Some(w) = &self.w_data {
            let local = if self.w_fetch_label.is_empty() { "—" } else { self.w_fetch_label.as_str() };
            let src = if w.updated.is_empty() { "—" } else { w.updated.as_str() };
            refresh_resp
                .clone()
                .on_hover_text(format!("本地拉取 {}｜数据源更新 {}", local, src));
        }
        {
            let upd = self.w_data.as_ref().map(|w| w.updated.clone()).unwrap_or_default();
            let notice_on = self.w_notice_until.map(|t| t > Instant::now()).unwrap_or(false);
            if self.w_inflight {
                painter.text(
                    egui::pos2(x1 - 24.0, y0 + 8.0),
                    egui::Align2::RIGHT_CENTER,
                    "更新中…",
                    egui::FontId::proportional(10.5),
                    accent,
                );
            } else if notice_on {
                let col = if self.w_notice_err {
                    egui::Color32::from_rgba_unmultiplied(242, 110, 92, a)
                } else {
                    accent
                };
                painter.text(
                    egui::pos2(x1 - 24.0, y0 + 8.0),
                    egui::Align2::RIGHT_CENTER,
                    &self.w_notice,
                    egui::FontId::proportional(10.5),
                    col,
                );
                if self.w_notice_err {
                    if let Some(err) = self.w_err.clone() {
                        refresh_resp.clone().on_hover_text(err);
                    }
                }
            } else {
                // 迷你?
                // 迷你?
                let label = if !self.w_fetch_label.is_empty() {
                    self.w_fetch_label.clone()
                } else {
                    upd.clone()
                };
                if !label.is_empty() {
                    let text = if self.w_fetch_label == "缓存" {
                        "缓存数据".to_string()
                    } else {
                        format!("{} 更新", label)
                    };
                    painter.text(
                        egui::pos2(x1 - 24.0, y0 + 8.0),
                        egui::Align2::RIGHT_CENTER,
                        text,
                        egui::FontId::proportional(10.5),
                        muted2,
                    );
                }
            }
            if self.w_inflight {
                // 迷你?
                let c = egui::pos2(x1 - 6.0, y0 + 8.0);
                let r = 5.5;
                let spin_t = ui.input(|i| i.time) as f32;
                let start = spin_t * 5.0;
                let steps = 16;
                let pts: Vec<egui::Pos2> = (0..=steps)
                    .map(|i| {
                        let ang = start + (i as f32 / steps as f32) * std::f32::consts::TAU * 0.72;
                        egui::pos2(c.x + ang.cos() * r, c.y + ang.sin() * r)
                    })
                    .collect();
                painter.add(egui::Shape::line(pts, egui::Stroke::new(1.6, accent)));
            } else {
                painter.text(
                    egui::pos2(x1 - 4.0, y0 + 8.0),
                    egui::Align2::RIGHT_CENTER,
                    "\u{21bb}",
                    egui::FontId::proportional(14.0),
                    muted,
                );
            }
        }

        // 无数据：加载�?/ 失败提示
        let Some(w) = self.w_data.clone() else {
            let title = if self.w_err.is_some() { "天气加载失败" } else { "天气加载中…" };
            painter.text(
                egui::pos2(rect.center().x, rect.center().y - 8.0),
                egui::Align2::CENTER_CENTER,
                title,
                egui::FontId::proportional(13.0),
                muted,
            );
            if let Some(err) = &self.w_err {
                let g = ui.fonts(|f| {
                    f.layout(err.clone(), egui::FontId::proportional(10.5), muted2, (x1 - x0).max(120.0))
                });
                let eh = g.size().y;
                painter.galley(
                    egui::pos2(rect.center().x - g.size().x * 0.5, rect.center().y + 10.0),
                    g,
                    muted2,
                );
                painter.text(
                    egui::pos2(rect.center().x, rect.center().y + 14.0 + eh),
                    egui::Align2::CENTER_CENTER,
                    "点击右上角 ↻ 重试",
                    egui::FontId::proportional(10.0),
                    muted2,
                );
            }
            if refresh_resp.clicked() {
                self.start_weather_fetch();
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
            return;
        };

        let f = self.cfg.weather_unit == 1;
        let unit = weather::temp_unit(f);
        let temp = weather::conv_temp(w.temp, f);
        let (hi, lo) = w.today_high_low();
        let st = platform::local_time();
        let is_night = st.wHour >= 18 || st.wHour < 6;
        let sky = weather::sky_of(&w.today_type, is_night);
        let now_t = ui.input(|i| i.time);

        // ---- Hero（紧凑版：图�?温度不再过大�?---
        let hero_top = y0 + 26.0;
        draw_weather_icon(painter, sky, egui::pos2(x0 + 26.0, hero_top + 22.0), 46.0, a, now_t, bg_col);
        painter.text(egui::pos2(x0 + 58.0, hero_top + 12.0), egui::Align2::LEFT_CENTER, format!("{:.0}{}", temp, unit), bold_font(30.0), ink);
        painter.text(egui::pos2(x0 + 60.0, hero_top + 34.0), egui::Align2::LEFT_CENTER, &w.today_type, egui::FontId::proportional(12.5), muted);
        painter.text(
            egui::pos2(x0 + 60.0, hero_top + 51.0),
            egui::Align2::LEFT_CENTER,
            format!("最高 {:.0}°  最低 {:.0}°", weather::conv_temp(hi as f32, f), weather::conv_temp(lo as f32, f)),
            egui::FontId::proportional(11.0),
            muted2,
        );
        // AQI / 湿度（右上信息块�?
        {
            let q = if w.quality.is_empty() { "—".to_string() } else { w.quality.clone() };
            painter.text(egui::pos2(x1, hero_top - 2.0), egui::Align2::RIGHT_TOP, "空气质量", egui::FontId::proportional(9.5), muted2);
            painter.text(egui::pos2(x1, hero_top + 11.0), egui::Align2::RIGHT_TOP, &q, bold_font(14.0), aqi_color(&w.quality, a));
            let pm = if w.pm25 > 0.0 { format!("PM2.5 {:.0}", w.pm25) } else { "PM2.5 —".to_string() };
            painter.text(egui::pos2(x1, hero_top + 30.0), egui::Align2::RIGHT_TOP, pm, egui::FontId::proportional(10.0), muted2);
            let hum = if w.humidity.is_empty() { "—".to_string() } else { w.humidity.clone() };
            painter.text(egui::pos2(x1, hero_top + 44.0), egui::Align2::RIGHT_TOP, format!("湿度 {}", hum), egui::FontId::proportional(10.0), muted2);
        }

        // 分隔�?
        let div_y = hero_top + 63.0;
        painter.line_segment([egui::pos2(x0, div_y), egui::pos2(x1, div_y)], egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, (a as f32 * 0.06) as u8)));

        // ---- 可滚动区�? 天列�?+ 详情�?---
        let list_top = div_y + 8.0;
        let list_bottom = rect.bottom() - CONTENT_PAD_B;
        let view_h = (list_bottom - list_top).max(1.0);
        let days = &w.days;
        let nshow = days.len().min(7);
        let row_h = 24.0f32;
        let title_h = 15.0f32;
        let days_h = title_h + nshow as f32 * row_h;
        let hourly_h = if w.hours.is_empty() { 0.0 } else { 62.0 };
        // 感冒提示按宽度自动换行，先量出高度，保证滚动区把整段文字算进去（不会被底部裁掉）
        let ganmao_galley = if w.ganmao.trim().is_empty() {
            None
        } else {
            Some(ui.fonts(|fnt| {
                fnt.layout(w.ganmao.clone(), egui::FontId::proportional(11.0), muted, (x1 - x0).max(1.0))
            }))
        };
        let ganmao_h = ganmao_galley.as_ref().map(|g| g.size().y).unwrap_or(0.0);
        let detail_h = 80.0 + ganmao_h + 4.0;
        let total_h = hourly_h + days_h + 8.0 + detail_h;
        let max_scroll = (total_h - view_h).max(0.0);
        let mut w_scroll_activity = false;
        // 调试：WINISLE_W_SCROLL=<px> 直接把天气页滚到该偏移（用于验证“滚动时滚动条出现”）
        if let Ok(v) = std::env::var("WINISLE_W_SCROLL") {
            if let Ok(px) = v.parse::<f32>() {
                self.w_scroll_target = px;
                self.w_scroll = px;
                self.w_scroll_act = std::time::Instant::now();
                w_scroll_activity = true;
            }
        }
        // 调试：WINISLE_W_SCROLL_ONCE 仅在首帧注入一次滚动，之后停止（用于验证“停止后自动淡出”）
        if std::env::var("WINISLE_W_SCROLL_ONCE").is_ok() {
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                self.w_scroll_target = 90.0;
                self.w_scroll = 90.0;
                self.w_scroll_act = std::time::Instant::now();
                w_scroll_activity = true;
            }
        }
        if let Some(ph) = ui.ctx().pointer_hover_pos() {
            if rect.contains(ph) && max_scroll > 0.0 {
                let d = ui.input(|i| i.raw_scroll_delta.y);
                if d.abs() > 0.01 {
                    self.w_scroll_target -= d;
                    w_scroll_activity = true;
                }
            }
        }
        self.w_scroll_target = self.w_scroll_target.clamp(0.0, max_scroll);
        let dt = ui.input(|i| i.stable_dt).clamp(0.001, 0.05);
        let k = 1.0 - (-dt / 0.07).exp();
        self.w_scroll += (self.w_scroll_target - self.w_scroll) * k;
        self.w_scroll = self.w_scroll.clamp(0.0, max_scroll);
        let w_settling = (self.w_scroll_target - self.w_scroll).abs() > 0.5;
        if (self.w_scroll_target - self.w_scroll).abs() > 0.25 {
            ui.ctx().request_repaint();
        }
        // 滚动条自动隐藏：仅在滚动 / 惯性收敛中 / 停止后 1.2 秒内显示，随后淡出
        if w_scroll_activity {
            self.w_scroll_act = std::time::Instant::now();
        }
        let w_since = self.w_scroll_act.elapsed().as_secs_f32();
        let w_want_vis = if max_scroll > 0.0 && (w_scroll_activity || w_settling || w_since < 1.2) {
            1.0
        } else {
            0.0
        };
        let wkv = 1.0 - (-dt / 0.12).exp();
        self.w_scroll_vis += (w_want_vis - self.w_scroll_vis) * wkv;
        if self.w_scroll_vis < 0.005 {
            self.w_scroll_vis = 0.0;
        } else if self.w_scroll_vis > 0.995 {
            self.w_scroll_vis = 1.0;
        }
        if (self.w_scroll_vis - w_want_vis).abs() > 0.005 {
            ui.ctx().request_repaint();
        }
        if std::env::var("WINISLE_WDBG").is_ok() {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("D:\\LingDao\\_wdebug.txt") {
                let _ = writeln!(f, "max={:.1} scroll={:.1} clamp={} want={:.0} vis={:.3} rect=({:.0},{:.0},{:.0},{:.0})", max_scroll, self.w_scroll, self.w_scroll_target, w_want_vis, self.w_scroll_vis, rect.left(), rect.top(), rect.right(), rect.bottom());
            }
        }
        let list_rect = egui::Rect::from_min_max(egui::pos2(rect.left(), list_top), egui::pos2(rect.right(), list_bottom));
        let lp = painter.with_clip_rect(list_rect);
        let p: &egui::Painter = &lp;
        let top = list_top - self.w_scroll;

        // ---- 未来 24 小时�? 小时间隔，来�?7timer�?---
        if !w.hours.is_empty() {
            p.text(egui::pos2(x0, top + 6.0), egui::Align2::LEFT_CENTER, "未来 24 小时", egui::FontId::proportional(11.0), muted2);
            let hn = w.hours.len().min(8);
            let cw = (x1 - x0) / hn as f32;
            for (i, h) in w.hours.iter().take(hn).enumerate() {
                let cx = x0 + cw * (i as f32 + 0.5);
                p.text(egui::pos2(cx, top + 24.0), egui::Align2::CENTER_CENTER, &h.label, egui::FontId::proportional(9.5), muted2);
                draw_weather_icon(p, h.sky, egui::pos2(cx, top + 38.0), 14.0, a, now_t, bg_col);
                p.text(egui::pos2(cx, top + 51.0), egui::Align2::CENTER_CENTER, format!("{:.0}°", weather::conv_temp(h.temp as f32, f)), egui::FontId::proportional(10.5), ink);
            }
        }
        let base = top + hourly_h;

        p.text(egui::pos2(x0, base + 6.0), egui::Align2::LEFT_CENTER, format!("未来 {} 天", w.days.len()), egui::FontId::proportional(11.0), muted2);

        // 温度区间归一�?
        let mut mn = i32::MAX;
        let mut mx = i32::MIN;
        for d in days.iter().take(nshow) {
            mn = mn.min(d.low);
            mx = mx.max(d.high);
        }
        if nshow == 0 {
            mn = 0;
            mx = 1;
        }
        let span = (mx - mn).max(1) as f32;
        let bar_x0 = x0 + 150.0;
        let bar_x1 = x1 - 96.0;
        for (i, d) in days.iter().take(nshow).enumerate() {
            let ry = base + title_h + i as f32 * row_h + row_h * 0.5;
            if ry < list_top - row_h || ry > list_bottom + row_h {
                continue;
            }
            let label = if i == 0 { "今天".to_string() } else { d.week.replace("星期", "周") };
            p.text(egui::pos2(x0, ry), egui::Align2::LEFT_CENTER, &label, egui::FontId::proportional(12.0), if i == 0 { ink } else { muted });
            let mmdd = if d.date.len() >= 10 { d.date[5..].replace('-', "/") } else { d.date.clone() };
            p.text(egui::pos2(x0 + 34.0, ry), egui::Align2::LEFT_CENTER, &mmdd, egui::FontId::proportional(9.5), muted2);
            draw_weather_icon(p, weather::sky_of(&d.day_type, false), egui::pos2(x0 + 94.0, ry), 18.0, a, now_t, bg_col);
            p.text(egui::pos2(x0 + 112.0, ry), egui::Align2::LEFT_CENTER, &d.day_type, egui::FontId::proportional(11.0), muted);
            // 区间�?
            p.line_segment([egui::pos2(bar_x0, ry), egui::pos2(bar_x1, ry)], egui::Stroke::new(3.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, (a as f32 * 0.08) as u8)));
            let t_low = (d.low - mn) as f32 / span;
            let t_high = (d.high - mn) as f32 / span;
            let sx = bar_x0 + (bar_x1 - bar_x0) * t_low;
            let ex = bar_x0 + (bar_x1 - bar_x0) * t_high;
            p.line_segment([egui::pos2(sx, ry), egui::pos2(ex, ry)], egui::Stroke::new(3.0, accent));
            p.text(
                egui::pos2(x1, ry),
                egui::Align2::RIGHT_CENTER,
                format!("{:.0}° / {:.0}°", weather::conv_temp(d.high as f32, f), weather::conv_temp(d.low as f32, f)),
                egui::FontId::proportional(11.5),
                ink,
            );
        }

        // ---- 详情 ----
        let dy = base + days_h + 8.0;
        let wind = w.today().map(|d| format!("{} {}", d.wind_dir, d.wind_level)).unwrap_or_default();
        let sun = w.today().map(|d| format!("{} / {}", d.sunrise, d.sunset)).unwrap_or_default();
        let items = [
            ("风向风力", wind.as_str()),
            ("日出 / 日落", sun.as_str()),
            ("空气质量", if w.quality.is_empty() { "—" } else { w.quality.as_str() }),
            ("PM2.5", ""),
        ];
        // 详情分两列。排版遵循“就近成组”：标题与其数值贴近（组内 ~14pt），
        // 而上一组数值与下一组标题之间留更大间距（组�?~20pt），整体节奏协调、不忽远忽近�?
        let col_w = (x1 - x0) / 2.0;
        for (idx, (k2, v2)) in items.iter().enumerate() {
            let cx = x0 + (idx % 2) as f32 * col_w;
            let cy = dy + (idx / 2) as f32 * 34.0;
            p.text(egui::pos2(cx, cy + 4.0), egui::Align2::LEFT_CENTER, *k2, egui::FontId::proportional(10.0), muted2);
            let disp = if *k2 == "PM2.5" {
                if w.pm25 > 0.0 { format!("{:.0}", w.pm25) } else { "—".to_string() }
            } else {
                v2.to_string()
            };
            p.text(egui::pos2(cx, cy + 18.0), egui::Align2::LEFT_CENTER, disp, egui::FontId::proportional(12.0), ink);
        }
        // 感冒提示：作为独立一组，标题与正文贴近（组内 ~14pt），与上方内容留出组间间�?
        let gy = dy + 72.0;
        p.text(egui::pos2(x0, gy), egui::Align2::LEFT_CENTER, "感冒提示", egui::FontId::proportional(10.0), muted2);
        if let Some(g) = &ganmao_galley {
            p.galley(egui::pos2(x0, gy + 8.0), g.clone(), muted);
        }

        // 数据来源 / 缓存提示
        if w.from_cache {
            p.text(egui::pos2(x1, list_bottom - 2.0), egui::Align2::RIGHT_BOTTOM, "离线数据", egui::FontId::proportional(9.0), muted2);
        }

        // 滚动条（滚动时才淡入，停止约 1.2 秒后自动淡出；细 2px 样式）
        if max_scroll > 4.0 && self.w_scroll_vis > 0.01 {
            let sv = self.w_scroll_vis;
            let tx = rect.right() - 4.0;
            let track = egui::Rect::from_min_max(egui::pos2(tx, list_top + 2.0), egui::pos2(tx + 2.0, list_bottom - 2.0));
            painter.rect_filled(track, 1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, (12.0 * sv) as u8));
            let th = ((view_h / total_h.max(1.0)) * track.height()).max(14.0);
            let range = (track.height() - th).max(1.0);
            let tp = (self.w_scroll / max_scroll.max(1.0)) * range;
            let thumb = egui::Rect::from_min_size(egui::pos2(tx, track.top() + tp), egui::vec2(2.0, th));
            painter.rect_filled(thumb, 1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, (60.0 * sv) as u8));
        }

        // 刷新点击热区
        if refresh_resp.clicked() {
            self.start_weather_fetch();
        }
        // 图标为程序化动画，按需重绘
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }

    /// 右下角齿轮按钮（现已改为左侧菜单底部「设置」入口，保留备用）。
    #[allow(dead_code)]
    fn draw_gear(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, content: egui::Rect, e: f32, vis: f32) {
        let vis = vis.clamp(0.0, 1.0);
        let a = ((((e - 0.5) * 2.0) * 255.0).clamp(0.0, 255.0) * vis) as u8;
        let c = egui::pos2(content.right() - CONTENT_PAD_X - 12.5, content.top() + CONTENT_PAD_T + 12.5);
        let hit = egui::Rect::from_center_size(c, egui::vec2(30.0, 30.0));
        let resp = ui.interact(hit, ui.id().with("gear"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        // 与返回按钮一致：无底色、无描边，只留齿轮图标，悬停不高亮
        let col = egui::Color32::from_rgba_unmultiplied(170, 178, 195, a);
        painter.text(c, egui::Align2::CENTER_CENTER, "\u{E713}", icon_font(15.0), col); // Settings 齿轮
        if vis > 0.5 && resp.clicked() { self.settings_scroll = 0.0; self.settings_scroll_target = 0.0; self.settings_scroll_act = std::time::Instant::now() - std::time::Duration::from_secs(10); self.show_settings = true; self.mark_upd_seen_rail(); }
    }

    /// 后台检查更新（不阻塞 UI）。
    fn start_update_check(&mut self) {
        if self.upd_checking || self.upd_busy {
            return;
        }
        // 当天已确认是最新版本时不再重复检查（第二天恢复）
        if self.cfg.upd_checked_day == update::today_local() && self.upd_ver.is_none() {
            return;
        }
        self.upd_checking = true;
        self.upd_msg = "检查中…".to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(update::check());
        });
        self.upd_rx = Some(rx);
    }

    /// 后台下载更新包（带进度），完成后自动启动静默安装器。
    fn start_update_install(&mut self) {
        use update::Manifest;
        let m: Manifest = match &self.upd_ver {
            Some(m) => m.clone(),
            None => return,
        };
        if self.upd_busy {
            return;
        }
        self.upd_busy = true;
        self.upd_dl_pct = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
        self.upd_msg = format!("正在下载 v{}…", m.version);
        let pct = self.upd_dl_pct.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        std::thread::spawn(move || {
            let r = update::download_setup_with_progress(&m, pct.clone()).and_then(|p| {
                // 迷你?
                std::thread::sleep(Duration::from_millis(700));
                // 迷你?
                let mut cfg = Config::load();
                cfg.upd_checked_day = update::today_local();
                cfg.save();
                update::run_setup(&p)
            });
            let _ = tx.send(r);
        });
        self.upd_inst_rx = Some(rx);
    }

    /// 主循环里轮询检查结果（每帧调用，非阻塞）。
    fn poll_update(&mut self) {
        let got = self
            .upd_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok());
        if let Some(r) = got {
            self.upd_rx = None;
            self.upd_checking = false;
            self.upd_last_check = Instant::now();
            match r {
                Ok(Some(m)) => {
                    self.upd_msg = format!("发现新版本 v{}", m.version);
                    self.upd_ver = Some(m);
                    // 如果此刻已经在设置页里，视为“设置已看过”，收起�?设置角标不再出现
                    if self.show_settings {
                        self.mark_upd_seen_rail();
                    }
                }
                Ok(None) => {
                    self.upd_ver = None;
                    self.upd_msg = "已是最新版本".to_string();
                    self.cfg.upd_checked_day = update::today_local();
                    self.cfg.save();
                }
                Err(e) => {
                    self.upd_msg = format!("检查失败：{}", e);
                }
            }
        }
        if let Some(rx) = self.upd_inst_rx.as_ref() {
            if let Ok(r) = rx.try_recv() {
                self.upd_inst_rx = None;
                self.upd_busy = false;
                match r {
                    Ok(()) => self.upd_msg = "安装程序已启动…".to_string(),
                    Err(e) => self.upd_msg = format!("更新失败：{}", e),
                }
            }
        }
        // 启动后 5s 首查，之后每 4 小时；当天已确认最新则不再自动查
        if self.upd_rx.is_none()
            && !self.upd_checking
            && self.upd_last_check.elapsed() > Duration::from_secs(4 * 3600)
            && self.cfg.upd_checked_day != update::today_local()
        {
            self.start_update_check();
        }
    }
    /// 启动一次天气后台拉取（非阻塞，不卡 UI）�?
    fn start_weather_fetch(&mut self) {
        if self.w_inflight {
            return;
        }
        self.w_inflight = true;
        self.w_err = None;
        self.w_notice_until = None;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // 固定跟随当前位置：每次拉取前都重新定位，失败兜底北京
            let real = weather::locate()
                .map(|(_, k)| k)
                .unwrap_or_else(|| "101010100".to_string());
            let _ = tx.send(weather::fetch(&real));
        });
        self.w_rx = Some(rx);
    }

    /// 每帧轮询天气拉取结果，并按节流自动刷新（成功 30 分钟一次，失败 60 秒重试）�?
    fn poll_weather(&mut self) {
        if let Some(rx) = self.w_rx.as_ref() {
            if let Ok(r) = rx.try_recv() {
                self.w_rx = None;
                self.w_inflight = false;
                self.w_last_fetch = Instant::now();
                match r {
                    Ok(w) => {
                        // 天气固定跟随当前位置，始终采纳最近一次定位结果
                        {
                            let mut w = w;
                            if w.city.is_empty() {
                                w.city = if !self.cfg.weather_city_name.is_empty() {
                                    self.cfg.weather_city_name.clone()
                                } else {
                                    "当前位置".to_string()
                                };
                            }
                            self.w_data = Some(w);
                            self.w_err = None;
                            let lt = platform::local_time();
                            self.w_fetch_label = format!("{:02}:{:02}", lt.wHour, lt.wMinute);
                            self.w_notice = "✓ 已更新".to_string();
                            self.w_notice_err = false;
                            self.w_notice_until = Some(Instant::now() + Duration::from_secs(4));
                        }
                    }
                    Err(e) => {
                        self.w_err = Some(e);
                        self.w_notice = "更新失败".to_string();
                        self.w_notice_err = true;
                        self.w_notice_until = Some(Instant::now() + Duration::from_secs(6));
                        if self.w_data.is_none() {
                            self.w_data = weather::load_cache(&self.cfg.weather_city);
                            if self.w_data.is_some() {
                                self.w_fetch_label = "缓存".to_string();
                            }
                        }
                    }
                }
            }
        }
        let due = if self.w_err.is_some() {
            Duration::from_secs(60)
        } else {
            Duration::from_secs(1800)
        };
        if self.w_rx.is_none() && !self.w_inflight && self.w_last_fetch.elapsed() > due {
            self.start_weather_fetch();
        }
    }

    /// 切换城市：换 key / 名称、落盘、清空旧数据并立即触发拉取。key 为空=自动定位�?
    fn set_weather_city(&mut self, key: &str, name: &str) {
        if self.cfg.weather_city == key && self.cfg.weather_city_name == name {
            return;
        }
        self.cfg.weather_city = key.to_string();
        self.cfg.weather_city_name = name.to_string();
        self.cfg.save();
        self.w_data = weather::load_cache(key);
        self.w_fetch_label = if self.w_data.is_some() { "缓存".to_string() } else { String::new() };
        self.w_err = None;
        self.w_scroll = 0.0;
        self.w_scroll_target = 0.0;
        self.w_last_fetch = Instant::now() - Duration::from_secs(3600);
        self.start_weather_fetch();
    }

    /// 城市搜索：起后台线程拉取候选（防抖�?poll_city_search 控制）�?
    fn start_city_search(&mut self) {
        let q = self.w_search_query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.w_search_inflight = true;
        self.w_search_last_query = q.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(weather::search_cities(&q));
        });
        self.w_search_rx = Some(rx);
    }

    /// 每帧轮询搜索结果；输入停顿约 320ms 后自动发起查询�?
    fn poll_city_search(&mut self) {
        if let Some(rx) = self.w_search_rx.as_ref() {
            if let Ok(list) = rx.try_recv() {
                self.w_search_rx = None;
                self.w_search_inflight = false;
                self.w_search_results = list;
                self.w_search_results_at = Some(Instant::now());
                // 仅调试：用 WINISLE_SEARCH_PICK=城市名或代码 直接选中结果，避免真实鼠标点击。
                if let Ok(pick) = std::env::var("WINISLE_SEARCH_PICK") {
                    if let Some(hit) = self
                        .w_search_results
                        .iter()
                        .find(|h| h.key == pick || h.name == pick)
                        .cloned()
                    {
                        self.set_weather_city(&hit.key, &hit.name);
                        self.close_city_search();
                        self.w_search_results.clear();
                    }
                }
            }
        }
        if !self.w_city_search {
            return;
        }
        let q = self.w_search_query.trim().to_string();
        if q.is_empty() {
            return;
        }
        if !self.w_search_inflight
            && q != self.w_search_last_query
            && self.w_search_last_edit.elapsed() >= Duration::from_millis(320)
        {
            self.start_city_search();
        }
    }
    /// 打开城市搜索页：记录来源，返回时才知道该回天气页还是设置页�?
    fn open_city_search(&mut self, from_weather: bool) {
        self.show_settings = true;
        self.w_city_search = true;
        self.w_city_search_from_weather = from_weather;
        self.w_search_need_focus = true;
        self.w_search_focus_tries = 5;
        self.w_search_query.clear();
        self.w_search_results.clear();
        self.w_search_results_at = None;
        self.w_search_last_query.clear();
        self.settings_scroll = 0.0;
        self.settings_scroll_target = 0.0;
    }

    fn close_city_search(&mut self) {
        self.w_city_search = false;
        if self.w_city_search_from_weather {
            self.show_settings = false;
        }
        self.w_city_search_from_weather = false;
        self.w_search_need_focus = false;
        self.w_search_focus_tries = 0;
    }

    /// 城市搜索子页（覆盖设置页内容区）：输入城�?区县，选中即切换；也可恢复自动定位�?
    fn draw_city_search_page(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, content: egui::Rect, e: f32, vis: f32) {
        let vis = vis.clamp(0.0, 1.0);
        let clip = painter.with_clip_rect(content);
        let painter: &egui::Painter = &clip;
        let mask = egui::Color32::from_rgba_unmultiplied(
            self.cfg.bg[0], self.cfg.bg[1], self.cfg.bg[2], (self.cfg.alpha as f32 * vis) as u8,
        );
        let rr = self.cfg.radius.min(content.height() * 0.5);
        painter.rect_filled(content, rr, mask);
        let dy = (1.0 - vis) * 18.0;
        let content = egui::Rect::from_min_max(
            egui::pos2(content.left(), content.top() + dy),
            egui::pos2(content.right(), content.bottom() + dy),
        );
        let a = ((((e - 0.5) * 2.0) * 255.0).clamp(0.0, 255.0) * vis) as u8;
        let ink = egui::Color32::from_rgba_unmultiplied(233, 236, 242, a);
        let muted2 = egui::Color32::from_rgba_unmultiplied(107, 113, 128, a);
        let accent = egui::Color32::from_rgba_unmultiplied(52, 211, 153, a);

        let x0 = content.left() + CONTENT_PAD_X;
        let x1 = content.right() - CONTENT_PAD_X;
        let y0 = content.top() + CONTENT_PAD_T;

        // 返回 + 标题
        let back = egui::Rect::from_center_size(egui::pos2(x0 + 9.0, y0 + 10.0), egui::vec2(26.0, 26.0));
        let br = ui
            .interact(back, ui.id().with("w_city_back"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let bt = ui
            .ctx()
            .animate_bool_with_time(ui.id().with("w_city_back_hover"), br.hovered(), 0.12);
        painter.text(
            back.center(),
            egui::Align2::CENTER_CENTER,
            "\u{2039}",
            egui::FontId::proportional(20.0),
            lerp_color(muted2, accent, bt),
        );
        if br.clicked() {
            self.close_city_search();
        }
        painter.text(
            egui::pos2(x0 + 26.0, y0 + 10.0),
            egui::Align2::LEFT_CENTER,
            "搜索城市",
            bold_font(15.0),
            ink,
        );

        // 搜索�?+ 自动定位：用低透明度薄荷色叠在内容区背景上�?
        // 不引入任何突兀的灰色块，保证与深色内容区融在一起�?
        let mint = |al: f32| {
            egui::Color32::from_rgba_unmultiplied(52, 211, 153, (a as f32 * al).clamp(0.0, 255.0) as u8)
        };
        let iy = y0 + 30.0;
        let auto_w = 64.0f32;
        let input_r = egui::Rect::from_min_max(
            egui::pos2(x0, iy),
            egui::pos2(x1 - auto_w - 6.0, iy + 26.0),
        );
        painter.rect_filled(input_r, 7.0, egui::Color32::from_rgba_unmultiplied(22, 23, 28, a));
        painter.rect_stroke(
            input_r,
            7.0,
            egui::Stroke::new(
                0.8,
                egui::Color32::from_rgba_unmultiplied(150, 158, 175, (a as f32 * 0.50) as u8),
            ),
            egui::StrokeKind::Inside,
        );
        let mut q = std::mem::take(&mut self.w_search_query);
        let te = egui::TextEdit::singleline(&mut q)
            .frame(false)
            .id(ui.id().with("w_city_input"))
            .hint_text("城市 / 区县，如 杭州、浦东新区")
            .font(egui::FontId::proportional(12.0))
            .text_color(ink);
        // TextEdit 默认会铺一�?extreme_bg_color；此处临时改为透明�?
        // 让上面绘制的薄荷色底透出来，避免出现一块灰色输入框�?
        let tresp = ui
            .scope(|ui| {
                let v = ui.visuals_mut();
                v.extreme_bg_color = egui::Color32::TRANSPARENT;
                v.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                v.widgets.hovered.bg_fill = egui::Color32::TRANSPARENT;
                v.widgets.active.bg_fill = egui::Color32::TRANSPARENT;
                ui.put(input_r.shrink2(egui::vec2(8.0, 3.0)), te)
            })
            .inner;
        self.w_search_query = q;
        if self.w_search_need_focus && self.w_search_focus_tries > 0 {
            if !tresp.has_focus() {
                tresp.request_focus();
            }
            if tresp.has_focus() {
                self.w_search_need_focus = false;
            } else {
                self.w_search_focus_tries = self.w_search_focus_tries.saturating_sub(1);
            }
        }
        let ft = ui
            .ctx()
            .animate_bool_with_time(ui.id().with("w_city_input_focus"), tresp.has_focus(), 0.16);
        painter.rect_stroke(
            input_r,
            7.0,
            egui::Stroke::new(
                0.8 + 0.2 * ft,
                egui::Color32::from_rgba_unmultiplied(
                    150,
                    158,
                    175,
                    (a as f32 * (0.50 + 0.35 * ft)) as u8,
                ),
            ),
            egui::StrokeKind::Inside,
        );
        if ft > 0.01 {
            painter.rect_stroke(
                input_r.expand(1.0),
                8.0,
                egui::Stroke::new(
                    0.6,
                    egui::Color32::from_rgba_unmultiplied(
                        150,
                        158,
                        175,
                        (a as f32 * 0.12 * ft) as u8,
                    ),
                ),
                egui::StrokeKind::Inside,
            );
        }
        if tresp.changed() {
            self.w_search_last_edit = Instant::now();
            if self.w_search_query.trim().is_empty() {
                self.w_search_results.clear();
                self.w_search_results_at = None;
                self.w_search_last_query.clear();
            }
        }
        let auto_r = egui::Rect::from_min_max(
            egui::pos2(x1 - auto_w, iy),
            egui::pos2(x1, iy + 26.0),
        );
        let ar = ui
            .interact(auto_r, ui.id().with("w_city_auto"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let at = ui
            .ctx()
            .animate_bool_with_time(ui.id().with("w_city_auto_hover"), ar.hovered(), 0.13);
        painter.rect_filled(
            auto_r,
            7.0,
            lerp_color(
                accent,
                egui::Color32::from_rgba_unmultiplied(108, 232, 186, a),
                at,
            ),
        );
        painter.text(
            auto_r.center(),
            egui::Align2::CENTER_CENTER,
            "自动定位",
            egui::FontId::proportional(10.0),
            egui::Color32::from_rgba_unmultiplied(6, 42, 32, a),
        );
        if ar.clicked() {
            self.set_weather_city("", "");
            self.close_city_search();
        }

        // 结果列表
        let mut yy = iy + 34.0;
        let q = self.w_search_query.trim().to_string();
        if q.is_empty() {
            painter.text(
                egui::pos2(x0 + 2.0, yy + 4.0),
                egui::Align2::LEFT_TOP,
                "输入城市或区县名，从全国范围搜索",
                egui::FontId::proportional(11.0),
                muted2,
            );
        } else if self.w_search_inflight && self.w_search_results.is_empty() {
            painter.text(
                egui::pos2(x0 + 2.0, yy + 4.0),
                egui::Align2::LEFT_TOP,
                "搜索中…",
                egui::FontId::proportional(11.0),
                muted2,
            );
        } else if self.w_search_results.is_empty() {
            painter.text(
                egui::pos2(x0 + 2.0, yy + 4.0),
                egui::Align2::LEFT_TOP,
                format!("未找到「{}」，换个名称试试", q),
                egui::FontId::proportional(11.0),
                muted2,
            );
        } else {
            let list = self.w_search_results.clone();
            let list_age = self
                .w_search_results_at
                .map(|t| t.elapsed().as_secs_f32())
                .unwrap_or(1.0);
            for (i, hit) in list.iter().take(6).enumerate() {
                // 结果逐条错峰淡入 + 轻微上移，避免列表突然出�?
                let raw = ((list_age - i as f32 * 0.028) / 0.16).clamp(0.0, 1.0);
                let appear = raw * raw * (3.0 - 2.0 * raw);
                if appear <= 0.001 {
                    continue;
                }
                let row_top = yy + (1.0 - appear) * 5.0;
                let row = egui::Rect::from_min_size(egui::pos2(x0, row_top), egui::vec2(x1 - x0, 28.0));
                let hr = ui
                    .interact(row, ui.id().with(("w_city_row", i)), egui::Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                let ht = ui
                    .ctx()
                    .animate_bool_with_time(ui.id().with(("w_city_row_hover", i)), hr.hovered(), 0.11);
                if ht > 0.001 {
                    painter.rect_filled(row, 6.0, mint(0.10 * appear * ht));
                }
                let row_ink = egui::Color32::from_rgba_unmultiplied(233, 236, 242, (a as f32 * appear) as u8);
                let row_dim = egui::Color32::from_rgba_unmultiplied(
                    107,
                    113,
                    128,
                    (a as f32 * 0.60 * appear) as u8,
                );
                painter.text(
                    egui::pos2(x0 + 8.0, row_top + 14.0),
                    egui::Align2::LEFT_CENTER,
                    &hit.name,
                    egui::FontId::proportional(12.0),
                    row_ink,
                );
                painter.text(
                    egui::pos2(x1 - 8.0, row_top + 14.0),
                    egui::Align2::RIGHT_CENTER,
                    &hit.province,
                    egui::FontId::proportional(9.5),
                    row_dim,
                );
                if hr.clicked() && appear > 0.5 {
                    let key = hit.key.clone();
                    let name = hit.name.clone();
                    self.set_weather_city(&key, &name);
                    self.close_city_search();
                }
                yy += 28.0;
            }
            if list_age < 0.5 {
                ui.ctx().request_repaint();
            }
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
    }

    /// 内置设置页（音乐模块内，覆盖右侧内容区）：歌词 / 显示 两组可调项。
    fn draw_settings_page(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, content: egui::Rect, e: f32, vis: f32) {
        let vis = vis.clamp(0.0, 1.0);
        let full = content;
        // 不依赖上一帧可能滞后的窗口原点，保证能精确贴到最?最右角?
        // 返回时反向淡出，避免硬切造成突兀（界面约定：一切界面变化都要有过渡）�?
        let dt = ui.input(|i| i.stable_dt).clamp(0.001, 0.05);
        let target = if self.w_city_search { 1.0 } else { 0.0 };
        let k = 1.0 - (-dt / 0.085).exp();
        self.w_city_search_anim += (target - self.w_city_search_anim) * k;
        if (target - self.w_city_search_anim).abs() > 0.002 {
            ui.ctx().request_repaint();
        }
        let search_anim = self.w_city_search_anim.clamp(0.0, 1.0);
        let base_vis = vis * (1.0 - search_anim);
        // 搜索页完全展开时不必再绘制设置页（也避免底层控件被误触）；
        // 过渡过程中两侧同时绘制，形成交叉淡入淡出�?
        if !(self.w_city_search && search_anim >= 0.999) {
            self.draw_settings_base(ui, painter, content, e, base_vis);
        }
        if search_anim > 0.001 {
            // 搜索页在最上层：先吃掉背景点击，避免穿透到设置页控�?
            let _ = ui.interact(full, ui.id().with("w_city_modal"), egui::Sense::click());
            self.draw_city_search_page(ui, painter, full, e, vis * search_anim);
        }
    }

    fn draw_settings_base(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, content: egui::Rect, e: f32, vis: f32) {
        let vis = vis.clamp(0.0, 1.0);
        let full = content;
        // 待机表情网格需要所有表情的动画帧，提前全部加载
        if self.face_mode_active() {
            for i in 0..FACES.len() {
                self.ensure_face_frames(ui.ctx(), i);
            }
        }
        // 用与岛体一致的底色做遮罩：随 vis 淡入，盖住下方的音乐页，实现“展开”过渡
        let clip_p = painter.with_clip_rect(full);
        let painter: &egui::Painter = &clip_p;
        let mask = egui::Color32::from_rgba_unmultiplied(
            self.cfg.bg[0], self.cfg.bg[1], self.cfg.bg[2], (self.cfg.alpha as f32 * vis) as u8,
        );
        let rr = self.cfg.radius.min(full.height() * 0.5);
        painter.rect_filled(full, rr, mask);
        // 轻微上滑
        let dy = (1.0 - vis) * 18.0;
        let content = egui::Rect::from_min_max(
            egui::pos2(full.left(), full.top() + dy),
            egui::pos2(full.right(), full.bottom() + dy),
        );
        let a = ((((e - 0.5) * 2.0) * 255.0).clamp(0.0, 255.0) * vis) as u8;
        let ink = egui::Color32::from_rgba_unmultiplied(233, 236, 242, a);
        let muted2 = egui::Color32::from_rgba_unmultiplied(107, 113, 128, a);
        let accent = egui::Color32::from_rgba_unmultiplied(52, 211, 153, a);

        // 顶部按钮区（页眉）：预留固定高度，返回按钮居中其右，与下方内容拉开距离，避免拥挤
        let header_h = 48.0f32;
        let back_r = 11.0f32;
        // 返回按钮（右上角）：无底色、无描边，只留一枚箭头图标，与页面底色融为一体；悬停不做高亮
        let bc = egui::pos2(content.left() + CONTENT_PAD_X + back_r, content.top() + header_h * 0.5);
        let bh = egui::Rect::from_center_size(bc, egui::vec2(30.0, 30.0));
        let bresp = ui.interact(bh, ui.id().with("set_back"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        painter.text(bc, egui::Align2::CENTER_CENTER, "\u{E72B}", icon_font(12.5), ink); // Back
        if bresp.clicked() { self.show_settings = false; self.w_city_search = false; self.w_city_search_from_weather = false; }

        let left = content.left() + CONTENT_PAD_X;
        let right = content.right() - CONTENT_PAD_X;
        let row_h = 30.0f32;
        let label_h = 18.0f32;
        let group_gap = 12.0f32;
        // 待机表情：可视化点选网格（7 列 × 2 行，每个格子播放对应表情动画）
        let tile_h = 60.0f32;
        let tile_gap = 5.0f32;
        let grid_h = tile_h * 2.0 + tile_gap;
        // 设置内容区可滚动（设置项变多时自动出现滚动）
        let list_top = content.top() + header_h;
        let list_bottom = content.bottom() - CONTENT_PAD_B;
        let view_h = (list_bottom - list_top).max(1.0);
        let face_on = self.face_mode_active();
        let mut total_h = label_h * 6.0 + group_gap * 5.0 + row_h * 11.0 + label_h;
        if face_on {
            total_h += row_h + grid_h;
        }
        let max_scroll = (total_h - view_h).max(0.0);
        // 滚轮只累加“目标值”，再让实际偏移每帧平滑逼近，形成丝滑的过渡
        let mut scroll_activity = false;
        if let Some(ph) = ui.ctx().pointer_hover_pos() {
            if full.contains(ph) && max_scroll > 0.0 {
                let d = ui.input(|i| i.raw_scroll_delta.y);
                if d.abs() > 0.01 {
                    self.settings_scroll_target -= d;
                    scroll_activity = true;
                }
            }
        }
        self.settings_scroll_target = self.settings_scroll_target.clamp(0.0, max_scroll);
        // 仅调试：WINISLE_SET_SCROLL=<px> 直接设定滚动位置（用于无鼠标截图验证）
        if let Ok(v) = std::env::var("WINISLE_SET_SCROLL") {
            if let Ok(px) = v.trim().parse::<f32>() {
                self.settings_scroll_target = px.clamp(0.0, max_scroll);
                self.settings_scroll = self.settings_scroll_target;
            }
        }
        // 指数平滑（时间常�?~70ms），越接近目标越慢，收尾自然
        let dt = ui.input(|i| i.stable_dt).clamp(0.001, 0.05);
        let k = 1.0 - (-dt / 0.07).exp();
        self.settings_scroll += (self.settings_scroll_target - self.settings_scroll) * k;
        self.settings_scroll = self.settings_scroll.clamp(0.0, max_scroll);
        // 调试钩子：WINISLE_SETTINGS_SCROLL=<px> 直接把设置页滚动到指定位置（离屏截图核对用）
        if let Ok(sv) = std::env::var("WINISLE_SETTINGS_SCROLL") {
            if let Ok(v) = sv.parse::<f32>() {
                self.settings_scroll_target = v.clamp(0.0, max_scroll);
                self.settings_scroll = self.settings_scroll_target;
            }
        }
        let settling = (self.settings_scroll_target - self.settings_scroll).abs() > 0.5;
        if (self.settings_scroll_target - self.settings_scroll).abs() > 0.25 {
            ui.ctx().request_repaint();
        }
        // 滚动条自动隐藏：滚动 / 惯性滚动期间显示，停止�?1 秒后淡出
        if scroll_activity {
            self.settings_scroll_act = std::time::Instant::now();
        }
        let since_act = self.settings_scroll_act.elapsed().as_secs_f32();
        let want_vis = if max_scroll > 0.0 && (scroll_activity || settling || since_act < 1.2) {
            1.0
        } else {
            0.0
        };
        let kv = 1.0 - (-dt / 0.12).exp();
        self.settings_scroll_vis += (want_vis - self.settings_scroll_vis) * kv;
        if self.settings_scroll_vis < 0.005 {
            self.settings_scroll_vis = 0.0;
        } else if self.settings_scroll_vis > 0.995 {
            self.settings_scroll_vis = 1.0;
        }
        if (self.settings_scroll_vis - want_vis).abs() > 0.005 {
            ui.ctx().request_repaint();
        }
        let list_rect = egui::Rect::from_min_max(
            egui::pos2(full.left(), list_top),
            egui::pos2(full.right(), list_bottom),
        );
        let lp = painter.with_clip_rect(list_rect);
        let painter: &egui::Painter = &lp;
        let mut y = list_top - self.settings_scroll;
        let mut changed = false;

        let row_label = |painter: &egui::Painter, y: f32, s: &str, c: egui::Color32| {
            painter.text(egui::pos2(left, y + row_h * 0.5), egui::Align2::LEFT_CENTER, s, egui::FontId::proportional(13.5), c);
        };
        let tg = |y: f32| egui::Rect::from_min_size(egui::pos2(right - 32.0, y + row_h * 0.5 - 8.0), egui::vec2(32.0, 16.0));

        // 分组：收起态（可组合组件：拖动排序 + 开关）
        painter.text(egui::pos2(left, y), egui::Align2::LEFT_TOP, "收起态", egui::FontId::proportional(13.0), muted2);
        y += label_h;
        {
            let mut order = self.comp_order();
            let comps_top = y;
            let row_w = right - left;
            let toggle_w = 44.0;
            let drag_id = ui.id().with("cset_drag");
            let dragging: Option<String> = ui.data(|d| d.get_temp::<String>(drag_id));
            let n = order.len();
            // 每个组件一个平滑的槽位动画（索引）：重排时各行顺滑滑动到新位置
            let slots: Vec<f32> = order
                .iter()
                .enumerate()
                .map(|(i, (id, _))| {
                    ui.ctx().animate_value_with_time(ui.id().with(("cset_pos", id)), i as f32, 0.09)
                })
                .collect();
            let ptr = ui.ctx().pointer_hover_pos();
            let down = ui.ctx().input(|i| i.pointer.any_down());
            let force_hl: Option<String> = std::env::var("WINISLE_SET_HL").ok();
            let drag_preview: Option<String> = std::env::var("WINISLE_DRAG_PREVIEW").ok();
            // 正在拖动：被拖行直接跟随指针，其余行动画到新槽位
            let drag_fy: Option<(String, f32)> = if let Some(did) = dragging.clone() {
                if down {
                    ptr.map(|p| (did, (((p.y - comps_top) / row_h) - 0.5).clamp(0.0, (n.max(1) - 1) as f32)))
                } else {
                    None
                }
            } else {
                None
            };
            // 被拖行最后绘制，浮到最上层
            let mut draw_order: Vec<usize> = (0..n).collect();
            if let Some((did, _)) = &drag_fy {
                if let Some(ci) = order.iter().position(|(x, _)| x == did) {
                    draw_order.retain(|&k| k != ci);
                    draw_order.push(ci);
                }
            }
            for &i in &draw_order {
                let (id, on) = order[i].clone();
                let is_drag = (drag_fy.as_ref().map(|(d, _)| d == &id).unwrap_or(false) && down)
                    || drag_preview.as_deref() == Some(id.as_str());
                // 上一版手感：被拖行跟随指针（连续），其余行平滑让位；被拖行用不透明浮起卡片盖住，
                // 因此即使过渡中短暂覆盖到相邻行，也不会出现两层透叠的“重影”。
                let slot = if is_drag {
                    drag_fy.as_ref().map(|(_, f)| *f).unwrap_or(i as f32)
                } else if drag_preview.as_deref() == Some(id.as_str()) {
                    i as f32
                } else {
                    slots[i]
                };
                let ry = comps_top + slot * row_h;
                let rrect = egui::Rect::from_min_size(egui::pos2(left, ry), egui::vec2(row_w, row_h));
                let hovered = ptr.map(|p| rrect.contains(p)).unwrap_or(false);
                // 被拖行：先画一块不透明的“浮起卡片”底，盖住其下方/相邻内容，任何情况下都不会出现透叠
                if is_drag {
                    // 卡片严格限制在内容区内（左=left、右=right），不再向外溢出，避免压到面板留白/圆角
                    let card = egui::Rect::from_min_max(
                        egui::pos2(left, ry + 1.0),
                        egui::pos2(right, ry + row_h - 1.0),
                    );
                    painter.rect_filled(card.translate(egui::vec2(0.0, 3.0)), 8.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 70));
                    let c = egui::Color32::from_rgb(
                        self.cfg.bg[0].saturating_add(16),
                        self.cfg.bg[1].saturating_add(16),
                        self.cfg.bg[2].saturating_add(18),
                    );
                    painter.rect_filled(card, 8.0, c);
                    painter.rect_stroke(card, 8.0, egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(52, 211, 153, 110)), egui::StrokeKind::Inside);
                }
                // 行高亮：与内容区同宽（left..right，正好包住尾部开关，不外溢），低透薄荷绿融入背景
                if hovered || is_drag || force_hl.as_deref() == Some(id.as_str()) {
                    let ha = ((if is_drag { 30.0 } else { 14.0 }) * (a as f32 / 255.0)) as u8;
                    let hl = egui::Rect::from_min_max(
                        egui::pos2(left, ry + 2.0),
                        egui::pos2(right, ry + row_h - 2.0),
                    );
                    painter.rect_filled(hl, 7.0, egui::Color32::from_rgba_unmultiplied(52, 211, 153, ha));
                }
                // 拖拽热区（左侧，排除右侧开关）
                let hrect = egui::Rect::from_min_max(rrect.min, egui::pos2(rrect.right() - toggle_w, rrect.bottom()));
                let hresp = ui.interact(hrect, ui.id().with(("cset_h", &id)), egui::Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::Grab);
                if hresp.drag_started() {
                    ui.data_mut(|d| d.insert_temp(drag_id, id.clone()));
                }
                // 拖动图标：手绘 2×3 圆点，垂直居中，保证与文字同一水平线
                let hcx = left + 9.0;
                let hcy = ry + row_h * 0.5;
                for col in 0..2 {
                    for rr in 0..3 {
                        painter.circle_filled(
                            egui::pos2(hcx + col as f32 * 4.0, hcy - 5.0 + rr as f32 * 5.0),
                            1.35,
                            muted2,
                        );
                    }
                }
                // 开关可用性：已启用可随时关闭；未启用则需“加上它也不超过 520”才允许开启
                let can_toggle = if on {
                    true
                } else {
                    let mut probe = order.clone();
                    probe[i].1 = true;
                    self.order_base_width(&probe) <= CW_MAX + 0.01
                };
                let label_col = if can_toggle { ink } else { muted2 };
                painter.text(egui::pos2(left + 24.0, ry + row_h * 0.5), egui::Align2::LEFT_CENTER, Self::comp_label(&id), egui::FontId::proportional(13.5), label_col);
                // 开关
                let trect = egui::Rect::from_min_size(egui::pos2(right - 37.0, ry + row_h * 0.5 - 8.0), egui::vec2(32.0, 16.0));
                if let Some(v) = Self::draw_toggle(ui, painter, trect, on, ui.id().with(("cset_tg", &id)), accent, a, can_toggle) {
                    order[i].1 = v;
                    self.set_comp_order(&order);
                    changed = true;
                }
            }
            y = comps_top + n as f32 * row_h;
            // 拖动中：按指针实时重排（写入配置，下一帧各行动画滑到新位置）
            if let Some((did, _)) = drag_fy.clone() {
                if let Some(p) = ptr {
                    if let Some(cur) = order.iter().position(|(x, _)| x == &did) {
                        // 提前 0.30 行触发让位（不必等拖到正中），让被占位那行“提前知道、先让开”，
                        // 减少拖动时压到还没让开的行。逐帧最多挪一格，节奏更快。
                        let fy = (p.y - comps_top) / row_h;
                        let mut target = cur;
                        // 用“半个行高”作为换位阈值：这是左右两侧对称的临界点，
                        // 拖动中指针停住不动时不会来回翻（旧版 0.30 阈值会造成
                        // 让位区间重叠 → 每帧在相邻两格间反复横跳，表现为“一直振动移不开”）。
                        if fy - cur as f32 > 0.5 {
                            target = (cur + 1).min(n - 1);
                        } else if cur as f32 - fy > 0.5 {
                            target = cur.saturating_sub(1);
                        }
                        if target != cur {
                            let item = order.remove(cur);
                            order.insert(target, item);
                            self.set_comp_order(&order);
                            changed = true;
                        }
                    }
                }
                ui.ctx().request_repaint();
            } else if dragging.is_some() {
                ui.data_mut(|d| d.remove_temp::<String>(drag_id));
            }
            // 动画进行中持续重绘（松手后的回位滑行也依赖它）
            if slots.iter().enumerate().any(|(i, v)| (v - i as f32).abs() > 0.001) {
                ui.ctx().request_repaint();
            }
            // 宽度提示：已用宽度 / 上限
            let used = self.order_base_width(&order);
            let comp_n = order.iter().filter(|(_, on)| *on).count();
            let hint = format!("占宽 {:.0} / {:.0}（{} 项启用）", used, CW_MAX, comp_n);
            painter.text(egui::pos2(left, y + label_h * 0.5), egui::Align2::LEFT_CENTER, hint, egui::FontId::proportional(11.0), muted2);
            y += label_h;
        }
        if face_on {
            // 待机表情：可视化点选网格（每个格子实时播放对应表情动画，点击即可选中）
            row_label(painter, y, "待机表情", ink);
            y += row_h;
            let area = egui::Rect::from_min_size(egui::pos2(left, y), egui::vec2(right - left, grid_h));
            let tsec = ui.input(|i| i.time);
            let mut face_uvs: Vec<Option<(f32, f32, f32, f32)>> = Vec::with_capacity(FACES.len());
            for fi in 0..FACES.len() {
                let has = self.face_frames.get(fi).map_or(false, |o| o.is_some());
                face_uvs.push(if has { Some(self.face_content_uv(fi)) } else { None });
            }
            if let Some(v) = Self::draw_face_grid(ui, painter, area, self.cfg.idle_face, ui.id().with("face_grid"), a, accent, &self.face_frames, &face_uvs, tsec) {
                self.cfg.idle_face = v;
                changed = true;
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
            y += grid_h;
        }

        // 分组：歌�?
        // 分组：音乐（展开面板）
        y += group_gap;
        painter.text(egui::pos2(left, y), egui::Align2::LEFT_TOP, "音乐", egui::FontId::proportional(13.0), muted2);
        y += label_h;
        // 波纹频谱（展开态音乐面板底部的水波反射）
        row_label(painter, y, "波纹频谱", ink);
        { let trect = tg(y); if let Some(v) = Self::draw_toggle(ui, painter, trect, self.cfg.water_spectrum, ui.id().with("tg_water_spec"), accent, a, true) { self.cfg.water_spectrum = v; changed = true; } }
        y += row_h;

        y += group_gap;
        painter.text(egui::pos2(left, y), egui::Align2::LEFT_TOP, "歌词", egui::FontId::proportional(13.0), muted2);
        y += label_h;
        // 换行滚动动画
        row_label(painter, y, "换行滚动动画", ink);
        { let trect = tg(y); if let Some(v) = Self::draw_toggle(ui, painter, trect, self.cfg.roll_anim, ui.id().with("tg_roll"), accent, a, true) { self.cfg.roll_anim = v; changed = true; } }
        y += row_h;
        // 跑马灯边缘渐隐（左侧淡出 / 右侧渐显）
        row_label(painter, y, "边缘渐隐", ink);
        { let trect = tg(y); if let Some(v) = Self::draw_toggle(ui, painter, trect, self.cfg.edge_fade, ui.id().with("tg_edge_fade"), accent, a, true) { self.cfg.edge_fade = v; changed = true; } }
        y += row_h;

        // 分组：窗口（左右吸附）
        y += group_gap;
        painter.text(egui::pos2(left, y), egui::Align2::LEFT_TOP, "窗口", egui::FontId::proportional(13.0), muted2);
        y += label_h;
        row_label(painter, y, "左右吸附", ink);
        { let trect = tg(y); if let Some(v) = Self::draw_toggle(ui, painter, trect, self.cfg.side_snap, ui.id().with("tg_side_snap"), accent, a, true) { self.cfg.side_snap = v; changed = true; } }
        y += row_h;

        // 分组：天气（温度单位）——城市固定跟随当前位置，无需选择
        y += group_gap;
        painter.text(egui::pos2(left, y), egui::Align2::LEFT_TOP, "天气", egui::FontId::proportional(13.0), muted2);
        y += label_h;
        row_label(painter, y, "温度单位", ink);
        {
            let seg = egui::Rect::from_min_max(egui::pos2(right - 104.0, y + 5.0), egui::pos2(right, y + row_h - 5.0));
            if let Some(v) = Self::draw_segment(ui, painter, seg, &["摄氏度", "华氏度"], self.cfg.weather_unit as usize, ui.id().with("seg_unit"), accent, a) {
                self.cfg.weather_unit = v as u8;
                changed = true;
            }
        }
        y += row_h;

        // 分组：通用（软件更新，所有模块共用）
        y += group_gap;
        painter.text(egui::pos2(left, y), egui::Align2::LEFT_TOP, "通用", egui::FontId::proportional(13.0), muted2);
        y += label_h;
        row_label(painter, y, "软件更新", ink);
        {
            let up_row = egui::Rect::from_min_max(
                egui::pos2(right - 220.0, y + 3.0),
                egui::pos2(right, y + row_h - 3.0),
            );
            let today = update::today_local();
            let checked_today = self.cfg.upd_checked_day == today;
            let ver = self.upd_ver.as_ref().map(|m| m.version.clone()).unwrap_or_default();
            let pct = self.upd_dl_pct.load(std::sync::atomic::Ordering::Relaxed);
            let (txt, col, green, show_dot, progress, enabled) = if self.upd_busy {
                if pct >= 100 {
                    (format!("v{} · 正在安装 v{}…", update::VERSION, ver), accent, true, false, Some(100u8), false)
                } else {
                    (format!("v{} · 正在下载 v{}… {}%", update::VERSION, ver, pct), accent, true, false, Some(pct), false)
                }
            } else if let Some(m) = &self.upd_ver {
                (
                    format!("v{} · 更新 (v{})", update::VERSION, m.version),
                    accent,
                    true,
                    self.show_row_update(),
                    None,
                    true,
                )
            } else if self.upd_checking {
                (format!("v{} · 检查中…", update::VERSION), muted2, false, false, None, false)
            } else if self.upd_msg.contains("失败") {
                (self.upd_msg.clone(), egui::Color32::from_rgba_unmultiplied(230, 120, 120, a), false, false, None, true)
            } else if checked_today {
                (format!("v{} · 已是最新版本", update::VERSION), muted2, false, false, None, false)
            } else {
                (format!("v{} · 检查更新", update::VERSION), muted2, false, false, None, true)
            };
            let font = egui::FontId::proportional(11.5);
            // 统一排版：版本号、圆点、文案同一字号、同一基线（与「v0.2.1 · 检查更新」预览一致）
            painter.text(egui::pos2(right, y + row_h * 0.5), egui::Align2::RIGHT_CENTER, txt, font, col);
            if green {
                // 绿色状态仅在整行文字上体现；小圆点固定在文字右上角，留在面板内
                if show_dot {
                    painter.circle_filled(egui::pos2(right - 3.0, y + row_h * 0.5 - 8.0), 3.0, accent);
                }
            }
            if let Some(p) = progress {
                let bar = egui::Rect::from_min_max(
                    egui::pos2(up_row.left() + 4.0, y + row_h - 5.0),
                    egui::pos2(up_row.right(), y + row_h - 2.0),
                );
                painter.rect_filled(bar, 1.5, egui::Color32::from_rgba_unmultiplied(42, 58, 52, a));
                let fill = egui::Rect::from_min_max(
                    bar.left_top(),
                    egui::pos2(bar.left() + bar.width() * p as f32 / 100.0, bar.bottom()),
                );
                painter.rect_filled(fill, 1.5, accent);
            }
            let resp = ui.interact(up_row, ui.id().with("upd_row"), egui::Sense::click());
            let resp = if enabled {
                resp.on_hover_cursor(egui::CursorIcon::PointingHand)
            } else {
                resp
            };
            if enabled && resp.clicked() {
                self.mark_upd_seen_row();
                if self.upd_ver.is_some() {
                    self.start_update_install();
                } else {
                    self.start_update_check();
                }
            }
        }
        y += row_h;

        // 滚动条（内容超出时，滚动时淡入、停止后自动淡出）——细样式，与天气页滚动条一致（2px�?
        if max_scroll > 0.0 && self.settings_scroll_vis > 0.01 {
            let sv = self.settings_scroll_vis;
            let tx = full.right() - 4.0;
            let track = egui::Rect::from_min_max(
                egui::pos2(tx, list_top + 2.0),
                egui::pos2(tx + 2.0, list_bottom - 2.0),
            );
            painter.rect_filled(track, 1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, (12.0 * sv) as u8));
            let th = ((view_h / total_h.max(1.0)) * track.height()).max(14.0);
            let range = (track.height() - th).max(1.0);
            let tp = (self.settings_scroll / max_scroll.max(1.0)) * range;
            let thumb = egui::Rect::from_min_size(egui::pos2(tx, track.top() + tp), egui::vec2(2.0, th));
            painter.rect_filled(thumb, 1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, (60.0 * sv) as u8));
        }

        if changed { self.cfg.save(); }
    }

    /// 开关控件（绘制 + 点击），带滑动/变色动画。返回 Some(新值) 表示被点击切换。
    fn draw_toggle(ui: &mut egui::Ui, painter: &egui::Painter, rect: egui::Rect, on: bool, id: egui::Id, accent: egui::Color32, a: u8, enabled: bool) -> Option<bool> {
        let t = ui.ctx().animate_bool_with_time(id.with("anim"), on, 0.16);
        // 禁用（如已达宽度上限、不允许再开启）时整体压暗且不响应点击
        let da = if enabled { a } else { ((a as f32) * 0.42) as u8 };
        let lerp_u = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        let bg = egui::Color32::from_rgba_unmultiplied(
            lerp_u(78, accent.r()), lerp_u(82, accent.g()), lerp_u(96, accent.b()), da,
        );
        painter.rect_filled(rect, rect.height() * 0.5, bg);
        let r = rect.height() * 0.5 - 2.0;
        let kx0 = rect.left() + r + 2.0;
        let kx1 = rect.right() - r - 2.0;
        let kx = kx0 + (kx1 - kx0) * t;
        painter.circle_filled(egui::pos2(kx, rect.center().y), r, egui::Color32::from_rgba_unmultiplied(246, 248, 251, da));
        if !enabled { return None; }
        let resp = ui.interact(rect.expand(5.0), id, egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() { Some(!on) } else { None }
    }

    /// 滑块控件（绘�?+ 拖动）。返�?Some(新�? 表示被拖�?点击修改�?
    #[allow(clippy::too_many_arguments)]
    fn draw_slider(ui: &mut egui::Ui, painter: &egui::Painter, zone: egui::Rect, val: f32, min: f32, max: f32, step: f32, suffix: &str, enabled: bool, id: egui::Id, accent: egui::Color32, a: u8) -> Option<f32> {
        let track = egui::Rect::from_min_max(egui::pos2(zone.left(), zone.center().y - 2.0), egui::pos2(zone.right() - 56.0, zone.center().y + 2.0));
        let frac = if max > min { ((val - min) / (max - min)).clamp(0.0, 1.0) } else { 0.0 };
        let active_col = if enabled { accent } else { egui::Color32::from_rgba_unmultiplied(120, 124, 138, a) };
        painter.rect_filled(track, 2.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26));
        let fx = track.left() + track.width() * frac;
        painter.rect_filled(egui::Rect::from_min_max(track.min, egui::pos2(fx, track.bottom())), 2.0, active_col);
        painter.circle_filled(egui::pos2(fx, track.center().y), 6.0, active_col);
        let shown = if (val - val.round()).abs() < 0.01 { format!("{:.0}{}", val.round(), suffix) } else { format!("{:.1}{}", val, suffix) };
        let tcol = if enabled { egui::Color32::from_rgba_unmultiplied(200, 206, 220, a) } else { egui::Color32::from_rgba_unmultiplied(120, 124, 138, a) };
        painter.text(egui::pos2(zone.right(), zone.center().y), egui::Align2::RIGHT_CENTER, shown, egui::FontId::proportional(12.0), tcol);
        if !enabled { return None; }
        let hit = track.expand2(egui::vec2(6.0, 10.0));
        let resp = ui.interact(hit, id, egui::Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.dragged() || resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let f = ((pos.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
                let raw = min + f * (max - min);
                let stepped = if step > 0.0 { (raw / step).round() * step } else { raw };
                return Some(stepped.clamp(min, max));
            }
        }
        None
    }

    /// 分段选择控件（如「时间 | 表情」）。高亮块在选项间平滑滑动，文案颜色随之渐变。
    /// 返回 Some(新索引) 表示被点击切换。
    fn draw_segment(ui: &mut egui::Ui, painter: &egui::Painter, rect: egui::Rect, opts: &[&str], sel: usize, id: egui::Id, accent: egui::Color32, a: u8) -> Option<usize> {
        let resp = ui.interact(rect, id, egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let n = opts.len().max(1);
        let seg_w = rect.width() / n as f32;
        // 高亮块位置动画（在选项之间平滑滑动）
        let target = sel as f32;
        let pos = ui.ctx().animate_value_with_time(id.with("seg_pos"), target, 0.20);
        // 轨道：用左侧菜单的深色(22,23,28)
        painter.rect_filled(rect, rect.height() * 0.5, egui::Color32::from_rgba_unmultiplied(22, 23, 28, a));
        // 高亮块（随动画移动）
        let hx = rect.left() + seg_w * pos;
        let hr = egui::Rect::from_min_max(
            egui::pos2(hx + 1.0, rect.top() + 1.0),
            egui::pos2(hx + seg_w - 1.0, rect.bottom() - 1.0),
        );
        painter.rect_filled(hr, hr.height() * 0.5, egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), a));
        let lerp_u = |x: u8, y: u8, t: f32| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        let mut out = None;
        for (i, label) in opts.iter().enumerate() {
            // 文案颜色：按高亮块覆盖比例在「未选中/选中」之间渐变
            let w = (1.0 - (pos - i as f32).abs()).clamp(0.0, 1.0);
            let tc = egui::Color32::from_rgba_unmultiplied(
                lerp_u(178, 16, w),
                lerp_u(186, 26, w),
                lerp_u(202, 22, w),
                a,
            );
            let c = egui::pos2(rect.left() + seg_w * (i as f32 + 0.5), rect.center().y);
            painter.text(c, egui::Align2::CENTER_CENTER, *label, egui::FontId::proportional(12.0), tc);
            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let sr = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + seg_w * i as f32, rect.top()),
                        egui::pos2(rect.left() + seg_w * (i + 1) as f32, rect.bottom()),
                    );
                    if sr.contains(p) { out = Some(i); }
                }
            }
        }
        if (pos - target).abs() > 0.001 { ui.ctx().request_repaint(); }
        out
    }

    /// 待机表情选择器：左侧实时预览 + 名称 + 左右箭头。返回 Some(新编号)。
    fn draw_face_stepper(ui: &mut egui::Ui, painter: &egui::Painter, rect: egui::Rect, cur: u8, id: egui::Id, a: u8, face_frames: &[Option<Vec<egui::TextureHandle>>]) -> Option<u8> {
        let resp = ui.interact(rect, id, egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let kept = face_kept_indices();
        let n = kept.len().max(1);
        let cur_pos = kept
            .iter()
            .position(|&x| x == cur as usize)
            .unwrap_or(0);
        let cur_i = kept.get(cur_pos).copied().unwrap_or(0);
        let tex = face_frames.get(cur_i).and_then(|o| o.as_ref()).and_then(|v| v.get(0));
        let fh = (rect.height() - 6.0).min(16.0).max(8.0);
        let fw = match tex {
            Some(t) => {
                let s = t.size_vec2();
                fh * (s.x / s.y.max(1.0))
            }
            None => fh,
        };
        let pv = egui::pos2(rect.left() + fw * 0.5 + 1.0, rect.center().y);
        draw_robot_face(painter, pv, fh, a, cur_i as u8, 0.0, tex);
        painter.text(
            egui::pos2(rect.left() + fw + 8.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            FACES[cur_i],
            egui::FontId::proportional(12.5),
            egui::Color32::from_rgba_unmultiplied(230, 234, 242, a),
        );
        let arrow_c = egui::Color32::from_rgba_unmultiplied(150, 158, 175, a);
        let lz = egui::Rect::from_center_size(egui::pos2(rect.right() - 36.0, rect.center().y), egui::vec2(22.0, rect.height()));
        let rz = egui::Rect::from_center_size(egui::pos2(rect.right() - 12.0, rect.center().y), egui::vec2(22.0, rect.height()));
        painter.text(lz.center(), egui::Align2::CENTER_CENTER, "\u{2039}", egui::FontId::proportional(17.0), arrow_c);
        painter.text(rz.center(), egui::Align2::CENTER_CENTER, "\u{203A}", egui::FontId::proportional(17.0), arrow_c);
        let mut out = None;
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                if lz.contains(p) {
                    out = Some(kept[(cur_pos + n - 1) % n] as u8);
                } else if rz.contains(p) {
                    out = Some(kept[(cur_pos + 1) % n] as u8);
                }
            }
        }
        out
    }

    /// 待机表情：可视化点选网格（7 列 × 2 行）。每个格子实时播放该表情的逐帧动画，
    /// 点击选中并高亮。返回 Some(新编号)。
    fn draw_face_grid(
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        area: egui::Rect,
        cur: u8,
        id: egui::Id,
        a: u8,
        accent: egui::Color32,
        face_frames: &[Option<Vec<egui::TextureHandle>>],
        uvs: &[Option<(f32, f32, f32, f32)>],
        t: f64,
    ) -> Option<u8> {
        let kept = face_kept_indices();
        let n = kept.len();
        let cols = 7usize;
        let rows = (n + cols - 1) / cols;
        let gap = 5.0f32;
        let cell_w = area.width() / cols as f32;
        let cell_h = (area.height() - (rows as f32 - 1.0) * gap) / rows as f32;
        let metas = Self::face_anim_meta();
        let mut out = None;
        for (pos, &i) in kept.iter().enumerate() {
            let r = pos / cols;
            let c = pos % cols;
            let trect = egui::Rect::from_min_size(
                egui::pos2(area.left() + c as f32 * cell_w + gap * 0.5, area.top() + r as f32 * (cell_h + gap)),
                egui::vec2(cell_w - gap, cell_h),
            );
            let resp = ui
                .interact(trect, id.with(i), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let sel = i == cur as usize;
            // 背景：统一用左侧菜单深色(22,23,28)；选中只靠绿色描边表示，中间不加浅灰层
            painter.rect_filled(trect, 8.0, egui::Color32::from_rgba_unmultiplied(22, 23, 28, a));
            if sel {
                painter.rect_stroke(
                    trect,
                    egui::CornerRadius::same(8),
                    egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), a)),
                    egui::StrokeKind::Inside,
                );
            }
            // 逐帧动画
            let fc = metas.get(i).map(|m| m.2 as usize).unwrap_or(1).max(1);
            let frame = (((t * 1000.0) / 80.0) as usize) % fc;
            let tex = face_frames
                .get(i)
                .and_then(|o| o.as_ref())
                .and_then(|v| v.get(frame));
            let fh_max = (cell_h - 8.0).clamp(16.0, 46.0);
            let (pw, ph) = metas
                .get(i)
                .map(|m| (m.0 as f32, m.1 as f32))
                .unwrap_or((77.0, 80.0));
            let uv = uvs.get(i).and_then(|o| *o);
            // 表情可视区域：顶部留 4px、底部给名称留 14px，整体在区域内垂直居中，避免偏上。
            let av_top = trect.top() + 4.0;
            let av_bot = trect.bottom() - 14.0;
            let av_h = (av_bot - av_top).max(8.0);
            let cy = (av_top + av_bot) * 0.5;
            if let (Some(tx), Some((u0, v0, u1, v1))) = (tex, uv) {
                // 与收起态保持一致的渲染：按内容包围盒裁剪 + 两眼间距收拢
                let content_aspect =
                    ((pw * (u1 - u0)) / (ph * (v1 - v0)).max(1.0)).max(0.2);
                let mut draw_h = fh_max.min(av_h);
                let mut fw = draw_h * content_aspect;
                let maxw = (trect.width() - 6.0).max(8.0);
                if fw > maxw {
                    fw = maxw;
                    draw_h = fw / content_aspect;
                }
                draw_face_img_cropped(
                    painter,
                    egui::pos2(trect.center().x, cy),
                    fw,
                    draw_h,
                    a,
                    tx,
                    (u0, v0, u1, v1),
                    FACE_EYE_GAP_REDUCE,
                );
            } else {
                let draw_h = fh_max.min(av_h);
                draw_robot_face(painter, egui::pos2(trect.center().x, cy), draw_h, a, i as u8, 0.0, tex);
            }
            // 名称
            let lcol = if sel {
                egui::Color32::from_rgba_unmultiplied(235, 238, 245, a)
            } else {
                egui::Color32::from_rgba_unmultiplied(150, 158, 175, a)
            };
            painter.text(
                egui::pos2(trect.center().x, trect.bottom() - 9.0),
                egui::Align2::CENTER_CENTER,
                FACES[i],
                egui::FontId::proportional(9.0),
                lcol,
            );
            if resp.clicked() {
                out = Some(i as u8);
            }
        }
        out
    }

    // ---------------- 设置面板 ----------------
    fn settings_ui(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            self.confirm_open = false;
            self.settings_hwnd = 0;
            self.settings_shown = false;
            return;
        }
        apply_style(ctx, true);
        let face_idx0 = self.cfg.idle_face as usize;
        self.ensure_face_frames(ctx, face_idx0);
        let c = palette(true);
        let focus = self.focus_settings;
        let mut do_hide = false;
        let mut do_exit = false;
        let mut confirm_open = self.confirm_open;

        // 无系统标题栏、可拖动、自绘；圆角交给 Win11 DWM，避免透明窗口四角发黑?
        let first = !self.settings_shown;
        let mut builder = egui::ViewportBuilder::default()
            .with_title("WinIsle 设置")
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size([440.0, 320.0])
            .with_min_inner_size([440.0, 320.0])
            .with_max_inner_size([440.0, 320.0]);
        // 首帧先隐藏，?OS 定位+定尺寸后?reveal_centered 显示，避?先闪一下再居中"?
        if first {
            builder = builder.with_visible(false);
        }

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("winisle_settings"),
            builder,
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    confirm_open = true;
                }
                if focus {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }

                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        let rect = ui.max_rect();
                        let p = ui.painter().clone();
                        p.rect_filled(rect, egui::CornerRadius::same(0), c.bg);

                        // 标题栏：绿点 + 名称
                        p.circle_filled(egui::pos2(rect.min.x + 22.0, rect.min.y + 26.0), 4.5, c.accent);
                        p.text(
                            egui::pos2(rect.min.x + 32.0, rect.min.y + 26.0),
                            egui::Align2::LEFT_CENTER,
                            "WinIsle",
                            egui::FontId::proportional(15.0),
                            c.text,
                        );

                        let header = egui::Rect::from_min_max(
                            egui::pos2(rect.min.x, rect.min.y),
                            egui::pos2(rect.max.x - 48.0, rect.min.y + 48.0),
                        );
                        let drag = ui.interact(header, egui::Id::new("winisle_title_drag"), egui::Sense::drag());
                        if drag.drag_started() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                        }

                        let btn = egui::Rect::from_min_size(
                            egui::pos2(rect.max.x - 42.0, rect.min.y + 12.0),
                            egui::vec2(30.0, 30.0),
                        );
                        let cresp = ui.interact(btn, egui::Id::new("winisle_close"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                        if cresp.hovered() {
                            p.rect_filled(btn, egui::CornerRadius::same(8), egui::Color32::from_rgb(238, 238, 240));
                        }
                        let xcol = if cresp.hovered() {
                            egui::Color32::from_rgb(60, 60, 66)
                        } else {
                            c.muted
                        };
                        p.text(btn.center(), egui::Align2::CENTER_CENTER, "✕", egui::FontId::proportional(16.0), xcol);
                        if cresp.clicked() {
                            confirm_open = true;
                        }

                        let cx = rect.center().x;
                        p.text(
                            egui::pos2(cx, rect.min.y + 148.0),
                            egui::Align2::CENTER_CENTER,
                            "WinIsle",
                            egui::FontId::proportional(34.0),
                            c.text,
                        );
                        p.text(
                            egui::pos2(cx, rect.min.y + 196.0),
                            egui::Align2::CENTER_CENTER,
                            "正在系统托盘中运行",
                            egui::FontId::proportional(14.0),
                            c.muted,
                        );
                        p.text(
                            egui::pos2(cx, rect.min.y + 262.0),
                            egui::Align2::CENTER_CENTER,
                            "点右上角关闭可选择隐藏或退出",
                            egui::FontId::proportional(11.5),
                            c.muted2,
                        );
                    });

                // 关闭询问 —?应用内小模态（点空白处 / Esc 可取消）
                if confirm_open {
                    let resp = egui::Modal::new(egui::Id::new("winisle_confirm_modal"))
                        .frame(
                            egui::Frame::NONE
                                .fill(c.bg)
                                .inner_margin(egui::Margin::same(18))
                                .corner_radius(egui::CornerRadius::same(14))
                                .stroke(egui::Stroke::new(1.0_f32, c.line)),
                        )
                        .show(ctx, |ui| {
                            ui.set_width(240.0);
                            ui.vertical_centered(|ui| {
                                ui.label(egui::RichText::new("关闭 WinIsle").size(16.0).strong().color(c.text));
                                ui.add_space(6.0);
                                ui.label(egui::RichText::new("隐藏到托盘，还是彻底退出？").size(12.0).color(c.muted));
                                ui.add_space(14.0);
                                ui.horizontal(|ui| {
                                    let total = 96.0 * 2.0 + 10.0;
                                    let pad = ((ui.available_width() - total) / 2.0).max(0.0);
                                    ui.add_space(pad);
                                    if ui
                                        .add_sized(
                                            [96.0, 36.0],
                                            egui::Button::new(egui::RichText::new("隐藏到托盘").size(12.5).color(c.text))
                                                .fill(c.card2)
                                                .corner_radius(egui::CornerRadius::same(10)),
                                        )
                                        .clicked()
                                    {
                                        do_hide = true;
                                        confirm_open = false;
                                    }
                                    ui.add_space(10.0);
                                    if ui
                                        .add_sized(
                                            [96.0, 36.0],
                                            egui::Button::new(egui::RichText::new("彻底退出").size(12.5).color(c.danger))
                                                .fill(c.danger_dim)
                                                .corner_radius(egui::CornerRadius::same(10)),
                                        )
                                        .clicked()
                                    {
                                        do_exit = true;
                                        confirm_open = false;
                                    }
                                });
                            });
                        });
                    if resp.should_close() {
                        confirm_open = false;
                    }
                }
            },
        );

        // ?Win11 DWM 给这个窗口加原生圆角（彻底去掉四角发黑）
        if !self.settings_shown {
            let h = platform::find_process_window("WinIsle 设置");
            if h != 0 {
                platform::set_round_corners(h);
                platform::reveal_centered(h);
                self.settings_hwnd = h;
                self.settings_shown = true;
            }
        }

        self.focus_settings = false;
        self.confirm_open = confirm_open;
        if do_exit {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            return;
        }
        if do_hide {
            self.settings_open = false;
            self.confirm_open = false;
            self.settings_hwnd = 0;
            self.settings_shown = false;
        }
    }
}

impl eframe::App for IslandApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // 仅调试：WINISLE_PPP=<f> 强制像素比（用于无鼠标截图验证下的稳定渲染）
        if let Ok(v) = std::env::var("WINISLE_PPP") {
            if let Ok(ppp) = v.trim().parse::<f32>() {
                ctx.set_pixels_per_point(ppp);
            }
        }
        // 仅调试：WINISLE_IDLE_FACE=<n> 指定当前选中的待机表情（用于无鼠标截图验证选中描边）
        if let Ok(v) = std::env::var("WINISLE_IDLE_FACE") {
            if let Ok(n) = v.trim().parse::<u8>() {
                self.cfg.idle_face = n;
            }
        }
        let _pf_t0 = std::time::Instant::now();
        // [FIX] egui 的 request_repaint_after 会从每个 delay 里减去 input.predicted_dt
        // （egui context.rs:187-190），而 eframe 从不设置 predicted_dt，它一直是默认的
        // 1/60s≈16.67ms（egui input.rs:94）。于是我们 request_repaint_after(16ms) 被算成
        // 16-16.67 → 饱和为 0 → 每帧立即重绘 → GUI 主线程 220fps 忙循环、吃满 1 核。
        // 把 predicted_dt 归零后，delay 会被如实使用（不再被偷偷减掉）。
        ctx.input_mut(|i| i.predicted_dt = 0.0);
        self.perf_region_ms = 0.0;
        self.perf_netease_ms = 0.0;
        self.poll_update();
        self.poll_weather();
        self.poll_city_search();
        self.ensure_window(ctx, frame);
        // 只有城市搜索页需要键盘输入时才临时允许窗口被激活；普通展开/收起一律不抢焦点。
        let want_activatable = self.w_city_search;
        if want_activatable != self.w_activatable_last {
            self.w_activatable_last = want_activatable;
            platform::set_activatable(self.hwnd, want_activatable);
            if want_activatable {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
        platform::ensure_erase_guard(self.hwnd);
        let _pf_t_win = std::time::Instant::now();
        // 久置后第一次刷新前：先把客户区同步刷成深色，盖住系统重绘那一帧可能露出的白底。
        // 这是"预填充背景"（double-buffer 的常用做法），不是简单改色——它保证那一帧本来就是深色。
        let _gap = self.last_tick.elapsed();
        self.last_tick = std::time::Instant::now();
        // 首次加载原型唱机贴图（播放/暂停两态）
        if self.tt_play.is_none() {
            self.tt_play = Some(make_tex(ctx, "tt_play", include_bytes!("../assets/tt_play.rgba"), 540, 540));
            self.tt_pause = Some(make_tex(ctx, "tt_pause", include_bytes!("../assets/tt_pause.rgba"), 540, 540));
        }
        if self.tt_base.is_none() {
            self.tt_base = Some(make_tex(ctx, "tt_base", include_bytes!("../assets/tt_base.rgba"), 540, 540));
            self.tt_arm0 = Some(make_tex(ctx, "tt_arm0", include_bytes!("../assets/tt_arm.rgba"), 540, 540));
        }

        self.refresh_monitor(ctx);
        let _pf_t_a0 = std::time::Instant::now();

        // 音乐模块：主线程非阻塞推进 SMTC 状态机，取最新快照
        let _pf_t_snap0 = std::time::Instant::now();
        self.music = self.tracker.snapshot();
        let _pf_snap_ms = _pf_t_snap0.elapsed().as_secs_f32() * 1000.0;
        // 记录本次快照是否真的来自 SMTC（否则后面的窗口标题兜底会覆盖它）
        self.from_smtc = self.music.has_session && !self.music.title.is_empty();
        // 兜底 0（汽水优先 CDP）：汽水在播时始终用 CDP 快照校正，SMTC 仅作元数据兜底。
        // 汽水 SMTC 无歌词、进度常为静止值，CDP 才能拿到真实歌名/进度/逐字歌词。
        let app_l = self.music.app_id.to_ascii_lowercase();
        let cur_is_soda = app_l.contains("汽水") || app_l.contains("soda") || app_l.contains("qishui");
        // 汽水可用性探测放到后台线程（进程枚举 + CDP 探测约 20–35ms，绝不能放主线程每秒做，
        // 否则每秒掉一两帧，动画看起来就是“卡一下、不丝滑”）。
        if self.soda_live_rx.is_none() && self.soda_live_at.elapsed() >= Duration::from_millis(1500) {
            self.soda_live_at = Instant::now();
            let (tx, rx) = std::sync::mpsc::channel();
            self.soda_live_rx = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(crate::soda_patch::soda_running() && crate::soda_patch::cdp_up());
            });
        }
        if let Some(rx) = &self.soda_live_rx {
            match rx.try_recv() {
                Ok(v) => {
                    self.soda_live_cached = v;
                    self.soda_live_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.soda_live_rx = None;
                }
            }
        }
        let soda_live = self.soda_live_cached;
        let soda_ctx = cur_is_soda || (self.music.title.is_empty() && soda_live);
        crate::soda_now::request(soda_ctx);
        if cur_is_soda || self.music.title.is_empty() {
            if let Some((nm, ar, dur, pos, playing, cov)) = crate::soda_now::snapshot() {
                // CDP 在歌单/场景页有时会短暂返回“上一首”的旧歌名；若 SMTC 已有更新的歌名，
                // 就不要用 CDP 的旧名去覆盖，否则标题会来回跳 → 歌词被反复清空重载（“渲染了又卡回去又重新渲染”）。
                let smtc_has_title = self.from_smtc && !self.music.title.is_empty();
                let stale_name = smtc_has_title && nm.trim() != self.music.title.trim();
                if !nm.trim().is_empty() && !stale_name {
                    self.music.has_session = true;
                    self.music.playing = playing;
                    self.music.title = nm;
                    self.music.artist = ar;
                    if self.music.app_id.is_empty() || cur_is_soda {
                        self.music.app_id = "汽水音乐".to_string();
                    }
                    if dur > 1.0 {
                        self.music.duration = dur;
                    }
                    self.music.position = pos;
                    self.music.has_position = pos > 0.0;
                    self.from_smtc = true;
                    // 歌单/场景模式下从底部播放条取到的封面：变了才重建纹理
                    if let Some(cb) = cov {
                        use std::hash::{Hash, Hasher};
                        let mut hh = std::collections::hash_map::DefaultHasher::new();
                        cb.as_slice().hash(&mut hh);
                        let h = hh.finish();
                        if h != self.soda_cover_hash || self.cover_tex.is_none() {
                            if let Ok(img) = image::load_from_memory(cb.as_slice()) {
                                let rgba = img.to_rgba8();
                                let (w, hgt) = rgba.dimensions();
                                let ci = egui::ColorImage::from_rgba_unmultiplied(
                                    [w as usize, hgt as usize],
                                    rgba.as_raw(),
                                );
                                self.cover_tex = Some(
                                    ctx.load_texture("cover", ci, egui::TextureOptions::LINEAR),
                                );
                                self.soda_cover_hash = h;
                            }
                        }
                    }
                }
            }
        }
        let _pf_t_a1 = std::time::Instant::now();
        // 不依赖上一帧可能滞后的窗口原点，保证能精确贴到最?最右角?
        {
            let a = self.music.app_id.to_ascii_lowercase();
            let is_netease = self.music.has_session
                && (a.contains("cloudmusic") || a.contains("netease") || a.contains("网易云"));
            platform::set_netease_reader_enabled(is_netease);
        }
        // 汽水 CDP 补丁自检：汽水自动更新会覆盖 app.asar 里的调试端口补丁导致歌词失效。
        // 这里每 15s 检查一次，发现没补丁就自动重打（播放中只打文件不重启，暂停/未播放时自动重启生效）。
        if !self.soda_boot_checked && std::env::var_os("WINISLE_NOSODA").is_none() {
            self.soda_boot_checked = true;
            // 启动时若汽水没在运行，直接把补丁补好（运行中会被独占锁，交给下面的 5s 自检）。
            crate::soda_patch::boot_check();
        }
        if self.last_soda_heal.elapsed() > Duration::from_secs(15)
            && std::env::var_os("WINISLE_NOSODA").is_none()
            && !self
                .soda_tick_busy
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            self.last_soda_heal = Instant::now();
            let aid = self.music.app_id.to_lowercase();
            let is_soda = self.music.has_session
                && (aid.contains("汽水") || aid.contains("soda") || aid.contains("qishui"));
            if is_soda {
                let playing = self.music.playing;
                let flag = self.soda_tick_busy.clone();
                std::thread::spawn(move || {
                    crate::soda_patch::tick(playing);
                    flag.store(false, std::sync::atomic::Ordering::Relaxed);
                });
            } else {
                self.soda_tick_busy
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
        // 调试：WINISLE_DEMO="歌名|歌手" 时注入一条虚拟曲目，用于无播放器环境验证界面/动画
        if let Ok(spec) = std::env::var("WINISLE_DEMO") {
            let mut it = spec.splitn(2, '|');
            let t = it.next().unwrap_or("Demo").to_string();
            let ar = it.next().unwrap_or("Demo Artist").to_string();
            self.music.has_session = true;
            self.music.title = t;
            self.music.artist = ar;
            self.music.app_id = std::env::var("WINISLE_DEMO_APP").unwrap_or_else(|_| "cloudmusic.exe".into());
            let demo_paused = std::env::var_os("WINISLE_DEMO_PAUSED").is_some();
            self.music.playing = !demo_paused;
            self.music.position = 42.0;
            self.music.duration = 240.0;
            self.music.has_position = true;
            if let Ok(p) = std::env::var("WINISLE_POS") {
                if let Ok(v) = p.parse::<f64>() {
                    self.music.position = v;
                }
            }
            self.from_smtc = false;
            self.ui_playing = !demo_paused;
        }
        // 以 SMTC 真实 Position 作为歌词/进度时间基准（最小化到托盘也有效）。
        // 关键点：像汽水音乐这类播放器会【一直上报同一个静止 position】（haspos=true 但数值不变），
        // 若拿它反复校准，会把不断外推的进度一次次拉回原点，表现为“歌词来回跳/跟不上”。
        // 所以只有当「上报值真的变了」（播放推进到位、或拖动进度）时才校准；数值没变就继续自我外推。
        if self.music.has_position && self.music.position > 0.0 {
            let rep = self.music.position;
            let prev = self.smtc_rep_pos;
            let rep_changed = (rep - prev).abs() > 0.05;
            if !self.spos_valid {
                self.spos = rep;
                self.spos_at = Instant::now();
                self.spos_valid = true;
                self.smtc_rep_pos = rep;
            } else if rep_changed {
                let cur = self.current_pos();
                let d = rep - cur;
                // 向前明显领先（>=0.5s，粗量化/初次偏旧）就校准；
                // 向后只有「上报值真的倒退了」（拖动进度/回退），而不是“推进得比外推慢”时才回退——
                // 迷你?
                let rewound = rep + 0.5 < prev;
                if d > 0.5 || (d < -1.5 && rewound) {
                    self.spos = rep;
                    self.spos_at = Instant::now();
                }
                self.smtc_rep_pos = rep;
            }
            // 数值未变：什么都不做，让 current_pos() 继续按自身时钟外推（静止上报≠暂停）
        }
        // 暂停期间冻结：不断刷新采样时刻，恢复播放时不会把暂停时长算进进度。
        if self.spos_valid && !self.is_playing() {
            self.spos_at = Instant::now();
        }
        // 临时诊断（WINISLE_PDBG=1）：每秒记录进度/播放状态关键变量，定位“进度不走”。
        if std::env::var_os("WINISLE_PDBG").is_some() {
            use std::sync::atomic::{AtomicU64, Ordering};
            static LAST: AtomicU64 = AtomicU64::new(0);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if now_ms.saturating_sub(LAST.load(Ordering::Relaxed)) >= 1000 {
                LAST.store(now_ms, Ordering::Relaxed);
                use std::io::Write;
                let ci = self.cur_lyric_idx();
                let lt = ci
                    .and_then(|i| self.lyr_lines.get(i))
                    .map(|l| l.t)
                    .unwrap_or(-1.0);
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(crate::paths::file("pdbg.log"))
                {
                    let _ = writeln!(
                        f,
                        "pos={:.2} spos={:.2} spos_valid={} since_spos={:.2} playing={} from_smtc={} m.playing={} m.pos={:.2} haspos={} ci={:?} ci_anim={:.2} lyr_t={:.2} nlyr={} title={:?} soda={} app={:?} sprx={} trial={} hint={} clipstart={:.2} clipdur={:.2} full={:.2} trstart={:.2}",
                        self.current_pos(),
                        self.spos,
                        self.spos_valid,
                        self.spos_at.elapsed().as_secs_f64(),
                        self.is_playing(),
                        self.from_smtc,
                        self.music.playing,
                        self.music.position,
                        self.music.has_position,
                        ci,
                        self.lyr_ci_anim,
                        lt,
                        self.lyr_lines.len(),
                        self.music.title,
                        self.app_is_soda(),
                        self.music.app_id,
                        self.soda_prog_rx.is_some(),
                        self.soda_trial_active,
                        self.soda_trial_hint,
                        self.soda_clip_start,
                        self.soda_clip_dur,
                        self.soda_full_dur,
                        self.trial_start,
                    );
                }
            }
        }
        // 兜底：若 SMTC 拿不到（本机 SMTC 常超时），改从播放器窗口标题抓 “歌名 - 歌手”（如网易云）。
        // EnumWindows + 进程路径查询很慢（数十 ms），绝不能放主线程每次做——放后台线程，结果异步取回。
        if self.media_title_rx.is_none()
            && self.music.title.is_empty()
            && !soda_live
            && self.media_title_at.elapsed() >= Duration::from_millis(800)
        {
            self.media_title_at = Instant::now();
            let (tx, rx) = std::sync::mpsc::channel();
            self.media_title_rx = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(platform::media_title_fallback());
            });
        }
        if let Some(rx) = &self.media_title_rx {
            match rx.try_recv() {
                Ok(v) => {
                    self.media_title_cache = v;
                    self.media_title_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.media_title_rx = None;
                }
            }
        }
        if !self.music.title.is_empty() {
            self.media_title_cache = None;
        }
        if self.music.title.is_empty() {
            if let Some((exe, t, a)) = self.media_title_cache.clone() {
                self.music.has_session = true;
                self.music.playing = true;
                self.music.title = t;
                self.music.artist = a;
                self.music.app_id = exe;
                self.spos_valid = false;
            }
        }
        let _pf_t_a2 = std::time::Instant::now();
        // 记录“好标题”：标题稳定时才刷新；短暂读不到时用最近一次的好标题顶住（最�?15s），避免闪烁
        if !self.music.title.is_empty() {
            self.last_title = Some((
                self.music.app_id.clone(),
                self.music.title.clone(),
                self.music.artist.clone(),
            ));
            self.last_title_at = Instant::now();
        } else if let Some((exe, t, a)) = self.last_title.clone() {
            if self.music.title.is_empty() && self.player_is_running() {
                self.music.has_session = true;
                self.music.playing = true;
                self.music.title = t;
                self.music.artist = a;
                self.music.app_id = exe;
            }
        }
        // 只要拿到了歌�?歌手（无论来�?SMTC 还是窗口标题兜底），就视为有会话
        if !self.music.title.is_empty() {
            self.music.has_session = true;
        }
        // 播放器是否还在：用后台缓存（~1s 更新）。进程已退出就尽快淡出音乐模块，
        // 不再死等“连续 2 秒空标题”——否则收起态会先僵住、再一下子把音乐模块剔除（看着“卡一下”）。
        // 400ms 去抖，避免进程枚举/标题瞬抖导致误判。
        let player_running_now = if let Some(ms) = std::env::var("WINISLE_PLAYER_GONE_MS")
            .ok()
            .and_then(|s| s.parse::<u128>().ok())
        {
            // 调试：模拟“播放器在某时刻退出”，用于验证收起态剔除音乐模块的淡出是否顺滑。
            static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let t0 = *T0.get_or_init(Instant::now);
            t0.elapsed().as_millis() < ms
        } else {
            self.player_is_running() || std::env::var_os("WINISLE_DEMO").is_some()
        };
        let want_music = !self.music.title.is_empty() && player_running_now;
        let mut music_present = want_music;
        if !want_music {
            // 只有“此前确实有音乐”时才做 400ms 丢失去抖（防止歌曲元数据/进程枚举瞬抖导致模块闪烁）。
            // 冷启动时 music_was_present 为 false → 不授予这段“保持显示”的宽限；否则会在真正读到音乐
            // 之前先把收起条展开、再缩回、再展开（用户反馈的“展开后又缩回去又展开一次”）。
            if self.music_was_present {
                let since = *self.music_lost_since.get_or_insert_with(Instant::now);
                if since.elapsed() < Duration::from_millis(400) {
                    music_present = true;
                }
            } else {
                self.music_lost_since = None;
            }
        } else {
            self.music_lost_since = None;
        }
        self.music_was_present = music_present;
        // 不依赖上一帧可能滞后的窗口原点，保证能精确贴到最?最右角?
        {
            let dt = ctx.input(|i| i.stable_dt).min(0.05);
            let target_amt = if music_present { 1.0f32 } else { 0.0 };
            let k = 1.0 - (-7.0 * dt).exp();
            self.music_amt += (target_amt - self.music_amt) * k;
            if (self.music_amt - target_amt).abs() < 0.004 {
                self.music_amt = target_amt;
            } else {
                // [FIX] 用 16ms 延迟重绘（而非立即 request_repaint）驱动平滑收敛：
                // 立即重绘会绕过帧预算、把「载入曲目/切歌/暂停切换」时的淡入淡出顶到 ~200fps 空转。
                // 幅度按真实 dt 推进，改 16ms 后收敛时长不变（≈0.8s），只是帧数降到 60fps。
                ctx.request_repaint_after(Duration::from_millis(16));
            }
            // 「音乐在运动」判定：正在播放，或 music_amt 仍在淡入淡出 → 需要 60fps；
            // 暂停且已稳定（amt 到位且未在播放）→ false，允许收起态降到 16.7fps 省 CPU。
            self.music_motion = (self.music_amt - target_amt).abs() >= 0.004
                || (self.music_amt > 0.01 && self.is_playing());
            // 调试钩子：强制按“无音乐”渲染收起态（用于截图表情）
            if std::env::var("WINISLE_FACE").is_ok() {
                self.music_amt = 0.0;
            }
            // 淡出结束后再真正清掉歌词/封面与残留音乐状态
            if !music_present && self.music_amt < 0.02 {
                self.lyr_lines.clear();
                self.lyr_key.clear();
                self.lyr_dur = 0.0;
                self.lyr_done = false;
                self.trial_start = 0.0;
                self.trial_probe = None;
                self.soda_full_dur = 0.0;
                self.soda_clip_start = 0.0;
                self.soda_clip_dur = 0.0;
                self.soda_trial_active = false;
                self.soda_trial_hint = false;
                self.soda_last_p = -1.0;
                self.cover_tex = None;
                self.music = smtc::Track::default();
                self.media_title_cache = None;
                self.last_title = None;
                self.override_play = None;
                self.pause_pending_at = None;
                self.spos_valid = false;
                self.clock_playing = false;
                self.ui_playing = true;
            }
        }
        // 调试（WINISLE_WDBG=1）：逐帧记录 music_amt 与收起态目标宽度，验证“剔除音乐模块”是否平滑收窄。
        if std::env::var_os("WINISLE_WDBG").is_some() {
            use std::io::Write;
            static LAST_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if now_ms.saturating_sub(LAST_MS.load(std::sync::atomic::Ordering::Relaxed)) >= 16 {
                LAST_MS.store(now_ms, std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(crate::paths::file("wdbg.log"))
                {
                    let _ = writeln!(f, "ms={} amt={:.4} w={:.2}", now_ms, self.music_amt, self.collapsed_w());
                }
            }
        }
        // 网易云专用兜底：仅当 SMTC 没有提供可用时间轴时，才读进程内存里的真实位置。
        // （将来播放器通过 SMTC 上报了进度，就自动优先用 SMTC，不再依赖内存偏移。）
        let smtc_timeline_ok = self.from_smtc
            && self.music.has_position
            && self.music.duration > 20.0
            && self.music.position > 0.0;
        let is_netease_ctx = {
            let a = self.music.app_id.to_ascii_lowercase();
            a.contains("cloudmusic") || a.contains("netease") || a.contains("网易云")
        };
        if is_netease_ctx && !smtc_timeline_ok && self.player_running_cached && self.netease_at.elapsed() >= Duration::from_millis(450) {
            self.netease_at = Instant::now();
            let _ne_t = std::time::Instant::now();
            let _ne_res = platform::netease_pos_dur().or_else(platform::netease_posdur_cached);
            self.perf_netease_ms = _ne_t.elapsed().as_secs_f32() * 1000.0;
            // 记录旧进度，用于判断“是否真的在走”。但要区分“暂停不动”和“用户拖动进度条(seek)”：
            // seek 时位置会突变（前后都可能，通常跨度大），必须当作在播放/保持原状态并立即对齐，
            // 不能误判成暂停，否则拖完进度后歌词会“愣一下才追上来”。
            let now = std::time::Instant::now();
            let prev_valid = self.spos_valid;
            let prev_raw = self.netease_prev_raw;
            if let Some((p, d)) = _ne_res {
                self.netease_dur = d;
                // 网易云进程内存里的 position 是量化的（约 0.5~1s 才跳一次），偶发还会在两个
                // 近邻值间抖动。先从原始值判断“有没有真的前进 / 是否自身倒退了（seek）”。
                let raw_moved = prev_raw < 0.0 || (p - prev_raw).abs() >= 0.02;
                let raw_rewound = prev_raw >= 0.0 && p + 0.5 < prev_raw;
                let raw_jumped_fwd = prev_raw >= 0.0 && p - prev_raw > 2.0;
                if raw_moved {
                    self.netease_prev_raw = p;
                    self.netease_raw_move_at = now;
                }
                // 播放态：只有“原始位置在 1.6s 内持续不变”才判定为暂停。
                // 不能拿单次 450ms 采样是否推进来判断，否则量化期间会被误判暂停，
                // 造成自维护时钟冻结/回跳，进度显示便在 2.30 / 2.31 之间来回跳。
                if raw_jumped_fwd {
                    // 位置大跳（seek）：保持当前播放态，仅对齐位置（下面处理）
                } else if raw_moved {
                    self.ui_playing = true;
                    self.music.playing = true;
                    self.clock_playing = true;
                } else if self.netease_raw_move_at.elapsed() > Duration::from_millis(1600) {
                    self.ui_playing = false;
                    self.music.playing = false;
                    self.clock_playing = false;
                }
                // 时间轴：以自维护时钟为主，原始位置只做“有界修正”；
                // -2.0..0.25 秒的落后/±0.01 抖动一律忽略，保证进度单调、不倒退。
                if !prev_valid {
                    self.spos = p;
                    self.spos_at = now;
                    self.spos_valid = true;
                } else {
                    let cur = self.current_pos();
                    let diff = p - cur;
                    if diff > 2.0 || raw_rewound {
                        // 用户拖动进度（大跳，或原始值自身倒退）：立即对齐
                        self.spos = p;
                        self.spos_at = now;
                    } else if diff > 0.25 {
                        // 原始位置更靠前：半程追赶，平滑不跳
                        self.spos = cur + diff * 0.5;
                        self.spos_at = now;
                    } else if diff < -2.0 {
                        // 主时钟明显超前（极少见）：拉回
                        self.spos = p;
                        self.spos_at = now;
                    }
                    self.spos_valid = true;
                }
                if self.music.app_id.is_empty() {
                    self.music.app_id = "cloudmusic.exe".to_string();
                }
            } else if self.music.app_id.to_ascii_lowercase().contains("cloudmusic") {
                if let Some(f) = platform::netease_frac_cached() {
                    let d = self.duration_est();
                    if d > 1.0 {
                        self.spos = f * d;
                        self.spos_at = Instant::now();
                        self.spos_valid = true;
                    }
                }
            }
        }
        // 汽水：SMTC 的 position / playing 都会偶发失真（position 冻结、playing 闪烁），
        // 直接读汽水播放器界面：进度取底部条 `.slider-fill`（亚秒级真实推进），
        // 播放态取中间大按钮图标 —— 两者都比 SMTC 可靠。这里不再要求 from_smtc，只要在放汽水就采样。
        if self.app_is_soda() {
            if self.soda_pos_at.elapsed() >= Duration::from_millis(400)
                && self.soda_prog_rx.is_none()
                && std::env::var_os("WINISLE_NOSODAPROG").is_none()
            {
                self.soda_pos_at = Instant::now();
                let dur = if self.music.duration > 20.0 {
                    self.music.duration
                } else {
                    self.duration_est()
                };
                let (tx, rx) = std::sync::mpsc::channel();
                self.soda_prog_rx = Some(rx);
                std::thread::spawn(move || {
                    let pr = crate::cdp_lyric::fetch_progress(dur);
                    let pl = crate::cdp_lyric::fetch_play_state().map(|(p, _, _)| p);
                    let tm = crate::cdp_lyric::fetch_trial_meta();
                    let _ = tx.send((pr, pl, tm));
                });
            }
            let recv = self.soda_prog_rx.as_ref().map(|rx| rx.try_recv());
            match recv {
                Some(Ok((pr, pl, tm))) => {
                    self.soda_prog_rx = None;
                    if let Some(m) = tm {
                        // 组件树里可能残留上一首的 track；曲名对不上就不要用它的时长/试听信息。
                        let a = m.track_name.trim();
                        let b = self.music.title.trim();
                        let same_track = a.is_empty()
                            || b.is_empty()
                            || a == b
                            || a.contains(b)
                            || b.contains(a);
                        if same_track {
                            self.soda_full_dur = m.full_duration;
                            self.soda_clip_start = m.clip_start;
                            self.soda_clip_dur = m.clip_duration;
                            self.soda_trial_hint = m.limited_free;
                        }
                    }
                    let prev_pl = self.soda_playing;
                    let mut force_sync = false;
                    // 暂停请求后 3 秒内视作“待确认”：这段时间播放器进度可能仍在推进，
                    // 若照常做软校正会先冲过头、等确认暂停再弹回（歌词“滚一下又回去”）。
                    let pause_pending = self
                        .pause_pending_at
                        .map(|t| t.elapsed() < Duration::from_secs(3))
                        .unwrap_or(false);
                    if let Some(p) = pl {
                        if prev_pl != Some(p) {
                            force_sync = true; // 播放/暂停切换时精确对齐一次
                        }
                        self.soda_playing = Some(p);
                        // 与真实播放态一致地同步 music.playing，供 UI 图标/呼吸灯使用
                        self.music.playing = p;
                    }
                    if let Some((p, tot, _age)) = pr {
                        // DOM 进度值是「上一次跳变瞬间」的进度；age 是距跳变的秒数，
                        // 因此当前真实进度 ≈ p + age（播放速率按 1.0 计）。
                        let p_now = p.min(tot.max(1.0));
                        self.soda_dur = tot;
                        // 汽水非会员试听：底部播放条只显示 30s 片段时长，而整首歌时长来自 track.duration。
                        // 判定为片段播放时，把片段起点记到 trial_start，让歌词和进度条都按整首歌的绝对时间显示。
                        let full = if self.soda_full_dur > 1.0 {
                            self.soda_full_dur
                        } else if self.music.duration > 1.0 {
                            self.music.duration
                        } else {
                            0.0
                        };
                        let is_trial = self.soda_trial_hint
                            && full > 30.0
                            && self.soda_clip_dur >= 10.0
                            && self.soda_clip_dur < full * 0.95;
                        self.soda_trial_active = is_trial;
                        if is_trial {
                            self.trial_start = self.soda_clip_start.max(0.0) as f32;
                        } else {
                            self.trial_start = 0.0;
                        }
                        // 汽水播放条的 DOM 进度约 1 秒才跳一次；fetch_progress 每 400ms 轮询，
                        // 同一个旧值会被读到 2~3 次。若把“旧值”当成落后去回拉，自维护时钟就会
                        // 每 1~2 秒被拉回去一次，歌词已渲染的字会闪一下又重新渲染。
                        // 因此只在进度值“确实变了”时校正，重复值一律忽略（时钟本来就在实时推进）。
                        let changed = (p - self.soda_last_p).abs() > 0.002;
                        self.soda_last_p = p;
                        let cur = self.current_pos();
                        let diff = p_now - cur;
                        if self.soda_playing == Some(false) {
                            // 已暂停：进度必须精确停在播放器真实位置，绝不做软校正（否则会
                            // 从 2:31 一点点滑回 2:30，看起来就是“2:30/2:31 来回跳”）。
                            self.spos = p;
                            self.spos_at = Instant::now();
                            self.spos_valid = true;
                            self.pause_pending_at = None;
                        } else if pause_pending {
                            // 已请求暂停、播放器还没确认：保持当前位置不动，等确认后再精确对齐
                        } else if force_sync || diff.abs() > 2.0 {
                            // 播放态切换 / 拖动跳变：精确对齐一次
                            self.spos = p_now;
                            self.spos_at = Instant::now();
                            self.spos_valid = true;
                        } else if changed {
                            // 只在进度值真的跳变时对齐一次（每约 1 秒），且用 p+age 还原到“现在”。
                            // 重复旧值完全忽略，避免把时钟每 400ms 往回拉、造成歌词字闪一下。
                            // 每秒一次的微小修正（通常几十毫秒）不可见，却能消除长期漂移。
                            self.spos = p_now.max(cur);
                            self.spos_at = Instant::now();
                            self.spos_valid = true;
                        }
                    }
                }
                Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                    self.soda_prog_rx = None;
                }
                Some(Err(std::sync::mpsc::TryRecvError::Empty)) | None => {}
            }
        }        // VIP 试听片段起点探测：播放时�?<60s（试听片段）时，扫描网易云内存日志里
        // 本轮播放开始后�?freeTrialInfo.start，把“片段内相对时间”映射到整首歌绝对时间�?
        if self.app_is_soda() {
            // 汽水：不走网易云内存探测；试听起点由上面的 soda 进度分支维护，这里绝不能清零。
            self.trial_probe = None;
        } else if let Some((at, n)) = self.trial_probe {
            if self.music.title.is_empty() || self.netease_dur >= 60.0 {
                self.trial_probe = None;
                self.trial_start = 0.0;
            } else if at.elapsed() >= Duration::from_millis(700) {
                if let Some(s) = platform::netease_trial_start_since(&self.trial_since) {
                    self.trial_start = s;
                    self.trial_probe = None;
                } else if n >= 6 {
                    self.trial_probe = None;
                } else {
                    self.trial_probe = Some((Instant::now(), n + 1));
                }
            }
        }
        // 切歌检测：标题变化即切歌（轻量节流避免重复请求）。
        // 注意：标题「短暂变空」不算切歌——汽水在歌单/场景页时 CDP 偶尔读不到歌名，
        // 若把空标题当成切歌会清空已渲染的歌词 → 表现为“渲染了又卡回去又重新渲染”。只有出现
        // 新的非空标题才真正切歌。
        let title_now = self.music.title.clone();
        let key = title_now.clone();
        if !key.is_empty() && key != self.lyr_key {
            self.lyr_key = key.clone();
            self.lyr_idx_last = -1;
            // 试听起点重置：记下切歌时刻，稍后扫描网易云内存日志里本轮播放的 freeTrialInfo。
            // 若是本次启动后的第一首歌（可能启动时已在播放，日志早于启动时刻），
            // since 留空表示接受内存里最新的一条记录。
            let first_song = self.trial_since.is_empty();
            self.trial_start = 0.0;
            self.soda_full_dur = 0.0;
            self.soda_clip_start = 0.0;
            self.soda_clip_dur = 0.0;
            self.soda_trial_active = false;
            self.soda_trial_hint = false;
            self.soda_last_p = -1.0;
            self.trial_since = if first_song {
                String::new()
            } else {
                platform::local_stamp()
            };
            self.trial_probe = Some((Instant::now(), 0));
            self.lyr_lines.clear();
            self.lyr_done = false;
            self.lyr_retry_n = 0;
            self.lyr_retry_at = Instant::now() - Duration::from_secs(10);
            self.lyr_inflight = false;
            self.cover_tex = None;
            self.pending_title = None;
            self.netease_dur = 0.0;
            // 进度：汽水只用自�?SMTC/CDP 时间；其它播放器优先用网易云内存真实位置
            if self.app_is_soda() {
                self.spos_valid = self.music.has_position && self.music.position > 0.0;
                self.spos = self.music.position;
                self.spos_at = Instant::now();
            } else if let Some((p, d)) = platform::netease_pos_dur() {
                self.netease_dur = d;
                self.spos = p;
                self.spos_at = Instant::now();
                self.spos_valid = true;
            } else {
                self.spos_valid = self.music.has_position && self.music.position > 0.0;
                self.spos = self.music.position;
                self.spos_at = Instant::now();
            }
            self.clock_base = if self.music.position > 0.5 { self.music.position } else { 0.0 };
            self.smtc_rep_pos = if self.music.has_position { self.music.position } else { -1.0 };
            self.clock_start = Instant::now();
            // 播放器常不报 position/playing：只要有歌名就假设在播放，避免进度卡死
            self.clock_playing = if !self.music.title.is_empty() { true } else { self.is_playing() };
            self.prev_playing = self.clock_playing;
            self.ui_playing = true;
            if !self.music.title.is_empty() {
                let dur_now = self.lyric_duration();
                self.lyr_dur = dur_now;
                let is_soda = {
                    let a = self.music.app_id.to_ascii_lowercase();
                    a.contains("汽水") || a.contains("soda") || a.contains("qishui")
                };
                self.lyr_inflight = true;
                let _ = self.lyr_tx.send((key.clone(), self.music.title.clone(), self.music.artist.clone(), dur_now, is_soda, false));
            }
        } else {
            // 同名不同版本纠偏：拿到播放器真实总时长后，若与当初选歌词用的时长差 >4s�?
            // 说明选到了别的版本（时间轴不同，歌词必然对不上）�?按真实时长重新取一次�?
            // 注意：VIP 试听上报�?40s 片段不算“真实总时长”，必须 >=60s 才参与纠偏�?
            if !self.music.title.is_empty()
                && self.netease_dur >= 60.0
                && !self.lyr_lines.is_empty()
                && (self.lyr_dur < 20.0 || (self.netease_dur - self.lyr_dur).abs() > 4.0)
            {
                self.lyr_dur = self.netease_dur;
                let is_soda = {
                    let a = self.music.app_id.to_ascii_lowercase();
                    a.contains("汽水") || a.contains("soda") || a.contains("qishui")
                };
                let _ = self.lyr_tx.send((
                    key.clone(),
                    self.music.title.clone(),
                    self.music.artist.clone(),
                    self.netease_dur,
                    is_soda,
                    false,
                ));
            }
            // 有效播放状态：有可信 position 时用真实状态；否则只要有歌名就假设在播放
            // （很多播放器 SMTC 只有歌名，不给 position/playing，若照它会把进度冻住）
            let playing_eff = if let Some((v, tt)) = self.override_play {
                if Instant::now() < tt {
                    v
                } else if self.spos_valid {
                    self.is_playing()
                } else {
                    !self.music.title.is_empty()
                }
            } else if self.spos_valid {
                self.is_playing()
            } else {
                !self.music.title.is_empty()
            };
            if playing_eff != self.prev_playing {
                let cur = self.current_pos();
                self.clock_base = cur;
                self.clock_start = Instant::now();
                self.clock_playing = playing_eff;
                self.prev_playing = playing_eff;
            }
            // SMTC 若给出可信进度则校准（部分播放器恒为 0，忽略）
            if self.music.position > 0.5 && (self.current_pos() - self.music.position).abs() > 2.5 {
                self.clock_base = self.music.position;
                self.clock_start = Instant::now();
            }
        }
        // 汽水：歌词页往往比切歌晚一两秒才更新，没拿到歌词时自动重试几次
        // （否则会一直停在「歌词加载中」或贴上一首）�?
        if !self.music.title.is_empty()
            && self.lyr_lines.is_empty()
            && !self.lyr_inflight
            && self.lyr_retry_n < 60
            && self.lyr_retry_at.elapsed() > Duration::from_millis(2500)
        {
            let a = self.music.app_id.to_ascii_lowercase();
            if a.contains("汽水") || a.contains("soda") || a.contains("qishui") {
                self.lyr_retry_at = Instant::now();
                self.lyr_retry_n += 1;
                self.lyr_done = false;
                let dur_now = self.lyric_duration();
                self.lyr_inflight = true;
                // 重试用轻量 CDP 探测（不扫内存），歌词页/“纯音乐，请欣赏”迟到也能补上。
                let _ = self.lyr_tx.send((
                    self.lyr_key.clone(),
                    self.music.title.clone(),
                    self.music.artist.clone(),
                    dur_now,
                    true,
                    true,
                ));
            }
        }        // 收集歌词后台线程结果
        while let Ok((k, opt, cov)) = self.lyr_rx.try_recv() {
            self.lyr_inflight = false;
            let same = k == self.lyr_key;
            if same {
                self.lyr_done = true;
            }
            if same || self.lyr_lines.is_empty() {
                self.lyr_lines = opt.unwrap_or_default();
                self.lyr_drag_off = 0.0;
                self.lyr_scroll_target = 0.0;
                self.lyr_ci_anim = -1.0;
                self.lyr_release_at = None;
                self.lyr_return_from = 0.0;
                self.lyr_return_start = None;
                if !same {
                    self.lyr_key = k.clone();
                }
            }
            if let Some(bytes) = cov {
                if let Ok(img) = image::load_from_memory(&bytes) {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
                    self.cover_tex = Some(ctx.load_texture("cover", ci, egui::TextureOptions::LINEAR));
                }
            }
        }

        // 调试：WINISLE_FAKE_LYRICS=1（配合 WINISLE_DEMO）注入一组按时间推进的假歌词，
        // 用于无播放器环境预览收起态“上滚”动画。
        if std::env::var_os("WINISLE_FAKE_LYRICS").is_some() && self.lyr_lines.len() < 2 {
            let demo = [
                "第一句 风继续吹不忍远离",
                "第二句 心里极渴望希望留下伴着你",
                "第三句 风继续吹不忍远离",
                "第四句 心里亦有泪不愿流泪望着你",
                "第五句 过去多少快乐记忆",
                "第六句 何妨与你一起去追",
                "第七句 要将忧郁苦痛洗去",
                "第八句 柔情蜜意我愿记取",
            ];
            let mut v = Vec::new();
            for (i, s) in demo.iter().enumerate() {
                v.push(lyrics::LrcLine {
                    t: 30.0 + i as f64 * 2.5,
                    text: (*s).to_string(),
                    segs: Vec::new(),
                });
            }
            self.lyr_lines = v;
        }

        // 调试：WINISLE_LONG="一句话" 覆盖为单条歌词（放在歌词抓取之后，避免被真实歌词覆盖），
        // 用于验证收起态歌词“自适应宽度 / 跑马灯 / 渐绿”渲染。
        if let Ok(long) = std::env::var("WINISLE_LONG") {
            let changed = self.lyr_lines.len() != 1 || self.lyr_lines.first().map(|l| l.text != long).unwrap_or(true);
            if changed {
                self.lyr_lines = vec![lyrics::LrcLine { t: 0.0, text: long, segs: Vec::new() }];
                self.lyr_idx_last = -1;
            }
            self.lyr_key = self.music.title.clone();
        }

        // 收起态歌词“上滚”动画：检测当前句索引是否变化，换到新一句时触发一次上滚过渡�?
        // 同一切歌内才动画（上一句索引仍有效）；切歌�?lyr_key 分支已把 lyr_idx_last �?-1，故不会误动画�?
        let _pf_t_m2 = std::time::Instant::now();
        let _pf_t_m3 = std::time::Instant::now();
        {
            let cur_idx = self.cur_lyric_idx().map(|i| i as i32).unwrap_or(-1);
            if cur_idx != self.lyr_idx_last {
                let prev = self.lyr_idx_last;
                let valid_prev =
                    prev >= 0 && (prev as usize) < self.lyr_lines.len() && cur_idx >= 0;
                if valid_prev {
                    self.lyr_anim_at = Some(Instant::now());
                    self.lyr_anim_from = Some(prev as usize);
                } else {
                    self.lyr_anim_at = None;
                    self.lyr_anim_from = None;
                }
                self.lyr_idx_last = cur_idx;
            }
            // 动画超时兜底清理（正常情况下 draw 里到点也会清�?
            if let Some(at) = self.lyr_anim_at {
                if at.elapsed() > Duration::from_millis(700) {
                    self.lyr_anim_at = None;
                    self.lyr_anim_from = None;
                }
            }
        }

        // 托盘事件：仅“退出”
        if let Some(t) = &self.tray {
            match t.poll() {
                Some(TrayAction::Exit) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                None => {}
            }
        }

        // 双击再次启动（已在运行时）只静默记录，不自动弹窗；窗口只通过托盘左键打开?
        if let Some(t) = trigger_mtime() {
            if self.last_trigger.map_or(true, |lt| t > lt) {
                self.last_trigger = Some(t);
            }
        }

        let _pf_t_state = std::time::Instant::now();
        self.handle_drag(ctx);
        self.drive_animation(ctx);
        self.step_cb_anim(ctx);
        // 托盘「退出」→ 播放退场动画（淡出 + 上滑），结束后才真正退出进程。
        if self.exit_start.is_none()
            && crate::tray::QUIT_REQUESTED.load(std::sync::atomic::Ordering::SeqCst)
        {
            self.exit_start = Some(Instant::now());
            ctx.request_repaint();
        }
        if let Some(t0) = self.exit_start {
            let p = (t0.elapsed().as_secs_f32() / 0.18).clamp(0.0, 1.0);
            if p >= 1.0 {
                std::process::exit(0);
            }
            ctx.request_repaint();
        }
        // 调试：展开后若指定了 WINISLE_PAGE，定位对应视图
        if let Some(p) = self.dbg_page {
            if self.expanded > 0.9 {
                match p {
                    DbgView::Settings => self.show_settings = true,
                    DbgView::Schedule => self.active = Module::Schedule,
                    DbgView::Weather => self.active = Module::Weather,
                }
            }
        }
        // 完全收起后：保留上一次展开时的页面（模块 + 是否在设置页），只清理临时滚动状态
        if self.dbg_page.is_none() && self.target < 0.5 && self.expanded < 0.02 {
            let ai = match self.active { Module::Music => 0.0f32, _ => 1.0f32 };
            self.last_active = self.active;
            self.rail_from = ai;
            self.rail_anim_at = None;
            self.lyr_drag_off = 0.0;
            self.lyr_scroll_target = 0.0;
            self.lyr_ci_anim = -1.0;
            self.lyr_release_at = None;
            self.lyr_return_from = 0.0;
            self.lyr_return_start = None;
        }
        // 决定窗口是否处于“大窗”态：仅展开/展开动画/归位动画/拖动时用整块画布。
        self.full = self.island_visible
            && (self.expanded > 0.001 || self.target > 0.5 || self.settle.is_some() || self.dragging);
        if self.settle.is_some() {
            self.step_settle(ctx);
        } else if !self.dragging {
            self.apply_geometry(ctx);
        }
        // 岛体以外"不占用桌?+ 点击穿透到下层程序"：用 SetWindowRgn 把窗口裁剪到
        // 比岛体略大一?PAD)的区域。区域比可见圆角外扩几像素，egui 抗锯齿圆角完全落?
        // 区域内、不会被硬裁剪，所以圆角依旧平滑；区域外窗口根本不存在，点击自然落到下面?
        // 窗口尺寸始终不变 -> 不重?surface -> 不闪烁?
        self.apply_region(ctx);
        // 每帧刷新命中矩形：后台穿透轮询线程据此判断光标是否在岛体/面板内（跨进程点击穿透）。
        self.set_hit_region(ctx);

        // 启动 cloak 释放 + 入场动画起点：等窗口被 apply_geometry 移到最终贴边位置后再解除 cloak。
        // 关键：在“绘制之前”解除 cloak 并同时开始淡入，这样解除 cloak 的那一帧就已经按低不透明度绘制，
        // 不会先闪一帧全不透明的岛体（既有入场动画，又不露左上残影）。
        if self.cloaked {
            self.cloak_frames = self.cloak_frames.saturating_add(1);
            if self.cloak_frames >= 8 {
                platform::set_cloak(self.hwnd, false);
                self.cloaked = false;
                self.intro_start = Some(Instant::now());
                self.intro_done = false;
            }
        }

        let _pf_t_pre = std::time::Instant::now();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| self.draw(ui));
        let _pf_t_draw = std::time::Instant::now();
        self.settings_ui(ctx);
        let _pf_t_post = std::time::Instant::now();

        // 入场动画收尾：淡入/上浮（intro_dy 0.34s）跑完后视为完成，回到稳态高帧率。
        if !self.intro_done {
            if let Some(t0) = self.intro_start {
                if t0.elapsed().as_secs_f32() >= 0.34 {
                    self.intro_done = true;
                }
            }
        }

        // 自适应重绘：展开态、播放中、悬停/拖动时保持高帧率（约120fps）；收起且静止时降到 10fps，
        // 避免空闲也满核渲染导致发热、掉帧和「不丝滑」。
        let pointer_active = ctx.input(|i| i.pointer.has_pointer() || i.pointer.any_down());
        // “需要高帧率”的条件：入场/退场、展开收起过渡、音乐动画、拖动、吸附、更新检查、鼠标悬停。
        // 注意：不再用「expanded>0.01」把展开面板永久钉在 60fps——展开后若完全静止（未播放、无悬停、
        // 无过渡），allow idle 降帧，省 CPU。交互/动画结束后留 600ms 宽限，过渡（<0.6s）不被打断。
        let busy = !self.intro_done
            || self.cloaked
            || self.exit_start.is_some()
            || self.intro_dy() != 0.0
            || (self.cb_anim > 0.002 && self.cb_anim < 0.998)
            || (self.expanded - self.target).abs() > 0.002
            || self.music_motion
            || self.dragging
            || self.settle.is_some()
            || self.upd_ver.is_some()
            || self.upd_checking
            || self.upd_busy
            || pointer_active;
        if busy {
            self.last_busy = Instant::now();
        }
        let animating = (busy || self.last_busy.elapsed() < Duration::from_millis(600))
            && std::env::var_os("WINISLE_IDLE").is_none();
        // 目标帧间隔：动画态锚定 16ms（约 60fps，满足“动态模块 ≥60fps”的硬约束）；静止态 200ms（约 5fps）。
        // 历史上这里是 8ms（约 125fps），实测“收起 + 播放中”会常驻吃满 1 个 CPU 核，故按 60fps 锚定。
        let target_ms: u64 = if animating { 16 } else { 200 };
        if std::env::var_os("WINISLE_PERF").is_some() {
            let el = _pf_t0.elapsed().as_secs_f32() * 1000.0;
            let (dt, udt) = ctx.input(|i| (i.stable_dt, i.unstable_dt));
            if el > 12.0 || dt > 0.020 {
                use std::io::Write as _;
                let line = format!(
                    "{} update={:.1}ms (win={:.1} state={:.1} a0={:.1} a1={:.1} a2={:.1} m1={:.1} m2={:.1} m3={:.1} post={:.1} draw={:.1}) snap={:.1} player={:.1} ne={:.1} region={:.1}ms stable_dt={:.1}ms exp={:.3} tgt={:.3} amt={:.3} playing={}\n",
                    chrono_like_now(),
                    el,
                    (_pf_t_win - _pf_t0).as_secs_f32() * 1000.0,
                    (_pf_t_state - _pf_t_win).as_secs_f32() * 1000.0,
                    (_pf_t_a0 - _pf_t_win).as_secs_f32() * 1000.0,
                    (_pf_t_a1 - _pf_t_a0).as_secs_f32() * 1000.0,
                    (_pf_t_a2 - _pf_t_a1).as_secs_f32() * 1000.0,
                    0.0,
                    0.0,
                    (_pf_t_m3 - _pf_t_m2).as_secs_f32() * 1000.0,
                    (_pf_t_pre - _pf_t_state).as_secs_f32() * 1000.0,
                    (_pf_t_draw - _pf_t_pre).as_secs_f32() * 1000.0,
                    _pf_snap_ms,
                    0.0,
                    self.perf_netease_ms,
                    self.perf_region_ms,
                    dt * 1000.0,
                    self.expanded,
                    self.target,
                    self.music_amt,
                    self.music.playing,
                );
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(crate::paths::file("perf.log"))
                {
                    let _ = f.write_all(line.as_bytes());
                }
            }
        }
        // [DIAG] 帧率计数器：仅 WINISLE_PERF 时每秒往 perf.log 写一行 FPS，用于确认实际帧率是否被
        // vsync 限制在 60，还是被“逐帧 request_repaint”顶成了无上限满速。
        let _diag_interval_ms = self.frame_last.elapsed().as_secs_f32() * 1000.0;
        let _diag_work_ms = _pf_t0.elapsed().as_secs_f32() * 1000.0;
        if std::env::var_os("WINISLE_PERF").is_some() {
            use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
            static FRAMES: AtomicU32 = AtomicU32::new(0);
            static LAST_MS: AtomicU64 = AtomicU64::new(0);
            FRAMES.fetch_add(1, Ordering::Relaxed);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let last = LAST_MS.load(Ordering::Relaxed);
            if last == 0 {
                LAST_MS.store(now_ms, Ordering::Relaxed);
            } else if now_ms.saturating_sub(last) >= 1000 {
                let fr = FRAMES.swap(0, Ordering::Relaxed);
                LAST_MS.store(now_ms, Ordering::Relaxed);
                use std::io::Write as _;
                if let Ok(mut fh) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(crate::paths::file("perf.log"))
                {
                    let causes: Vec<String> = ctx
                        .repaint_causes()
                        .iter()
                        .map(|c| format!("{}:{}", c.file, c.line))
                        .collect();
                    let _ = writeln!(
                        fh,
                        "FPS={} interval={:.1}ms work={:.1}ms animating={} exp={:.3} amt={:.3} playing={} causes={:?}",
                        fr, _diag_interval_ms, _diag_work_ms, animating, self.expanded, self.music_amt, self.music.playing, causes
                    );
                }
            }
        }
        // 真正的帧预算限速：用本帧实际耗时(_pf_t0 起算)补足到目标间隔后再请求下一帧，
        // 让事件循环在“活干完后”休眠剩余时间，而不是无条件立即再画——这是把
        // “收起 + 播放中”从吃满 1 核拉回来的关键（旧代码 frame_last 只写不读，等于没有限速）。
        // 自适应帧预算：用「上一帧周期(interval) − 上一帧申请的休眠(prev_req_ms)」估算每帧
        // 真实工作耗时（含 egui 绘制 + GPU swap），再从目标间隔里扣掉它，让帧周期精确落在
        // target_ms ≈ 16ms（60fps）。predicted_dt 已归零，request_repaint_after 的时长会被
        // 如实当作休眠（不会再被 egui 偷偷减 16.67ms），所以这里扣工作耗时是安全且精确的。
        // 自适应帧预算：predicted_dt 已归零，request_repaint_after 会如实等待，因此直接用
        // 「本帧实际工作耗时」补足到目标间隔即可：休眠 (target_ms − 本帧work) → 帧周期精确落在
        // target_ms。不用「上一帧周期估算」，避免启动/负载变化时估算震荡把帧率顶到 >60。
        let req_ms = ((target_ms as f32) - _diag_work_ms).max(1.0);
        self.prev_req_ms = req_ms;
        self.frame_last = Instant::now();
        ctx.request_repaint_after(Duration::from_secs_f32(req_ms / 1000.0));
    }
}

fn chrono_like_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let s = d.as_secs() % 86400;
    format!("{:02}:{:02}:{:02}.{:03}", s / 3600, (s % 3600) / 60, s % 60, d.subsec_millis())
}

/// 按 UV 子矩形把表情贴图裁到给定宽高绘制（去掉原图透明留白，使内容真实铺满）。
/// `gap_reduce`：把左右两半（两只眼睛）各自向中间靠拢 gap_reduce/2 逻辑像素，
/// 即将两眼的间距缩小 gap_reduce 像素。
fn draw_face_img_cropped(
    painter: &egui::Painter,
    c: egui::Pos2,
    w: f32,
    h: f32,
    a: u8,
    tx: &egui::TextureHandle,
    uv: (f32, f32, f32, f32),
    gap_reduce: f32,
) {
    if a == 0 || w <= 1.0 || h <= 1.0 {
        return;
    }
    let full = egui::Rect::from_center_size(c, egui::vec2(w, h));
    let col = egui::Color32::from_rgba_unmultiplied(255, 255, 255, a);
    if gap_reduce.abs() < 0.01 {
        painter.image(
            tx.id(),
            full,
            egui::Rect::from_min_max(egui::pos2(uv.0, uv.1), egui::pos2(uv.2, uv.3)),
            col,
        );
        return;
    }
    // 以内容包围盒的水平中点把贴图切成“左眼/右眼”两半，各自向中间平移半量。
    let um = (uv.0 + uv.2) * 0.5;
    let half = gap_reduce * 0.5;
    let lw = w * (um - uv.0) / (uv.2 - uv.0).max(1e-4);
    let rw = w * (uv.2 - um) / (uv.2 - uv.0).max(1e-4);
    let left_rect = egui::Rect::from_min_size(
        egui::pos2(full.left() + half, full.top()),
        egui::vec2(lw, h),
    );
    let right_rect = egui::Rect::from_min_size(
        egui::pos2(full.right() - half - rw, full.top()),
        egui::vec2(rw, h),
    );
    painter.image(
        tx.id(),
        left_rect,
        egui::Rect::from_min_max(egui::pos2(uv.0, uv.1), egui::pos2(um, uv.3)),
        col,
    );
    painter.image(
        tx.id(),
        right_rect,
        egui::Rect::from_min_max(egui::pos2(um, uv.1), egui::pos2(uv.2, uv.3)),
        col,
    );
}

/// 收起态「待机表情」：简洁的机器人脸（两只眼睛 + 一张嘴），带眨眼/呼吸微动，
/// 用 egui 基本图元绘制，颜色统一用主题绿以呼应整体风格。expr 见 FACES。
fn draw_robot_face(painter: &egui::Painter, c: egui::Pos2, h: f32, a: u8, expr: u8, t: f64, tex: Option<&egui::TextureHandle>) {
    if a == 0 || h <= 1.0 {
        return;
    }
    let bob = ((t * 1.7).sin() as f32) * h * 0.028;
    // 有贴图则 1:1 直接绘制 Otto 原版像素表情（绿字 + 透明底），保持原始比例
    if let Some(tx) = tex {
        let s = tx.size_vec2();
        let fw = h * (s.x / s.y.max(1.0));
        let center = egui::pos2(c.x, c.y);
        let rect = egui::Rect::from_center_size(center, egui::vec2(fw, h));
        painter.image(
            tx.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, a),
        );
        return;
    }
    let pi = std::f32::consts::PI;
    let col = egui::Color32::from_rgba_unmultiplied(126, 234, 182, a);
    let cy = c.y + bob;
    let eye_dx = h * 0.26;
    let eye_cy = cy - h * 0.10;
    let eye_w = h * 0.185;
    let eye_h = h * 0.26;
    let lw = (h * 0.085).max(1.4);
    let bp = ((t * 0.30) % 1.0) as f32;
    let blink = bp > 0.94;
    let my = cy + h * 0.24;

    // 圆角竖眼（含眨眼）；直接用 w*0.5 圆角成胶囊
    let round_open = |dx: f32, w: f32, hh: f32| {
        if blink {
            let r = egui::Rect::from_center_size(
                egui::pos2(c.x + dx, eye_cy),
                egui::vec2(w * 1.9, (h * 0.10).max(1.5)),
            );
            painter.rect_filled(r, (h * 0.05).max(1.0), col);
        } else {
            let r = egui::Rect::from_center_size(egui::pos2(c.x + dx, eye_cy), egui::vec2(w, hh));
            painter.rect_filled(r, w * 0.5, col);
        }
    };
    // 实心半圆眼：round_top=true 圆顶平底(开心眯眼)；false 平顶圆底(难过垂眼)
    let half_eye = |dx: f32, w: f32, hh: f32, round_top: bool| {
        let n = 18;
        let mut pts: Vec<egui::Pos2> = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let ang = pi * (i as f32) / (n as f32);
            let x = c.x + dx + w * 0.5 * ang.cos();
            let sy = if round_top { 1.0 } else { -1.0 };
            let y = eye_cy - hh * ang.sin() * sy;
            pts.push(egui::pos2(x, y));
        }
        painter.add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
    };
    // 旋转椭圆（生气斜眼）
    let ellipse = |cx: f32, cyy: f32, w: f32, hh: f32, ang: f32| {
        let n = 22;
        let mut pts: Vec<egui::Pos2> = Vec::with_capacity(n);
        for i in 0..n {
            let a2 = 2.0 * pi * (i as f32) / (n as f32);
            let x0 = w * 0.5 * a2.cos();
            let y0 = hh * 0.5 * a2.sin();
            let xr = x0 * ang.cos() - y0 * ang.sin();
            let yr = x0 * ang.sin() + y0 * ang.cos();
            pts.push(egui::pos2(cx + xr, cyy + yr));
        }
        painter.add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
    };
    // 爱心（两圆 + 三角，避免凹多边形填充问题）
    let heart = |cx: f32, cyy: f32, s: f32| {
        let r = s * 0.30;
        painter.circle_filled(egui::pos2(cx - s * 0.24, cyy - s * 0.16), r, col);
        painter.circle_filled(egui::pos2(cx + s * 0.24, cyy - s * 0.16), r, col);
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(cx - s * 0.52, cyy - s * 0.02),
                egui::pos2(cx + s * 0.52, cyy - s * 0.02),
                egui::pos2(cx, cyy + s * 0.58),
            ],
            col,
            egui::Stroke::NONE,
        ));
    };
    // 弧线：depth>0 微笑(U)；depth<0 皱眉(∩)；ycen 为基线
    let arc = |w: f32, depth: f32, ycen: f32| {
        let n = 14;
        let mut pts = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let fx = i as f32 / n as f32;
            let x = -w * 0.5 + w * fx;
            let u = (x / (w * 0.5)).clamp(-1.0, 1.0);
            let y = depth * (1.0 - u * u);
            pts.push(egui::pos2(c.x + x, ycen + y));
        }
        painter.add(egui::Shape::line(pts, egui::Stroke::new(lw, col)));
    };
    let flat_mouth = |w: f32| {
        let r = egui::Rect::from_center_size(
            egui::pos2(c.x, my),
            egui::vec2(w, (h * 0.09).max(1.4)),
        );
        painter.rect_filled(r, h * 0.045, col);
    };
    // 闭眼弧线（up=true 上弯 ‿ 的反向 → 拱形）
    let closed_eye_line = |dx: f32, w: f32, up: bool| {
        let n = 12;
        let mut pts = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let fx = i as f32 / n as f32;
            let x = -w * 0.5 + w * fx;
            let u = (x / (w * 0.5)).clamp(-1.0, 1.0);
            let y = (1.0 - u * u) * h * 0.10 * if up { -1.0 } else { 1.0 };
            pts.push(egui::pos2(c.x + dx + x, eye_cy + y));
        }
        painter.add(egui::Shape::line(pts, egui::Stroke::new(lw, col)));
    };

    match expr {
        // 0 平静：两只圆眼 + 小扁嘴
        0 => {
            round_open(-eye_dx, eye_w, eye_h);
            round_open(eye_dx, eye_w, eye_h);
            flat_mouth(h * 0.30);
        }
        // 1 开心：眯眼笑（圆顶）+ 大笑弧
        1 => {
            half_eye(-eye_dx, eye_w * 1.5, eye_h * 0.95, true);
            half_eye(eye_dx, eye_w * 1.5, eye_h * 0.95, true);
            arc(h * 0.54, h * 0.25, my);
        }
        // 2 难过：垂眼（平顶圆底）+ 皱眉
        2 => {
            half_eye(-eye_dx, eye_w * 1.4, eye_h * 0.82, false);
            half_eye(eye_dx, eye_w * 1.4, eye_h * 0.82, false);
            arc(h * 0.40, -h * 0.12, my + h * 0.03);
        }
        // 3 生气：斜眼 + 怒符 + 小扁嘴
        3 => {
            ellipse(c.x - eye_dx, eye_cy, eye_w * 1.5, eye_h * 0.78, 0.5);
            ellipse(c.x + eye_dx, eye_cy, eye_w * 1.5, eye_h * 0.78, -0.5);
            let gx = c.x + eye_dx + eye_w * 1.5;
            let gy = eye_cy - eye_h * 1.15;
            let s = h * 0.05;
            painter.add(egui::Shape::line(
                vec![egui::pos2(gx - s, gy - s), egui::pos2(gx + s, gy + s)],
                egui::Stroke::new(lw * 0.7, col),
            ));
            painter.add(egui::Shape::line(
                vec![egui::pos2(gx + s, gy - s), egui::pos2(gx - s, gy + s)],
                egui::Stroke::new(lw * 0.7, col),
            ));
            flat_mouth(h * 0.26);
        }
        // 4 哭泣：垂眼 + 泪流 + 皱眉
        4 => {
            half_eye(-eye_dx, eye_w * 1.3, eye_h * 0.78, false);
            half_eye(eye_dx, eye_w * 1.3, eye_h * 0.78, false);
            for side in [-1.0f32, 1.0] {
                let tx = c.x + side * eye_dx;
                let mut yv = eye_cy + eye_h * 0.85;
                for _ in 0..3 {
                    painter.add(egui::Shape::line(
                        vec![egui::pos2(tx, yv), egui::pos2(tx, yv + h * 0.06)],
                        egui::Stroke::new(lw * 0.7, col),
                    ));
                    yv += h * 0.11;
                }
                painter.circle_filled(egui::pos2(tx, yv + h * 0.01), h * 0.035, col);
            }
            arc(h * 0.38, -h * 0.11, my + h * 0.04);
        }
        // 5 爱心眼：心形眼 + 微笑
        5 => {
            heart(c.x - eye_dx, eye_cy, eye_w * 2.5);
            heart(c.x + eye_dx, eye_cy, eye_w * 2.5);
            arc(h * 0.44, h * 0.14, my);
        }
        // 6 害羞：圆眼 + 腮红斜线 + 汗滴 + 微笑
        6 => {
            round_open(-eye_dx, eye_w, eye_h);
            round_open(eye_dx, eye_w, eye_h);
            for side in [-1.0f32, 1.0] {
                let bx = c.x + side * (eye_dx + eye_w * 0.95);
                for k in 0..3 {
                    let o = (k as f32 - 1.0) * h * 0.05;
                    painter.add(egui::Shape::line(
                        vec![
                            egui::pos2(bx - h * 0.04, eye_cy + eye_h * 0.55 + o + h * 0.04),
                            egui::pos2(bx + h * 0.02, eye_cy + eye_h * 0.55 + o - h * 0.02),
                        ],
                        egui::Stroke::new(lw * 0.6, col),
                    ));
                }
            }
            let sx = c.x + eye_dx + eye_w * 0.8;
            let sy = eye_cy - eye_h * 1.1;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(sx, sy - h * 0.07),
                    egui::pos2(sx - h * 0.04, sy + h * 0.02),
                    egui::pos2(sx + h * 0.04, sy + h * 0.02),
                ],
                col,
                egui::Stroke::NONE,
            ));
            painter.circle_filled(egui::pos2(sx, sy + h * 0.03), h * 0.04, col);
            arc(h * 0.40, h * 0.16, my);
        }
        // 7 思考：一大一小眼 + 小扁嘴
        7 => {
            round_open(-eye_dx, eye_w * 1.2, eye_h);
            painter.circle_filled(
                egui::pos2(c.x + eye_dx + h * 0.03, eye_cy - h * 0.04),
                eye_w * 0.7,
                col,
            );
            flat_mouth(h * 0.24);
        }
        // 8 眨眼：左眼闭 + 右眼圆 + 微笑
        8 => {
            closed_eye_line(-eye_dx, eye_w * 1.7, true);
            round_open(eye_dx, eye_w, eye_h);
            arc(h * 0.42, h * 0.15, my);
        }
        // 9 酷：墨镜 + 淡笑
        9 => {
            let lens_w = eye_w * 1.7;
            let lens_h = eye_h * 0.95;
            for side in [-1.0f32, 1.0] {
                let r = egui::Rect::from_center_size(
                    egui::pos2(c.x + side * eye_dx, eye_cy),
                    egui::vec2(lens_w, lens_h),
                );
                painter.rect_filled(r, h * 0.055, col);
            }
            let br = egui::Rect::from_center_size(
                egui::pos2(c.x, eye_cy - lens_h * 0.18),
                egui::vec2((eye_dx * 2.0 - lens_w).max(1.0), h * 0.05),
            );
            painter.rect_filled(br, h * 0.02, col);
            arc(h * 0.30, h * 0.07, my);
        }
        // 10 亲亲：弯眼 + 嘟嘴 + 小爱心
        10 => {
            closed_eye_line(-eye_dx, eye_w * 1.7, true);
            closed_eye_line(eye_dx, eye_w * 1.7, true);
            painter.circle_filled(egui::pos2(c.x, my), h * 0.055, col);
            heart(c.x + h * 0.30, my - h * 0.02, h * 0.18);
        }
        // 11 自信：半眯眼 + 淡笑
        11 => {
            half_eye(-eye_dx, eye_w * 1.5, eye_h * 0.55, false);
            half_eye(eye_dx, eye_w * 1.5, eye_h * 0.55, false);
            arc(h * 0.36, h * 0.12, my);
        }
        // 12 困倦：闭眼线 + Zz
        12 => {
            closed_eye_line(-eye_dx, eye_w * 1.8, false);
            closed_eye_line(eye_dx, eye_w * 1.8, false);
            painter.text(
                egui::pos2(c.x + eye_dx + eye_w * 0.6, eye_cy - h * 0.28),
                egui::Align2::LEFT_CENTER,
                "z",
                egui::FontId::proportional((h * 0.20).max(7.0)),
                col,
            );
            painter.text(
                egui::pos2(c.x + eye_dx + eye_w * 1.5, eye_cy - h * 0.44),
                egui::Align2::LEFT_CENTER,
                "z",
                egui::FontId::proportional((h * 0.15).max(6.0)),
                col,
            );
        }
        // 13 困惑：一大一小眼 + 问号 + 小扁�?
        _ => {
            round_open(-eye_dx, eye_w * 1.2, eye_h);
            painter.circle_filled(
                egui::pos2(c.x + eye_dx + h * 0.03, eye_cy + h * 0.02),
                eye_w * 0.7,
                col,
            );
            painter.text(
                egui::pos2(c.x + eye_dx + eye_w * 0.4, eye_cy - eye_h * 1.25),
                egui::Align2::LEFT_CENTER,
                "?",
                egui::FontId::proportional((h * 0.24).max(7.0)),
                col,
            );
            flat_mouth(h * 0.24);
        }
    }
}

/// 空气质量文本 -> 颜色（优=薄荷绿，�?黄绿，轻�?黄，中度=橙，重度=红）�?
fn aqi_color(q: &str, a: u8) -> egui::Color32 {
    let (r, g, b) = if q.contains("优") {
        (52, 211, 153)
    } else if q.contains("良") {
        (176, 208, 82)
    } else if q.contains("轻度") {
        (238, 198, 74)
    } else if q.contains("中度") {
        (238, 150, 60)
    } else if q.contains("重度") || q.contains("严重") {
        (230, 92, 82)
    } else {
        (150, 158, 175)
    };
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// 程序化天气图标（8 种，含简单动画）。`t` 为秒级时间；`bg` 用于夜间月牙的遮罩底色�?
fn draw_weather_icon(p: &egui::Painter, sky: crate::weather::Sky, c: egui::Pos2, s: f32, a: u8, t: f64, bg: egui::Color32) {
    use crate::weather::Sky;
    let mk = |r: u8, g: u8, b: u8| egui::Color32::from_rgba_unmultiplied(r, g, b, a);
    let sun = mk(255, 206, 92);
    let sun2 = mk(246, 170, 60);
    let cloud = mk(206, 216, 230);
    let rain = mk(120, 182, 238);
    let snow = mk(233, 244, 255);
    let bolt = mk(247, 206, 84);
    let fogc = mk(174, 185, 200);
    let moon = mk(226, 233, 245);

    let sun_at = |pp: &egui::Painter, cc: egui::Pos2, rad: f32, rays: bool| {
        pp.circle_filled(cc, rad, sun);
        pp.circle_filled(cc, rad * 0.70, sun2);
        if rays {
            let n = 8;
            for i in 0..n {
                let ang = t as f32 * 0.7 + i as f32 * (std::f32::consts::TAU / n as f32);
                let d1 = rad * 1.30;
                let d2 = rad * 1.62;
                pp.line_segment(
                    [
                        egui::pos2(cc.x + ang.cos() * d1, cc.y + ang.sin() * d1),
                        egui::pos2(cc.x + ang.cos() * d2, cc.y + ang.sin() * d2),
                    ],
                    egui::Stroke::new((s * 0.045).max(1.4), sun),
                );
            }
        }
    };
    let cloud_at = |pp: &egui::Painter, base: egui::Pos2, cw: f32| {
        let rr = cw * 0.26;
        pp.circle_filled(egui::pos2(base.x - cw * 0.22, base.y), rr, cloud);
        pp.circle_filled(egui::pos2(base.x + cw * 0.02, base.y - rr * 0.55), rr * 1.18, cloud);
        pp.circle_filled(egui::pos2(base.x + cw * 0.24, base.y), rr * 0.92, cloud);
        pp.rect_filled(
            egui::Rect::from_min_max(egui::pos2(base.x - cw * 0.24, base.y), egui::pos2(base.x + cw * 0.26, base.y + rr)),
            rr,
            cloud,
        );
    };

    match sky {
        Sky::Sun => sun_at(p, c, s * 0.30, true),
        Sky::Partly => {
            sun_at(p, egui::pos2(c.x - s * 0.14, c.y - s * 0.16), s * 0.20, true);
            cloud_at(p, egui::pos2(c.x + s * 0.06, c.y + s * 0.14), s * 0.92);
        }
        Sky::Cloud => cloud_at(p, egui::pos2(c.x, c.y + s * 0.10), s),
        Sky::Rain => {
            cloud_at(p, egui::pos2(c.x, c.y - s * 0.10), s * 0.92);
            for i in 0..3 {
                let bx = c.x - s * 0.26 + i as f32 * s * 0.26;
                let ph = (t as f32 * 1.7 + i as f32 * 0.55) % 1.0;
                let by = c.y + s * 0.20 + ph * s * 0.42;
                p.line_segment(
                    [egui::pos2(bx, by), egui::pos2(bx - s * 0.05, by + s * 0.16)],
                    egui::Stroke::new((s * 0.045).max(1.3), rain),
                );
            }
        }
        Sky::Thunder => {
            cloud_at(p, egui::pos2(c.x, c.y - s * 0.12), s * 0.92);
            let bx = c.x;
            let by = c.y + s * 0.06;
            p.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(bx - s * 0.07, by),
                    egui::pos2(bx + s * 0.09, by),
                    egui::pos2(bx - s * 0.01, by + s * 0.20),
                ],
                bolt,
                egui::Stroke::NONE,
            ));
            p.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(bx - s * 0.01, by + s * 0.18),
                    egui::pos2(bx + s * 0.13, by + s * 0.13),
                    egui::pos2(bx - s * 0.03, by + s * 0.40),
                ],
                bolt,
                egui::Stroke::NONE,
            ));
        }
        Sky::Snow => {
            cloud_at(p, egui::pos2(c.x, c.y - s * 0.12), s * 0.92);
            for i in 0..3 {
                let bx = c.x - s * 0.26 + i as f32 * s * 0.26;
                let ph = (t as f32 * 0.7 + i as f32 * 0.33) % 1.0;
                let by = c.y + s * 0.20 + ph * s * 0.38;
                p.circle_filled(egui::pos2(bx, by), (s * 0.035).max(1.4), snow);
            }
        }
        Sky::Fog => {
            cloud_at(p, egui::pos2(c.x, c.y - s * 0.22), s * 0.90);
            for i in 0..3 {
                let off = (t as f32 * 0.6 + i as f32 * 0.33).sin() * s * 0.06;
                let yy = c.y + s * 0.14 + i as f32 * s * 0.13;
                p.line_segment(
                    [egui::pos2(c.x - s * 0.34 + off, yy), egui::pos2(c.x + s * 0.30 + off, yy)],
                    egui::Stroke::new((s * 0.05).max(1.5), fogc),
                );
            }
        }
        Sky::Night => {
            let mar = s * 0.30;
            p.circle_filled(egui::pos2(c.x + s * 0.03, c.y), mar, moon);
            p.circle_filled(egui::pos2(c.x + s * 0.17, c.y - s * 0.09), mar * 0.92, bg);
            for i in 0..3 {
                let tw = ((t as f32 * 2.0 + i as f32 * 1.3).sin() * 0.5 + 0.5) as f32;
                let sx = c.x - s * 0.26 + i as f32 * s * 0.24;
                let sy = c.y - s * 0.24 + (i % 2) as f32 * s * 0.14;
                let col = egui::Color32::from_rgba_unmultiplied(226, 233, 245, (tw * 200.0 * (a as f32 / 255.0)) as u8);
                p.circle_filled(egui::pos2(sx, sy), (s * 0.02).max(0.9), col);
            }
        }
    }
}

/// 左侧菜单图标：0=音乐(音符) 1=日程(日历) 2=天气(云)——使用系统图标字体。
fn draw_nav_icon(painter: &egui::Painter, kind: u8, c: egui::Pos2, size: f32, color: egui::Color32) {
    match kind {
        // 音乐：连续双八分音符「♫」，实心，风格贴近参考图
        0 => { painter.text(c, egui::Align2::CENTER_CENTER, "\u{266B}", egui::FontId::proportional(size * 1.15), color); }
        1 => { painter.text(c, egui::Align2::CENTER_CENTER, "\u{E787}", icon_font(size * 1.15), color); } // Calendar
        _ => { painter.text(c, egui::Align2::CENTER_CENTER, "\u{E753}", icon_font(size * 1.15), color); } // Cloud
    }
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let load = |fonts: &mut egui::FontDefinitions, key: &str, path: &str| -> bool {
        match std::fs::read(path) {
            Ok(bytes) => {
                fonts
                    .font_data
                    .insert(key.to_owned(), egui::FontData::from_owned(bytes).into());
                true
            }
            Err(_) => false,
        }
    };
    let has_segoe = load(&mut fonts, "segoe", r"C:\Windows\Fonts\segoeui.ttf");
    load(&mut fonts, "segoeb", r"C:\Windows\Fonts\segoeuib.ttf");
    load(&mut fonts, "deng", r"C:\Windows\Fonts\Deng.ttf");
    load(&mut fonts, "sym", r"C:\Windows\Fonts\seguisym.ttf");
    let has_mdl2 = load(&mut fonts, "mdl2", r"C:\Windows\Fonts\segmdl2.ttf");
    // 韩文（Hangul，如라랄라/내 사랑）不在 等线(Deng)/Segoe UI 里，会显示成方块□；
    // 补一款 Windows 自带韩文字体 malgun.ttf（맑은 고딕）作兜底（放在 deng 之后，不影响中文优先走 deng）。
    let has_malgun = load(&mut fonts, "malgun", r"C:\Windows\Fonts\malgun.ttf");
    // 藏文（如 ANU《FLY》整首藏语歌词）不在 等线/Segoe UI/Malgun 里，会显示成方块□；
    // 补一款 Windows 自带藏文字体 himalaya.ttf（Microsoft Himalaya）作兜底。
    let has_himalaya = load(&mut fonts, "himalaya", r"C:\Windows\Fonts\himalaya.ttf");

    // 思源黑体 Noto Sans SC（用户选定，内嵌保证各机器字形一致）：
    // 作为中文主字库放在 Segoe UI 之后——拉丁/数字仍走 Segoe UI，中文走思源黑体；
    // 并提供真正的粗体（notob），修复此前“中文标题无真粗体、发细发平”的问题。
    fonts.font_data.insert(
        "noto".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/NotoSansSC-Regular.otf")).into(),
    );
    fonts.font_data.insert(
        "notob".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/NotoSansSC-Bold.otf")).into(),
    );

    if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        if has_segoe {
            list.insert(0, "segoe".to_owned());
        }
        // 中文优先思源黑体（紧随 Segoe UI，拉丁仍命中 segoe）；等线保留为次级兜底。
        list.insert(if has_segoe { 1 } else { 0 }, "noto".to_owned());
        list.push("deng".to_owned());
        list.push("sym".to_owned());
        if has_malgun {
            list.push("malgun".to_owned());
        }
        if has_himalaya {
            list.push("himalaya".to_owned());
        }
    }
    if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        list.push("noto".to_owned());
        list.push("deng".to_owned());
        list.push("sym".to_owned());
        if has_malgun {
            list.push("malgun".to_owned());
        }
        if has_himalaya {
            list.push("himalaya".to_owned());
        }
    }
    // 粗体字族：拉丁用 Segoe UI Bold，中文用思源黑体 Bold（真粗体），再兜底常规 noto / 等线。
    let mut bold_list = vec!["segoeb".to_owned(), "notob".to_owned(), "noto".to_owned(), "deng".to_owned()];
    if has_malgun {
        bold_list.push("malgun".to_owned());
    }
    if has_himalaya {
        bold_list.push("himalaya".to_owned());
    }
    fonts.families.insert(egui::FontFamily::Name(Arc::from("bold")), bold_list);
    if has_mdl2 {
        fonts.families.insert(
            egui::FontFamily::Name(Arc::from("icons")),
            vec!["mdl2".to_owned()],
        );
    }
    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result<()> {
    // 单实例：已在运行 ?通知它打开面板，本进程退出?
    if platform::ensure_single_instance() {
        let _ = std::fs::write(trigger_path(), b"open");
        return Ok(());
    }

    // 调试：WINISLE_LYRIC_TEST="歌名|歌手" 时只跑歌词抓取并打印，不启动界面
    if let Ok(spec) = std::env::var("WINISLE_LYRIC_TEST") {
        let (t, a) = spec.split_once('|').unwrap_or((spec.as_str(), ""));
        match lyrics::fetch(t, a) {
            Some(v) => println!(
                "LYRIC OK lines={} first3={:?}",
                v.len(),
                v.iter().take(3).map(|l| (l.t, l.text.clone())).collect::<Vec<_>>()
            ),
            None => println!("LYRIC NONE for {}|{}", t, a),
        }
        return Ok(());
    }

    // 启动时不再默认弹出窗口；设置窗口只通过"托盘图标左键单击"打开?
    // 不再有“单击托盘/双击启动弹出设置窗口”的行为；设置后续在岛内模块里做。
    let open_panel = false;

    // 启动实时音频电平采样（驱动频谱随音乐跳动）
    crate::audio_meter::spawn();
    // 启动汽水「正在播放」CDP 后备采样（SMTC 整体抽风时识别不到音乐）
    crate::soda_now::spawn();

    // 托盘「退出」看门狗：仅作**兜底**。正常由 UI 在退场动画（淡出+上滑 0.18s）播完后自行 exit；
    // 这里给足 1500ms 余量，避免像以前那样 150ms 就强杀、把退场动画截断成“直接关闭”。
    std::thread::spawn(|| loop {
        use std::sync::atomic::Ordering::SeqCst;
        if crate::tray::QUIT_REQUESTED.load(SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            std::process::exit(0);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    });

    // 调试：WINISLE_AUTOQUIT_MS=N 启动 N 毫秒后触发托盘「退出」流程（用于验证退场动画）。
    if let Ok(ms) = std::env::var("WINISLE_AUTOQUIT_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                crate::tray::QUIT_REQUESTED
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            });
        }
    }

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WinIsle")
            .with_inner_size([W_EXPANDED + 2.0 * MARGIN, H_EXPANDED + 2.0 * MARGIN])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_active(false)
            // 先隐藏创建，等首帧按最终贴边位置摆好后再显示，避免“先在左上闪一下再跳到中间”。
            .with_visible(false),
        // MSAA 4→0：叠层透明窗口整块每帧重绘，4x 多重采样只增加 GPU 带宽与合成开销，
        // 对这类纯 2D 圆角/文字（egui 自身带形状与字体抗锯齿）几乎无视觉收益，故关闭。
        multisampling: 0,
        vsync: false,
        ..Default::default()
    };
    eframe::run_native(
        "WinIsle",
        opts,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            apply_style(&cc.egui_ctx, false);
            let tray = Tray::new();
            Ok(Box::new(IslandApp::new(tray, open_panel)))
        }),
    )
}


fn make_tex(ctx: &egui::Context, name: &str, bytes: &'static [u8], w: usize, h: usize) -> egui::TextureHandle {
    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], bytes);
    ctx.load_texture(name, img, egui::TextureOptions::LINEAR)
}

