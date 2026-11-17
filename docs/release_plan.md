# 灵岛 LingDao · 发布与自动更新方案（v0.1 草案）

> 状态：2026-11-17 方案讨论稿。当前项目还不是 git 仓库、未配置 GitHub remote。

## 1. 发布链路

- `git init`，`.gitignore` 排除 `target/`、`data/`、`LingDao.exe`、`build_out.txt`；提交 `src/`、`assets/`、`proto/`、`scripts/`、`Cargo.toml`、`Cargo.lock`、`build.rs`、`app.rc`、`README.md`。
- GitHub Actions：打 tag `v*` 触发，Windows runner 构建（注意 `build.rs` 依赖 mingw `windres`，需固定 GNU 工具链或改用 `winresource`）。
- 每个 Release 上传：
  - `LingDao-Setup-x.y.z.exe`：Inno Setup 安装包，**按用户安装**到 `%LOCALAPPDATA%\Programs\LingDao`（无需管理员），创建开始菜单/桌面快捷方式、可选开机自启、带卸载器。
  - `LingDao-x.y.z-portable.zip`：绿色版。
  - `latest.json`：`{version, notes, url, sha256, size, mandatory, pub_date}`。
  - 可选 `latest.json.sig`：Ed25519 签名，客户端内置公钥校验。
- 版本号单一来源：`Cargo.toml` 的 version 由 CI 写入，同时同步 `app.rc` 的 VERSIONINFO。

## 2. 应用内更新器（客户端）

- 启动后 5–8s 首次检查，之后每 4 小时；手动「检查更新」。
- 拉取 `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`（不走 API，避开 60/h 限流）。
- 状态机：`Idle / Checking / Available(info) / Downloading(pct) / Ready / Installing / Error`。
- 下载到 `%LOCALAPPDATA%\LingDao\updates\<version>\`，校验 SHA256（可选签名）→ 校验通过才允许执行。
- 用户点「立即更新」后：后台下载 → 提示「重启并更新」→ 启动安装器
  `LingDao-Setup-x.y.z.exe /SILENT /CLOSEAPPLICATIONS /RESTARTAPPLICATIONS`
  安装器负责关闭旧进程、替换、重新拉起；失败保留旧版可回滚。
- 记住「稍后提醒 / 跳过此版本」到配置。
- 安全：HTTPS + 校验和（+签名），禁止降级；不执行未校验的文件。

## 3. 更新提示 UI（原型见 proto/update_prompt.html）

- **推荐组合：方案 A + 方案 C**
  - A：展开面板顶部 34px 细横幅，只在有更新时出现；`[查看更新] [×]`，点开确认弹窗，不直接升级。
  - C：设置页「通用 → 软件更新」常驻行（当前版本、检查/更新按钮）；齿轮图标上有更新时显示小绿点。
- 备选 B：右上角悬浮卡片（不挤压布局，但会被面板收起吞掉）。
- 收起态可选：胶囊右端 9px 小绿点（默认不开）。
- 更新弹窗：下载进度、后台继续/取消；完成后「重启并更新」。

## 4. 给别人下载前必须补的产品化项

1. **数据目录**：安装版把 `data/`（配置/缓存/日志）迁到 `%APPDATA%\LingDao\data`；开发版仍可用 exe 同目录（用 `portable.flag` 或 `LINGDAO_DATA` 区分），并做旧数据迁移。
2. **汽水安装位置发现**：除现有路径外，增加注册表卸载项、`Program Files`、开始菜单快捷方式解析；启动不依赖桌面「汽水音乐.lnk」。
3. **提权打补丁**：汽水装在 `Program Files` 时写 `app.asar` 需要管理员。增加 `--apply-soda-patch` 自提权（ShellExecute `runas`，一次 UAC）与失败提示；设置页显示「汽水：已连接 / 需修复 / 不支持」状态。
4. **安装器**：Inno Setup per-user 安装、单实例、开机自启选项、卸载保留/清除用户数据可选。
5. **签名**：Authenticode 代码签名（减少 SmartScreen/杀软误报）；至少提供 SHA256 与签名校验。
6. **系统前提**：Windows 10/11 x64；网易云需 Windows 客户端已登录并在播放；汽水补丁属修改第三方客户端，分发需在说明中告知并保留一键还原原版。
7. **状态可见**：设置页展示网易云/汽水连接状态、补丁状态、歌词源，避免「不出歌词但不知道为什么」。

## 5. 需要拍板

- GitHub 仓库地址 / 仓库名（是否 public）。
- 安装方式：per-user 安装包 + 绿色版（推荐）还是只做绿色版。
- 是否做代码签名证书。
- 更新提示选哪个组合（推荐 A+C）。
- 汽水补丁在分发版里默认「自动修复」还是「首次提示用户授权」。
