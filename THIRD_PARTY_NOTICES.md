# 第三方组件与素材授权声明（Third-Party Notices）

**WinIsle** 本体以 [MIT License](LICENSE) 发布，版权所有 © 2026 WinIsle contributors。

WinIsle 在开发与运行中使用了以下第三方开源组件与素材。它们各自遵循其原始许可协议，版权归原作者所有。分发 WinIsle 时请一并保留本文件及相关许可声明。

## Rust 依赖库

WinIsle 基于 Rust 生态构建。以下为直接依赖（含传递依赖）的许可概览；完整依赖树见 `Cargo.lock`。

| 组件 | 用途 | 许可协议 |
| --- | --- | --- |
| [egui](https://github.com/emilk/egui) / [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) | 即时模式 GUI 框架 | MIT OR Apache-2.0 |
| [glow](https://github.com/grovesNL/glow) | OpenGL 绑定（eframe 后端） | MIT OR Apache-2.0 |
| [winit](https://github.com/rust-windowing/winit) | 窗口与事件（eframe 底层） | Apache-2.0 |
| [tray-icon](https://github.com/tauri-apps/tray-icon) | 系统托盘 | MIT OR Apache-2.0 |
| [windows](https://github.com/microsoft/windows-rs) | Win32 / WinRT（SMTC、DWM 等） | MIT OR Apache-2.0 |
| [raw-window-handle](https://github.com/rust-windowing/raw-window-handle) | 原生窗口句柄 | MIT OR Apache-2.0 |
| [ureq](https://github.com/algesten/ureq) | HTTP 客户端（更新检查） | MIT OR Apache-2.0 |
| [serde_json](https://github.com/serde-rs/json) | JSON 解析 | MIT OR Apache-2.0 |
| [image](https://github.com/image-rs/image) | 图像解码 | MIT OR Apache-2.0 |

> 上述组件多为 MIT / Apache-2.0 双许可，可任选其一遵循。相应完整许可文本见各组件的上游仓库。

## 字体

| 资源 | 许可协议 |
| --- | --- |
| Segoe UI / Segoe MDL2 Assets（Windows 系统字体，仅调用系统已安装字体） | 随 Windows 系统授权，不随本程序分发 |

## 美术素材

| 资源 | 来源 | 许可协议 |
| --- | --- | --- |
| 待机动画表情（Otto Emoji GIF Component） | 作者 **小鹏（txp666）**；经 [joelbaby/esp32-oled-animated-faces](https://github.com/joelbaby/esp32-oled-animated-faces) 整理与生成脚本 | MIT（全文见下方附录） |
| 唱机贴图 / 图标 / 界面插画 | WinIsle 原创（© 2026 WinIsle contributors） | MIT |

## 说明

- 本项目对第三方组件的使用遵循其各自的许可条款；本项目自身代码的 MIT 许可**不覆盖**上述第三方组件与素材。
- 如你是上述任一作品的版权持有人，认为本项目使用方式不妥，请通过 <https://github.com/lmf0828/WinIsle/issues> 联系我们，我们会及时处理。

---

## 附录：Otto 表情素材 MIT 许可证全文

Copyright (c) 2025 小鹏（Otto Emoji GIF Component）
Copyright (c) 2026 esp32-oled-animated-faces contributors

MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
