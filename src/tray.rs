//! 系统托盘小图标（基于 tray-icon / muda 跨平台封装）。
//! 常驻通知区域；右键 → 菜单（仅"退出"一项）。左键不弹菜单、不打开任何窗口。

use tray_icon::menu::{Menu, MenuEvent, MenuItem, MenuId};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub enum TrayAction {
    Exit,
}

pub struct Tray {
    _icon: TrayIcon,
    exit_item: MenuId,
}

impl Tray {
    /// 创建托盘图标；失败（极少数环境）返回 None，应用照常以灵动岛形式运行。
    pub fn new() -> Option<Tray> {
        let menu = Menu::new();
        let exit = MenuItem::new("退出", true, None);
        menu.append_items(&[&exit]).ok()?;

        let (rgba, w, h) = make_icon_rgba(32, 32);
        let icon = Icon::from_rgba(rgba, w, h).ok()?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false) // 只有右键弹菜单（仅退出）；左键无动作
            .with_tooltip("MyIsle")
            .with_icon(icon)
            .build()
            .ok()?;

        Some(Tray {
            _icon: tray,
            exit_item: exit.id().clone(),
        })
    }

    /// 非阻塞读取一次托盘事件。
    pub fn poll(&self) -> Option<TrayAction> {
        while let Ok(_ev) = TrayIconEvent::receiver().try_recv() { /* 左键点击不触发任何动作 */ }
        while let Ok(me) = MenuEvent::receiver().try_recv() {
            if me.id == self.exit_item {
                return Some(TrayAction::Exit);
            }
        }
        None
    }
}

/// 运行时生成托盘图标：深色圆角方块 + 居中青绿圆点（与灵动岛品牌一致）。
fn make_icon_rgba(w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let fw = w as f32;
    let fh = h as f32;
    let cx = fw / 2.0;
    let cy = fh / 2.0;
    let rad = 7.0f32; // 圆角
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let dx = (fx - fw / 2.0).abs() - (fw / 2.0 - rad);
            let dy = (fy - fh / 2.0).abs() - (fh / 2.0 - rad);
            let ox = dx.max(0.0);
            let oy = dy.max(0.0);
            let outside = (ox * ox + oy * oy).sqrt() - rad;
            let idx = ((y * w + x) * 4) as usize;
            if outside <= 0.0 {
                let mut c = [30.0f32, 32.0, 38.0];
                let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
                if d < 6.4 {
                    let t = (d / 6.4).clamp(0.0, 1.0);
                    c = [
                        94.0 * (1.0 - t) + 16.0 * t,
                        240.0 * (1.0 - t) + 185.0 * t,
                        182.0 * (1.0 - t) + 129.0 * t,
                    ];
                }
                buf[idx] = c[0] as u8;
                buf[idx + 1] = c[1] as u8;
                buf[idx + 2] = c[2] as u8;
                buf[idx + 3] = 255;
            }
        }
    }
    (buf, w, h)
}
