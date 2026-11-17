# 灵岛 LingDao · Windows 灵动岛

对标开源项目 **岛仔 (Renly1994/xiaotang-daozai)** 的自研实现：**Rust + egui/eframe 0.31**（glow/OpenGL 后端），无 Electron、无 MSVC 依赖。

当前版本：**音乐模块**（网易云音乐为主）+ 桌面音乐托盘图标 + 顶部吸附可拖动刘海。

---

## 目录结构

```
D:\LingDao\
  LingDao.exe          运行中的程序（部署产物，双击/开机自启即用）
  data\                运行时数据（配置/缓存/日志/偏移表），程序自动创建
  Cargo.toml / Cargo.lock
  src\
    main.rs            eframe App：窗口/动画/收起收起态、音乐模块 UI、设置窗
    platform.rs        Win32/DWM FFI：置顶无边框、逐帧窗口区域(SetWindowRgn)、
                       WM_ERASEBKGND 守卫、拖动/命中、网易云内存读取+偏移自动发现
    smtc.rs            Windows SMTC：会话枚举、元数据、播放控制、时间轴
    lyrics.rs          网易云歌词/封面：搜索打分、YRC 逐字、LRC 解析、缓存
    tray.rs            系统托盘（左键无动作，右键仅「退出」）
    paths.rs           数据目录 = exe 同目录\data
  assets\              唱机贴图（底座/唱臂 540x540 RGBA，编译期 include_bytes!）
  proto\               HTML 高保真原型（expanded.html 为母版）+ 唱机切图素材
  scripts\genv.ps1     构建环境（winlibs mingw + 共用 D:\rust 工具链）
  scripts\build.ps1    一键 build --release + 停旧进程 + 覆盖 exe + 启动
```

---

## 构建 / 部署

```powershell
# 一键：编译 + 停旧进程 + 覆盖 D:\LingDao\LingDao.exe + 重启
powershell -ExecutionPolicy Bypass -File D:\LingDao\scripts\build.ps1

# 只编译
. D:\LingDao\scripts\genv.ps1
cd D:\LingDao
cargo build --release
```

要点：
- 工具链是 **stable-x86_64-pc-windows-gnu**（mingw-w64，winlibs MSVCRT），装在 `D:\rust`；`genv.ps1` 里配好 CARGO_HOME / RUSTUP_HOME / CC / AR，并**清空代理环境变量**（本机代理 7890 不通，cargo 必须直连）。
- **覆盖 exe 前必须先 `Stop-Process LingDao`**，否则报“文件正由另一进程使用”。
- 首次编译约 2 分钟，增量约 5–10 秒。

---

## 已实现功能

- **收起态**：刘海形胶囊（顶部贴边、底部圆角），只显示时间；有音乐时自动加宽，显示 频谱动画 + 逐字渐绿歌词（超长跑马灯只滚一遍）+ 时间。收起态频谱为轻量自走动画（3 根绿色柱按正弦相位起伏，不感知音频），保持安静的节奏感。歌词换行时竖向上滚（旧句上移淡出、新句自下方滚入，340ms ease-out）。
- **展开态**（悬浮触发）：左侧导航栏（音乐 / 天气）+ 右侧内容区（宽 520-54）。默认内容为音乐底页：三层圆盘（外灰环 / 黑胶音纹 / 中心圆形真实封面，随播放旋转）+ 唱臂（底座静止、播放 -22° / 暂停 -40° 绕固定支点摆动）+ 歌名与歌手同一行（`歌名 - 歌手`）+ 进度条与时间 + 上一首/播放暂停/下一首 + 音源 + 可用鼠标滚轮上下滚动的整行歌词列表（含作词/作曲等署名；当前句逐字渐绿；滚动与换行均带平滑过渡；停止滚动 2 秒后约 0.25 秒快速平滑回到当前句，回位全过程歌词渲染照常随歌推进）。右上角齿轮按钮打开**岛内设置页**（见下）。
- **左侧独立模块**：左侧导航是彼此独立的模块（音乐 / 天气），点哪个就显示哪个，互不覆盖、也**没有“返回音乐”按钮**。音乐在后台持续运行（播放不受影响），只是切到别的模块时不再绘制它。切换模块时菜单选中项和右侧内容区都有滑入/淡入过渡；完全收起后再展开会回到上一次停留的页面（模块与设置页状态都会保留）。（日程模块暂时移除，后续再加。）
- **岛内设置页**（音乐模块内，右上角齿轮或调试钩子打开，覆盖右侧内容；右上角圆形 ‹ 按钮关闭回音乐页，无标题文字）：分组「歌词」= 换行滚动动画开关 / 滚动时长(200–800ms) / 逐字渐绿。超长歌词跑马灯、收起态频谱固定开启，歌词字号固定 14px，歌词时间偏移采用自动校准（均不再提供手动开关）。设置项较多时内容区支持滚轮上下滚动。所有改动即时生效并写入 `data\config.ini`（键：`roll_anim` `roll_ms` `char_green`；`lyric_shift_ms` `marquee` `lyric_font` `spectrum` 保留兼容但不再由界面修改）。
- **拖动**：按住岛顶部 16 逻辑像素拖动，松手带缓动"轻回弹"吸附到顶部，横向位置按 `x_frac` 记忆。
- **数据源**：网易云音乐 SMTC 元数据 + `cloudmusic.dll` 内存 position/duration（支持最小化/重启后自动对齐；偏移**自动发现并持久化**到 `data\netease_offsets.txt`）+ 网易云 LRC/YRC 歌词与专辑封面（按时长选版本，避免同名不同版本错位）。
- **VIP 试听时长**：VIP 歌常只给 30~40s 试听片段，播放器上报的 duration 是**片段时长**。歌词选版/缩放一律用 `durations.txt` 里 `song/detail` 的完整时长（≥60s），绝不按试听片段等比压缩完整歌词（否则整首歌词会挤成一团、乱跳）；进度条仍按实际片段时长走。旧的“被压扁”坏缓存会被自动识别并重新拉取。若试听片段不是从 0 开始（例如直接从副歌起播），程序会从网易云进程内存日志里的 `freeTrialInfo.start` 读出片段在整首歌里的起点，把它加到播放位置后再匹配歌词，保证高亮行与歌声对齐（非试听歌曲不受影响）。
- 单实例（命名互斥体）、开机自启（`HKCU\...\Run` 带 `--autostart`，写入项名 LingDao）。

---

## 视觉 / 交互规范（务必遵守）

- **按钮不加独立背景色**：所有图标按钮（返回箭头、设置齿轮等）都不要单独的圆形/圆角底色或描边，只画图标本身、颜色与所在页面底色融为一体；悬停也不做背景/颜色高亮，仅保留手型光标与点击。按钮是“浮在背景上的图标”，不是带底板的控件。

---

## 关键常量（src/main.rs）

- 逻辑尺寸：`W_COLLAPSED=150 x H_COLLAPSED=30`、`W_COLLAPSED_MUSIC≈356`、`W_EXPANDED=520 x H_EXPANDED=300`、`MARGIN=6`、`TOP_OVERLAP=3`
- 本机 DPI 200% → 物理窗 1064x624
- 圆盘：`outer_r=68/180*tt_size`、`vinyl_r=64.2/180`、`cover_r=46.6/180`；灰环 `rgb(54,58,66)`、黑胶 `rgb(18,19,24)`；`disc_angle += dt*0.45`（约 14s/圈）
- 动画：展开/收起指数缓动；松手吸附 ease-out-back 260ms

---

## 踩过的坑（改代码前务必看）

1. **窗口透明/白边**：普通 OpenGL 窗口 + 逐帧 `SetWindowRgn` 时，系统会在新暴露区域用“窗口类背景刷”补一帧 → 顶部白边。已用：`WM_ERASEBKGND` 直接 `return 1` 的子类化守卫 + `ensure_erase_guard` 每帧校验（winit 会重设 WNDPROC）+ 持续 16ms 重绘（防 DWM 回收表面）+ 顶部压边 3px。**千万不要在 GL 窗口上用 GDI（GetDC+FillRect）预填充**，会 GDI/GL 混用闪烁。
2. **动画期间不要画文字**：`expanded ∈ (0.02, 0.98)` 时只画背景，否则逐帧重排/裁剪导致歌词闪烁。
3. **网易云进度**：SMTC 对网易云 `haspos=true 但 position=0 / duration=0`（不给时间轴）→ 必须读进程内存。网易云更新会改偏移：程序会**自动发现**（两次快照差分找每秒 +1 的 f64），结果存 `data\netease_offsets.txt`；如异常可删除该文件或建 `data\offset_scan.flag` 强制重扫。
4. **SMTC 服务可能卡死**：`NPSMSvc`（Now Playing Session Manager）。症状=所有 SMTC 调用超时。处理：管理员 `sc stop NPSMSvc_<sid> & sc start NPSMSvc_<sid>` 或重启。
5. **编码**：源码是 UTF-8。**不要用 PowerShell 的 `Get-Content/Set-Content`（PS5.1 默认 cp936）处理源码**，会把中文变乱码；用 edit 工具或 `[IO.File]::ReadAllText/WriteAllText(..., UTF8)`。
6. **DPI**：本机 2880x1800@200%。截图脚本必须先 `SetProcessDPIAware()`，否则 `GetWindowRect/CopyFromScreen` 只拿到逻辑坐标（1/4 屏）。
7. **汽水音乐（SodaMusic）**：目前能通过 SMTC 读到歌名/歌手/时长，但它的 `GetTimelineProperties().Position` **长期上报同一个静止值**（`haspos=true` 但数值不变，只在偶发事件时才跳）。若拿这种“静止上报”反复校准进度，会把不断外推的时钟一次次拉回原点 → 表现为**歌词来回跳/跟不上**。现已改为：只有上报值真正变化（推进到位或拖动）才校准，数值没变就继续自我外推（见 `smtc_rep_pos` 与 `current_pos()`）。但自维护时钟遇到暂停/拖动/重启仍会漂移，所以汽水在播时现在**每 700ms 通过 CDP 从汽水播放器界面读取真实播放秒数**（`cdp_lyric::fetch_progress`，从界面上的 `当前 / 总长` 里挑总长与歌曲时长匹配的那一组），直接校准 `spos`，进度条与歌词索引都以它为准 → 拖动/切歌后也跟手。**歌词来源（2026-11 起）**：①首选 `src/cdp_lyric.rs` —— 通过 CDP 读汽水渲染进程里 `LyricsView` 组件的 `props.lyric.content`，这是汽水官方逐字歌词（`[start_ms,dur_ms]<off,dur,?>字...` 格式，解析成 `LrcLine`+逐字 `Seg`），**迷你条/不显示歌词页时也有**，且完全不联网；②兜底 `src/soda_lyrics.rs` 扫内存里的 `mediaDetail` JSON（只在汽水实际渲染过该曲目歌词页时有效）。**绝不跨源调网易云/QQ/LRCLIB**，从根上杜绝串歌。**进度/总时长也有坑（2026-11 修）**：汽水页面上同时存在两种「时间」——进度条上的 `.slider-tip`（鼠标悬停位置的提示，如 `00:22 / 03:26`）和真正播放条的 `.bottom-player .indicator`（如 `02:50 / 03:26`），必须只读后者；且 SMTC 给的总时长可能是错的（实测某歌 SMTC 报 60s、真实 194s），所以汽水在播时每 400ms 读播放条：总时长取 `.indicator` 的「当前/总长」，当前秒数取**滑块 `.slider-thumb` 在 `.slider-track` 上的百分比**（亚秒精度；注意不能读 `.slider-fill`，它的 left/width 是分段高亮，不是进度），同时校正 `spos` 和 `soda_dur`（`duration_est` 优先用它）。小幅倒退（<1.2s）视为采样误差忽略，避免逐字高亮“唱过去又退回一两个字”。逐字渐绿的像素边界按每个字形实际宽度累加（`lyric_fill_dx`），不再是 `进度×整行宽度`，中英文混排也不漂。另外汽水歌词页常比切歌晚 1~2 秒更新，拿不到歌词时会自动重试（每 2.2s、最多 8 次），避免停在「歌词加载中」。
8. **多播放器同开**：`smtc.rs::pick_session` 只显示一路——优先“正在播放”、多路在播时粘住上次的 app；两个都在播时不会自动跟随你新切的那路（需把另一路暂停）。且 [src/main.rs] 的“网易云内存兜底”不检查当前选中是否网易云，选非网易云时可能串到网易云进度。
9. **网易云歌词按 song id 取（不再靠歌名搜索）**：`src/netease_mem.rs::current_song_id` 从客户端本地播放队列 `%LOCALAPPDATA%\Netease\CloudMusic\webdata\file\playingList`（JSON，每项含 `track.id/name/artists[].name/duration(ms)`，播放时实时更新）里，用 SMTC 的「歌名+歌手+时长(±5s)」精确命中当前歌曲，拿到确切 `id` 后调 `/api/song/lyric?id=<id>`。这样同名/翻唱/伴奏不会再错配（如 Keep the Roses 曾被贴成 Laleh 的 roses）。队列里找不到才回退到歌名搜索（带时长闸门）。
10. **汽水 CDP 破解读官方歌词（关键，已改汽水安装文件）**：汽水是 Electron，但直接用 `--remote-debugging-port` 会被它**丢弃参数**（launcher）或因 DSH 环境里的 `ELECTRON_RUN_AS_NODE=1` 被继承当成 Node 运行而秒退。现做法：给 `D:\qishuiMusic\App\Soda Music\3.8.0\resources\app.asar` 打补丁，在 `entry.js` 顶部前插
    `try{const {app}=require("electron");app.commandLine.appendSwitch("remote-debugging-port","9222");app.commandLine.appendSwitch("remote-allow-origins","*");}catch(e){}`
    然后 `@electron/asar pack ... --unpack "*.node"` 重新打包覆盖（原文件备份在 `D:\LingDao\data\app.asar.bak`，约 8.9MB；`app.asar.unpacked\` 目录不要动）。此后**正常用桌面快捷方式启动汽水即可**，本机 `http://127.0.0.1:9222/json/list` 会出现 3 个 page target，主页面是 `app://resources/main.asar/main.html`。岛通过这些 target 的 WebSocket 执行 `Runtime.evaluate` 读 `LyricsView.props.lyric`。调试脚本：`D:\node\node.exe D:\LingDao\scripts\cdp_eval.js "<JS表达式>" [target子串]`。**自动修复（`src\soda_patch.rs`）**：灵岛运行时每 15s 自检——发现 `app.asar` 里没有调试端口补丁（典型场景：汽水自动更新覆盖了补丁）就**自动重打包补回**（纯 Rust 实现 asar 读写，不依赖 node，日志写 `data\soda_patch.log`，打补丁前自动把原版备份到 `data\app.asar.bak`）。
  - 注意：汽水运行时会**独占锁定 app.asar**，所以补丁只能在汽水关闭时写入。因此未打补丁且汽水在运行时，会自动「关闭汽水 → 打补丁 → 重新拉起」——一次更新只打断这一次，之后长期有效。
  - 若补丁已写入但 CDP 还没生效（刚补完没重启）：未在播放时自动重启汽水；正在播放则等暂停后再重启，不打断听歌。
  - 设置 `LINGDAO_SODA_FORCE_RESTART=1` 可强制播放中也重启；`SODA_HOME` 可指定汽水安装根目录（默认扫 `D:\qishuiMusic\App\Soda Music\*` 和 `%LOCALAPPDATA%\Programs\SodaMusic`，按版本号取最新）。CDP 端口只监听 127.0.0.1，勿改成 0.0.0.0。
  - **托盘也不掉帧**：补丁里同时注入了 `--disable-background-timer-throttling`、`--disable-renderer-backgrounding`、`--disable-backgrounding-occluded-windows`、`--disable-features=CalculateNativeWinOcclusion`，所以汽水缩到托盘/隐藏时渲染进程不被节流，CDP 读到的时间与歌词继续推进（实测隐藏 10s，进度正好走 10s）。
  - **启动即自愈**：灵岛启动时 `soda_patch::boot_check()` 若发现汽水在运行但补丁缺失（比如更新后），会自动关一次汽水→重打补丁→重新拉起；已打补丁但 CDP 持续 60s 不通（渲染进程崩了）也会强制重启一次。全程无需用户操作。
  - **网易云同样全自动**：`platform::start_netease_reader()` 每 200ms 直接读 `cloudmusic.dll` 内存里的 position/duration（最小化/托盘精确），版本更新导致偏移失效时后台 `netease_ensure_offsets()` 用两次快照差自动重新发现偏移，不用人工改；歌词按本地播放队列里的 song id 精确取（`netease_mem`）。

---

## 调试钩子（环境变量）

- `LINGDAO_DEMO="歌名|歌手"`：注入虚拟曲目（无播放器时看 UI）
- `LINGDAO_LONG="一段很长的歌词"`：注入超长歌词测跑马灯
- `LINGDAO_FAKE_LYRICS=1`：注入 8 句假歌词（每 2.5s 换行，配合 `LINGDAO_DEMO` 可无播放器预览收起态歌词上滚动画）
- `LINGDAO_PAGE=settings|weather`：启动后展开即自动打开对应页面（截图/调试用；设置页可免手点齿轮）
- `LINGDAO_LYRIC_TEST="歌名|歌手"`：只抓歌词并打印 `lines=N` 后退出
- `LINGDAO_OFFSET_SCAN=1`：启动时强制重扫网易云内存偏移

---

## 下一步（TODO）

- 天气模块实体内容（左侧菜单已就位，暂为「即将上线」占位页）；日程模块先行移除，后续再加
- 其它播放器适配（QQ 音乐/酷狗等，需要各自的元数据/进度/歌词来源）
- 汽水试听（VIP 30s）时，进度条总时长仍取 SMTC 上报值，可能出现 `00:38 / 00:30` 的显示；歌词时间本身按官方完整时长走（不影响同步，只影响总时长显示）
- 把 `data\config.ini` 的其余项（位置/字号/圆角等）也搬进岛内设置页可视化调节（目前设置页只覆盖歌词/显示相关项）

---

## 发布与自动更新（2026-11 起）
- 有更新时：展开态内容区右上角显示「发现新版本 vX · 立即更新」胶囊，点击即下载并静默安装重启；收起态右端显示一个小绿点提示。

- **本地打包**：`powershell -ExecutionPolicy Bypass -File scripts\pack.ps1 -Version 0.2.0 -Repo owner/repo`
  - 产物在 `dist\`：`LingDao-<ver>-portable.zip`、`LingDao-Setup-<ver>.exe`（需安装 Inno Setup 6）、`latest.json`。
  - `scripts\set_version.ps1` 会同步更新 `Cargo.toml` 与 `app.rc` 的版本号。
- **GitHub 自动发布**：打 tag 即可，例如 `git tag v0.2.0 && git push origin v0.2.0`。
  - `.github/workflows/release.yml` 在 windows-latest 上构建、跑测试、生成绿色版 zip、Inno 安装包、`latest.json`，并上传到 Release。
  - 构建时注入 `LINGDAO_REPO=${{ github.repository }}`，客户端据此拼 `https://github.com/<repo>/releases/latest/download/latest.json`。
- **代码签名**：见 `docs/SIGNING.md`（配置 `WINDOWS_PFX_BASE64` / `WINDOWS_PFX_PASSWORD` 两个 Secret 后，CI 自动签名 exe 和安装包）。
- **客户端更新**：`src/update.rs` 启动 5s 后首查、此后每 4h；`设置 → 通用 → 软件更新` 显示版本/检查/下载/安装；安装器以 `/SILENT /CLOSEAPPLICATIONS /RESTARTAPPLICATIONS` 运行。
- **数据目录**：发布版用 `%APPDATA%\LingDao\data`；开发/绿色版若 exe 同目录已有 `data\` 则沿用；可用 `LINGDAO_DATA_DIR` 覆盖、`LINGDAO_PORTABLE=1` 强制本地 data。