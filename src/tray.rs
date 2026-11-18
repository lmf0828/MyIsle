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
            .with_tooltip("WinIsle")
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

/// 托盘图标：与 exe 图标同一套「WinIsle」品牌（深色圆角方块 + M 立于青绿孤岛）。
/// 32×32 RGBA 由 assets/winisle_32.rgba 在构建期嵌入，避免运行时绘制差异。
fn make_icon_rgba(_w: u32, _h: u32) -> (Vec<u8>, u32, u32) {
    (include_bytes!("../assets/winisle_32.rgba").to_vec(), 32, 32)
}
