# HANDOFF · WinIsle 交接说明

> 这是一份**跨会话交接单**。新会话读到这里时，请把下面内容当作既有事实，不要重新试错。
> 最后更新：2026-11-16 14:00（上一会话收尾时写入）。

## 1. 项目是什么

Windows 桌面「灵动岛」工具，对标开源项目岛仔（Renly1994/xiaotang-daozai）。
技术栈：**Rust + egui/eframe 0.31（glow/OpenGL 后端）**，无 Electron、无 MSVC（mingw-gnu）。
当前功能：顶部吸附刘海 / 悬停展开 / 可拖动+轻回弹吸附 / 音乐模块（网易云为主）。

## 2. 目录 & 构建

- 程序：`D:\LingDao\WinIsle.exe`（正在运行的那个）
- 源码：`D:\LingDao\src\`（main.rs / platform.rs / smtc.rs / lyrics.rs / tray.rs / paths.rs）
- 资源：`D:\LingDao\assets\`（唱机贴图 RGBA，include_bytes 编译进 exe）
- 原型：`D:\LingDao\proto\expanded.html`（母版）+ 切图素材
- 数据：`D:\LingDao\data\`（config.ini / cache / netease_offsets.txt / 日志）——**exe 同目录的 data**，不写 C 盘
- 构建+部署（一键，先停旧进程再覆盖 exe 再启动）：
  ```powershell
  powershell -ExecutionPolicy Bypass -File D:\LingDao\scripts\build.ps1
  ```
- 环境变量脚本：`D:\LingDao\scripts\genv.ps1`（D:\rust 的 winlibs mingw，清代理）

## 3. 最近已修的坑（重要，别回退）

1. **数据目录**：全部走 `crate::paths::file()/ensure()` → `D:\LingDao\data`。
2. **网易云是多进程**（1 主 + 1 渲染 + 1 GPU，都是 cloudmusic.exe）：
   `platform::netease_pos_dur()` 只认 **有可见播放窗口的那个进程**（`active_cloudmusic_pid()` 排序优先），
   否则会读到 GPU/渲染子进程里别的歌的旧 position/duration（此前 184s/209s 乱值）。
3. **歌词匹配必须极严格**（`src/lyrics.rs`）：
   - 归一化歌名（去空格/标点/大小写）**完全一致 +10 分**；仅"包含" +4；
   - 歌手一致 +6；
   - 三级池：`>=16`（歌名+歌手）→ `>=10`（歌名）→ `>=4`；**只靠歌手匹配的其它歌一律排除**
     （曾把同歌手《不还你》的英文副歌贴到《爱过我的每一天》上）。
   - 候选搜索 `limit=30`；同池内按时长最接近排序（差 >8s 排后）；最终差 >2.5s 按比例缩放整条时间轴。
   - 缓存 key = 歌名+歌手+时长 5s 分桶；`data\cache\durations.txt` 记 `歌名\t歌手\t毫秒`。
4. **总时长优先级**（`duration_est`）：SMTC(≥20s) → `netease_dur`（内存）→ `netease_posdur_cached()` → `known_duration`（网易云接口缓存）→ 歌词末句+6。
5. **网易云 total duration / position** 只能读进程内存：偏移 `pos=0x21a84d8 / dur=0x2215798`（映像 41058304 版），
   旧版 41054208 用 `0x21a74d8/0x2214778`；`netease_ensure_offsets()` 会自动发现并写 `data\netease_offsets.txt`。
   **注意：自动发现前提是“正在播放、位置在走”**；暂停时 `best_d=0` 会放弃（日志有 `snap ... best_d=0.000`，属正常）。
6. **窗口/绘制**：透明窗口逐帧 `SetWindowRgn` 时系统会补背景帧 → 顶部白边；已用
   `WM_ERASEBKGND return 1` 守卫 + `ensure_erase_guard`（每帧校验 WNDPROC）+ 16ms 持续重绘 + 顶部压边 3px。
   **禁止在 GL 窗口上用 GDI（GetDC/FillRect）预填充**（会闪）。动画中 `expanded∈(0.02,0.98)` 只画背景不画字。
7. **编码**：源码 UTF-8。**绝不能用 PS5.1 的 `Get-Content/Set-Content`（cp936）处理源码**；用 edit 工具或 `[IO.File]::ReadAllText/WriteAllText(..., UTF8)`。
8. `scripts/build.ps1` 已修：必须等 WinIsle 进程真正退出再覆盖 exe（否则“文件被占用”）。
9. **多语言歌词乱码/方块**：歌词含韩文（如《ONLY》金新菲Feebie 的 `라랄라랄라라 부르는 노래`、`내 사랑`）时会显示成方块 □——因为原来只加载 Segoe UI / 等线(Deng)，二者都不含 Hangul。已在 `install_fonts`（`src/main.rs`）追加 `C:\Windows\Fonts\malgun.ttf`（맑은 고딕）作为 Proportional / Monospace / bold 家族的**末位兜底**（放在 deng 之后，中文仍优先走 Deng，不受影响）。再遇其它语种方块同法补字库（日文 `meiryo.ttc`/`YuGoth`、泰文 `Leelawadee` 等）。

## 4. 已验证结论（2026-11-16）

- 《爱过我的每一天》林忆莲 → 正确选中 `id=256476`（score=16，234s，专辑《属于我的林忆莲》），歌词“谁的眼让我看不见你的脸…”。
- 《背叛》曹格 → 读 `pos/dur` = 3.93/321.67s（05:21），歌词“雨/不停落下来/花…”。
- 收起态/展开态/唱臂/三层圆盘均已按用户逐轮反馈调好（常量见 README）。

## 5. 已知限制 / 待办

- 汽水音乐：窗口标题无歌名 + SMTC 不给时间轴 → 目前读不到（只支持网易云）。
- 某首歌网易云**确实没有对应版本**时，只能取最接近版本 + 等比缩放，仍可能有秒级偏差（宁可近似也不乱配）。
- 展开区其它模块（天气/日历/未读）未做；托盘右键目前只有“退出”；岛内设置面板未做（用户想以后做“岛内设置模块”）。
- 自启：`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下 `WinIsle`（带 `--autostart`）。

## 6. 调试钩子

- `WINISLE_DEMO="歌名|歌手"`：注入虚拟曲目（无播放器时看 UI）
- `WINISLE_LONG="长歌词"`：注入超长歌词测跑马灯
- `WINISLE_LYRIC_TEST="歌名|歌手"`：只抓歌词打印 `lines=N` 后退出
- `WINISLE_OFFSET_SCAN=1` 或建 `data\offset_scan.flag`：强制重扫网易云内存偏移
- 日志：`data\lyrics_debug.log`（选词评分/缩放）、`data\netease_tl.log`（pos/dur/偏移扫描）、`data\smtc_debug.log`

## 7. 跨会话怎么用这份文件

新会话没有旧对话的上下文。请在第一条消息里对新的我说：
> 继续 D:\LingDao 的WinIsle项目，先读 D:\LingDao\README.md 和 D:\LingDao\HANDOFF.md，以这两份为准。

这样新的我就知道：目录、构建方式、已经踩过的坑、当前状态与待办，不需要重新摸索。
