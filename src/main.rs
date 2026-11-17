#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! MyIsle —??Windows 灵动岛?
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

use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tray::{Tray, TrayAction};

// 收起?/ 展开态的窗口尺寸（逻辑点）
const W_COLLAPSED: f32 = 150.0;
const W_COLLAPSED_MUSIC: f32 = 356.0;
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
}

// ============================ 配置 ============================
#[derive(Clone, PartialEq, Debug)]
struct Config {
    position: DockPosition,
    /// 岛体中心在屏幕上的水平位置比?0..1)。顶端吸附时用它决定左右位置?
    x_frac: f32,
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
    /// 展开态歌词字号
    lyric_font: f32,
    /// 收起态频谱动画开关
    spectrum: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            position: DockPosition::TopCenter,
            x_frac: 0.5,
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
            roll_ms: 340.0,
            char_green: true,
            marquee: true,
            lyric_font: 13.0,
            spectrum: true,
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
                    "autostart" => cfg.autostart = v == "1" || v == "true",
                    "theme" => cfg.theme = Theme::from_str(v),
                    "lyric_shift_ms" => cfg.lyric_shift_ms = v.parse().unwrap_or(cfg.lyric_shift_ms),
                    "roll_anim" => cfg.roll_anim = v == "1" || v == "true",
                    "roll_ms" => cfg.roll_ms = v.parse().unwrap_or(cfg.roll_ms),
                    "char_green" => cfg.char_green = v == "1" || v == "true",
                    "marquee" => cfg.marquee = v == "1" || v == "true",
                    "lyric_font" => cfg.lyric_font = v.parse().unwrap_or(cfg.lyric_font),
                    "spectrum" => cfg.spectrum = v == "1" || v == "true",
                    _ => {}
                }
            }
        }
        // 目前仅支?顶端吸附"，统一按顶部停靠处理（左右位置?x_frac 决定）?
        cfg.position = DockPosition::TopCenter;
        cfg.x_frac = cfg.x_frac.clamp(0.0, 1.0);
        cfg
    }

    fn save(&self) {
        let text = format!(
            "# MyIsle 配置\nposition={}\nx_frac={}\nfont={}\ndelay_ms={}\nbg={},{},{}\nalpha={}\nradius={}\nshow_seconds={}\nshow_date={}\nautostart={}\ntheme={}\nlyric_shift_ms={}\nroll_anim={}\nroll_ms={}\nchar_green={}\nmarquee={}\nlyric_font={}\nspectrum={}\n",
            self.position.as_str(),
            self.x_frac,
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
            self.lyric_font,
            self.spectrum as u8,
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
    if on {
        let val = format!("\"{}\" --autostart", exe);
        let _ = std::process::Command::new("reg")
            .args(["add", key, "/v", "MyIsle", "/t", "REG_SZ", "/d", &val, "/f"])
            .status();
    } else {
        let _ = std::process::Command::new("reg")
            .args(["delete", key, "/v", "MyIsle", "/f"])
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
    } else if s.contains("kuwo") {
        "酷我音乐".to_string()
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
/// 左侧导航对应的独立模块（互不覆盖；音乐在后台持续运行，切到别的模块不影响它）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Module {
    Music,
    Schedule,
    Weather,
}

/// 仅用于调试钩子 MYISLE_PAGE：启动后直接定位到某个视图。
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
    lyr_tx: std::sync::mpsc::Sender<(String, String, String, f64, bool)>,
    lyr_rx: std::sync::mpsc::Receiver<(String, Option<Vec<lyrics::LrcLine>>, Option<Vec<u8>>)>,
    lyr_key: String,
    lyr_dur: f64, // 当初请求歌词时采用的“期望时长”，用于发现同名不同版本(时长不符)后重取
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
    /// 以 SMTC 真实 Position 为准的时间基准（播放器不给 position 时回退到 clock）
    spos: f64,
    spos_at: Instant,
    spos_valid: bool,
    /// 上一次从 SMTC 收到的原始 position（用于判断播放器是否真的在推进位置）；
    /// 像汽水这类播放器会持续上报同一个静止 position，不能拿它反复把进度往回拉。
    smtc_rep_pos: f64,
    /// 上次执行「汽水 CDP 补丁自检」的时间（节流用）。
    last_soda_heal: Instant,
    /// 启动后是否已做过一次「汽水补丁」检查（未运行时直接补好）。
    soda_boot_checked: bool,
    /// 汽水补丁自检是否正在后台执行，避免重复进入（该检查含网络/进程操作，放后台防卡顿）。
    soda_tick_busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 上次从汽水界面读取真实播放位置的时间（节流用）。
    soda_pos_at: Instant,
    /// 汽水进度查询放在后台线程，避免 CDP 网络抖动阻塞 UI 渲染（丝滑度关键）。
    soda_prog_rx: Option<std::sync::mpsc::Receiver<Option<(f64, f64)>>>,
    /// 从汽水播放条读到的真实总时长（秒），用于纠正 SMTC 给错的时长。
    soda_dur: f64,
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
    /// 兜底：SMTC 不可用时，从播放器窗口标题抓 “歌名 - 歌手”
    media_title_at: Instant,
    media_title_cache: Option<(String, String, String)>,
    /// 播放器进程检测结果缓存（枚举进程较慢，最多每秒查一次，避免无音乐时每帧 50ms+ 卡顿）
    player_running_cached: bool,
    player_running_at: Instant,
    /// 从“有歌”变成“没有歌”的起始时刻，用于延迟清除残留的音乐状态
    music_lost_since: Option<Instant>,
    /// 最近一次拿到的好歌名（用于播放器短时读不到标题时保持稳定，避免闪烁/来回切换）
    last_title: Option<(String, String, String)>,
    last_title_at: Instant,
    /// 音乐模块显示程度 0..1：用于宽胶囊/内容/空态的平滑过渡动画
    music_amt: f32,
    /// 最近一次显示过的歌词行号（滞回，避免边界处来回跳）
    lyr_idx_last: i32,
    /// 收起态歌词“上滚”动画：换行瞬间的时刻，以及上一句（滚出）的索引
    lyr_anim_at: Option<Instant>,
    lyr_anim_from: Option<usize>,
    /// 展开态歌词滚轮浏览：相对“当前句居中”的像素偏移（0=跟随当前进度）
    lyr_drag_off: f32,
    /// 滚轮目标偏移（`lyr_drag_off` 平滑趋近它，形成滚动过渡）
    lyr_scroll_target: f32,
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
    /// 调试：MYISLE_PAGE=settings/schedule/weather 时启动即定位对应视图（用于截图验证）
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
}

impl IslandApp {
    fn new(tray: Option<Tray>, open_panel: bool) -> Self {
        platform::start_netease_reader();
        let cfg = Config::load();
        let (lyr_tx, lyr_req_rx) = std::sync::mpsc::channel::<(String, String, String, f64, bool)>();
        let (lyr_res_tx, lyr_rx) =
            std::sync::mpsc::channel::<(String, Option<Vec<lyrics::LrcLine>>, Option<Vec<u8>>)>();
        std::thread::spawn(move || {
            while let Ok((key, title, artist, dur, soda)) = lyr_req_rx.recv() {
                let (r, cov) = lyrics::fetch_full(&title, &artist, dur, soda);
                // 诊断日志：记录每次歌词/封面请求结果
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
        // SMTC 采用主线程非阻塞状态机（见 smtc.rs），无需后台线程。
        let dbg_page = match std::env::var("MYISLE_PAGE").ok().as_deref() {
            Some("settings") => Some(DbgView::Settings),
            Some("schedule") => Some(DbgView::Schedule),
            Some("weather") => Some(DbgView::Weather),
            _ => None,
        };
        Self {
            hwnd: 0,
            initialized: false,
            expanded: 0.0,
            target: 0.0,
            collapse_at: None,
            monitor: egui::vec2(1920.0, 1080.0),
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
            spos: 0.0,
            spos_at: Instant::now(),
            spos_valid: false,
            smtc_rep_pos: -1.0,
            last_soda_heal: Instant::now() - Duration::from_secs(3600),
            soda_boot_checked: false,
            soda_tick_busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            soda_pos_at: Instant::now() - Duration::from_secs(3600),
            soda_prog_rx: None,
            soda_dur: 0.0,
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
            media_title_at: Instant::now() - Duration::from_secs(10),
            media_title_cache: None,
            player_running_cached: false,
            player_running_at: Instant::now() - Duration::from_secs(10),
            music_lost_since: None,
            last_title: None,
            last_title_at: Instant::now() - Duration::from_secs(60),
            music_amt: 0.0,
            lyr_idx_last: -1,
            lyr_anim_at: None,
            lyr_anim_from: None,
            lyr_drag_off: 0.0,
            lyr_scroll_target: 0.0,
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
    
    
        }
    }

    fn ensure_window(&mut self, frame: &mut eframe::Frame) {
        if self.initialized {
            return;
        }
        if let Ok(handle) = frame.window_handle() {
            if let RawWindowHandle::Win32(w) = handle.as_raw() {
                self.hwnd = w.hwnd.get();
                platform::make_island(self.hwnd);
                platform::install_erase_guard(self.hwnd);
                if std::env::var_os("MYISLE_LAYERED").is_some() {
                    platform::set_layered(self.hwnd);
                }
                // 首次创建后先把客户区预填充成深色（背景刷已置空，避免首帧任何未定义底色）
                platform::fill_dark_now(self.hwnd);
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
        if std::env::var_os("MYISLE_FORCE_EXPAND").is_some() {
            // 仅调试：无鼠标环境下强制展开（用于截图/自动化）
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

    /// 收起态宽度：有音乐时加宽到 W_COLLAPSED_MUSIC（放歌词/歌名），否则窄胶囊。
    fn collapsed_w(&self) -> f32 {
        W_COLLAPSED + (W_COLLAPSED_MUSIC - W_COLLAPSED) * self.music_amt.clamp(0.0, 1.0)
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
        if self.from_smtc { self.music.playing } else { self.ui_playing }
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

    /// 总时长估计：取 SMTC 时长与「歌词最后一句+6s」的较大者
    /// （汽水非会员常只给 30s 试听时长，用歌词长度兜底，进度条才不会被提前走完）。
    fn duration_est(&self) -> f64 {
        // 汽水：以播放条上读到的真实总时长为准（SMTC 常给错，如 30s 试听/旧值）。
        {
            let a = self.music.app_id.to_ascii_lowercase();
            if (a.contains("汽水") || a.contains("soda") || a.contains("qishui")) && self.soda_dur >= 20.0 {
                return self.soda_dur;
            }
        }
        // 优先用播放器上报的真实总时长；仅当缺失/明显异常（如汽水非会员只给 30s 试听）才用歌词兜底。
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
            d = d.max(l.t + 6.0);
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
            if l.t <= pos + 0.2 {
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
        // 无逐字时间：按字数估算本句“唱完时长”，长间奏不计入
        let mut est = 0.3_f64;
        for ch in self.lyr_lines[idx].text.chars() {
            if ch.is_whitespace() { continue; }
            est += if (ch as u32) < 0x2E80 { 0.12 } else { 0.28 };
        }
        let next = self.lyr_lines.get(idx + 1).map(|l| l.t).unwrap_or(cur + est);
        let gap = (next - cur).max(0.0);
        let dur = est.min(gap).max(0.6);
        ((pos - cur) / dur).clamp(0.0, 1.0) as f32
    }

    /// 按给定的展开?e 计算岛体矩形（相对固定窗口左上角）。e=1 时尺寸为 expanded_size()?
    fn pill_rect_e(&self, full: egui::Rect, e: f32) -> egui::Rect {
        let target = self.expanded_size();
        let cw = self.collapsed_w();
        let w = cw + (target.x - cw) * e;
        let h = H_COLLAPSED + (target.y - H_COLLAPSED) * e;
        // 顶端吸附：窗口始终以岛体中心为水平中心、贴住顶部（上方留 MARGIN 透明外框）。左右位置由 x_frac 在 layout_screen 里换算。
        let x = full.left() + (full.width() - w) * 0.5;
        let y = full.top() + MARGIN - TOP_OVERLAP;
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h + TOP_OVERLAP))
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
        platform::set_hit_pill(
            ((pill.left() - win.left()) * ppp) as i32,
            ((pill.top() - win.top()) * ppp) as i32,
            ((pill.right() - win.left()) * ppp) as i32,
            ((pill.bottom() - win.top()) * ppp) as i32,
        );
    }

    /// 窗口固定为大窗口(W_EXPANDED×H_EXPANDED)；收起时?SetWindowRgn 把窗口裁剪到岛体?
    /// 因此这里始终按大窗口计算 origin 与岛体屏幕矩形（岛体相对窗口的本地位置由停靠位决定）?
    fn layout_screen(&self) -> (egui::Rect, egui::Rect) {
        let w = W_EXPANDED + 2.0 * MARGIN;
        let h = H_EXPANDED + 2.0 * MARGIN;
        // 顶端吸附 + 水平自由定位：岛体中心 = x_frac * 屏宽（左右夹紧，保证完全可见）；
        // 展开变大时若靠近两端，会自动向屏幕内侧生长（左端向右长、右端向左长）。
        let cw = self.collapsed_w();
        let pw = cw + (W_EXPANDED - cw) * self.expanded;
        let ph = H_COLLAPSED + (H_EXPANDED - H_COLLAPSED) * self.expanded;
        let center = (self.cfg.x_frac * self.monitor.x)
            .clamp(cw * 0.5, self.monitor.x - cw * 0.5);
        let left = (center - pw * 0.5).clamp(0.0, (self.monitor.x - pw).max(0.0));
        let pill = egui::Rect::from_min_size(egui::pos2(left, -TOP_OVERLAP), egui::vec2(pw, ph + TOP_OVERLAP));
        // 窗口四周各留出 MARGIN 透明外框：origin 上移 MARGIN 使岛体顶贴屏顶，左右居中。
        let origin = egui::pos2(pill.center().x - w * 0.5, -MARGIN);
        let full = egui::Rect::from_min_size(origin, egui::vec2(w, h));
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
        scr.expand(6.0 * ppp).contains(egui::pos2(cx as f32, cy as f32))
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
        let handle = egui::Rect::from_min_max(
            egui::pos2(pill.left(), pill.top()),
            egui::pos2(pill.right(), (pill.top() + 16.0).min(pill.bottom())),
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
                    let (win, _p) = self.layout_screen();
                    self.dragging = true;
                    self.drag_off = cur - win.min;
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
            self.snap_to_nearest(cur);
            // 松手后 300ms 内不响应悬停，避免光标仍停在岛上立刻又展开
            self.hover_hold = Some(Instant::now() + Duration::from_millis(300));
            // 归位动画：从松手位置平滑滑到顶部吸附点
            let from = egui::pos2(self.last_geom.2, self.last_geom.3);
            let (_f, pill) = self.layout_screen();
            let to = egui::pos2(pill.center().x - (W_EXPANDED + 2.0 * MARGIN) * 0.5, -MARGIN);
            self.settle = Some((from, to, Instant::now()));
        }
        self.drag_last_btn = down;
        if self.dragging {
            // 拖动期间保持置顶，避免落到别的窗口后面
            platform::keep_on_top(self.hwnd);
            let desired = cur - self.drag_off;
            let win_w = W_EXPANDED + 2.0 * MARGIN;
            let cw = self.collapsed_w();
            let local_left = (win_w - cw) * 0.5;
            let min_x = -local_left;
            let max_x = (self.monitor.x - cw - local_left).max(min_x);
            let dx = desired.x.clamp(min_x, max_x);
            let dy = desired.y.max(-MARGIN);
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(dx, dy)));
            self.last_geom.2 = dx;
            self.last_geom.3 = dy;
        }
    }

    fn snap_to_nearest(&mut self, cur: egui::Pos2) {
        let mx = self.monitor.x.max(1.0);
        // 用松手瞬间的光标反推岛体中心（win.min = cur - drag_off；岛体中?= win.min + 半窗宽）?
        // 不依赖上一帧可能滞后的窗口原点，保证能精确贴到最?最右角?
        let cx = cur.x - self.drag_off.x + (W_EXPANDED + 2.0 * MARGIN) * 0.5;
        let margin = (self.collapsed_w() * 0.5) / mx;
        self.cfg.x_frac = (cx / mx).clamp(margin, 1.0 - margin);
        self.cfg.save();
    }

    /// 松手归位动画：从松手时的窗口原点平滑滑到顶部吸附点，?220ms，ease-out 缓动?
    fn step_settle(&mut self, ctx: &egui::Context) {
        let dur = Duration::from_millis(260);
        if let Some((from, to, start)) = self.settle {
            let t = (start.elapsed().as_secs_f32() / dur.as_secs_f32()).clamp(0.0, 1.0);
            // ease-out-back：末端轻微超过目标再回落，形?贴上去再弹一?的手感?
            let c1 = 0.55_f32;
            let c3 = c1 + 1.0;
            let u = t - 1.0;
            let e = 1.0 + c3 * u * u * u + c1 * u * u;
            let p = from + (to - from) * e;
            // 顶部吸附时超调会让窗口略微越过屏幕顶边；限制最多越?6 逻辑像素，避免岛被明显切掉?
            let p = egui::pos2(p.x, p.y.max(-(MARGIN + 8.0)));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(p));
            self.last_geom.2 = p.x;
            self.last_geom.3 = p.y;
            if t >= 1.0 {
                self.settle = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(8));
            }
        }
    }

    fn apply_geometry(&mut self, ctx: &egui::Context) {
        let (full, _pill) = self.layout_screen();
        let (w, h) = (full.width(), full.height());
        let origin = full.min;
        if (w - self.last_geom.0).abs() > 0.5 || (h - self.last_geom.1).abs() > 0.5 {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
        }
        if (origin.x - self.last_geom.2).abs() > 0.5 || (origin.y - self.last_geom.3).abs() > 0.5 {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(origin));
        }
        self.last_geom = (w, h, origin.x, origin.y);
    }

    /// ?圆角区域"把窗口裁剪成当前岛体形状（随动画逐帧跟随，圆角半径与绘制一致）?
    /// 关键的圆角：区域必须是圆角矩形，不能是外接矩形——否则圆角外侧那一小块会留在窗口里
    /// 却是透明的，展开时沿两个圆角闪出白色缺口。这里用圆角区域把圆角也裁掉，形?绘制形状?
    /// 仅当区域(l,t,r,b,rad)变化时才调用 SetWindowRgn?
    fn apply_region(&mut self, ctx: &egui::Context) {
        if self.hwnd == 0 {
            return;
        }
        if std::env::var_os("MYISLE_NO_REGION").is_some() {
            if self.region != (0, 0, 0, 0, 0) {
                platform::clear_window_region(self.hwnd);
                self.region = (0, 0, 0, 0, 0);
            }
            return;
        }
        let ppp = ctx.pixels_per_point();
        // 用“窗口本地坐标”计算岛体矩形（不做屏幕坐标换算）：这样无论窗口当前实际在屏幕何处
        // （拖动时窗口原点每帧变化），区域都与窗口内绘制的岛体严格一致，不会因假设原点不同步而露出窗口边框。
        let win = egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(W_EXPANDED + 2.0 * MARGIN, H_EXPANDED + 2.0 * MARGIN),
        );
        let pill = self.pill_rect(win);

        let winw = (win.width() * ppp).round() as i32;
        let winh = (win.height() * ppp).round() as i32;

        // 关键：窗口区域始终“贴合当前岛体形状”（逐帧跟随动画），绝不暴露整个窗口矩形。
        // —— 这样动画途中不会出现“展开后大小”的整块窗口（含窗口边框/阴影），
        //    只在面板自身范围内改变形状；窗口边缘一栏被裁掉，故看不到边框。
        // 逐帧改变区域可能触发 WM_ERASEBKGND（已由 erase_guard 拦截，不会白擦）。
        let target: (i32, i32, i32, i32, i32) = if !self.island_visible {
            (0, 0, 1, 1, 0)
        } else {
            let pad = (1.0 * ppp).round() as i32;
            let clampi = |v: i32, lo: i32, hi: i32| v.max(lo).min(hi);
            let l = clampi(((pill.left() - win.left()) * ppp).round() as i32 - pad, 0, winw);
            let t = clampi(((pill.top() - win.top()) * ppp).round() as i32 - pad, 0, winh);
            let r = clampi(((pill.right() - win.left()) * ppp).round() as i32 + pad, 0, winw);
            let b = clampi(((pill.bottom() - win.top()) * ppp).round() as i32 + pad, 0, winh);
            let rad_l = self.cfg.radius.min(pill.height() * 0.5).min(pill.width() * 0.5);
            let rad = ((rad_l * ppp).round() as i32 + pad).max(1);
            (l, t, r.max(l + 1), b.max(t + 1), rad)
        };
        if target != self.region {
            // 动画/拖动过程中不要逐帧重建窗口区域：SetWindowRgn 每帧调用会明显掉帧。
            // 过渡期间只“扩张不收缩”（用新旧区域的并集），等动画结束落定后再一次性裁到最终形状。
            let transitioning = (self.expanded - self.target).abs() > 0.002
                || (self.music_amt > 0.001 && self.music_amt < 0.999)
                || self.dragging
                || self.settle.is_some();
            let mut apply = target;
            if transitioning && self.region.0 != i32::MIN && !(self.region.2 <= 1 && self.region.3 <= 1) {
                let (pl, pt, pr, pb, prad) = self.region;
                apply = (
                    target.0.min(pl),
                    target.1.min(pt),
                    target.2.max(pr),
                    target.3.max(pb),
                    target.4.max(prad),
                );
            }
            if apply != self.region {
                let cr = self.cfg.position.corner_radius(1.0);
                let _rt = std::time::Instant::now();
                platform::set_window_region_round(
                    self.hwnd, apply.0, apply.1, apply.2, apply.3, apply.4,
                    cr.nw == 0, cr.ne == 0, cr.sw == 0, cr.se == 0,
                );
                self.perf_region_ms = _rt.elapsed().as_secs_f32() * 1000.0;
                self.region = apply;
            }
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        if !self.island_visible {
            return;
        }
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
        let has = self.music_amt > 0.01;
        if e < 0.5 {
            let a = ((0.5 - e) * 2.0 * 255.0) as u8;
            let amt = self.music_amt.clamp(0.0, 1.0);
            let am = (a as f32 * amt) as u8; // 音乐内容的淡入淡出
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
                // 左侧跳动频谱：底部与歌词文字底部对齐，只有上半部分随动画长高/回落
                let lyric_font = egui::FontId::proportional(14.0);
                // 行框底部会低于汉字实际墨迹底部约 3pt，这里减掉，使频谱底边与歌词/时间文字底边齐平
                let base_y = cy + painter
                    .layout_no_wrap("国".to_owned(), lyric_font, fg)
                    .size()
                    .y
                    * 0.5
                    - 3.0;

                for k in 0..3 {
                    let ph = (t * 4.4 + (k as f64) * 1.4).sin() as f32 * 0.5 + 0.5;
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
                // 中间滚动：优先显示当前歌词（逐字渐绿），无歌词则显示 歌名 · 歌手
                let base = egui::Color32::from_rgba_unmultiplied(200, 205, 218, am);
                // 逐字渐绿开关：关闭时当前句不做绿色填充
                let fill_green = if self.cfg.char_green { green } else { base };
                let text_rect = egui::Rect::from_min_size(
                    egui::pos2(sx + 22.0, pill.top()),
                    egui::vec2((pill.right() - 52.0) - (sx + 22.0), pill.height()),
                );
                let cur = self.cur_lyric_idx();
                if let Some(i) = cur {
                    let line = self.lyr_lines[i].text.clone();
                    let prog = self.line_progress(i);
                    // 用"这一句已经唱了多久"作为滚动相位：换到新的一句时跑马灯从最左重新开始
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
                                egui::FontId::proportional(14.0), obase, obase, 0.0, *olt,
                            );
                        }
                        let ndy = lh * (1.0 - roll);
                        self.draw_lyric_offset(
                            &painter, text_rect, text_rect, ndy, &line,
                            egui::FontId::proportional(14.0), base, fill_green, prog, lt,
                        );
                    } else {
                        self.draw_lyric_line(&painter, text_rect, &line, egui::FontId::proportional(14.0), base, fill_green, prog, lt);
                    }
                } else {
                    let text = if self.music.title.is_empty() {
                        "正在播放".to_owned()
                    } else if self.music.artist.is_empty() {
                        self.music.title.clone()
                    } else {
                        format!("{} · {}", self.music.title, self.music.artist)
                    };
                    self.draw_marquee(&painter, text_rect, &text, egui::FontId::proportional(14.0), fg, t);
                }
                // 右侧时间
                painter.text(
                    egui::pos2(pill.right() - 13.0, cy),
                    egui::Align2::RIGHT_CENTER,
                    &time_s,
                    time_font,
                    fg,
                );
            } else {
                let mut s = format!("{:02}:{:02}", st.wHour, st.wMinute);
                if self.cfg.show_seconds {
                    s = format!("{}:{:02}", s, st.wSecond);
                }
                let centered_a = (a as f32 * (1.0 - amt)) as u8; // 无音乐时居中时间，随音乐淡入而淡出
                let col = egui::Color32::from_rgba_unmultiplied(238, 240, 246, centered_a);
                let font = bold_font(self.cfg.font);
                // 无音乐时只居中显示时间
                painter.text(
                    egui::pos2(pill.center().x, pill.center().y - self.cfg.font * 0.06),
                    egui::Align2::CENTER_CENTER,
                    &s,
                    font,
                    col,
                );
            }
            // 有新版本时，收起态右端显示一个小绿点（不打扰，但能看见）
            if self.upd_ver.is_some() {
                painter.circle_filled(
                    egui::pos2(pill.right() - 6.0, pill.center().y),
                    2.6,
                    egui::Color32::from_rgba_unmultiplied(52, 211, 153, 210),
                );
            }
        } else {
            self.draw_expanded_panel(ui, &painter, pill, e, t);
        }
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
            egui::pos2(rect.center().x, rect.center().y + 14.0),
            egui::Align2::CENTER_CENTER,
            "打开汽水音乐播放后自动显示歌词",
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
    ) {
        let galley = painter.layout_no_wrap(text.to_owned(), font, color);
        let tw = galley.size().x;
        let y = rect.center().y - galley.size().y * 0.5;
        if tw <= rect.width() {
            painter.galley(egui::pos2(rect.center().x - tw * 0.5, y), galley, color);
        } else {
            let clip = painter.with_clip_rect(rect);
            let gap = 40.0;
            let span = tw + gap;
            let off = ((t * 28.0) as f32) % span;
            clip.galley(egui::pos2(rect.left() - off, y), galley.clone(), color);
            clip.galley(egui::pos2(rect.left() - off + span, y), galley, color);
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
    ) {
        let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), base);
        let tw = galley.size().x;
        let y = rect.center().y - galley.size().y * 0.5;
        let scroll = tw > rect.width();
        // 只滚一遍：滚到句尾与右边对齐就停住，不再循环重复展示
        let off = if scroll {
            let max_off = (tw - rect.width()).max(0.0);
            ((t * 30.0) as f32).min(max_off)
        } else {
            0.0
        };
        let clip_p = painter.with_clip_rect(rect);
        // 需要渐绿时，预生成一份 fill 色 galley
        let galf = if prog > 0.0 {
            Some(painter.layout_no_wrap(text.to_owned(), font.clone(), fill))
        } else {
            None
        };
        for k in 0..1 {
            let left = if scroll {
                rect.left() - off + (k as f32) * tw
            } else {
                rect.center().x - tw * 0.5
            };
            clip_p.galley(egui::pos2(left, y), galley.clone(), base);
            // 逐字渐绿：两段（含滚动时的第二份）都按同一进度填充，保证滚动时也持续变绿
            if let Some(gf) = &galf {
                let right = left + self.lyric_fill_dx(&galley, prog);
                let gcl = egui::Rect::from_min_max(
                    egui::pos2(left, rect.top()),
                    egui::pos2(right, rect.bottom()),
                )
                .intersect(rect);
                if gcl.width() > 0.5 {
                    let g = painter.with_clip_rect(gcl);
                    g.galley(egui::pos2(left, y), gf.clone(), fill);
                }
            }
        }
    }

    /// 画一行歌词（支持竖直偏移 dy 与自定义裁剪 clip），供收起态“上滚”动画使用：
    /// base 打底，prog>0 时从左到右用 fill 渐绿；过长则横向滚动（只滚一遍，滚到句尾停住）。
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
    ) {
        let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), base);
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
        } else {
            rect.center().x - tw * 0.5
        };
        painter
            .with_clip_rect(clip)
            .galley(egui::pos2(left, y), galley, base);
        if prog > 0.0 {
            let galf = painter.layout_no_wrap(text.to_owned(), font, fill);
            let right = left + self.lyric_fill_dx(&galf, prog);
            let gcl = egui::Rect::from_min_max(
                egui::pos2(left, y),
                egui::pos2(right, y + gh),
            )
            .intersect(clip);
            if gcl.width() > 0.5 && gcl.height() > 0.5 {
                painter
                    .with_clip_rect(gcl)
                    .galley(egui::pos2(left, y), galf, fill);
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
            self.disc_angle = (self.disc_angle + dt * 0.45) % std::f32::consts::TAU; // 约 14s/圈（放慢）
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
        // 视觉圆盘略往下放，别顶着面板上沿
        let disc_top_nudge = 10.0f32;
        let tt_rect = egui::Rect::from_min_size(
            egui::pos2(
                cx0 - disc_pad,
                rect.top() + CONTENT_PAD_T - disc_pad + disc_top_nudge,
            ),
            egui::vec2(tt_size, tt_size),
        );
        let dc = tt_rect.center();
        let vinyl_r = 64.2 / 180.0 * tt_size; // 黑胶半径（灰环再细约2px）
        let cover_r = 46.6 / 180.0 * tt_size; // 中心封面半径（再放大 ~3px 物理）
        // 1) 最外灰色环
        painter.circle_filled(dc, outer_r, egui::Color32::from_rgba_unmultiplied(54, 58, 66, a));
        
        // 2) 中间黑胶
        painter.circle_filled(dc, vinyl_r, egui::Color32::from_rgba_unmultiplied(18, 19, 24, a));
        for k in 0..9 {
            let rr = cover_r + (vinyl_r - cover_r) * (k as f32 + 0.5) / 9.0;
            painter.circle_stroke(dc, rr, egui::Stroke::new(0.5, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 9)));
        }
        if let Some(ct) = self.cover_tex.as_ref() {
            // 3) 中心圆形封面（随盘旋转）
            let mut mesh = egui::Mesh::with_texture(ct.id());
            mesh.vertices.push(egui::epaint::Vertex { pos: dc, uv: egui::pos2(0.5, 0.5), color: egui::Color32::from_rgba_unmultiplied(255, 255, 255, a) });
            let n = 64usize;
            for i in 0..n {
                let base = (i as f32) / (n as f32) * std::f32::consts::TAU;
                let th = base + self.disc_angle;
                let (s, c) = (th.sin(), th.cos());
                let (sb, cb) = (base.sin(), base.cos());
                mesh.vertices.push(egui::epaint::Vertex { pos: egui::pos2(dc.x + cover_r * c, dc.y + cover_r * s), uv: egui::pos2(0.5 + 0.5 * cb, 0.5 + 0.5 * sb), color: egui::Color32::from_rgba_unmultiplied(255, 255, 255, a) });
            }
            for i in 0..n {
                let a1 = 1 + i as u32;
                let a2 = 1 + ((i + 1) % n) as u32;
                mesh.indices.extend_from_slice(&[0, a1, a2]);
            }
            painter.add(egui::Shape::mesh(mesh));
            
            painter.circle_filled(dc, 2.6, egui::Color32::from_rgba_unmultiplied(11, 14, 19, a));
            painter.circle_filled(dc, 1.0, egui::Color32::from_rgba_unmultiplied(107, 119, 132, a));
        } else {
            // 无真实封面：中心用按歌名哈希的渐变圆 + 音符兜底
            let mut h: u64 = 1469598103934665603;
            for b in self.music.title.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(1099511628211);
            }
            let hue = (h % 360) as f32;
            let top = hsv(hue, 0.55, 0.62, a);
            let bot = hsv((hue + 40.0) % 360.0, 0.62, 0.34, a);
            let n = 40usize;
            for i in 0..n {
                let y0 = -cover_r + 2.0 * cover_r * (i as f32) / (n as f32);
                let y1 = -cover_r + 2.0 * cover_r * ((i + 1) as f32) / (n as f32);
                let half0 = (cover_r * cover_r - y0 * y0).max(0.0).sqrt();
                let half1 = (cover_r * cover_r - y1 * y1).max(0.0).sqrt();
                let hw = half0.max(half1);
                let yy = dc.y + (y0 + y1) * 0.5;
                let strip = egui::Rect::from_min_max(
                    egui::pos2(dc.x - hw, yy - cover_r / n as f32),
                    egui::pos2(dc.x + hw, yy + cover_r / n as f32),
                );
                painter.rect_filled(strip, 0.0, lerp_color(top, bot, (i as f32) / (n as f32)));
            }
            painter.text(dc, egui::Align2::CENTER_CENTER, "♪", egui::FontId::proportional(cover_r * 0.9), egui::Color32::from_rgba_unmultiplied(255, 255, 255, 210));
            
            painter.circle_filled(dc, 2.6, egui::Color32::from_rgba_unmultiplied(11, 14, 19, a));
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

        // 右侧文字区
        let tx = tt_rect.right() + 18.0;
        let title = if self.music.title.is_empty() { "未知曲目".to_string() } else { self.music.title.clone() };
        let artist = self.music.artist.clone();
        let avail = (cx1 - tx).max(40.0);
        let af = egui::FontId::proportional(12.0);
        // 歌名与歌手之间留出固定间隙，连接符「-」在间隙里水平、垂直都居中
        let gap = if artist.is_empty() { 0.0 } else { 16.0 };
        let aw = if artist.is_empty() {
            0.0
        } else {
            ui.fonts(|f| f.layout_no_wrap(artist.clone(), af.clone(), dim).size().x)
        };
        // 歌名自适应字号，确保「歌名 - 歌手」能在一行内展示
        let mut tsz = 16.0f32;
        let mut tw = 0.0f32;
        loop {
            let tf = bold_font(tsz);
            tw = ui.fonts(|f| f.layout_no_wrap(title.clone(), tf, ink).size().x);
            if tw + gap + aw <= avail || tsz <= 11.5 {
                break;
            }
            tsz -= 1.0;
        }
        // 三个元素共用同一条垂直中线，保证歌名、连接符、歌手在同一水平线上
        let title_cy = rect.top() + CONTENT_PAD_T + 41.0 - 10.0;
        painter.text(
            egui::pos2(tx, title_cy),
            egui::Align2::LEFT_CENTER,
            &title,
            bold_font(tsz),
            ink,
        );
        if !artist.is_empty() {
            let mid_x = tx + tw + gap * 0.5;
            let artist_x = tx + tw + gap;
            painter.text(egui::pos2(mid_x, title_cy), egui::Align2::CENTER_CENTER, "-", af.clone(), dim);
            painter.text(egui::pos2(artist_x, title_cy), egui::Align2::LEFT_CENTER, &artist, af, dim);
        }

        // 进度条 + 时间（滑块式）
        let dur = self.duration_est();
        let pos = self.current_pos();
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
        let prev_c = egui::pos2(tx + 14.0, btn_y);
        let rprev = egui::Rect::from_center_size(prev_c, egui::vec2(22.0, 22.0));
        self.ctrl_hit.push(rprev);
        let resp_prev = ui.interact(rprev, ui.id().with("mctrl_prev"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        painter.circle_filled(prev_c, 11.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 18));
        let c_dn = egui::Color32::from_rgba_unmultiplied(200, 210, 220, 200);
        painter.text(prev_c, egui::Align2::CENTER_CENTER, "\u{E892}", icon_font(12.0), c_dn); // Previous
        if resp_prev.clicked() {
            clicked = Some("prev");
        }
        let play_c = egui::pos2(tx + 52.0, btn_y);
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
        let next_c = egui::pos2(tx + 92.0, btn_y);
        let rnext = egui::Rect::from_center_size(next_c, egui::vec2(22.0, 22.0));
        self.ctrl_hit.push(rnext);
        let resp_next = ui.interact(rnext, ui.id().with("mctrl_next"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        painter.circle_filled(next_c, 11.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 18));
        painter.text(next_c, egui::Align2::CENTER_CENTER, "\u{E893}", icon_font(12.0), c_dn); // Next
        if resp_next.clicked() {
            clicked = Some("next");
        }
        if !self.music.app_id.is_empty() {
            painter.text(egui::pos2(cx1, btn_y - 6.0), egui::Align2::RIGHT_CENTER, src_label(&self.music.app_id), egui::FontId::proportional(10.5), faint);
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
                }
                _ => {
                    self.ui_playing = true;
                    self.override_play = Some((true, Instant::now() + Duration::from_millis(1200)));
                }
            }
            self.tracker.send(cmd);
        }

        // ---------- 歌词区（可上下拖动浏览；松手后自动回到当前进度） ----------
        let cur = self.cur_lyric_idx();
        let lfont = 14.0f32;
        let line_font = egui::FontId::proportional(lfont);
        let row_h = 22.0f32;
        let vtop = rect.top() + CONTENT_PAD_T + 146.0 - 10.0;
        let vbot = rect.bottom() - CONTENT_PAD_B - 10.0;
        // 歌词与「圆盘+信息」共用同一条视觉中线：左边也去掉圆盘方框的透明留白。
        let lx0 = cx0 - disc_pad;
        let lyric_rect = egui::Rect::from_min_max(egui::pos2(lx0, vtop), egui::pos2(cx1, vbot));
        let view_h = (vbot - vtop).max(row_h);
        let rows = ((view_h / row_h).floor() as i32).max(1);
        let grid_top = vtop + (view_h - rows as f32 * row_h) * 0.5;
        let anchor = (rows - 1) as f32 * 0.5;
        let center_y = grid_top + anchor * row_h;
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
                    self.draw_lyric_line(&clip, lrect, &self.lyr_lines[i].text, line_font.clone(), gdim, fill, pr, lt);
                } else {
                    self.draw_lyric_line(&clip, lrect, &self.lyr_lines[i].text, line_font.clone(), faint, faint, 0.0, t);
                }
            }
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
        // 左边缘两角跟随岛体本身圆角（顶部贴边为直角、底部为圆角），右侧保持平直，
        // 避免菜单底色与岛体之间出现明显“色块断层”。
        let rail_r = self.cfg.radius.min(pill.height() * 0.5) as u8;
        painter.rect_filled(
            rail,
            egui::CornerRadius { nw: 0, ne: 0, sw: rail_r, se: 0 },
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
            painter.text(egui::pos2(pill.left() + rail_w * 0.5, pill.top() + 15.0), egui::Align2::CENTER_CENTER, &ts, bold_font(fsz), egui::Color32::from_rgba_unmultiplied(233, 236, 242, a));
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

        let eff_shown = if self.show_settings { -2.0 } else { shown_idx };
        // 顶部两个模块的选中高亮（停留在设置页时不显示）
        if !self.show_settings {
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
            draw_nav_icon(painter, *icon_kind, egui::pos2(cx, iy + 14.5), 18.0, icol);
            painter.text(egui::pos2(cx, iy + 33.0), egui::Align2::CENTER_CENTER, *label, egui::FontId::proportional(10.5), icol);
            let resp = ui.interact(item, ui.id().with(("rail", idx)), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                self.show_settings = false;
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

        // 底部「设置」入口（所有模块的公共设置都在这里）
        {
            let set_y = pill.bottom() - item_h;
            let item = egui::Rect::from_min_size(egui::pos2(pill.left() + 9.0, set_y), egui::vec2(rail_w - 18.0, item_h - 8.0));
            if self.show_settings {
                painter.rect_filled(item, 10.0, egui::Color32::from_rgba_unmultiplied(52, 211, 153, (a as f32 * 0.14) as u8));
            }
            let icol = if self.show_settings { accent } else { muted };
            let cx = item.center().x;
            painter.text(egui::pos2(cx, set_y + 14.5), egui::Align2::CENTER_CENTER, "\u{E713}", icon_font(17.0), icol);
            painter.text(egui::pos2(cx, set_y + 33.0), egui::Align2::CENTER_CENTER, "设置", egui::FontId::proportional(10.5), icol);
            let resp = ui.interact(item, ui.id().with("rail_settings"), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() && !self.show_settings {
                self.settings_scroll = 0.0;
                self.show_settings = true;
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

        // ---- 右侧内容区 ----
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
                Module::Weather => self.draw_placeholder_page(&cclip, cslide, e, "天气", "天气模块即将上线"),
            }
        }
        if self.show_settings || svis > 0.001 {
            self.draw_settings_page(ui, painter, content, e, svis);
        }        // 模块切换过渡：用岛体底色做淡入遮罩，让新页面“浮现”
        if switch_p < 0.999 {
            let ma = ((1.0 - switch_p).powf(1.5) * self.cfg.alpha as f32) as u8;
            let mask = egui::Color32::from_rgba_unmultiplied(self.cfg.bg[0], self.cfg.bg[1], self.cfg.bg[2], ma);
            let rr = self.cfg.radius.min(content.height() * 0.5);
            painter.with_clip_rect(content).rect_filled(content, rr, mask);
        }
        // 发现新版本：内容区右上角悬浮一条「发现新版本 vX · 立即更新」小胶囊，点击即下载并安装
        if !self.show_settings && self.music_amt > 0.5 {
            if let Some(ver) = self.upd_ver.as_ref().map(|m| m.version.clone()) {
                let bh = 26.0f32;
                let bw = if self.upd_busy { 168.0 } else { 214.0 };
                let br = egui::Rect::from_min_size(
                    egui::pos2(content.right() - CONTENT_PAD_X - bw, content.top() + 9.0),
                    egui::vec2(bw, bh),
                );
                painter.rect_filled(br, bh * 0.5, egui::Color32::from_rgba_unmultiplied(52, 211, 153, 26));
                painter.circle_filled(egui::pos2(br.left() + 13.0, br.center().y), 3.2, accent);
                let label = if self.upd_busy {
                    format!("正在下载 v{}…", ver)
                } else {
                    format!("发现新版本 v{}", ver)
                };
                painter.text(
                    egui::pos2(br.left() + 23.0, br.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::proportional(11.5),
                    accent,
                );
                if !self.upd_busy {
                    painter.text(
                        egui::pos2(br.right() - 12.0, br.center().y),
                        egui::Align2::RIGHT_CENTER,
                        "立即更新 ›",
                        egui::FontId::proportional(11.5),
                        accent,
                    );
                }
                let resp = ui
                    .interact(br, ui.id().with("upd_banner"), egui::Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if resp.clicked() && !self.upd_busy {
                    self.start_update_install();
                }
            }
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
        if vis > 0.5 && resp.clicked() { self.settings_scroll = 0.0; self.show_settings = true; }
    }

    /// 后台检查更新（不阻塞 UI）。
    fn start_update_check(&mut self) {
        if self.upd_checking || self.upd_busy {
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

    /// 后台下载更新包并在校验通过后启动安装器。
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
        self.upd_msg = format!("正在下载 v{}…", m.version);
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        std::thread::spawn(move || {
            let r = update::download_setup(&m).and_then(|p| update::run_setup(&p));
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
                }
                Ok(None) => {
                    self.upd_ver = None;
                    self.upd_msg = "已是最新版本".to_string();
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
        // 启动后 5s 首查，之后每 4 小时
        if self.upd_rx.is_none() && !self.upd_checking && self.upd_last_check.elapsed() > Duration::from_secs(4 * 3600) {
            self.start_update_check();
        }
    }
    /// 内置设置页（音乐模块内，覆盖右侧内容区）：歌词 / 显示 两组可调项。
    fn draw_settings_page(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, content: egui::Rect, e: f32, vis: f32) {
        let vis = vis.clamp(0.0, 1.0);
        let full = content;
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
        if bresp.clicked() { self.show_settings = false; }

        let left = content.left() + CONTENT_PAD_X;
        let right = content.right() - CONTENT_PAD_X;
        let row_h = 30.0f32;
        let label_h = 22.0f32;
        // 设置内容区可滚动（设置项变多时自动出现滚动）
        let list_top = content.top() + header_h;
        let list_bottom = content.bottom() - CONTENT_PAD_B;
        let view_h = (list_bottom - list_top).max(1.0);
        let total_h = label_h * 2.0 + row_h * 4.0;
        let max_scroll = (total_h - view_h).max(0.0);
        if let Some(ph) = ui.ctx().pointer_hover_pos() {
            if full.contains(ph) && max_scroll > 0.0 {
                let d = ui.input(|i| i.raw_scroll_delta.y);
                self.settings_scroll = (self.settings_scroll - d).clamp(0.0, max_scroll);
            }
        }
        self.settings_scroll = self.settings_scroll.clamp(0.0, max_scroll);
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
        let sl = |y: f32| egui::Rect::from_min_max(egui::pos2(right - 140.0, y + 3.0), egui::pos2(right, y + row_h - 3.0));

        // 分组：歌词
        painter.text(egui::pos2(left, y), egui::Align2::LEFT_TOP, "歌词", egui::FontId::proportional(11.5), muted2);
        y += label_h;
        // 换行滚动动画
        row_label(painter, y, "换行滚动动画", ink);
        { let trect = tg(y); if let Some(v) = Self::draw_toggle(ui, painter, trect, self.cfg.roll_anim, ui.id().with("tg_roll"), accent, a) { self.cfg.roll_anim = v; changed = true; } }
        y += row_h;
        // 滚动时长
        row_label(painter, y, "滚动时长", if self.cfg.roll_anim { ink } else { muted2 });
        { let zone = sl(y); if let Some(v) = Self::draw_slider(ui, painter, zone, self.cfg.roll_ms, 200.0, 800.0, 10.0, " ms", self.cfg.roll_anim, ui.id().with("sl_rollms"), accent, a) { self.cfg.roll_ms = v; changed = true; } }
        y += row_h;
        // 逐字渐绿
        row_label(painter, y, "逐字渐绿", ink);
        { let trect = tg(y); if let Some(v) = Self::draw_toggle(ui, painter, trect, self.cfg.char_green, ui.id().with("tg_green"), accent, a) { self.cfg.char_green = v; changed = true; } }
        y += row_h;

        // 分组：通用（软件更新，所有模块共用）
        painter.text(egui::pos2(left, y), egui::Align2::LEFT_TOP, "通用", egui::FontId::proportional(11.5), muted2);
        y += label_h;
        row_label(painter, y, "软件更新", ink);
        {
            let up_row = egui::Rect::from_min_max(
                egui::pos2(right - 168.0, y + 3.0),
                egui::pos2(right, y + row_h - 3.0),
            );
            let resp = ui.interact(up_row, ui.id().with("upd_row"), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let (txt, col) = if self.upd_busy {
                (self.upd_msg.clone(), accent)
            } else if let Some(m) = &self.upd_ver {
                (format!("v{} 可更新 ›", m.version), accent)
            } else if self.upd_checking {
                ("检查中…".to_string(), muted2)
            } else if self.upd_msg.contains("失败") {
                (self.upd_msg.clone(), egui::Color32::from_rgba_unmultiplied(230, 120, 120, a))
            } else {
                (format!("v{} · 检查更新", update::VERSION), muted2)
            };
            painter.text(egui::pos2(right, y + row_h * 0.5), egui::Align2::RIGHT_CENTER, txt, egui::FontId::proportional(11.5), col);
            if resp.clicked() {
                if self.upd_ver.is_some() {
                    self.start_update_install();
                } else {
                    self.start_update_check();
                }
            }
        }
        y += row_h;

        // 滚动条（内容超出时）
        if max_scroll > 0.0 {
            let track = egui::Rect::from_min_max(
                egui::pos2(full.right() - 5.0, list_top),
                egui::pos2(full.right() - 2.0, list_bottom),
            );
            painter.rect_filled(track, 1.5, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 16));
            let frac = (view_h / total_h).clamp(0.0, 1.0);
            let th = (view_h * frac).max(24.0).min(view_h);
            let ty = list_top + (view_h - th) * (self.settings_scroll / max_scroll);
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(track.left(), ty), egui::pos2(track.right(), ty + th)),
                1.5,
                egui::Color32::from_rgba_unmultiplied(200, 210, 224, 90),
            );
        }

        if changed { self.cfg.save(); }
    }

    /// 开关控件（绘制 + 点击），带滑动/变色动画。返回 Some(新值) 表示被点击切换。
    fn draw_toggle(ui: &mut egui::Ui, painter: &egui::Painter, rect: egui::Rect, on: bool, id: egui::Id, accent: egui::Color32, a: u8) -> Option<bool> {
        let t = ui.ctx().animate_bool_with_time(id.with("anim"), on, 0.16);
        let resp = ui.interact(rect.expand(5.0), id, egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let lerp_u = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        let bg = egui::Color32::from_rgba_unmultiplied(
            lerp_u(78, accent.r()), lerp_u(82, accent.g()), lerp_u(96, accent.b()), a,
        );
        painter.rect_filled(rect, rect.height() * 0.5, bg);
        let r = rect.height() * 0.5 - 2.0;
        let kx0 = rect.left() + r + 2.0;
        let kx1 = rect.right() - r - 2.0;
        let kx = kx0 + (kx1 - kx0) * t;
        painter.circle_filled(egui::pos2(kx, rect.center().y), r, egui::Color32::from_rgba_unmultiplied(246, 248, 251, a));
        if resp.clicked() { Some(!on) } else { None }
    }

    /// 滑块控件（绘制 + 拖动）。返回 Some(新值) 表示被拖动/点击修改。
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

    // ---------------- 设置面板 ----------------
    fn settings_ui(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            self.confirm_open = false;
            self.settings_hwnd = 0;
            self.settings_shown = false;
            return;
        }
        apply_style(ctx, true);
        let c = palette(true);
        let focus = self.focus_settings;
        let mut do_hide = false;
        let mut do_exit = false;
        let mut confirm_open = self.confirm_open;

        // 无系统标题栏、可拖动、自绘；圆角交给 Win11 DWM，避免透明窗口四角发黑?
        let first = !self.settings_shown;
        let mut builder = egui::ViewportBuilder::default()
            .with_title("MyIsle 设置")
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
            egui::ViewportId::from_hash_of("myisle_settings"),
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
                            "MyIsle",
                            egui::FontId::proportional(15.0),
                            c.text,
                        );

                        let header = egui::Rect::from_min_max(
                            egui::pos2(rect.min.x, rect.min.y),
                            egui::pos2(rect.max.x - 48.0, rect.min.y + 48.0),
                        );
                        let drag = ui.interact(header, egui::Id::new("myisle_title_drag"), egui::Sense::drag());
                        if drag.drag_started() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                        }

                        let btn = egui::Rect::from_min_size(
                            egui::pos2(rect.max.x - 42.0, rect.min.y + 12.0),
                            egui::vec2(30.0, 30.0),
                        );
                        let cresp = ui.interact(btn, egui::Id::new("myisle_close"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
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
                            "MyIsle",
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
                    let resp = egui::Modal::new(egui::Id::new("myisle_confirm_modal"))
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
                                ui.label(egui::RichText::new("关闭 MyIsle").size(16.0).strong().color(c.text));
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
            let h = platform::find_process_window("MyIsle 设置");
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
        let _pf_t0 = std::time::Instant::now();
        self.perf_region_ms = 0.0;
        self.perf_netease_ms = 0.0;
        self.poll_update();
        self.ensure_window(frame);
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

        if let Some(ms) = ctx.input(|i| i.viewport().monitor_size) {
            if ms.x > 1.0 && ms.y > 1.0 {
                self.monitor = ms;
            }
        }

        // 音乐模块：主线程非阻塞推进 SMTC 状态机，取最新快照
        let _pf_t_snap0 = std::time::Instant::now();
        self.music = self.tracker.snapshot();
        let _pf_snap_ms = _pf_t_snap0.elapsed().as_secs_f32() * 1000.0;
        // 记录本次快照是否真的来自 SMTC（否则后面的窗口标题兜底会覆盖它）
        self.from_smtc = self.music.has_session && !self.music.title.is_empty();
        // 汽水 CDP 补丁自检：汽水自动更新会覆盖 app.asar 里的调试端口补丁导致歌词失效。
        // 这里每 15s 检查一次，发现没补丁就自动重打（播放中只打文件不重启，暂停/未播放时自动重启生效）。
        if !self.soda_boot_checked {
            self.soda_boot_checked = true;
            // 启动时若汽水没在运行，直接把补丁补好（运行中会被独占锁，交给下面的 5s 自检）。
            crate::soda_patch::boot_check();
        }
        if self.last_soda_heal.elapsed() > Duration::from_secs(15)
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
        // 调试：MYISLE_DEMO="歌名|歌手" 时注入一条虚拟曲目，用于无播放器环境验证界面/动画
        if let Ok(spec) = std::env::var("MYISLE_DEMO") {
            let mut it = spec.splitn(2, '|');
            let t = it.next().unwrap_or("Demo").to_string();
            let ar = it.next().unwrap_or("Demo Artist").to_string();
            self.music.has_session = true;
            self.music.title = t;
            self.music.artist = ar;
            self.music.app_id = "cloudmusic.exe".into();
            self.music.playing = true;
            self.music.position = 42.0;
            self.music.duration = 240.0;
            self.music.has_position = true;
            self.from_smtc = false;
            self.ui_playing = true;
        }
        if let Ok(long) = std::env::var("MYISLE_LONG") {
            // 调试：注入一条超长“歌词”用于检查收起态跑马灯/渐绿渲染
            self.lyr_lines = vec![lyrics::LrcLine { t: 0.0, text: long, segs: Vec::new() }];
            self.lyr_key = "LONG".into();
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
                // 后者只会造成进度被反复往回拽（汽水那种低频率粗上报正是如此）。
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
        // 兜底：若 SMTC 拿不到（本机 SMTC 常超时），改从播放器窗口标题抓 “歌名 - 歌手”（如网易云）。
        if self.media_title_at.elapsed() >= Duration::from_millis(200) {
            self.media_title_cache = if self.music.title.is_empty() {
                platform::media_title_fallback()
            } else {
                None
            };
            self.media_title_at = Instant::now();
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
        // 记录“好标题”：标题稳定时才刷新；短暂读不到时用最近一次的好标题顶住（最多 15s），避免闪烁
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
        // 只要拿到了歌名/歌手（无论来自 SMTC 还是窗口标题兜底），就视为有会话
        if !self.music.title.is_empty() {
            self.music.has_session = true;
        }
        // 播放器退出后：连续 2 秒拿不到歌且播放器进程已不在，就清空残留音乐状态，收起态回到只显示时间
        let _pf_pr0 = std::time::Instant::now();
        let _player_running = if self.music.title.is_empty() {
            self.player_is_running()
        } else {
            true
        };
        let _pf_player_ms = _pf_pr0.elapsed().as_secs_f32() * 1000.0;
        if self.music.title.is_empty() && !_player_running {
            let since = *self.music_lost_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_secs(2) {
                self.music = smtc::Track::default();
                self.media_title_cache = None;
                self.last_title = None;
                self.override_play = None;
                self.spos_valid = false;
                self.clock_playing = false;
                self.ui_playing = true;
            }
        } else {
            self.music_lost_since = None;
        }
        // 音乐模块的出现/消失做平滑过渡：宽度、内容透明度、空态之间渐入渐出
        {
            let dt = ctx.input(|i| i.stable_dt).min(0.05);
            let target_amt = if !self.music.title.is_empty() { 1.0f32 } else { 0.0 };
            let k = 1.0 - (-7.0 * dt).exp();
            self.music_amt += (target_amt - self.music_amt) * k;
            if (self.music_amt - target_amt).abs() < 0.004 {
                self.music_amt = target_amt;
            } else {
                ctx.request_repaint();
            }
            // 淡出结束后再真正清掉歌词/封面，避免淡出过程中文字突然变成“正在播放”
            if self.music_amt < 0.02 && self.music.title.is_empty() {
                self.lyr_lines.clear();
                self.lyr_key.clear();
                self.lyr_dur = 0.0;
                self.lyr_done = false;
                self.trial_start = 0.0;
                self.trial_probe = None;
                self.cover_tex = None;
            }
        }
        // 网易云专用兜底：仅当 SMTC 没有提供可用时间轴时，才读进程内存里的真实位置。
        // （将来播放器通过 SMTC 上报了进度，就自动优先用 SMTC，不再依赖内存偏移。）
        let smtc_timeline_ok = self.from_smtc
            && self.music.has_position
            && self.music.duration > 20.0
            && self.music.position > 0.0;
        if !smtc_timeline_ok && self.player_running_cached && self.netease_at.elapsed() >= Duration::from_millis(500) {
            self.netease_at = Instant::now();
            let _ne_t = std::time::Instant::now();
            let _ne_res = platform::netease_pos_dur().or_else(platform::netease_posdur_cached);
            self.perf_netease_ms = _ne_t.elapsed().as_secs_f32() * 1000.0;
            if let Some((p, d)) = _ne_res {
                self.netease_dur = d;

                self.spos = p;
                self.spos_at = Instant::now();
                self.spos_valid = true;
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
        // 汽水：SMTC 的 position 常常恒为 0.xx 静止值，自维护时钟遇到拖动/重开会漂移，
        // 导致进度条与歌词不跟手。直接从汽水播放器界面读取真实播放秒数来校正。
        if self.from_smtc {
            let a = self.music.app_id.to_ascii_lowercase();
            if a.contains("汽水") || a.contains("soda") || a.contains("qishui") {
                if self.soda_pos_at.elapsed() >= Duration::from_millis(400)
                    && self.soda_prog_rx.is_none()
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
                        let _ = tx.send(crate::cdp_lyric::fetch_progress(dur));
                    });
                }
                let recv = self.soda_prog_rx.as_ref().map(|rx| rx.try_recv());
                match recv {
                    Some(Ok(Some((p, tot)))) => {
                        self.soda_prog_rx = None;
                        self.soda_dur = tot;
                        // 进度条 width 的采样有量化误差：小幅“倒退”是误差，不是真的往回拖，
                        // 直接采用会让逐字高亮退回一两个字。略过小幅倒退，只接受前进或真正的后退拖动。
                        let cur = self.current_pos();
                        let small_back = p + 0.05 < cur && p > cur - 1.2;
                        if !small_back {
                            self.spos = p;
                            self.spos_at = Instant::now();
                            self.spos_valid = true;
                        }
                    }
                    Some(Ok(None)) | Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                        self.soda_prog_rx = None;
                    }
                    Some(Err(std::sync::mpsc::TryRecvError::Empty)) | None => {}
                }
            }
        }        // VIP 试听片段起点探测：播放时长 <60s（试听片段）时，扫描网易云内存日志里
        // 本轮播放开始后的 freeTrialInfo.start，把“片段内相对时间”映射到整首歌绝对时间。
        if let Some((at, n)) = self.trial_probe {
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
        // 切歌检测：标题变化即切歌（轻量节流避免重复请求）
        let title_now = self.music.title.clone();
        let key = title_now.clone();
        if key != self.lyr_key {
            self.lyr_key = key.clone();
            self.lyr_idx_last = -1;
            // 试听起点重置：记下切歌时刻，稍后扫描网易云内存日志里本轮播放的 freeTrialInfo。
            // 若是本次启动后的第一首歌（可能启动时已在播放，日志早于启动时刻），
            // since 留空表示接受内存里最新的一条记录。
            let first_song = self.trial_since.is_empty();
            self.trial_start = 0.0;
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
            // 进度优先用网易云内存真实位置，取不到再用播放器上报
            if let Some((p, d)) = platform::netease_pos_dur() {
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
                let _ = self.lyr_tx.send((key.clone(), self.music.title.clone(), self.music.artist.clone(), dur_now, is_soda));
            }
        } else {
            // 同名不同版本纠偏：拿到播放器真实总时长后，若与当初选歌词用的时长差 >4s，
            // 说明选到了别的版本（时间轴不同，歌词必然对不上）→ 按真实时长重新取一次。
            // 注意：VIP 试听上报的 40s 片段不算“真实总时长”，必须 >=60s 才参与纠偏。
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
        // （否则会一直停在「歌词加载中」或贴上一首）。
        if !self.music.title.is_empty()
            && self.lyr_lines.is_empty()
            && !self.lyr_inflight
            && self.lyr_retry_n < 8
            && self.lyr_retry_at.elapsed() > Duration::from_millis(2200)
        {
            let a = self.music.app_id.to_ascii_lowercase();
            if a.contains("汽水") || a.contains("soda") || a.contains("qishui") {
                self.lyr_retry_at = Instant::now();
                self.lyr_retry_n += 1;
                self.lyr_done = false;
                let dur_now = self.lyric_duration();
                self.lyr_inflight = true;
                let _ = self.lyr_tx.send((
                    self.lyr_key.clone(),
                    self.music.title.clone(),
                    self.music.artist.clone(),
                    dur_now,
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

        // 调试：MYISLE_FAKE_LYRICS=1（配合 MYISLE_DEMO）注入一组按时间推进的假歌词，
        // 用于无播放器环境预览收起态“上滚”动画。
        if std::env::var_os("MYISLE_FAKE_LYRICS").is_some() && self.lyr_lines.len() < 2 {
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

        // 收起态歌词“上滚”动画：检测当前句索引是否变化，换到新一句时触发一次上滚过渡。
        // 同一切歌内才动画（上一句索引仍有效）；切歌时 lyr_key 分支已把 lyr_idx_last 置 -1，故不会误动画。
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
            // 动画超时兜底清理（正常情况下 draw 里到点也会清）
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
        // 调试：展开后若指定了 MYISLE_PAGE，定位对应视图
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
        // 决定窗口是否处于"大窗?态：仅展开?展开动画时为大；收起静止时缩到岛体大小，
        // 从而真正不占用桌面（而不是留一?360x150 的透明大窗口）?
        self.full = self.island_visible && !self.dragging && (self.target > 0.5 || self.expanded > 0.001);
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

        let _pf_t_pre = std::time::Instant::now();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| self.draw(ui));
        let _pf_t_draw = std::time::Instant::now();
        self.settings_ui(ctx);
        let _pf_t_post = std::time::Instant::now();

        // 自适应重绘：展开态、播放中、悬停/拖动时保持 60fps；收起且静止时降到 10fps，
        // 避免空闲也满核渲染导致发热、掉帧和「不丝滑」。
        let pointer_active = ctx.input(|i| i.pointer.has_pointer() || i.pointer.any_down());
        let animating = (self.expanded - self.target).abs() > 0.002
            || self.expanded > 0.01
            || self.music.playing
            || self.music_amt > 0.01
            || self.dragging
            || self.settle.is_some()
            || pointer_active;
        let ms = if animating { 16 } else { 200 };
        if std::env::var_os("MYISLE_PERF").is_some() {
            let el = _pf_t0.elapsed().as_secs_f32() * 1000.0;
            let (dt, udt) = ctx.input(|i| (i.stable_dt, i.unstable_dt));
            if el > 12.0 || dt > 0.020 {
                use std::io::Write as _;
                let line = format!(
                    "{} update={:.1}ms (win={:.1} state={:.1} post={:.1} draw={:.1}) snap={:.1} player={:.1} ne={:.1} region={:.1}ms stable_dt={:.1}ms exp={:.3} tgt={:.3} amt={:.3} playing={}\n",
                    chrono_like_now(),
                    el,
                    (_pf_t_win - _pf_t0).as_secs_f32() * 1000.0,
                    (_pf_t_state - _pf_t_win).as_secs_f32() * 1000.0,
                    (_pf_t_pre - _pf_t_state).as_secs_f32() * 1000.0,
                    (_pf_t_draw - _pf_t_pre).as_secs_f32() * 1000.0,
                    _pf_snap_ms,
                    _pf_player_ms,
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
        ctx.request_repaint_after(Duration::from_millis(ms));
    }
}

fn chrono_like_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let s = d.as_secs() % 86400;
    format!("{:02}:{:02}:{:02}.{:03}", s / 3600, (s % 3600) / 60, s % 60, d.subsec_millis())
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
    if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        if has_segoe {
            list.insert(0, "segoe".to_owned());
        }
        list.push("deng".to_owned());
        list.push("sym".to_owned());
        if has_malgun {
            list.push("malgun".to_owned());
        }
    }
    if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        list.push("deng".to_owned());
        list.push("sym".to_owned());
        if has_malgun {
            list.push("malgun".to_owned());
        }
    }
    let mut bold_list = vec!["segoeb".to_owned(), "deng".to_owned()];
    if has_malgun {
        bold_list.push("malgun".to_owned());
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

    // 调试：MYISLE_LYRIC_TEST="歌名|歌手" 时只跑歌词抓取并打印，不启动界面
    if let Ok(spec) = std::env::var("MYISLE_LYRIC_TEST") {
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

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MyIsle")
            .with_inner_size([W_EXPANDED + 2.0 * MARGIN, H_EXPANDED + 2.0 * MARGIN])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_active(false),
        ..Default::default()
    };
    eframe::run_native(
        "MyIsle",
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