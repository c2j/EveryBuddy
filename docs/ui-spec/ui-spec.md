# EvoBuddy UI 规格 v0.4

本文件是 WorkBuddy 桌面端主工作区的**复刻规格**：把观察到的设计系统、布局、组件与动效固化成可实现的契约。技术路线已定为 **GTK 3 原生（gtk-rs）**，本文件以该路线为准。

- 状态：**v0.4，已覆盖"主工作区"的欢迎态与会话态（任务/对话/结果面板三栏）**。§4.3、§4.4 的几何均为 **CDP 实测**，两个状态都已按实测重建并逐项校验
- 命名：包名 `evobuddy`、可执行文件名 `evobuddy`、界面显示名 **EvoBuddy**（工作目录名 `EveryBuddy` 是历史遗留，不代表产品名）
- 同目录：`tokens.css`（设计 token 实文件）、`extract_tokens.js` / `build_gtk_tokens.js`（导出与编译脚本）、`gtk/`（GTK 产物）、`capture_dom.js` / `query_layout.js` / `watch_dom.js`（CDP 抓取、查询与连续录制工具）
- 尚未覆盖：设置面板、预览器、编辑器、终端、多智能体协作条等（见 §6、§10）

> v0.3 → v0.4 的主要变化：连续录制（21 帧）拿到会话态真实 DOM 后，三栏几何全部改为实测；发现分隔条其实是**全透明**覆盖层（不是可见分隔线）、顶栏只有两个操作按钮、工具行类名前缀是 `cr-tool-head` 而非早期 CSS 里的 `assistant-message-*`。另修正了 `gtk_widget_draw` 在有待分配控件时静默不画图导致的"空截图"陷阱（§10）。

> v0.2 → v0.3 的主要变化：拿到真实 DOM 与计算样式后，推翻了几处初稿假设——界面是**英文**、欢迎态是**两栏且无结果面板**、品牌标题**居中**、侧栏** 280px** 且带 4px 可拖拽分隔条。孤立的静态解包不足以支撑像素级复刻，见 §4。

---

## 1. 来源与置信度

| 项 | 值 |
|---|---|
| 产品 | WorkBuddy AI（`com.workbuddy.workbuddy-ai`） |
| 版本 | **5.8.4** |
| 版权 | Copyright © 2026 Tencent Technology (Shenzhen) Co., Ltd. |
| 形态 | **Electron**（`CFBundleExecutable = Electron`），`app.asar` 96 MB |
| 运行时 | Electron **37.10.3** → Chromium **138** / Node 22（实测 `Electron Framework` 版本，并经 CDP UA 字符串确认） |
| 渲染层 | **React + Vite/Rolldown** SPA |
| 素材规模 | 1164 JS / **143 CSS** / 221 webp / 73 png / 38 svg / 27 woff2 + 21 woff + 22 ttf |
| 设计系统 | `tokens.css` 148 KB，**2944 条** `--wb-*` 声明 |
| 运行环境 | app home `~/.workbuddy-ai/`（自带 `plugins/` 189 MB、`binaries/` 400 MB、SQLite `workbuddy.db`） |
| 语言 | 实测 locale `en-US`，界面为**英文**；中文（助理/项目/专家/自动化/更多）是对应槽位的翻译 |

**证据等级**：

| 证据 | 等级 | 范围 |
|---|---|---|
| 静态解包 + 窗口注册表 | 高 | 素材清单、窗口尺寸、token 全量 |
| **CDP 实测（v0.3 新增）** | 高 | 欢迎态 DOM 结构、计算样式、精确几何（§4.3、§5.1） |
| **CDP 连续录制（v0.4 新增）** | 高 | 会话态三栏 DOM、消息/工具行/输入区/结果面板几何、类名清单（§4.4、§5.2） |
| 类名推测 | 中 | 设置面板、编辑器、终端等尚未实机采集的部分（§5.2 下半表、§6） |
| 动效时长 | 中 | 来自 token 定义，未逐条实测（§7） |

> **不采信"CSS 里写过"**：`main-content-core.css` 里的 `message-*` / `assistant-message-*` / `chat-container*` 等命名在会话态实测里一个都没出现（§5.2）。CSS module 只能说明"某版本曾存在"，不能说明当前 DOM 用什么。

> 采集方式：只读解包到临时目录 + 只读 CDP（`DOMSnapshot` / `CSS.getComputedStyleForNode`）。原始 App 未做任何修改；抓取含个人工作区内容的产物只写入临时目录，不入仓库。


## 2. 技术路线（已定）：GTK 3 原生（gtk-rs）

**决定**：用 **GTK 3 + gtk-rs** 实现，不引入任何自带 Web 引擎。

理由：三条硬约束（macOS / Windows 7-8.1 / Linux glibc ≥ 2.28 且必须完全自包含）把 Web 引擎路线基本排除——WebView2 在 Win7 止步于 runtime 109，系统 WebKitGTK 在目标 Linux 上过旧，自带 Chromium 又受 Win7 的 109 天花板限制。

| 平台 | 引擎 | 说明 |
|---|---|---|
| macOS | GTK 3（Homebrew `gtk+3` 3.24.x） | **已实测**：CJK 输入、字体、渲染、CSS 加载均正常 |
| Windows | GTK 3（MSYS2 构建） | **未实测且存疑**：README「Prebuilt binaries」记着 MSYS2 自 2024-05 起不再支持 Win7/8.0，因此"没有可维护的 GTK 3 运行时"；这与本表原先"无 Win7 门槛"的说法直接冲突。二者必有一错，只有真机实测能定（缺口 23）。注意构建环境与目标机是两件事：MinGW-w64 产物未必受 MSYS2 运行时约束，但 GLib 新版可能已调用 Win8+ API。 |
| Linux | GTK 3（自带整套栈） | **已实测**：自带 GTK 3 + schema + 图标 + 字体，在裸 glibc 2.28 上可运行，不依赖系统 GTK（§2.1） |

**代价**：GTK 不是 Web 引擎，`tokens.css` 不能直接用（见 §3.5）。保真度靠 **token 对齐 + 逐屏校准**，而不是靠复用产品 CSS。

> 因此原 §8 的 "Chromium 109 降级对照" 不再是实现约束——**GTK 3 的 CSS 子集**才是。

### 2.1 待确认的风险：Linux "完全自包含"

目标 Linux 要求**完全自包含、不依赖系统库**。GTK 3 严格来说也是系统依赖，好消息是这整套栈**可以**自带。

**已实测（C 探针，非本程序）**：在 `debian:10`（Debian 10.13 / **glibc 2.28**，正好是要求的下限）里用 `libgtk-3-dev`（GTK **3.24.5**）编译一个等价探针，按下面清单打包成 74MB / 72 个 `.so` 的目录树后，在**另一个裸 `debian:10`**（`dpkg` 里没有任何 gtk/pango/cairo/fontconfig/harfbuzz/glib 包，**0 个字体文件**）里由 Xvfb 启动成功：GTK 起来、CSS 解析、窗口 1440×868、**CJK 字形正常渲染**（非豆腐块），并渲染出 PNG。

- 依赖：65 个已解析库中只有 6 个来自宿主，且正是刻意留给系统的 glibc 家族（`libc/libm/libpthread/librt/libdl/libresolv`）；glibc 应留在系统（向前兼容，且自带 libc 会破坏 NSS）。**要求的最小符号版本 = `GLIBC_2.25`**（≤ 2.28 ✓）。
- 必须在 glibc 2.28 的镜像里构建：符号版本由构建环境决定，在更新的发行版上编出来的二进制到 2.28 上直接起不来。rustup 自带工具链的 glibc 基线（2.17）比镜像更老，因此不冲突。
- 需要自己打包的 GTK **数据**：`org.gtk.Settings.*` 等 GSettings schema（**必须 `glib-compile-schemas` 编译**，且要连非 `*.gschema.xml` 的 enum 文件一起，否则后者报 "not (yet) defined"）、hicolor 图标、输入法模块、gdk-pixbuf loader，以及字体。**Adwaita 主题不用管**——它作为 GResource 编译在 `libgtk-3-0` 里。
- 两份 cache（immodules / pixbuf loaders）里是**构建期绝对路径**，需要用占位符 + 启动器在运行时改写；fontconfig 需要一个只有自带字体的配置与可写 cachedir。
- 字体必须自带（我们不带产品字体）：DejaVu 覆盖拉丁、WenQuanYi Micro Hei 覆盖 CJK。宿主假定零字体。
- 两个坑：`gdk-pixbuf-query-loaders` / `gtk-query-immodules-3.0` 在 Debian 上装在 multiarch libdir 下、**不在 PATH**；Debian 10 已 EOL，apt 要指向 `archive.debian.org` 并放宽 valid-until。
- **CJK 输入法**：Debian 默认的 immodules 只有 xim / cedilla 等，**没有 `im-ibus`**（它跟着 `ibus` 包走）。要支持中文输入需自带 `im-ibus.so` + `libibus-1.0.so.0`，或依赖已自带的 XIM。

**本程序已实测**：`cargo build --release --bin evobuddy` 在 `debian:10` + GTK 3.24.5 上零错误（gtk-rs 0.19，58 秒），打包成的 74MB / 72 `.so` 目录树在**另一个裸机上**用 `EB_SELFTEST=1 EB_SHOT=` 跑通：`css: parsed 72598 bytes`、`window 1440x868 (want 1440x868)`、`sash sidebar x=280`，几何逐项与 macOS 一致（`eb-sidebar 280x868`、`eb-content-topbar 281,0 1159x64`、`eb-composer-card 460,333 800x144`、`eb-detail <hidden>`）。渲染出的 PNG 采样为 `#f7f7f7` 侧栏 / `#ffffff` 路由，与 §4.3 实测值一致；与 macOS 的唯一差异是**文本宽度**（hero 151 vs 137、声明 330 vs 311），因为字体换成了自带的 DejaVu + WenQuanYi——这是"字体必须换"的必然结果（缺口 7）。

**下限没有余量**：整套产物要求的最小符号版本 = `GLIBC_2.28`，正好等于要求的底线（C 探针只需 2.25，多出来的来自 Rust std）。即"2.28 及以上可跑、以下不行"；若要下探到更老的发行版，需要 Rust 侧更低的 target baseline。

打包脚本在仓库 `packaging/linux/`：`package.sh` 构建并在 `dist/app` 摆出自包含目录树，同时产出 `dist/evobuddy-<版本>-linux-<arch>.tar.gz` + SHA-256（内含 `run`、`install.sh`、desktop entry、占位图标），并用 `appimagetool` 把同一棵树封成**单文件** `dist/evobuddy-<版本>-linux-<arch>.AppImage` + SHA-256——一个 ELF，内含 squashfs 载荷，`chmod +x` 后直接运行、无需解包（目录树下载下来"不是二进制文件而是一个目录"，这正是用户反馈的形态问题）。`verify.sh` 在裸 Debian 10 上把**两个产物**都跑一遍，并要求两者渲染出的 PNG 逐字节相同，于是"AppImage 就是同一个程序"是被检查的，不是被假设的。

`PLATFORM` 默认取**本机架构**，这不是图省事：AppImage runtime 会在 ELF 头 `EI_ABIVERSION` 字节里写 `AI`（这就是该格式的标记），真实内核忽略它，但 qemu 与 Rosetta 会以 `Exec format error` 拒绝——**任何 AppImage 都无法在模拟层下构建或运行**。所以原生架构是唯一能验证它的地方，CI 也在真实 runner 上按架构各建一份；`test.sh` 会先问环境"这里能不能运行 AppImage"，能则跑并要求渲染一致，不能则明说是模拟层限制而不是产物缺陷。**CI 现在直接调用这两个脚本**（缺口 12），所以发布路径和开发者路径不可能各写一份。

**macOS（v0.3.0 起，双架构独立 .app）**：`packaging/macos/bundle.sh` 对着 Homebrew 的 GTK 3 栈构建，然后把每一条 load command 改写为 `@rpath`（连同给每个 Mach-O 补 rpath），产出自包含的 `EvoBuddy.app`（连 loaders、immodules、schema、图标全部内置；**不带字体**——macOS 上 pango 走 CoreText 而非 fontconfig，实测 `FONTCONFIG_FILE` 不影响度量，系统字体栈本就人人都有，且规格书的几何实测本来就是 macOS 侧的值）。`verify.sh` 的验证分三层：静态审计每条 load command 与 LC_RPATH（只允许 @ 包内路径与系统路径）；检查模板类文本无构建期路径泄漏；最后**把 Homebrew prefix `chmod 000` 藏起来真跑一遍**几何自检（macOS 26 已移除 `DYLD_PRINT_*`，"藏起来"比"观察"更强，退出时恢复）。本机 arm64 实测通过，且 hero=137×42 与规格书 macOS 实测一致；x86_64 交给 CI 的 `macos-15-intel`（Intel 镜像的最后一班，macos-13 已于 2025-12 关停）。踩坑全集在 `packaging/macos/README.md`：`otool -L` 首行是自身 install name、拷贝名必须跟随 load command 名而非 realpath 名、brew 库自带 `@rpath`+自身 `LC_RPATH`、librsvg loader 是 re-export 壳（`-change` 改不动 `LC_REEXPORT_DYLIB`，改用 `.dylib` 孪生）、Apple Silicon 上 dlopen 未签名代码直接 SIGKILL 且 `codesign --deep` 不签 Resources 下的 Mach-O（须逐个显式签名）、以及链接期要 `-headerpad_max_install_names`。CI 侧还发现：runner 屏幕小于 1440×868，GTK 会把默认窗口钳到工作区（1222×778）——这是 OS 的正确行为而非缺陷，macOS 侧的几何断言因此只盯与屏幕无关的不变量（sash x=280、composer 800×144、chips 800×32），窗口尺寸只作信息披露。

**插件化骨架（M1，v0.4.0）**：`prototype/` 的测量工作区裁决了插件运行时（rutis + `wasm32-wasip1`，见 `prototype/probes/README.md`），M1 据此把仓库 workspace 化：`crates/eb`（GTK 宿主：渲染器、装配、EB_* 审计）、`crates/eb-api`（**插件唯一可依赖的 surface**：数据词汇、贡献注册表、`EbPlugin`/`AnswerSource`）、`crates/plugins/`（内置插件：`nav`/`notices`/`composer`/`demo-agent`）、`vendor/rutis`（vendored，收窄 tokio 特性）。两个架构决策：**数据插件 + 宿主渲染**（插件产出纯数据视图，永不触碰 widget——线程契约一句话，像素纪律单点收敛）；**M1 不设内核线程**（内置插件无异步工作，单线程 current_thread tokio 在挂载期 `block_on` 即可，点击审计逐字节稳定；内核线程桥推迟到第一个异步服务/插件出现时再立）。demo-agent 以 `AnswerSource` **服务**（`ctx.provide_as` trait 对象）而非贡献项存在——它是将来被真实 agent 替换的接缝。**等值证据**：改造前后 welcome/demo 两张渲染 PNG SHA 逐字节一致、点击审计 26 步（31 行 `click:` 输出）的步骤与文案全等。顺带记录审计的三处**环境敏感**字段：toplevels 计数（macOS 会话有时预建隐藏 `GtkTooltipWindow`）、popover 绝对坐标（含窗口服务选择的窗口原点）、以及代码区滚动宽度（本机连续运行观察到 579/656 与 579/646 两种读数）——同二进制两次运行也会不同，比较审计应看步骤与文案而非整行 diff（旧代码同样波动，已用 stash 实验证明）。dsh 兼容路线（Node+Cordis 宿主、服务由 Rust 兑现、"同步返回活对象的注册表必须与插件同进程"的裁决）原样保留在 `prototype/`，不进入 UI 构建。

**M2.0a/b（2026-10-18，已实现）**：`AnswerSource` 从同步整轮返回改为 `start(prompt, AgentSink) -> Box<dyn AnswerHandle>`；`AgentEvent`（started/status/delta/block/artifact/usage/finished/failed）经 `AgentSink`/`AgentStream` 回传。`start` 跑在 `AgentRuntime` 的专用线程（current-thread tokio）上，`Runner` 在 GTK 主线程 drain 并更新 `App`；队列是容量 64 的有界队列，send 按钮流式期间变 stop，`start` panic 转成 `Failed`。demo agent 同步发射，保持原有像素等值；`EB_FAKE_AGENT=1` 的 fake agent 验证慢流、取消与 stop。

**v0.4.0（run 39089801107，六个任务全绿）**：插件化骨架（M1）首次发布——workspace 化后的 `crates/eb`（宿主）/ `crates/eb-api`（surface）/ `crates/plugins/*`（内置插件）/ `vendor/rutis` 全链路在三平台 CI 上构建并验证通过：Linux 裸 Debian 10.13 上 tar 树与 AppImage 照旧断言渲染逐字节一致；macOS 双架构照旧审计 load command 并藏起 Homebrew prefix 真跑。等值迁移的证据（截图 SHA 逐字节一致、26 步点击审计全等）与两个架构决策见 §2"插件化骨架（M1）"段。发布资产 14 个，与 v0.3.0 同构。

## 3. 设计 Token 体系

### 3.1 分层

```
palette（770）   调色板原色：brand / gray / blue / green / orange / red / purple / cyan / black / white
  └ color（415） 语义色：text-* / bg-* / border-* / accent-* / link-* / mask-*
      └ 组件 token（button 195、tag 144、checkbox 118、tabs 112、input 82、table 80、modal 74…）
基础尺度：spacing 187 / font 290 / radius 73 / size 94 / shadow 64 / motion 104 / icon 59
```

**主题机制**：同一批变量，暗色在 `[data-theme="dark"]` 下重映射（**932 条覆盖**），不是两套样式表。实现只需一次 token 解析 + 一套暗色覆盖。

### 3.2 关键值速查

**品牌与语义色**

| Token | 值 | 用途 |
|---|---|---|
| `--eb-palette-brand-8` | `#00c29a` | 品牌主色（青绿） |
| `--eb-palette-brand-9` / `-10` | `#009273` / `#00614d` | 按下 / 深色 |
| `--eb-palette-brand-blue` | `#0052d9` | 强调蓝（与 brand 并列的双主色） |
| `--eb-palette-blue-7` | `#3370ff` | 交互蓝 |
| `--eb-color-text-link-default` | `#1470b4` | 链接 |
| `--eb-color-text-primary` | `= black-100` | 正文 |
| `--eb-color-text-secondary` | `= black-70`（实色 `#5d5d5d`） | 次要文字 |
| `--eb-color-text-placeholder` | `#8a9099` | 占位 |
| `--eb-color-bg-primary-default` | `= gray-3 #f2f2f2` | 一级背景 |
| `--eb-color-bg-secondary-default` | `= white-100` | 卡片/面板底 |
| `--eb-bg-card` | `#fff` / 暗 `#242424` | 卡片 |
| `--eb-bg-card-strong` | `#f5f5f5` / 暗 `#252525` | 强调卡片 |
| `--eb-bg-active-hover` | `#f7f8fa` / 暗 `#404045` | 悬停态 |
| `--eb-color-border-primary` | `= gray-5 #e6e6e6` | 主描边 |
| `--eb-color-border-solid` | `#dedede` | 实色描边 |
| 状态色 | red-8 `#f64041`、orange-8 `#ff7800`、green-8 `#0cbf5b`、purple-7 `#7b61ff` | |

**尺度**

| Token | 值 |
|---|---|
| `--eb-spacing-*` | `0, 2, 4, 5, 8, 12, 16, 20, 24, 32, 40, 48, 64` |
| `--eb-radius-*` | `xs 2, sm 4, md 6, lg 8, xl 12, 2xl 16, 3xl 20, 4xl 24, full 9999` |
| `--eb-size-*` | `3=12, 4=16, 5=20, 6=24, 7=28, 8=32, 9=36, 10=40, 11=44, 12=48, 13=52, 14=56, 17=68` |
| `--eb-font-size-*` | `1=8, 2=10, 3=12, 4=13, 5=14, 6=16, 7=18, 8=20, 9=24, 10=28, 11=32` |
| `--eb-font-weight-*` | `400 / 500 / 600 / 700` |
| `--eb-icon-size-*` | `sm=12, ms=14, md=16, lg=20, xl=24` |
| 类型级别 | `display 32/40/700`、`h1 24/32/600`、`h2 20/28`、正文 `14/22` |

**字族**

```css
/* UI */
-apple-system, BlinkMacSystemFont, "Segoe UI (Custom)", Roboto,
"Helvetica Neue", "Open Sans (Custom)", system-ui, sans-serif
/* 等宽 */
ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace
```

> 注意 `"Segoe UI (Custom)"`、`"Open Sans (Custom)"`、`"Consolas (Custom)"` 是**改名后的自带字体**（随包的 woff2）。复刻时必须换成自有字体（见 §9）。

**阴影**

| Token | 值（浅色） |
|---|---|
| `--eb-shadow-card` | `0 1px 2px #0000000a` |
| `--eb-shadow-card-soft` | `0 16px 32px -8px #00000008` |
| `--eb-shadow-card-hover` | `0 8px 20px #0000001f` |
| `--eb-shadow-dialog-md` | `0 8px 24px #00000014` |
| `--eb-shadow-dropdown` | `0 4px 6px -1px #0000004d, 0 2px 4px -1px #0003, 0 12px 28px #00000080` |
| `--eb-shadow-elevated` | `0 8px 24px #0000004d` |
| `--eb-shadow-heavy` | `0 20px 60px #00000026` |
| `--eb-shadow-focus-ring` | `0 0 0 2px #1a79ff1a` |
| `--eb-shadow-drawer` | `-4px -2px 16px #0000000f, 0 0 0 1px #00000008` |

### 3.3 预计算：已完成

`tokens.css` 有 **85 处 `color-mix()`**，已全部预计算成字面量（按 CSS 语义：premultiplied-alpha 的 sRGB 插值），`var()` 链也已全部展开——**未解析项剩 1 条**（`--eb-alert-label-color-bg`，疑似自引用）。

```css
/* 原式 */
--eb-shadow-dialog: 0 8px 48px -4px color-mix(in srgb, var(--eb-palette-black-100) 12%, transparent), …
/* 现在（tokens.resolved.json / gtk/tokens-*.css） */
--eb-shadow-dialog: 0px 8px 48px -4px rgba(0, 0, 0, 0.12), …
```

受影响的主要是 `shadow-*`、`input-*`、`bg-*-hover`、`bg-active` 一族。工具：`build_gtk_tokens.js`。

> 踩过的坑：原导出脚本要求声明以 `;` 结尾，而压缩后的 CSS **每个块的最后一条声明没有分号**——103 条 token（含 `--eb-shadow-dialog`）被静默丢弃。已修复。

### 3.4 文件

| 文件 | 内容 |
|---|---|
| `tokens.css` | `:root`（**1980** 条）+ `[data-theme="dark"]`（**932** 条），已重命名空间 `--wb-*` → `--eb-*` |
| `extract_tokens.js` | 导出脚本；输入 WorkBuddy 的 `tokens-*.css`，输出上表文件 |
| `build_gtk_tokens.js` | token 编译器：展开 `var()`、计算 `color-mix()`、归一化颜色（见 §3.5） |
| `gtk/tokens-light.css` | **1043 条** `@define-color`，浅色基准 |
| `gtk/tokens-dark.css` | 暗色**差异集**（794 条差异中的纯色项）；运行时替换 provider 即切主题 |
| `gtk/tokens.resolved.json` | 全量已解析 token（浅 + 暗），供程序侧直接读取 |

> 另有 2 个非 token 规则块（`body[data-electron-desktop=true]`、`[data-platform=mac]`、`[class*=-modal-overlay]`）不含变量，未收录。

### 3.5 GTK 3 的硬约束（与 Web CSS 的差异）

GTK 3 的 CSS 是**另一个子集**，不是 Web CSS 的降级版。实测踩到的几条约束都**不报错、只静默失效**：

| 约束 | 说明 | 处置 |
|---|---|---|
| **无 CSS 变量** | 不支持 `var()` / 自定义属性，只有 `@define-color` | 编译期展开 |
| **无 `color-mix()`** | 同上 | 编译期按 sRGB premultiplied-alpha 插值算成 `rgba()` |
| **`@define-color` 不接受 8 位十六进制** | `#rrggbbaa`（如 `#1a79ff24`）被拒 | 一律归一化成 `#rrggbb` / `rgba(r, g, b, a)` |
| **一行非法 → 整张样式表作废** | 不像浏览器只丢一条规则；GTK 会丢弃**全部** CSS 并静默回落到默认主题 | 加载错误必须显式处理 + 冒烟测试 |
| **`min-height` / `padding` 是 content-box** | Web 的 `min-height: 32px; padding: 5px 12px`（border-box）总高 32；GTK 里是 32 + 12 = **42**。差了整整 10px | 定高按钮写成 `padding: 0 12px; min-height: 32px`——GtkButton 会把子控件垂直居中，视觉等价 |
| **`box-shadow` 裁剪到控件内** | GTK 3 支持 `box-shadow`，但外投影画不出控件边界（产品里输入卡的投影会铺到下方选择器行背后） | 目前接受降级；要还原需 Cairo 自绘或外层留白 |
| **`border: none` 未必清掉主题边框** | 主题的 `border-width: 1px` 可能残留（实测按钮凭空高 2px） | 显式写 `border-width: 0` |
| **无 flexbox / grid / `gap`** | 布局全在 Rust 里用 `GtkBox` 的 spacing + margin + 显式 spacer 表达 | 见 `src/bin/everybuddy.rs` |
| **无伪元素** | 产品把 AI 声明做成 `main.wb-home-route::after` | 改成一个真实 `GtkLabel`，`pack_end` 贴底 |
| **`Widget` 不能 downcast 成 `Container`** | gtk-rs 不把 `Widget` 建模为 `Container` 的父类，无法写通用的控件树遍历 | 冒烟测试改为构建时登记「探针」控件清单（`Probes`） |

> 因为"静默回落"看起来仍然像个正常界面，这块必须有回归测试。现在有三层：
> - `EB_SELFTEST=1`：解析若干 `@define-color`、打印窗口/侧栏实际尺寸与**每个探针控件的绝对几何**（`translate_coordinates`），随后退出。
> - `EB_SHOT=<path>`：额外用 Cairo 的 `gtk_widget_draw` 把窗口渲染成 PNG——这是唯一能抓到"颜色解析对了但贴错了控件""文本被分配尺寸裁掉"这类问题的检查。
> - 两条都必须**在 idle 回调里跑**：控件的 `::size-allocate` 先于其子控件分配，在信号回调里读布局会拿到半成品（子控件 allocation 是旧的，`gtk_widget_draw` 会直接断言失败）。


## 4. 主工作区布局规格

### 4.1 骨架

**欢迎态实测骨架见 §4.3**。下面这段是初稿依据的 `renderer/index.html` 骨架屏注释，保留仅作历史参考——其中的**语言（中文）、栏数（三栏）、标题对齐（左对齐）均与实机不符**：

```
┌──────────────────────────────────────────────────────────────┐
│ titlebar                                                     │
├────────────────┬─────────────────────────────────────────────┤
│ sidebar        │ content                                     │
│                │                                             │
│  [+ 新建任务]  │   [品牌标题]                                │
│                │                                             │
│  助理          │   [场景标签] 日常办公 · 代码开发 · 设计创意  │
│  项目          │                                             │
│  专家          │   ┌───────────────────────────────────────┐ │
│  自动化        │   │ input（提示词输入区）                  │ │
│  更多          │   └───────────────────────────────────────┘ │
│  ─ 任务分组 ─  │                                             │
│   · 会话 1     │                                             │
│   · 会话 2..4  │                                             │
│  ─ 空间分组 ─  │                                             │
│   · 空间 1..2  │                                             │
├────────────────┴─────────────────────────────────────────────┤
│ statusbar                                                    │
└──────────────────────────────────────────────────────────────┘
```

**实测更正**：欢迎态是**两栏**（侧栏 | 内容），**没有结果面板**。三栏只在会话态出现——第三栏是结果面板（产品内称 "Results Panel"，对应 `artifact-slot-panel` / `detail-panel-wrapper` / `result-content`），随任务产出内容后出现。

### 4.2 视图状态

`main-content` 由修饰类切换，这是主工作区的状态机：

| 状态类 | 含义 |
|---|---|
| `main-content--welcome` | 首启/新任务：品牌标题 + 场景标签 + 输入框 |
| `main-content--chat` | 会话进行中 |
| `main-content--projects` | 项目管理 |
| `main-content--automation` | 自动化 |
| `main-content--initializing` | 初始化中（对应 `main-content__loading`） |

配套：`chat-container--welcome`、`chat-container--share-mode`（分享模式）、`conversation-window-loading`（独立会话窗口）。

> **实测更正（欢迎态）**：欢迎态实际渲染的是 `wb-home-route` / `wb-home-page` / `wb-home-composer` / `wb-scene-tabs` 这一套类名（对应 `main-content-core.css` 里的新命名），上表的 `main-content--*` 修饰类来自较早的 CSS module，已不是欢迎态的实际切换方式。状态机语义仍然成立，但**不要按上表类名去找 DOM**。
>
> **实测更正（会话态）**：会话态（21 帧录制）里同样没有 `main-content--*`，实际是 `teams-container` → `_gridView_*` → 三个绝对定位列（`conversation-list` 280 / `conversation-shell` 580 / `detail-panel-container` 580）+ 两条全透明的 `_sash_*` 覆盖层。会话进行中的消息、工具行、输入区全部是 `cr-*` / `conversation-*` 前缀（§4.4、§5.2）。结果面板的开关不是顶栏按钮，而是侧栏任务行的选择状态。

### 4.3 欢迎态实测几何（CDP，1440×868 客户区）

抓取条件：`--remote-debugging-port` 连接 renderer，`DOMSnapshot.captureSnapshot`（`includeDOMRects`）+ `CSS.getComputedStyleForNode`，locale `en-US`，DPR=2。

> **两个必须记住的坑**（`docs/ui-spec/query_layout.js` 就是为它们写的）：
> 1. `DOMSnapshot` 的 `bounds` 是**设备像素**（本机 2×），要 ÷2 才是 CSS px。
> 2. 计算样式里的长度**已经是 CSS px**，不能再 ÷2；否则每个字号都会莫名减半。

窗口与两栏：

| 元素 | 类名 | 尺寸 (CSS px) | 位置 | 备注 |
|---|---|---|---|---|
| 应用容器 | `teams-container` | 1440×868 | 0,0 | bg `#f2f2f2` |
| 侧栏 | `conversation-list` | 280×868 | 0,0 | bg `#f7f7f7`；右侧边界上骑一条 4px **全透明**拖拽热区（`_sash_*`，不可见） |
| 内容容器 | `teams-content-wrapper` | 1160×868 | 280,0 | |
| 内容顶栏 | `workbuddy-topbar` | 1160×64 | 280,0 | gap 8，padding 0 16；欢迎态为空拖拽区 |
| 路由 | `wb-home-route` | 1160×868 | 280,0 | bg `#ffffff` |
| 主体 | `wb-home-route__body` | 1160×804 | 280,64 | |
| 页面 | `wb-home-page` | 848×804 | 436,64 | padding 0 24 |
| **居中列** | `wb-home-page__main-content` | 800×804 | 460,64 | 内容区 1160 内水平居中 |

侧栏自上而下：

| 元素 | 类名 | 尺寸 | 位置 | 备注 |
|---|---|---|---|---|
| 顶栏 | `conversation-list-topbar` | 279×64 | 0,0 | padding `0 12px 0 86px`——86px 是给 macOS 交通灯让位；含 3 个 32×32 幽灵按钮（16×16 图标，gap 4） |
| 头部 | `conversation-list-header` | 279×246 | 0,64 | gap 2px，padding `0 12px 20px 12px` |
| 品牌行 | `conversation-list-logo-row` | 255×48 | 12,64 | 含 12px 下内边距 |
| 品牌块 | `conversation-list-brand-block` | 255×36 | 12,64 | = 品牌行 18 + 版本行 18 |
| 品牌文字 | `logo-workbuddy-title` | 98.11×22 | 24,64 | 14px/700，`rgba(0,0,0,.9)` |
| 版本徽标 | `conversation-list-version-badge` | 30.88×18 | 24,82 | 10px/400，`rgba(0,0,0,.5)` |
| 导航列表 | `conversation-list-tabs` | 255×168 | 12,122 | 5 × 32 行 + gap 4 |
| 导航行 | `conversation-list-tab-button` | 255×32 | 12,122 起 | r 8px，padding `5px 12px`，gap 8；**选中 bg `#ebebeb` / 600**，未选中透明 / 400 |
| BETA 徽标 | `conversation-list-tab-beta` | 38.19×18 | — | r 100px，padding `0 5px`，10px/500，`rgba(0,0,0,.451)` |
| 滚动区 | `conversation-list-content` | 279×501.5 | 0,310 | padding `0 12px` |
| 空态 | `conversation-list-empty` | 255×109 | 12,310 | **顶部对齐**（不是垂直居中），padding `24px 12px` |
| 空态标题 | `conversation-list-empty-title` | 231×20 | 24,334 | 14px/500，`#000` |
| 空态说明 | `conversation-list-empty-description` | 231×33 | 24,362 | 12px/400，`rgba(0,0,0,.7)`，居中换行 |
| 底栏 | `conversation-list-footer` | 279×56.5 | 0,811.5 | padding `8px 12px 8px 8px` |
| 用户行 | `user-menu-trigger` | 223×40 | — | r 8px，padding `6px 8px`；头像 28×28 圆形 bg `#ebebeb`，12px/500 |
| 小程序按钮 | `user-menu-trigger-miniprogram` | 32×32 | — | r 6px，图标 16×16 |

居中列自上而下（**y 坐标已逐项对齐**）：

| 元素 | 类名 | 尺寸 | 位置 | 备注 |
|---|---|---|---|---|
| 标题 | `wb-home-header__title` | 421×42 | 649.6,127 | h1 30px/600，**居中**（不是左对齐） |
| 分段控件轨 | `wb-scene-tabs` | 163.74×36 | 778.1,185 | bg `#f2f2f2`，r 100px，padding 2，inset `0 0 0 0.5px #e6e6e6` |
| 分段滑块 | `wb-segmented__thumb` | 79×32 | 861.1,187 | bg `#fff`，r 9999px，shadow `0 1px 4px rgba(0,0,0,.05)` |
| 分段项 | `wb-segmented__item` | 79×32 | 860.9,187 | r 9999px，padding `5px 12px`；激活 14px/600 `#000`，未激活 14px/500 `rgba(0,0,0,.7)` |
| 场景行 | `wb-home-composer__chips` | 800×32 | 460,285 | gap 8，padding-right 120 |
| 场景 chip | `quick-actions__item` | 6 个，均高 32 | 460…1022 | 宽 107.5 / 82.5 / 115.2 / 68.6 / 57.7 / 90.4（内容撑开）；bg `#f7f7f7`，r 100px，padding `0 12px`，gap 4，14px/400，`rgba(0,0,0,.9)` |
| 输入卡 | `cr-input-container` | 800×144 | 460,333 | bg `#fff`，r 24px，padding-top 12，shadow `0 16px 32px -8px rgba(0,0,0,.03)` |
| 编辑器 | `cr-input-editor-host` / `_editable` | 798×74 / 766×74 | 461,346 | padding `0 16px`；15px/400 `rgba(0,0,0,.9)` |
| 工具条 | `cr-input-toolbar` | 798×56 | 461,420 | padding 12，gap 8 |
| 加号 | `cr-add-menu` | 32×32 | 473,432 | |
| 模型选择 | `cr-model-selector__trigger` | ~74.8×32 | 1132.3,432 | r 999px，padding `0 8px`，13px/400 |
| 发送 | `cr-input-toolbar__send` | 32×32 | 1215,432 | 圆形；无输入时禁用（`rgba(0,0,0,.2)`） |
| 选择器行 | `cr-input-footer` | 800×36 | 460,481 | padding `0 12px 4px 12px`，gap 4；**背景透明**——看到的浅灰是上方卡片的投影 |
| 工作区 | `cr-workspace-picker` | 165.14×32 | 472,481 | r 999px，padding `0 8px`，13.33px/400 `rgba(0,0,0,.5)` |
| 权限 | `cr-permission-setting` | 178.34×32 | 641.1,481 | 同上 |
| 输入区留白 | `wb-home-composer__below-input` | 800×236 | 460,569 | 欢迎态为空 |
| 吉祥物 | `wb-home-route__growth-buddy` | 120×120 | 1120,245 | 内含 140×140 溢出图片 |
| AI 声明 | `main.wb-home-route > ::after` | 1160×16.5 | 280,839.5 | 12px 居中，贴底（距底 12px）——是伪元素，不是真实节点 |

关键色（实测，非 token 名）：

```
应用底 / 分段轨   #f2f2f2      文本主     rgba(0,0,0,.9)
侧栏底 / chip     #f7f7f7      文本次     rgba(0,0,0,.7)
悬停/选中行       #ebebeb      文本三     rgba(0,0,0,.5)
分段轨描边        #e6e6e6      BETA 徽标  rgba(0,0,0,.451)
卡片底            #ffffff
```

> 注意：`rgba(0,0,0,.9)` 这类「alpha 黑」文本色**不在 `tokens.css` 里**（token 集只有实心灰，如 `--eb-color-text-secondary` = `rgba(0,0,0,.702)`）。它们是组件层自己定义的字面量，因此 `ui/gtk.css` 用 shell-local 的 `@define-color eb_text_*` 承载，见该文件开头注释。

### 4.4 会话态（三栏）实测几何（CDP，1440×868 客户区）

采集方式同 §4.3，但这次是**连续录制**：`docs/ui-spec/watch_dom.js` 把 DOM 快照按帧差异去重后落盘（本轮共 21 帧，见 §5.2）。

三栏：**侧栏 280 ｜ 会话 580 ｜ 结果面板 580**。栏位是绝对定位的固定列，分隔条不是 gap——它是一条 **4px 宽、完全透明**的覆盖层（`_sash_*` / `_vertical_*`，`position:absolute; z-index:10; background:rgba(0,0,0,0)`），**骑在栏边界上**（278–282、858–862），所以栏宽不被扣减、界面上也看不到分隔线。`conversation-shell` 实测 280,64 580×804；结果面板容器 860,0 580×868，其内层 `sidebar-next` 因左边框 1px 落在 861,0 579×868。

会话栏自上而下：

| 元素 | 类名 | 尺寸 | 位置 | 备注 |
|---|---|---|---|---|
| 会话顶栏 | `workbuddy-topbar`（`--mac`） | 580×64 | 280,0 | 高度 64，`position: sticky`，`justify-content: space-between`，gap 8 |
| 顶栏选项行 | `workbuddy-topbar-options` | 540×32 | **304,16** | gap 12；**内容左起点实测距栏左 24px**（右端 844，距栏右 16px） |
| 　标题组 | `workbuddy-topbar-left` / `-title-area` / `-title` | 158.39×32 / 158.39×24 | 304,16 / 304,20 | 任务名，14px/600（文本宽随标题变化，此帧 158.39） |
| 　操作组 | `workbuddy-topbar-actions` | **68×32** | 776,16 | **只有 2 个** 32×32 幽灵按钮（搜索在 776、分享在 812），gap 4，图标 16×16 |
| 会话壳 | `conversation-shell` / `__main` | 580×804 | 280,64 | bg `#ffffff` |
| 消息列表 | `cr-message-list` / `cr-message-list-viewport` | 580×636 | 280,64 | padding `0 40px` ⇒ 内容宽 500，起点 x=320 |
| 　消息文档 | `cr-document` / `__virtual-flow` | 500×636 | 320,64 | 虚拟列表；无顶部内边距，首帧从 y=64 起 |
| 　底部遮罩 | `cr-message-list__bottom-mask` | 580×72 | 280,628 | 列表底部渐隐遮罩 |
| 用户帧 | `cr-frame` `cr-frame--right` | 500×87.97 | 320,64 | padding `16px 0`；气泡 + 28px 操作行 |
| 　用户气泡 | `cr-frame__content` / `cr-self-bubble` | 182.24×43.97 | 637.8,80 | **宽度随文本**，右对齐到 820；bg `#f2f2f2`，r `16 16 0 16`，padding `8px 12px`；15px/400 `rgba(0,0,0,.9)`，行高 21 |
| 　用户操作行 | `cr-toolbar` | 98.52×28 | 721.5,124 | gap 8，右对齐到 820；`cr-toolbar__time` 34.52×20（14px/400 `rgba(0,0,0,.5)`）+ **两个** 24×24 r6 图标按钮（764 / 796，`rgba(0,0,0,.7)`） |
| 助手帧 | `cr-frame` `cr-frame--left` | 500×524 | 320,152 | 紧接上一帧（帧间无间隔） |
| 　助手容器 | `cr-agent` | 500×492 | 320,168 | |
| 　助手头像行 | `cr-agent__header` | 500×24.5 | 320,168 | gap 10；`__logo` 24×24 r50%、`__name` 94.52×24.5 14px/600 |
| 　助手状态行 | `cr-agent__processing-status`（`cr-agent--processing` / `--with-status`） | —— | 名字下方 8 | 15px/400 `rgba(0,0,0,.5)` |
| 　助手正文 | `cr-agent__body-row` / `__body` / `__content` | 500×455.5 / 417.25 | 320,204.5 | gap 8 |
| 折叠工具行 | `cr-tool-head`（`--collapsible`） | 500×28.5 | —— | padding `2px 0`，内部 24.5 高；`__icon` 14×14、`__arrow` 12×12、`__primary`（可点、链接色 `#1470b4`）、`__secondary`（`+42` `#088f50` / `−0` `#b42c3f`），14px/400 `rgba(0,0,0,.5)` |
| 工具卡（展开态） | `cr-collapse` / `__content` | —— | —— | 浅绿 `#e7f9ef`，r16（本轮只做折叠行，见 §10） |
| 输入区 | `conversation-input-area` | 580×168 | 280,700 | |
| 　输入栈 | `conversation-input-area__input-stack` | 532×134 | 304,700 | |
| 　输入卡 | `cr-input-box` / `__main` | 532×134 | 304,700 | bg `#fff`，r24，gap 4，padding-top 12；shadow 受 §10-3 裁剪限制 |
| 　编辑器 | `cr-input-editor-host` | 530×64 | 305,713 | 15px/400 |
| 　工具条 | `cr-input-toolbar` | 530×56 | 305,777 | padding 12，gap 8 |
| 　　左组 | `cr-input-toolbar__left` | 347.25×32 | 317,789 | 弹性伸展；`cr-add-menu` 32×32 @317 + 8 + `cr-permission-setting` 178.34×32 r999 @357（图标 17×17、label 13px/400、chevron 14×14） |
| 　　右组 | `cr-input-toolbar__right` | 150.75×32 | 672.2,789 | `cr-context-usage` 上下文用量环 32×32 @672.2 + 4 + `cr-model-selector` 74.75×32 r999 @708.2 + 8 + `cr-input-toolbar__send` 32×32 @791 |
| AI 声明 | `conversation-input-area__disclaimer` | 532×34 | 304,834 | padding 8px，12px/400 `rgba(0,0,0,.3)`；内层 `wb-homepage-disclaimer` 516×18 @312,842，**11px/400 `rgba(0,0,0,.4)`** |

> 会话态与欢迎态的输入区**不是同一个组件**：欢迎态是 800×144 卡片 + 独立的 800×36 选择器行；会话态是 532×134 卡片，权限 pill 在工具条**内部**，下方另有 34px 声明行。

> 本节全部数值都是 **1440 宽**下的实测。窗口拉窄时的三段保底（面板保住自身内容最小宽、会话降到 240；有效最小宽 1082）是**自创规则**，见 §10-31。

**块节奏**：每个单行消息块实测约 53 高 ⇒ 等价 `margin: 14px 0`（实现里是 `vbox(28)`）。

侧栏（有任务时，替换掉欢迎态的空态）：

| 元素 | 尺寸 | 位置 | 备注 |
|---|---|---|---|
| 区块标题 `conversation-section-label` | 255×30 | 12,310（在有任务时） | bg `#f7f7f7`（与侧栏同色，视觉不可见），r8，gap 4；文本 `-text` 12px/600 `rgba(0,0,0,.5)`，实测起点 x=24 |
| 任务项 `conversation-item` | 255×31 | 12,340 起 | r8，padding `5px 12px`，间隔 2；透明 / 悬停与选中 `#ebebeb`；标题 选中 14px/600 否则 14px/400 `rgba(0,0,0,.9)`；元信息 11px/400 `rgba(0,0,0,.5)`；状态字形 20×20（运行中 = `#00c29a` 转圈） |
| 分组区块 `conversation-section-label-groups` | 255×30 | 12,391（紧接任务项之后） | 工作区/分组标题；其下占位项 30.5（`padding: 6px 12px 6px 36px`） |

结果面板（`detail-panel-container` 860,0 580,868；内层 `sidebar-next` 861,0 579,868，左 1px 边框）：

| 元素 | 尺寸 | 位置 | 备注 |
|---|---|---|---|
| 头部 `sidebar-next-main-header` | 579×64 | 861,0 | gap 4，bg `#fff`；左组 `__left` 475×32 @877 |
| 标签页 `artifact-tab`（`--active`） | **240×32** | 877,15.5 | bg `#f2f2f2`，r8，padding `0 8px`，gap 4；`__icon` 16×16 @886、`__name` 184×22 @906（14px/600）、`__close` 14×14 @1094 |
| 标签列表 `artifact-tabs__list` | 240×32 | 877,15.5 | 单标签时与标签同宽 |
| 加号 `artifact-tabs__add` | 32×32 | 1121,15.5 | r8（与标签间隔 4） |
| 头部操作 `sidebar-next-main-header__actions` | 68×32 | 1356,15.5 | 2 个 32×32 |
| 面板正文 `detail-main__body` | 579×804 | 861,64 | bg `#fff` |
| 编辑器工具条带 | 579×**72** | 861,64 | 文件名 + 复制/操作（产品这里是 Monaco 自绘工具栏，属**近似**重建，见 §10） |
| 代码区 | 579×708 | 861,136 | 左侧 68px 行号槽（x=861…929），代码列 x=929…1440；14px/400 `#3b3b3b`，行高 20，白底 |

**抓取同时纠正了三处推测**（详见 §5.2）：

1. 点击 chip **不会**填充输入框、也不会切换路由。唯一的 DOM 差异是工具条**右组**多/少一个 32×32 的 **`cr-context-usage` 上下文用量环**（+4px gap = 36px），左组因为是弹性容器而反向变化同样幅度（实测 1303 ⇄ 1231 设备像素 ⇒ 651.5 ⇄ 615.5 CSS px）。
2. 会话顶栏**只有两个**操作按钮（曾按 3 个实现，已删除）。
3. 分段控件的选中类是 `wb-segmented__item--active`，`wb-scene-tabs` 实测 163.74×36 @778.1,185（内层 `wb-segmented` 159.74×32，滑块 `wb-segmented__thumb--measured` 79×32）——与 §4.3 的推测吻合。
4. **M2.1 补采（2026-10-18，5.8.4）修正会话宽度**：没有 artifact 面板时 frame 为 **752**；打开面板后 frame 收窄为 **500**，右侧面板仍 **579**。§4.4 的 500+40 gutter 只适用于面板打开态；详细块尺寸见 §5.3。

## 5. 组件清单（主工作区）

### 5.1 欢迎态（**已实测，优先实现**）

来自 CDP 抓取的 DOM，可直接按名定位：

| 组 | 类名 |
|---|---|
| 两栏壳 | `teams-container`、`teams-content-wrapper`、`teams-main-content` |
| 侧栏 | `conversation-list`、`conversation-list-topbar`、`conversation-list-topbar-actions`、`conversation-list-header`、`conversation-list-logo-row`、`conversation-list-brand-block`、`conversation-list-brand-line`、`logo-workbuddy-title`、`conversation-list-version-row`、`conversation-list-version-badge`、`conversation-list-tabs`、`conversation-list-tab-button`、`conversation-list-tab-button-label`、`conversation-list-tab-beta`、`conversation-list-content`、`conversation-list-empty`、`conversation-list-empty-title`、`conversation-list-empty-description`、`conversation-list-footer` |
| 用户区 | `user-menu-trigger`、`user-menu-trigger-miniprogram` |
| 内容顶栏 | `workbuddy-topbar`（`--mac` 修饰类） |
| 首页路由 | `wb-home-route`、`wb-home-route__body`、`wb-home-page`、`wb-home-page__main-content`、`wb-home-header`、`wb-home-header__title`、`wb-home-route__growth-buddy` |
| 分段控件 | `wb-scene-tabs`、`wb-segmented`、`wb-segmented__thumb`、`wb-segmented__item`、`wb-segmented__item-label` |
| 输入区 | `wb-home-composer`、`wb-home-composer__chips`、`wb-home-composer__below-input`、`quick-actions`、`quick-actions__list`、`quick-actions__item`、`cr-input-box`、`cr-input-box__main`、`cr-input-container`、`cr-input-editor-host`、`cr-input-toolbar`、`cr-input-toolbar__left`、`cr-input-toolbar__right`、`cr-add-menu`、`cr-model-selector`、`cr-model-selector__trigger`、`cr-input-toolbar__send`、`cr-input-footer`、`cr-workspace-picker`、`cr-permission-setting` |
| 分隔条 | `_sash_<hash>_vertical`（hash 随构建变化，用子串 `sash` 匹配；实测 4px 宽、**全透明**、`z-index: 10`，是骑在栏边界上的不可见拖拽热区） |

### 5.2 会话态（**已实测**）与其它态

会话态的类名来自 `watch_dom.js` 的连续录制（21 帧，目录见 §4.4 说明；下表的类名由 `dom-snapshot.json` 的 `class` 属性汇总得到，非推测）。

| 组 | 类名 |
|---|---|
| 三栏壳 | `teams-container`、`teams-grid-scroll-content`、`_grid_*`、`_gridView_*`、`_gridViewItem_*`（CSS module 哈希，用子串匹配）、`_sash_*`（同前） |
| 会话栏 | `workbuddy-topbar`（`--mac` / `-left` / `-options` / `-actions` / `-title` / `-title-area` / `-title--workspace` / `-edit-button`）、`conversation-shell`、`conversation-shell__main`、`conversation-page-chrome`（`__shell` / `__content`） |
| 消息列表 | `cr-message-list`（`--virtual-anchored`）、`cr-message-list-viewport`、`__content`（`--revealed`）、`__bottom-mask`、`cr-document`（`__virtual-flow` / `__virtual-item` / `__virtual-item--flow` / `__virtual-spacer`）、`cr-frame`（`--left` / `--right`）、`cr-frame__content`、`cr-text-block`（`--spaced-both`）、`cr-markdown`、`cr-inline-code` |
| 用户消息 | `cr-self-message`、`cr-self-message__content`、`cr-self-bubble`、`cr-user-content`、`cr-toolbar`、`cr-toolbar__time`、`cr-icon-button`、`cr-icon-button__icon` |
| 助手消息 | `cr-agent`（`--processing` / `--with-status`）、`cr-agent__header`、`__logo`、`__name`、`__processing-status`、`__body`、`__body-row`、`__content` |
| 工具调用（折叠行） | `cr-tool-head`（`--collapsible`）、`__arrow`、`__icon`（`--error`）、`__main`、`__primary`（`--clickable` / `--link`）、`__secondary`、`__status`；写入类 `cr-tool-write__*`（`--add` / `__add` / `__del`）、命令类 `cr-tool-exec__title` / `__command` / `__output`、差异视图 `cr-tool-diff`（`--all-add` / `--first-add` / `--last-add` / `--virtual`、`__line` / `__line--add` / `__sign` / `__viewport` / `__virtual-line`）、`cr-tool-call__slot`、`cr-tool-exp` |
| 可折叠块 | `cr-collapse`（`--collapsed` / `--completed` / `--summary`）、`cr-collapse__header` / `__icon` / `__lead-icon` / `__title` / `__content` / `__content-inner`（`--completed` / `--summary`） |
| 表格块 | `cr-table-block`、`cr-table-block__inner`、`cr-table-block__scroller`、`cr-message-quote-table-action` / `-menu` |
| 状态与上下文 | `cr-status-chips`、`cr-context-usage`（`--clickable`）、`cr-context-usage__ring`、`__ring-progress`、`cr-clickable-path`、`cr-clickable-path-icon`、`cr-path-type-file` |
| 输入区 | `conversation-input-area`、`__input-stack`、`__disclaimer`、`conversation-input`、`conversation-input__box`、`cr-input-container`（`--focused`）、`cr-input-container-wrapper`、`cr-input-box`、`cr-input-box__main`、`cr-input-editor-host`、`cr-input-toolbar`、`__left`、`__right`、`__send`、`cr-send-button`（`--sending` / `--stop` / `__icon`）、`cr-add-menu`（`__trigger`）、`cr-input-footer-item`（`__icon` / `__label` / `__chevron`）、`cr-permission-setting`、`cr-model-selector`（`__trigger` / `__trigger-icon` / `__trigger-label` / `__trigger-arrow`）、`cr-theme` |
| 结果面板 | `detail-panel-container`、`sidebar-next`、`sidebar-next-body`、`sidebar-next-main-header`（`__left` / `__tabs` / `__actions`）、`detail-panel`（`--no-header`）、`detail-layout`（`--stacked` / `__top` / `__content`）、`detail-main`、`detail-main__body`、`detail-main__preview-slot`（`-body`）、`detail-main__bottom-floating-slot`、`artifact-tabs`（`__list` / `__add`）、`artifact-tab`（`--active` / `__icon` / `__name` / `__close`）、`artifact-body`、`artifact-content`、`detail-new-tab-landing`（`__content` / `__list` / `__entry` / `__entry-icon` / `__entry-label`） |
| 侧栏任务区 | `conversation-sidebar`、`conversation-list-*`（见 §5.1）、`conversation-section`（`-label` / `-label-text` / `-label-tasks` / `-label-groups` / `-chevron` / `-chevron-expanded` / `-content` / `-groups` / `-motion` / `-compact-btn` / `-add-btn`）、`conversation-item`（`--enter` / `--active`）、`conversation-item__*`、`conversation-tasks-independent-block`、`conversation-list-bottom-sentinel` |
| 会话尾部 | `conversation-finished-footer`、`conversation-streaming-footer`、`conversation-timeline`、`conversation-agent-card`（`--standalone` / `--type-icon-hidden`）、`conversation-group-empty-placeholder` |
| 分段控件 | `wb-scene-tabs`、`wb-segmented`（`--medium`）、`wb-segmented__thumb`（`--measured`）、`wb-segmented__item`（`--string-label` / `--active`）、`wb-segmented__item-icon`、`wb-segmented__item-label` |
| 通用按钮 | `wb-button`（`--medium` / `--ghost` / `--plain` / `--circle` / `--icon-only`）、`wb-button__icon`（`--left`）、`wb-icon` |

**仍未实测**（后续批次需要单独抓取，或用 CSS module 名继续推断）：

| 组 | 类名 |
|---|---|
| 多智能体协作条 | `teams-top-bar`、`team-member-bar-slot`、`team-member-bar-status-icon`（`--completed` / `--failed`）、`team-member-bar-status-spinner`、`team-member-bar-collapse-trigger`、`team-member-return-main-button` |
| 其它结果面板形态 | `result-content`、`result-section`、`artifact-slot-panel`、`detail-panel-wrapper` |
| 其它输入形态 | `message-input`、`task-starter`、`file-drop-overlay`、`selection-quote-conversation-input` |
| 旧命名（可能已废弃） | `main-content`、`chat-container`、`messages-container`、`welcome-container`、`input-container`、`message`、`message-avatar`、`message-header`、`message-role`、`message-time`、`message-content`、`message-text`、`message-with-tools`、`assistant-message-*`、`tool-calls`、`tool-call-*`、`tool-inner`、`tool-icon`、`tool-explanation-text`、`tool-error` |
| 用户菜单 | `main-content__user-menu` |
| 悬浮层 | `poi-floating-panel`、`mcp-app-launcher-panel` |

> **命名断代**：会话态实测到的是 `cr-*` / `conversation-*` / `detail-*` / `artifact-*` 前缀，§5.2 早期从 `main-content-core.css` 提取的 `main-content` / `chat-container` / `message-*` / `assistant-message-*` 在本轮 21 帧里**一个都没出现**，应视为同一产品的旧版命名或他处样式，不要按名实现。

> `team-member-*` 说明主工作区内建**多智能体协作条**（含折叠、状态点、回主按钮）；本轮 21 帧里没有出现该条，所以仍按推测处理。

**本轮抓取澄清的行为歧义**

1. **点击 chip 不填输入框、不导航。** 录制里点击 chip 后唯一的 DOM 差异是输入工具条右组的 `cr-context-usage` 上下文用量环出现/消失（32×32 + 4px gap = 36px），左组因为是弹性容器而反向变化同幅度。因此「chip 预填输入框」是实现侧的选择而非产品行为：`spec::CHIP_PROMPTS` 存整句提示语，点击即填入输入框（已在 §4.4 记录）。
2. **顶栏操作按钮**：本轮 DOM 录制只数到两个（搜索、分享），当时据此删掉了第三个（结果面板开关）；2026-10-24 的整栏截图证明默认就有**三个**（搜索 / Share / 右面板开关，见 §5.6）——以截图为准，DOM 录制窗口太短是误判来源。任务行重开面板的行为保留（`open_task` 同时把 `detail_visible` 置为 `artifact.is_some()`）。
3. **分段控件**：`wb-scene-tabs` 163.74×36 @778.1,185，`wb-segmented` 159.74×32，滑块 `wb-segmented__thumb--measured` 79×32，选中类 `wb-segmented__item--active`——全部与 §4.3 的实现尺寸一致。

### 5.3 内容块实测（v5.8.4，2026-10-18）

采集条件：WorkBuddy AI 5.8.4 / Electron 37.10.3 / Chromium 138，macOS 26.2，
客户区 1440×868，DPR 2，locale en-US；只读 CDP，按状态命名抓取
`screenshot.png` + `dom-snapshot.json` + `layout.json`。数值为 CSS px。

**关键布局修正**：没有 artifact 面板时，会话 frame 是 **752** 宽（不是 580）；
打开 artifact 面板后，会话 frame 收窄到 **500**，右侧 `sidebar-next` 仍为
**579**。§4.4 的 500+40 gutter 只描述“面板打开”的情况，M2.2 要按这个宽度切换。

| 块 | 外层/关键尺寸 | 关键样式 |
|---|---|---|
| `cr-collapse` | 宽 752；header 26.25 高；collapsed 高 26.25；展开后 content 从 header 下方 26.25 开始 | header gap 8；title 15px/400 `rgba(0,0,0,.5)`；chevron 12×12；`cr-tool-exp` 752×318.5，padding `2px 0` |
| `cr-tool-diff` | `cr-tool-write__body` 752×278；viewport 750×252（12 行） | 圆角 16；gap 12；line 750×21，padding `0 16px`，gap 6，12px 等宽；sign 14，code 698；add `#e7f9ef`，remove `#fef0f0`，context 透明 |
| `cr-tool-exec` | `cr-tool-exp__content` 752×300；title/command 720 宽；200 行输出 720×3600 | 圆角 16；bg `#f2f2f2`；padding `12px 16px`；gap 4；12px/行高 19.2（输出 18px/行）；输出区纵向滚动 300 |
| `cr-table-block` | 752×476.75；内层 750×474.75；header 42.75；行 43.25；10 行 | 外圆角 12；header bg `#f7f7f7`，padding `8px 12px`，15px/600；单元格 padding `8px 12px`，15px/400；列宽约 211/317/222；打开 artifact 时 500 宽 |
| `cr-context-usage` | 按钮 32×32（圆角 16），环 16×16；popover 280×244 | popover 圆角 16、padding 16；百分比 20px/700；条 246×8 圆角 3，track `rgba(0,0,0,.08)`；图例 246×126 gap 4 |
| `cr-agent__processing-status` | 100.28×26.25 | 15px/400 `rgba(0,0,0,.5)`；运行中命令行有 `cr-tool-head__primary--loading`；示例 `Processed 13s` / `Running command | Used ◇ 0.8` |

用量环的分段颜色（本次采集）：System Prompt `#6366f1`、Tools `#10b981`、
Conversation `#f59e0b`、MCP `#8b5cf6`、Skills `#ec4899`；示例数值
`20.8% / 36.6K of 176.0K` 只是该次运行的读数，不应写死。

**状态覆盖**：`cr-collapse--collapsed`、`cr-collapse--completed`、展开态、
`cr-tool-diff` 的 all-add 与 mixed、exec 长输出、table 默认/artifact 打开、
usage popover、processing/done 都已采集。`cr-collapse--summary` 与
`cr-status-chips` 在 5.8.4 本次采集中**没有出现**；不要按旧类名硬实现，标为
未采集。

可复用数据在 `docs/ui-spec/fixtures/blocks/captured-5.8.4.json`。M2.2 已实现
diff / exec / table 渲染器、752/500 宽度切换、`cr-collapse` 工具组，以及
仅在 agent 上报 Usage 时出现的 `cr-context-usage` 环；popover 目前只显示
总量，五类分解仍待事件流补充。

### 5.4 筛选弹层实测（v5.8.4，2026-10-24，用户提供截图）

采集条件：WorkBuddy AI 5.8.4 macOS 截图裁剪（1256×1184，DPR 2，像素值按 ÷2
换算 CSS px）。侧栏顶栏漏斗按钮弹出的筛选卡：

| 部位 | 实测（CSS px） |
|---|---|
| 卡片 | 宽 240（含 1px 边框），圆角 10，边框 `#f2f2f2`，内容左右内缩 8 |
| 分组标题 | 高 37，文字垂直居中，13px `#7f7f7f`（=`@eb_text_tertiary`） |
| 选项行 | 高 31、行距 3（pitch 34），圆角 8，左右 padding 10；文字 13px `#181818` |
| 选中行 | bg `#f7f7f7`（=`@dropdown_item_bg_selected`），文字加粗 600，行尾绿色对勾 `#57bf9c`，对勾右缩进 10 |
| 分隔线 | 1px `#f2f2f2`，通内容宽；上间距 9，下接分组标题带（标题带自带余量） |
| Reset Filters | 行高 31、下外边距 9，文字 13px `#a3a3a3`（≈`@eb_chevron`） |

对勾只在选中行出现（状态组与时数组各一个）；未选中行无对勾、不加粗。悬停态
未采集，实现沿用选中底色（自创，见 §10-27）。

### 5.5 artifact 文件卡实测（v5.8.4，2026-10-24，用户提供截图）

采集条件：会话页截图裁剪（1656×352，DPR 2，像素值按 ÷2 换算 CSS px）。面板
收起（752 宽会话列）时，已完成 agent 回答正文下方的文件卡：

| 部位 | 实测（CSS px） |
|---|---|
| 卡片 | 两张 370×64，间距 12（2×370+12=752 恰好铺满列宽，等分拉伸）；圆角 10，底色 `#f2f2f2`（=`@bg_hover_light`），无可见边框 |
| 徽章 | 24×24 圆角方块 `#7A95F8`，白色字形；左内缩 14，徽章→文字间距 12 |
| 文件名 | 15px/600 `#000000e6`（=`@eb_text_primary`），超长省略号 |
| 大小 | 13px `#797979`（=`@eb_text_tertiary`），在文件名下方（如 `32.6 KB`、`131 B`） |
| View all 链接 | 卡片下方 26（文字顶沿起算），14px `#4C4C4C`（=`@eb_text_secondary`）+ 尾部 `›` 箭头（约 5.5×8.5，间距 ~4.5） |

自创部分（见 §10-28）：最多显示前 2 张、`N>2` 才有链接、扩展名→徽章映射、
悬停底色。点击行为按用户口述：点任意卡或链接都展开 artifact 面板，卡选中
自己那个文件的标签。

### 5.6 会话顶栏操作区实测（v5.8.4，2026-10-24，用户提供截图 ×5）

采集条件：默认态整栏 + 搜索悬停/展开、Share 悬停、面板开关悬停四张截图
（DPR 2，像素值按 ÷2 换算 CSS px）。**修正 §5.2 澄清 2**：顶栏右侧默认
就有**三个**图标按钮（搜索 / Share / 右面板开关），此前 DOM 录制只数到两
个是采集窗口太短，不是产品只有两个。

| 部位 | 实测（CSS px） |
|---|---|
| 按钮组 | 三枚 32×32 幽灵按钮，间距 4（pitch 36），字形约 14；常驻，面板开合不增删按钮 |
| 悬停底 | 32×32 圆角 8 方块 `#f0f0f0`（新增 shell 局部 token `@eb_icon_hover_bg`，`button.eb-icon:hover` 从 `@bg_tertiary` 改到实测值） |
| 搜索 | 收起态 = 放大镜，tooltip `Search in conversation ⌘F`；点击后原地展开为搜索药丸，放大镜按钮隐藏 |
| 搜索药丸 | 300×32，圆角 8（顶边内缩曲线按 r≈8 拟合），底色 `#f2f2f2`（=`@bg_hover_light`）；左内缩 9 放 14px 放大镜（`#797979`≈52% 黑，自绘 `search-dim`），占位文本 `Search in conversation`（≈`#a9a9a9`），尾部 `^` `v` `×` 三枚 24px 步进（pitch 24；`×` 约 8×8、`#484848`），右内缩 12；标题在药丸左侧**保持可见** |
| Share | 弯曲前向箭头（重绘 `share`），tooltip 只有 `Share` |
| 面板开关 | 圆角矩形 + 右侧实心导轨（新自绘 `panel-right`），tooltip `Open right panel ⇧⌘B`；面板打开时同一按钮 tooltip 变 `Close right panel ⇧⌘B`（产品帧未采集到开态 tooltip，措辞按对称推断，记录在 §10-29） |
| 快捷键 | ⌘F 展开药丸并聚焦、⇧⌘B 开合右面板，在窗口 key-press 里实现（macOS GTK 把 ⌘ 报成 CONTROL_MASK）；tooltip 里写快捷键的前提就是它们真的能用 |

## 6. 屏幕清单（全量 143 个 CSS module）

| 分组 | 模块 |
|---|---|
| 主壳 | `home` `main` `main-content-core` `index` `bootstrap` `route-config` `appearance` |
| 工作流页面 | `project-list-page` `project-detail-page` `colleague-chat-page` `skills-market` `space-library` `calendar` `automation` `task` `task-preview` |
| 设置面板（14） | `ModelsSettingsPanel` `MemorySettingsPanel` `SystemPermissionsPanel` `SecurityCenterPanel` `ExtensionsSettingsPanel` `DataManagementPanel` `ClawSettingsPanel` `PersonalizationPanel` `LaboratoryPanel` `SoftwareConfigPanel` `DeviceConnectionsPanel` `AccountPanel` `HelpFeedbackPanel` `BuddyAppsPanel` |
| 预览器 | `docx/pptx/sheet/pdf/image/video/audio/excalidraw/drawio/miora-preview-component`、`lightbox-shell-impl`、`media-preview` |
| 编辑器 | `markdown-editor-core` `codeEditor` `editor` `xterm` `desktop-terminal-view` |
| 选择器 | `skill-picker` `skill-selector` `expert-picker` `emoji-picker` `space-library-picker` `lexiang-content-picker` |
| 协作 | `collab` `colleagues-panel` `collaborator-avatar-stack` `enterprise-annotations` |
| 连接器 | `connector-capsule` `connector-device-code-modal` `connector-token-config-modal` `im-channels` |
| 商业化 | `purchase` `goods-purchase` `quota-toast` `force-upgrade` `cloud-assistant-upgrade` `storage-quota-exceeded-dialog-host` `high-credit-approval-floating-panel` |
| 平台外壳 | `mac-close-button` `mac-dmg-prompt-services` `window-apps` `desktop-sdk-setup` `workspace-preparing` |
| 基础设施 | `toast` `plugins` `resource-monitor-modal-host` `self-check` `keyboard-shortcuts` `oauth-callback` |

**实现侧类名（自造，非产品类名）**：壳里所有 `eb-*` 类都是自造的钩子，**不要当成实测类名**：`eb-nav` / `eb-nav-active` / `eb-chip` / `eb-segment` / `eb-task` / `eb-section-label` / `eb-tab` / `eb-tab-close` / `eb-menu` / `eb-menu-item` / `eb-missing` / `eb-missing-label` / `eb-detail*` / `eb-code` / `eb-code-gutter` / `eb-code-scroll` / `eb-unbuilt-*` 等。产品的对应物见上表（如标签页 = `artifact-tab`）。

**v0.1 范围内**：主壳 + 主工作区。其余为后续批次。

### 6.1 窗口注册表（尺寸来自 `main/window-apps.js`）

产品把窗口定义成一张注册表，每项含 preset、控件集、默认/最小尺寸与实例策略：

| 窗口 | preset | 默认 | 最小 | 实例 |
|---|---|---|---|---|
| `main` | main | 记忆值 | — | single |
| `office` | workspace | **1440×900** | 640×480 | multiple |
| `buddy-app` | workspace | 1440×852 | 640×480 | multiple |
| `settings` | settings | 900×680 | 800×640 | single |
| `global-search` | utility | 800×640 | 800×640 | single |
| `conversation` | utility | 640×800 | 480×600 | single |
| `image-viewer` | utility | 960×720 | 480×360 | single |
| `developer-profile` | utility | 640×720 | 480×560 | single |
| `im-merge-detail` | utility | 600×720 | 420×400 | multiple |
| `demo` | utility | 760×680 | 520×440 | multiple |
| `im-reaction-users` | utility | 320×322 | 320×320 | multiple |

主工作区（`office`）基准 **1440×900**；抓取与实现都在 **1440×868 客户区**（900 − 32px 标题栏）上比对。

## 7. 动效

| Token | 值 |
|---|---|
| `--eb-motion-duration-instant` | `.1s` |
| `--eb-motion-duration-fast` / `-sm` | `.15s` |
| `--eb-motion-duration-base` / `-medium` | `.2s` |
| `--eb-motion-duration-gentle` | `.3s` |
| `--eb-motion-duration-slow` | `.4s` |
| `--eb-motion-easing-standard` / `-ease-in-out` | `cubic-bezier(.4, 0, .2, 1)` |
| `--eb-motion-easing-decelerate` | `cubic-bezier(0, 0, .2, 1)` |
| `--eb-motion-easing-accelerate` | `cubic-bezier(.4, 0, 1, 1)` |
| `--eb-motion-easing-emphasized` | `cubic-bezier(.2, 0, 0, 1)` |

已知用法：骨架屏 shimmer、`welcome-container--entering/--exiting`、`chat-container__message-skeleton-content--hiding`、`team-member-bar-slot--fade-left/--fade-right`。

## 8. GTK 3 实现约束（替代原"CSS 基线"）

原方案以 Chromium 109 为基线并逐条降级；选 GTK 3 后该表作废，改为下表。原则上**只用 GTK 3 必然支持的属性**，其余用 Rust 侧布局等价实现。

| 需求（Web CSS） | GTK 3 | 处置 |
|---|---|---|
| CSS 变量 `var()` | ❌ | 编译期展开（§3.5） |
| `color-mix()` | ❌ | 编译期预计算（§3.5） |
| flex `gap` | ❌ | `GtkBox` 的 `spacing` 参数 |
| flex / grid 布局 | ❌ | `GtkBox` / `GtkGrid` / `GtkPaned` |
| `:has()`、container queries | ❌ | Rust 侧按状态切换 CSS class |
| `aspect-ratio` | ❌ | 显式 `set_size_request` + `GtkAspectFrame` |
| `backdrop-filter` | ❌ | 用不透明/半透明底色近似 |
| `transition` / `transform` | ⚠️ 部分 | `transition` 可用；复杂动画走 `gtk::Timeline` 或自绘 |
| `box-shadow`、`border-radius`、渐变 | ✅ | 直接用 |
| `min-width` / `padding` / `border` | ✅ | 直接用（§3.5 冒烟测试已实测生效） |

## 9. 资产与法务边界

| 类别 | 处置 |
|---|---|
| **token 数值**（色值 / 间距 / 曲线） | 事实，可用。已重命名空间 |
| **自有 CSS/JS 源码** | 有著作权，**不得整段复制**；本文档只记录可观察的数值与结构 |
| **字体**（27 woff2 / 21 woff / 22 ttf，含改名后的 `"Open Sans (Custom)"` 等） | **不得打包分发**，必须换自有字体并重测字宽 |
| **品牌资产**（`icon.icns`、`icon.png`、`claw-welcome-*-light.svg` 等） | **不得复用**，需自绘；产品名不得使用 "WorkBuddy" |
| **EULA / ToS** | 大概率禁止逆向；本项目定位为**互操作性研究**，不得发布任何暗示官方关联的产物 |

## 10. 已知缺口 / 下一步

**已完成（本轮）**

- 技术路线定为 **GTK 3 原生**（§2）；包名 `evobuddy`、显示名 **EvoBuddy**。
- token 全量预计算：85 处 `color-mix()`、全部 `var()` 链展开（§3.3）；GTK 产物 `tokens-light.css`（1043 条 `@define-color`）、`tokens-dark.css`、`tokens.resolved.json`（§3.4）。
- **欢迎态真实 DOM + 计算样式已采集**（CDP，端口 9333，只读；`capture_dom.js` / `query_layout.js`）。§4.3 几何全部实测，§5.1 是可定位的类名清单。
- **会话态连续录制已采集**（`watch_dom.js`，21 帧，含两个 popover 帧）。§4.4 的三栏几何全部实测；§5.2 的类名清单直接由 `dom-snapshot.json` 汇总，不再靠 CSS 名推断；早期基于 `main-content-core.css` 的 `message-*` / `assistant-message-*` 命名已判定为旧版命名（§5.2）。
- **两个状态都已按实测重建**，代码拆成模块：`src/main.rs`（薄壳）+ `spec.rs`（实测字面量）/ `model.rs`（状态）/ `widgets.rs` / `composer.rs` / `sidebar.rs` / `welcome.rs` / `conversation.rs` / `detail.rs` / `shell.rs`（装配）/ `selftest.rs`（自检）。gtk-rs 的 notebook 例子已挪到 `src/bin/notebook/` 作为 CJK 输入基线保留。
- 交互已连通：欢迎态（导航切换、分段开关、侧栏折叠、chip 预填输入框、输入框启用/发送、下拉 popover）+ 一条真实的会话状态（发送 → 建任务 → 开会话 → 开结果面板）。结果面板的「展开」按钮接的是真动作（`detail_maximised`：面板占满除侧栏外的全部宽度）。Enter 发送（含 IME 组合保护）与欢迎 → 会话的整条路由已用合成按键实测跑通：`EB_ENTER=1` 打印 `before=welcome` / `after=conversation`，见缺口 17/18。
- 实测校验通过（欢迎态）：`teams-container` 1440×868、侧栏 280、居中列 800、分段控件高 36、chip 行高 32、输入卡 800×144、选择器行高 36；hero / 分段 / chip / 输入卡的 y（127 / 185 / 285 / 333）与产品**逐项一致**；8 个关键色解析值与实测完全吻合。
- 实测校验通过（会话态）：会话顶栏 64 高、**操作组 68×32 @776,16（逐像素一致）**、消息列表 579×636、输入区 579×168、输入卡 531×134、**结果面板 861,0 579×868（一致）**、**结果面板头部 579×64（一致）**、标签页 240×32 @877,15.5（本轮修正后一致）、加号 1121、头部操作 68×32 @1356（一致）。
- **交互逐控件审计**（`EB_CLICK=1`，33 步「点一下 → 读回状态」，每步打印可观测变化）：导航、chip、分段开关、侧栏折叠 / 恢复、Enter 发送、发送按钮、结果面板展开 / 恢复 / 关闭、多文件标签切换与单标签关闭（`EB_ARTIFACT=1` 下断言 `tabs=2 → 1` 与选中文件 `notes.md → demo.py`，见 §10-26）、任务行重开面板、回答下 artifact 文件卡（`EB_ARTIFACT=1` 下断言 `cards=2` 且点第一卡把面板从 `demo.py` 切到 `notes.md`，见 §5.5/§10-28）、顶栏三件套（搜索药丸展开 / 收起、Share 弹自创 notice、面板开关按 tooltip 关面板，见 §5.6/§10-29）、消息快捷按钮（复制 `Copied` 反馈、编辑回填输入框、重新生成追加一轮对话、点赞弹自创 notice；见 §10-25）**全部有响应**。审计顺带查出并修掉三个**真实缺陷**（产品侧同样会看到的问题，不是测试环境问题）：
  1. **`set_no_show_all(true)` 必须放在 `show_all()` 之后**。`gtk_widget_show_all()` 对 `no_show_all` 控件**直接 return，也不向下递归**，所以此前所有下拉 popover 的内容、以及所有"隐藏待显"按钮（侧栏展开 `▶`、Code assist）的**子控件从未被 show 过**：下拉弹出来是个**空盒子**（requisition 恒为 66×42），按钮是个**空方块**。修法：新增 `widgets::hide_from_show_all()`（先 `show_all()`，再置 `no_show_all` + `set_visible(false)`），7 处调用点全部改掉。修后 popover requisition 回到按内容计算（筛选菜单 93×114、模型菜单 71×114、提示 294–324×48–80），审计打印 `3 of 3 menu rows visible`；离屏渲染 `menu.png` 里能直接看到模型下拉「Auto / Fast / Deep」。
  2. **侧栏分区标题点了没反应** —— 现在接真动作：折叠 / 展开（状态在 `App.collapsed`；因为 `show_all` 会把两个分区都显示出来，折叠只能在 `show_all()` **之后**施加），chevron `⌄` / `›` 随之切换。审计打印 `shown → hidden → shown` 的可见性链。
  3. **结果面板代码区横向被裁且无处可滚** —— 非换行 `GtkTextView` 上报的是"已排版行的宽度"，而它永远不会超过被分配到的宽度，请求宽度于是收敛到分配宽度、长行被裁掉。修法：用 Pango 量出最长行并 `set_size_request`，滚动区 `hadjustment` 由 `579/579` 变为 `579/646`（`upper > page_size`）⇒ 可横向滚动。

**缺口**

1. **吉祥物已用自有资产占位** —— 产品 `wb-home-route__growth-buddy` 不得复用；EvoBuddy 的 120×120 绿色 buddy 由 `crates/eb/assets/gen_icons.py` 绘制，以 overlay 放在输入区右上，不改变 hero/输入卡几何。
2. **图标已换成自有 PNG** —— 23 个图标由 `crates/eb/assets/gen_icons.py` 生成、`include_bytes!` 嵌入，不依赖 `hicolor`/Adwaita 或 librsvg；许可为仓库 MIT。
3. **GTK 3 的 `box-shadow` 会被裁剪到控件内（已调查，暂接受）** —— 输入卡的 `0 16px 32px -8px rgba(0,0,0,.03)` 在产品里会铺到下方选择器行背后。实测 832px 外层 gutter 仍看不到阴影；完全还原需 Cairo 自绘或改布局。
4. **输入卡高/宽差已修**（欢迎态 800×144，会话态 531/783×134；产品 532/784×134，剩余 1px 是 GtkPaned sash 折中，记录在 §4.4）。
5. **文案是占位** —— 产品文案（品牌标语、空态、AI 声明、输入框提示）未复制，各模块顶部的文本常量用自有措辞替代；需替换为 EvoBuddy 自己的文案。
6. **间距 / 圆角 / 字号已进生成器（部分）** —— `build_gtk_tokens.js` 现输出 `tokens-metrics.{json,rs,css}`，`spec.rs` 已用生成常量，新内容块用生成的 fs/radius 类；旧 `ui/gtk.css` 的结构性字面量尚未全部迁移，CI 用 `docs/ui-spec/check_tokens.sh` 防漂移。
7. **UI 字族未在 token 层定义** —— 由基础 CSS 直接声明；产品的 `"Segoe UI (Custom)"` / `"Open Sans (Custom)"` 是改名后的自包含字体，**不得打包**，必须换自有字体并重测字宽（会影响上述所有文本宽度）。
8. **i18n 文案清单**未提取（`zh-cn-*.js` / `zh-tw-*.js` 分片，可作为屏幕/状态枚举的权威来源）。
9. **z-index 层级表**未提取。
10. **工具调用只做了折叠态** —— 5.8.4 已补采 `cr-collapse` 展开/完成/折叠、`cr-tool-diff`（all-add 与 mixed）、`cr-tool-exec` 长输出、`cr-table-block`、`cr-context-usage` popover、`cr-agent__processing-status`；数值与类名见 §5.3，fixture 见 `docs/ui-spec/fixtures/blocks/captured-5.8.4.json`。**M2.2 已实现** diff / exec / table 三种块、752/500 宽度切换、`cr-collapse` 工具组与 usage 环；usage popover 的五类分解仍待事件流补充，结果面板正文仍是近似重建。`cr-status-chips` 在 5.8.4 本次采集中未出现，标为未采集。
11. **Linux 完全自包含：已验证；macOS 同（v0.3.0 起，双架构 .app）** —— 见 §2.1：C 探针与本程序都在裸 glibc 2.28 上跑通，自带 GTK 3.24.5 + schema + 图标 + 字体，几何与 macOS 一致（唯一差异是文本宽度）。产物已成形：`dist/evobuddy-<版本>-linux-<arch>.tar.gz`（内含 `run`、`install.sh`、desktop entry、图标、SHA-256）与单文件 `dist/evobuddy-<版本>-linux-<arch>.AppImage`（同样带 SHA-256），脚本在 `packaging/linux/`。macOS 侧 `packaging/macos/` 产出 `evobuddy-<版本>-macos-<arch>.tar.gz`（内含自包含 `EvoBuddy.app`，arm64/x86_64，验证方式是藏起 Homebrew 真跑，见 §2.1 与 packaging/macos/README.md）。剩余：ibus 输入法（缺口 22）。
12. **CI 已切到 `evobuddy`，并强制验证自包含（已完成）** —— `.github/scripts/build-linux.sh` 已删除：Linux 任务改为直接调用 `packaging/linux/package.sh` + `verify.sh`，开发者与 CI 走同一条路径；且在**裸 Debian 10 容器里真跑一次**（含 `EB_SELFTEST=1` 几何自检与 `install.sh`），所以"自包含"是每次打 tag 都被强制，而不是靠记性。CI 还会先用 `bash -n` / `dash -n` 检查脚本语法——`install.sh` 上一次语法错误正是直到裸机测试才暴露。Windows 任务只做了二元切换，原因见缺口 24。`notebook` demo bin 仍留在 `Cargo.toml` 里，但已不作为产物发布。

**已在 GitHub 上跑通（v0.2.0，run 39022034107）**：四个任务全绿，两台 runner（ubuntu-24.04 x86_64 / ubuntu-24.04-arm aarch64）都在**裸 Debian 10.13**（无 gtk/pango/cairo/fontconfig 包、**0 个字体文件**）里跑通了各自的产物：`css: parsed 72598 bytes`、`window 1440x868 (want 1440x868)`、`sash sidebar x=280`、66 个已解析库中 6 个来自宿主（全是 glibc 家族）、最大符号 `GLIBC_2.28`，且 `install.sh` 实跑通过。产物 30M / 29M，已连同 SHA-256 与 Windows zip 发布在 release 里。

**v0.2.1（run 39046105609，全绿：x86_64 2m15s / aarch64 2m26s / windows 3m23s）**：Linux 任务首次把 AppImage 也纳入裸机验证——`evobuddy-0.2.1-linux-{x86_64,aarch64}.AppImage`（28M / 27M）通过 type-2 魔数校验后，在**裸 Debian 10.13 上无 FUSE 执行**（`--appimage-extract-and-run`），渲染 PNG 与目录树**逐字节相同**（45209 字节；x86_64 与 arm64 互相一致，也与 macOS 历史值一致），glibc 下限不变（2.26/2.27/2.28）。Windows zip 更名为带版本号并补了 `.sha256`。release 资产共 11 个。

**v0.3.0（run 39083116807，六个任务全绿）**：macOS 双架构独立 .app 首次进入发布——`macos-15`（arm64）与 `macos-15-intel`（x86_64，Intel 镜像的最后一班）上 `verify.sh` 全部通过：load-command 审计零泄漏、把 Homebrew prefix（`/opt/homebrew`、`/usr/local`）`chmod 000` 藏起后真跑几何自检通过。前两轮 CI 各抓到一个真问题：runner 屏幕小于 1440×868，GTK 把窗口钳到工作区（1222×778），几何断言改为只盯屏幕无关的不变量；SVG 挪到 `packaging/` 后构建镜像缺 `COPY`（本地镜像缓存盖住了它，只有干净环境的 CI 才看得见——正是 CI 存在的意义）。release 资产 14 个：Linux 每架构 `tar.gz`+`AppImage`、macOS 每架构 `.tar.gz`（内含自包含 `EvoBuddy.app`）、Windows zip，各带 SHA-256。

**CI 第一次跑就抓到一个真 bug**（也印证了"必须真跑一次"）：`bundle.sh` 收尾的 `readelf` 统计在 `set -e` + `pipefail` 下必然退出 1（个别库如 `libcairo-gobject` 没有 `GLIBC_` 版本信息，`grep` 返回 1），顺带还把统计结果打印成了截断的错误集合。它躲过了此前所有本地验证，因为本地都是 `./package.sh | tail`——管道把退出码换成了 `tail` 的。教训已记进 `packaging/linux/README.md`。

**"下载下来是个目录"（v0.2.0 的用户反馈）→ 单文件 AppImage**：用户要的是"揭开后就是一个可执行文件"。GTK 3 无法静态链接成一个 `.exe`/ELF（它按需 dlopen 输入法模块、pixbuf loader 与主题），所以目录树是必然的；单文件且仍自包含的标准答案是 AppImage——同一棵树 + `AppRun` + 内嵌 squashfs + 一个 runtime。踩到的坑值得记下：`appimagetool` 与 runtime 都是 **static-pie 且 `EI_ABIVERSION` = `AI`**，真实内核忽略该字节，而本机 amd64 容器（Rosetta）直接 `Exec format error`；用 `gcc -static` / `-static-pie` 的对照实验排除了"静态二进制在此环境不可用"，最终用 `od` 逐字节对比定位到那个字节。结论：**模拟层下无法构建也无法运行 AppImage**，于是把 `PLATFORM` 默认改成宿主原生架构（在 Apple Silicon 上即 arm64，全流程原生可验证），x86_64 交给 CI 的真实 runner。顺带把 Windows zip 也补上版本号与 `.sha256`（此前只有 Linux 产物有校验和）。
13. **三栏分隔条有 1px 残差** —— 产品的分隔条是 4px **全透明**覆盖层，骑在栏边界上、不扣减栏宽（278–282 / 858–862）；`GtkPaned` 的手柄必然占位，当前折中成 1px 透明手柄 ⇒ 会话栏 579 而非实测的 580（结果面板 579 一致）。要完全对齐需自绘热区（`GtkOverlay` + 事件盒），或接受这 1px。
14. **会话顶栏内容左内缩 24px 的成因未确认** —— 实测 `workbuddy-topbar-options` 起点 x=304（距栏左 24）、右端 844（距栏右 16），左右不对称；但顶栏计算样式只有 `padding: 0 16px`，多出的 8px 疑似来自 `--mac` 修饰类或 `-options` 的 margin。当前实现未按实测追加这 8px（标题落在 297，产品 304）。
15. **chip 点击行为是推测** —— 实测点击 chip 既不清空也不导航（§5.2），「点击填入输入框」是实现侧的选择。
16. **结果面板正文是近似重建** —— 产品那里是 Monaco 自绘编辑器（72px 工具条带 + 代码区），当前用两个 `GtkTextView`（行号槽 + 代码列）拼出等效布局；GTK 3 CSS 不支持语法高亮，等宽字体也不同，故行宽/换行与产品不一致。标签名用 `max-width-chars = 24` + `EllipsizeMode::Middle` 限宽（这是 GTK 3 约定的组合），但**没有实测过超长文件名**（demo 名只有 12 字符），也没抓到产品对超长标签的处理方式。
17. **截图与合成事件的坑（已排除，记录备查）**
    - `gtk_widget_draw` 遇到 `priv->alloc_needed` 为真的控件会**静默不画**（只打一条 `Gtk-CRITICAL`，不画任何东西），而 GTK 自身的绘制路径不检查该标志。根因：刷新时新增控件排了一次 resize，而窗口尺寸不变、布局循环不会再跑，标志就永远留着。修复方式是在截图前 `window.size_allocate(&window.allocation())` 清标志（GTK 自己的机制也是这么做的）。**真实窗口一直是正常的，只有截图受影响**；也不要走 `screencapture`（无录屏权限，抓到全黑）。
    - 合成按键必须带一个**已映射的 GdkWindow**：`gtk_widget_event_internal` 会先跑 `event_window_is_still_viewable`，`GDK_KEY_PRESS` 的窗口为空或未映射就**直接丢弃**（连 handler 都不会进）。而 `gtk_test_widget_send_key` 又会用 `gtk_widget_get_window` 去查 keymap，文本视图是无窗口控件 ⇒ 对它调用只会打出 `GDK_IS_WINDOW` 断言并返回 false。所以 `EB_ENTER` 是自己造 `GdkEventKey`、挂上顶层窗口、再 `gtk_propagate_event` 给编辑器。
    - 另外：`gtk_widget_send_focus_change` 是给"顶层窗口不活跃"（CI / 无窗管）时补发 focus-in 用的；GTK 只在 `is_active` 时自己发，`grab_focus()` 单独用不够。
    - 合成**点击**必须落在已 realize 的控件上：`gtk_widget_event` 的 `WIDGET_REALIZED_FOR_EVENT` 断言只看 `_gtk_widget_get_realized()`，而"本帧刚变可见"的控件要等下一帧布局才 realize（合成事件会早于它到达，真实鼠标不可能这么早），表现是两条 `Gtk-CRITICAL` 且 handler 根本不进。审计的 `click()` 里先 `widget.realize()` 补上这一帧。
    - `gtk_widget_show_all()` 对 `no_show_all` 控件**直接 return**（且不递归）——详见上文"已完成"里的缺陷 1，这是本轮最贵的一个坑。
18. **IME 组合态下的 Enter 无法自动化验证** —— 实现侧已在 `key-press-event` 里先调用 `gtk_im_context_filter_keypress`（这是 GTK 专门给"在默认处理之前运行的处理器"的 API），输入法接管则 `Propagation::Stop`，不再发送。`EB_ENTER=1` 用合成事件跑通整条链路「聚焦 → 输入 → Enter → 建任务 → 切路由」，实测 `before=welcome` / `after=conversation`、且 buffer 被 `reset` 回提示语 ⇒ **无输入法时的 Enter 发送已验证**；**组合态仍未验证**，需要真实输入法介入（自动化提供不了）。`Shift+Enter` 与发送按钮始终是安全路径。
19. **"本构建未实现"提示 popover 是自创 chrome** —— 侧栏搜索 / 小程序、会话顶栏分享、结果面板新建标签、Code assist 这 5 个控件，产品的行为都没采集到（而且产品里全局搜索是**独立窗口** `global-search` 800×640，不是本栏内的东西；会话顶栏的搜索在 2026-10-24 截图里已采集到展开态，改为真实实现，见 §5.6）。实现侧的选择是：点击弹一个 `eb-notice` popover 明说缺什么，而不是静默无反应——静默会被读成"坏了"（这正是本轮用户反馈"部分能点，部分失效"的来源之一）。这些 popover、`eb-missing-label` 样式与文案都属自创，记录备案。
20. **结果面板标签体与分区的折叠行为也属实现侧选择** —— 标签体不再是按钮（唯一的标签就是当前标签，点它没有动作），标签里的 `×` 变成真的关闭按钮 `eb-tab-close`（14×14，靠 `button.eb-tab` → `.eb-tab` 的类名改动保持几何不变）；`eb-section-label` 的折叠是照着我写的 tooltip 实现的，产品实测里没抓到过这类点击。
21. **popover 与 overlay 滚动条在"无头 / 非活跃窗口"环境下的呈现无法完全验证** —— 审计环境没有窗管激活窗口（`window active=false`，还伴随 `Failed to initialize CVDisplayLink`），下拉在修好缺陷 1 后能按内容 sizing、也能在离屏渲染里看到（`menu.png`），但**真实活跃窗口下的呈现与焦点**未验证。同理代码区的横向滚动条在 macOS 上是 GTK 的 **overlay 指示条，静止时透明**，离屏截图看不到，只能用 `hadjustment` 的 `upper > page_size`（579/646）证明可滚动。
22. **自带 GTK 栈下的 CJK 输入法** —— Debian 10 的默认 immodules 只有 `im-xim` / `im-cedilla` / `im-ipa` 等，**没有 `im-ibus`**（它随 `ibus` 包分发），而 Linux 桌面上的中文输入绝大多数走 ibus。要在自带栈里打中文，得把 `im-ibus.so` 与 `libibus-1.0.so.0` 一并打包（并保证 `GTK_IM_MODULE=ibus` 时能找到），否则只剩已自带的 XIM。目前两者都未做，探针不带输入法。
23. **Windows 7 / 8.1 未验证，且文档自相矛盾** —— 硬要求里有 Win7/8.1，而 §2 表原先写"GTK 3（MSYS2 构建）无 Win7 门槛"，README「Prebuilt binaries」却写"MSYS2 自 2024-05 起不再支持 Win7/8.0，因此没有可维护的 GTK 3 运行时"。二者必有一错。要分清**构建环境**与**目标机**：Cygwin/msys2-runtime 3.5 不支持 Win7 只影响构建侧；MinGW-w64 产物理论上仍可声明 `_WIN32_WINNT` 兼容 Win7，但新版 GLib 可能已调用 Win8+ API（如 `GetSystemTimePreciseAsFileTime`），这只能真机实测。需要一台 Win7/8.1 虚拟机才能定论，目前无法验证。
24. **Windows 产物并不自包含（且未验证）** —— `evobuddy-<版本>-windows-x86_64.zip` 目前只带 GTK 3 的 DLL 与 exe，**不带 GTK 的数据文件**：GSettings schema、immodules、gdk-pixbuf loader cache、图标主题、字体。缺 `org.gtk.Settings.*` 时 GTK 会直接拒绝启动，所以这个 zip 在没装过 MSYS2/GTK 的机器上大概率起不来。Linux 侧靠启动器在运行时重写两份 cache 里的绝对路径；Windows 需要等价的 `.cmd` 启动器 + 真机验证，因此**刻意没有照抄一份没验证过的实现**。CI 里该任务设为 `continue-on-error`（不阻塞 Linux 发布），产物自带的 README 也如实写明"未验证、非自包含"。另外 UCRT64 目标是 Win10+，与缺口 23 的 Win7/8.1 问题叠加。（v0.2.1 起该 zip 带上了版本号并补了 `.sha256`，与 Linux 产物对齐；但"单文件"对 Windows 无解——GTK 3 无法静态链接进一个 exe。）
25. **消息框下方的快捷按钮整行属自创** —— 产品会话页的消息操作行从没被采集到（早期截图里用户气泡下方只有时间 + 我们自己的复制键）。本构建把它扩成:用户消息 = 复制 / 编辑重发（回填输入框）；已完成的 EvoBuddy 回答 = 复制回答（只含正文文本，不含思考/工具块）/ 点赞 / 点踩 / 重新生成（把该回答对应的用户提问按正常提交链路再发一次）。复制、编辑、重新生成都有真实行为并进了 33 步点击审计；点赞/点踩没有反馈通道可写，按 §10-19 的纪律弹 `eb-notice` popover 明说"本构建不发送反馈"。图标 `pencil/regenerate/like/dislike` 由 `gen_icons.py` 自绘。流式回答期间不画这一行（`agent_turn` 只在 `!streaming` 时打包 toolbar）。
26. **结果面板的多文件标签条属自创** —— §4.4 的 `artifact-tab` 240×32 只在**单标签**场景被采集到。本构建把面板扩成"任务写过的每个文件一个标签"（最多 12 个，重写同名文件会把它移到末尾并选中），但 240px 被降级为**上限**而非定宽：标签按内容收缩（名字过长按中间省略），整条标签列表包在不传播自然宽的横向 scroller 里，放不下时横向滚动——定宽 240 的两个标签会把表头自然宽顶到 776，GtkPaned 的 resize-child 语义会据此抢回分隔条位置（§4.4 宽度回归的根因）。标签内 `×` 只关掉那一个文件（关到空才收起面板，且不置"永久收起"旗，下一个写入的文件仍会打开面板）；顶栏"▥"仍是整面板收起。加号按钮维持自创 notice（本构建不能凭空新建 artifact）。
27. **筛选弹层的悬停底色属自创** —— 2026-10-24 用户提供的产品截图把筛选卡整体补齐了（§5.4：240 宽卡片、37 分组标题带、31 行高 34 pitch、选中 `#f7f7f7` 圆角 8 药丸 + 加粗 + 行尾 `#57bf9c` 对勾、`#f2f2f2` 分隔线、灰色 Reset Filters），旧版"✓ 前缀 + 4px 间距"的临时样式已按实测重做。截图里没有悬停帧，悬停沿用选中底色；卡片在离屏快照里必须把底色/边框画在 `.eb-filter-body`（子 widget）上而不是 `> contents` 节点——`snapshot()` 只画子 widget，contents 节点背景在离屏渲染中是透明的（旧 `.eb-menu-body` 同理，属同一坑）。
28. **artifact 文件卡的取数规则属自创** —— 2026-10-24 用户截图给出卡片外观（§5.5）与点击行为（点卡/点链接都展开面板），但没采集到"什么时候显示几张卡"。本构建定死为：**最后一个已完成 agent 轮**下、按写入顺序显示**前 2 个**文件、`N>2` 才有 `View all artifacts (N) ›` 链接（链接选最新文件）。徽章映射也是自创：扩展名命中代码表（c/py/js/ts/rs/…含 md/json/yaml 等配置文本）用 `</>`（`artifact-code`），其余用 `?`（`artifact-unknown`），两枚均由 `gen_icons.py` 自绘；悬停底色（`shade(#f2f2f2,.96)`）同样未采集。字节的显示单位（`B`/`KB`/`MB`，一位小数）按截图样本 `131 B`、`32.6 KB` 推断。卡片行折进所属轮的 sig，因此面板收起再写新文件也会重排；流式期间不画。

29. **顶栏搜索药丸的交互细节属自创** —— §5.6 截图给了外观与三个按钮的常驻布局，但没采集到：Escape 关药丸（本构建加了，与 `×` 同一路径）、无文件任务上面板开关的行为（本构建保持可见、点击无操作，不弹提示）、面板开态的 tooltip 措辞（`Close right panel ⇧⌘B` 按对称推断）、药丸内 `^` `v` 的 tooltip（`Previous/Next match`）。两个已知渲染差异：GTK3 的占位文本在**聚焦时不渲染**（产品截图里聚焦态仍显示占位），离屏快照因此看不到 `Search in conversation` 灰字；药丸放大镜/步进字形颜色（`#797979`/`#484848`）靠自绘 PNG 的 alpha 近似，不是逐像素一致。⌘F / ⇧⌘B 是真实实现的窗口快捷键（tooltip 采集值，兑现才写上去）。审计 +2 步（33 步）：`chat search` 展开药丸（`EB_SEARCH_SHOT` 快照钩子）、`chat search close` 收起、`panel toggle (close)` 验证开态 tooltip 并关闭面板；`chat share` 的 tooltip 从自创的 `Share task` 改成采集值 `Share`，`Search in conversation` 同步加 ` ⌘F` 后缀。

30. **侧栏在途任务的转圈样式属自创** —— §4.3 的 gtk.css 备注只采集到"运行中的任务行显示 spinner（替代行尾时间戳）"这一事实，产品帧没有给过光圈几何。本构建定为：16px 圆环、2px 圆头描边、300° 缺口弧、`rgba(0,0,0,.5)`（与 `@eb_text_tertiary` 同色）、900ms/圈、40ms 步进重绘；判定条件是"最后一个 agent 轮 `streaming==true`"（`task_is_running`），所以后台并发流式任务（§10 之外的 M2 修复）也在自己行上转。相位取**全局时钟**而不是实例计数器：侧栏每轮刷新整表重建，新行必须续着转而不是从头转。普通任务（Tasks 区）与工作区嵌套行共用 `task_row`，两处同时生效。

31. **极窄窗口的三栏保底规则属自创** —— 产品只在 1440×868 客户区被采集过（§4.3/§4.4），没有任何一帧是"用户把窗口拉到 1000px 以下"。本构建的窗口有效最小宽 **1082px**（GTK 由三栏控件的自然最小撑出来，实测值；`EB_WINDOW_WIDTH=900` 也只会得到 1082）。**（§10-36 之后修正：1082 是面板打开时的下限。面板收起时消息列的 scroller 只申报自己的 `min_content_width`（1）、不再把列的最小宽抬进窗口，所以窗口能一路拉到 800 以下——`EB_RESIZE_TEST=800` 实测 `toplevel=800 surface=800`；代价是旧转录右侧会被裁，见 §10-36 的残留条目。）**规则定在 `shell::outer_sash`（三段保底，从宽到窄）：① ≥1384（含实测基线 1440、1920）面板保持 580 份额，会话吃掉全部余量；② 1136–1384 会话列先让，但不低于 `MESSAGE_WIDTH(500)+24` ⇒ 分隔条恒为 `sidebar+524`=804，面板从 580 一路收到 332；③ <1136 面板保住**自身内容的实时最小宽**（`detail.root.preferred_width()`，两文件标签条实测 332），会话降到 `spec::CONVERSATION_MIN_WIDTH=240` 为止——这一档实际到不了（窗口先被 GTK 钳在 1082），它是防"分隔条跑到侧栏左边"的数学保底；1082 时会话拿到 470，面板 332、分隔条 750。面板的最小值取实时测量而不是写死常量：§10-26 之后标签条按内容收缩，标签数一变面板最小就变，写死的常量会再次和 GtkPaned 的 clamp 打架。
    动因是一个肉眼可见的错位：面板开着、窗口拉窄时，我们设定的分隔条位置（804）和 GtkPaned 实际能给的分配（750，被面板最小宽 clamp）不一致，消息列的 margin 却按 804 算 ⇒ 用户气泡越过分隔条压进面板。现在 margin 一律从 `outer_sash()` 的返回值推（`pane_width = sash - sidebar`），两者同源；`EB_DEBUG=1` 会打印 `layout: width=… pos=… detail_min=… conv_min=…` 供核对。窄态下面板顶部的 ×/+/◐/⤢ 与顶栏 ⇧⌘B 都仍可点，收起面板后会话列回到 752。`evobuddy` 新增单测 `the_outer_sash_holds_the_captured_share_then_the_two_floors`（含 640–1920 全区间不变量）。
32. **agent 在"回答完之后还在干活"时的状态行属自创** —— 产品从没采集过这种帧：ACP agent 先给本轮的 `stopReason=end_turn`，随后（Grok 的 `deep-research`/子 agent）继续往同一条 session 推 `session/update`。旧实现在响应处就 `Finished` 并**再也不读 stdout**，于是那半段结论永远不出现——用户看到的正是"话说到一半就结束了"。本构建把一轮的结束条件改成"响应 + 静默窗口"（`AcpConfig.tail`）：响应后先等 **2s**（`EVOBUDDY_ACP_TAIL_FIRST_SECS`）探有没有续流；一旦有，就按 **40s** 静默上限持续收（`…_TAIL_QUIET_SECS`），总时长再封顶 **900s**（`…_TAIL_MAX_SECS`）。40s 不是拍的：2026-10-27 用独立 stdio 驱动抓真实 Grok，它在 provider 退避时相邻两条 `retry_state` 的间隔实测 26.6–35.8s，窗口再小就会在两次重试之间把轮关掉。续流期间状态行文案为自创的 `Agent is still working`；provider 退避把 Grok 的 `retry_state` 原样抬成 `Retrying <attempt>/<max>: <reason>`（reason 按字符截 140），因为不显示时用户只会看到"界面卡住"。看门狗同步从"整轮 180s 墙钟"改成"**连续 180s 没有任何消息**才 kill"（`EVOBUDDY_ACP_TIMEOUT_SECS` 语义变更），并在 TERM 后 3s 仍未收尾时补 KILL。审计/单测：`late_updates_stay_in_the_same_turn`（断言续流文本落在同一轮、且 tail 期的权限请求仍被回答）、`provider_retries_reach_the_status_line`。补一条同抓包得出的规则：Grok 把 `retry_state`/`turn_completed` 装在私有 `_x.ai/session_notification` 信封里（那场 23.4 分钟**一条 `session/update` 都没有**，60 条退避全在私有方法里），所以两个信封都要认；`turn_completed` 只在续流阶段生效且只认 `error`/`cancelled`——`stop_reason="error"` 用它的 `agent_result` 把轮判成 `Failed`（否则后台工作死掉时界面"安静地成功"，用户只看到"稍后发结论"没下文），而 `"end_turn"` 必须忽略，它和 spec 响应同时到，认账会把续流窗口提前关掉。单测 `a_failed_continuation_ends_the_turn_with_the_agent_reason`。
33. **流式期间"滚上去就不抢回底部"属自创行为** —— 产品采集里没有"用户往上翻历史 + 同时有 delta 到达"这一帧，而旧实现每次都无条件跳到底部：读长任务时根本翻不动，delta 一到就被拽回最新消息。现在的规则（`Conversation::follow_output`）是**只在读者已经在底部时跟随**：判定用 `upper - value - page_size <= 24px`（`FOLLOW_SLACK`，约半行文本，容得下一次没甩到底的触控板轻扫），底部以内算"在看"，往上超过 24px 立刻解钉。重新钉住只有三个入口，且都是用户自己的动作：① 轮数变多（提交、重新生成——delta 只改已有轮的内容，不算）；② 切换任务或从欢迎页进入任务（`shell` 比对 `painted_task`，`follow_now()`）；③ 用户自己滚回底部。宿主自己的跳转用 `auto_scroll` 旗标包住 `set_value`，免得把"程序滚的"读成"用户滚的"；搜索跳转 `scroll_to_widget` **故意不包**，跳到中间某轮后就不再跟随，正好符合预期。没有加"↓ 有新消息"浮标：那是产品未采集的自创 chrome，先只把抢滚动修掉，需要时再按采集补。验证：`EB_FOLLOW_TEST=1`（配慢速 `EB_FAKE_AGENT`）滚上去后要求后续 delta 不动视图、滚回底部后要求继续跟随，退出码 0；临时把 `pinned` 判定旁路跑同一 harness 复现原症状——`follow-test: FAIL a delta yanked the view +348px while 89 chars streamed in`。该 harness 原来靠存档里恰好存在的长会话撑出滚动区，环境一换就退化成"没有可滚的范围"；现在它自己发一条足够长的 prompt 把转录撑高（`streamed()` 也改成读 `painted_task` 指向的任务），不再依赖状态文件内容。

34. **切换任务时"整表重建"换成按任务缓存的帧复用** —— 报障"在多个正在运行的任务间切换，多切换几次窗口就挂了"。实测（`EB_SWITCH_TEST=40` 配 3 个 20 轮、共 211KB 的存档，`EB_DEBUG=1`）：旧实现每次切换都落进 `fill_messages` 的全量分支（`reuse=0`），把整段转录按 `block_widget` 重建一遍——markdown 解析、代码卡、表格、diff 卡全来一次，20 轮约 40ms；收尾那句 `messages.show_all()` 再走全树约 1100 个控件（均值 9.5ms、峰值 25.1ms）。三个流式任务同时跑时，16ms 的 drain tick 与 33ms 的刷新合并把这类重排推到每秒 20–30 次，主循环追不上自己的节拍：harness 报 `the main loop starved by 1233ms during 40 switches`，且迟到量随切换次数单调增长。改两处：① 离开屏幕的会话帧不再销毁，按任务索引收进 `Conversation::detached`，换回来只是逐帧 `pack_start`；② `show_all` 下沉到"这一帧刚建出来的那个 frame"（`hide_from_show_all` 的控件本来就被 `no_show_all` 挡在外面，可见行为不变）。缓存条数上限 `DETACHED_CONVERSATIONS=4`，超了丢最久未用的那个，它下次被打开时重建——纯工程选择，产品没有对应采集帧，用户可见行为不变，但它是"内存换流畅"的显式取舍，所以记在这里。结果（2026-10-27 复测，同一存档、`EB_SWITCH_TEST=30`）：30 次切换 `worst_late=0ms worst_switch=0ms`，29/30 次走 re-attach、只有冷缓存的第一次全量重排（`conv` 63 次刷新均值 8.5ms、峰值 41.6ms；重排 23 次、平均 3.7 轮）。修复前是每次切换都全量重排（`conv` 均值 23.3ms）再叠全树 `show_all`。harness 同时补了第三条不变量：re-attach 的帧必须**真的被画出来**（可见且分配非零），并用 `EB_SWITCH_SHOT=<path>` 在最后一次切换后 300ms 出图人工核对。

35. **长会话只渲染最近 20 轮、滚到顶再往前加载，属实现侧为性能自创的行为** —— §10-34 修完之后，"切到轮数很多的任务"仍然卡，因为剩下的成本是**按总轮数**算的：122 轮的任务每次切换实测 `remove 18–24ms + attach 4ms`，再加上 `set_child_visible(true)` 触发的全树布局 68ms（树里挂着 6289 个控件），刷新内外合计约 150ms ⇒ `switch-test: FAIL the main loop starved by 3334ms during 20 switches`。产品采集里从来没有"长会话滚动到顶"这一帧，所以这一条整段都是自创行为，只记录不比对。定死的规则：
    - **渲染窗口**：`Conversation::first` 记"当前挂载的第一轮在任务里的下标"，`turns` 只覆盖 `task.turns[first..]`；打开任务/切换任务时窗口取最近 `ATTACH_TURNS=20` 轮，`fill_messages` 里所有下标都是**窗口相对**（要加回 `first` 才够到任务）。
    - **滚到顶自动加载更早**（用户选的方案，**不加**"加载更早"按钮之类的自创 chrome）：`value_changed` 里 `value <= LOAD_EDGE=400px` 且还有更早的轮，就只置 `pending_load` 并请求一次重绘；重绘里 `load_earlier` 建 `ATTACH_STEP=10` 帧、`reorder_child` 到列表头、`splice` 进 `turns`。一片一片来，一次手势只吃一片（门是下面那个 anchor 旗标），否则停在顶边会把整段历史在几十个连续重绘里挂回来。`DETACHED_FRAMES=30` 限制每个离屏会话缓存的帧数，防止"有人翻到过第一轮"让下一次切换又背上整段转录。
    - **滚动补偿必须锚在"帧"上，不能锚在内容高度上**：新帧插在读者上方，不补偿就会被整片推走。第一版只在 idle 里按 `upper` 增量补一次，实测冷加载仍偏 **1580px**（一片 9490px）——reveal 那一次协商用的是 Pango 的临时测量，之后上方内容又长高了一截，`upper` 前后差并不等于最终几何。现在记住**视口顶边下那一帧**和它距顶边的像素（`Conversation::anchor`），只要 adjustment 的 `changed`（内容高度变了）就把视图放回那条线，预算 `ANCHOR_SETTLE_PASSES=16` 次；读者自己滚动、或切到别的任务，立刻交出 anchor（那是他的位置，不是我们的）。宿主自己的跳转一律包在 `auto_scroll` 里，免得把"程序补的"读成"用户滚的"而解钉。
    - 结果（2026-10-27，`docs/make_heavy_state.py` 造的 3×120 轮存档）：30 次切换 `window=(100, 22)`、`worst_late=0ms worst_switch=0ms`、树从 1087 只长到 1139（修复前每次切换挂满 122 轮），`conv` 刷新均值 8.0ms/峰值 41.8ms，reveal 均值 10.6ms（原 68ms）。滚到顶一片 29–47ms，落在读者主动甩到顶的那一下，且两次甩顶的视线漂移实测 **0px/0px**。
    - **代价：会话内搜索现在只覆盖已挂载的窗口**。`search_recompute` 遍历的是 `Conversation::turns`（窗口），所以 120 轮任务里第 3 轮的内容在窗口滚到它之前搜不到；读者往上滚、每加载一片后重新输入查询就能命中。这是窗口化带来的真实功能收缩，不是渲染差异，记在这里以免被当成 bug 排查。（`search_refresh` 那段"重建后自动重算 hit 列表"是死代码，加载片不会自动补进当前查询的结果里。）
    - 验证：`EB_LOAD_TEST=1`（配 `EB_STATE` 指向 120 轮存档、`EB_LOAD_SHOT` 出图）连做两次"甩到顶"，每次要求恰好多 10 轮、`messages` 的子控件数等于渲染状态自报的挂载数、且锚定帧相对顶边的位置漂移 ≤4px。测量坑记一笔：`upper` 的增量不是可用不变量（reveal 之后还会漂 2px/帧量级），`Widget::translate_coordinates` 不随滚动值移动；能用的度量是 `frame.allocation().y - adjustment.value()`，即"视口顶边下那个内容像素"。

36. **窗口缩放后的三栏重排：`configure-event` 的返回值、消息列的居中方式，都是实测出来的坑** —— 报障"切换到某个任务时，屏幕只剩侧栏拉满整窗、没有会话区"。根因不在任务切换，而在窗口缩放：`shell::finish` 里挂的 `connect_configure_event` 返回了 `true`。configure-event 是 **RUN_LAST**，返回 `true` 会终止信号发射，也就跳过了 GTK 自己的 `gtk_window_configure_event` —— 那是 toplevel 记录新 frame 尺寸的唯一地方。于是窗口服务器把一块 **280px 的旧 surface 拉伸铺满 1470px 的 frame**：像素是侧栏的，几何是整窗的，看起来就是"侧栏占了全屏"。改两条：
    - **返回 `false`**，让默认处理器照常吃掉事件；我们的刷新只是顺带请求一次重排。
    - **比较事件里的宽度，不是 `window.size()`**。这个处理器跑在默认处理器**之前**，此时缓存的 `size()` 还是上一帧的值；拿"旧值 vs 旧值"判断"宽度变了没"会直接漏掉这次缩放，重排晚一个事件才发生。`gdk::EventConfigure::size().0` 才是新 frame 的宽度。

    同一个报障还牵着两个"窗口拉不小"的伴生缺陷：① 消息列 scroller 的横向策略原先是 `NEVER`，会把列的最小宽（实测 1158）**原样抬进窗口最小宽**，拖到那儿就再也拉不动；改成 `Automatic` 后 scroller 只申报自己的 `min_content_width`（1），列在 pane 里横向滚动。② 居中方式从"给 `messages` 加左右 margin"换成"由 scroller 居中"（`halign=CENTER` + `propagate_natural_width(true)` + `max_content_width=设计宽`）。原因：`GtkViewport` 给子控件的宽度是 `max(子最小宽 + 子 margin, 视口宽)`，而**换行后的中文段落的最小宽等于它上一次折断时的宽度**（`_gtk_text_layout_get_min_width` 只在空格处断行，一整段中文算一个"词"；`GtkTextView` 没有 `width-chars` 可以压住它）。margin 是在刷新里补设的，GTK 重排时用的还是**上一帧的 margin**，于是"变宽"方向会自我放大：实测一次 grow 后列停在 1220px 挂在 1389px 的 pane 里，再也回不来 —— 就是"转录画出 pane 边界"的样子。scroller 居中不吃 margin，grow/shrink 都稳定在 752。
    - **`max_content_width` 只会截断，不会撑起来**：artifact demo 的转录很短，自然宽只有 276，列就真的画成 276（`eb-messages 432,64 276x604`）。所以再给列本身一个宽度请求把自然宽钉到设计值：`messages.set_size_request(min(设计宽, pane 宽), -1)`。它**不会**把窗口最小宽抬起来 —— 列在可横向滚动的 scroller 里，scroller 申报的是自己的最小值，不是子的。
    - 顺带改掉一条过时断言：`selftest` 原先期望 `eb-messages == MESSAGE_WIDTH - 1`（margin 居中时 viewport 会少 1px 边框），现在列宽是精确的 500 / 752，期望改成定值。
    - 验证（`EB_RESIZE_TEST`，见 local-verification 配方 29）：800 → `toplevel=800 surface=800 sidebar=280 stack=534`；1200 → `toplevel=1200 surface=1200 stack=919 column=752`；grow 到 1270/1670 后列仍是 752（ratchet 消失）。截图核对：1200 宽时 752 的列正好居中、无裁切（`/tmp/eb-r1200-narrow.png`），面板打开时 500 的列在 580 的 pane 里左右各 40（`/tmp/eb-art500.png`）。
    - **已知残留（未修，代价记在这里）**：列宽超过 pane 时（800 窗口 → 751 vs 534）转录右侧被裁掉，横向滚动条是 overlay 的、静止时透明，所以看上去像"文字画到窗外"。harness 以 `note` 报出而不判失败，因为那是"这段转录已经在 752 上折断过一次"的 CJK 残留：要让它跟着变窄必须**重新折断**每个 `GtkTextView`（实测 `set_size_request` 只能抬高不能压低，`width-chars` 属性 `GtkTextView` 根本没有），而重折要在每次缩放步进里遍历整棵缓存的会话树，正是 §10-34/§10-35 花力气消掉的开销。当时的取舍是"要么窗口根本拉不小（就是这份报障的机制），要么窗口最小宽写成 sidebar+752，要么接受窄窗口的裁切"。新开在窄窗口的会话没有这个问题，只有从宽窗口缩下来的旧转录会这样。
    - **测量坑记一笔**：`steady` 只比"两次读数的 pane/列"不够。控件树可以在 frame 落地后的**一两拍里仍然拿着旧分配**（实测同一帧读出两次一模一样的 `stack=1159`），于是把中间态当成稳定态测，报出"侧栏拉满整窗"的假失败。现在要求两次读数一致 **且** pane 与 frame 相符（`|stack - (toplevel - 280)| <= 40`）才算落定；真的卡住仍会在 6s/10s 兜底处失败。





37. **转录滚动条从"消息列右缘"移到"pane 右缘"** —— 报障"WorkBuddy 的转录滚动条在窗口右侧，EvoBuddy 的和输入框右缘对齐，基本与消息里引用/代码框自己的滚动条平齐；一旦遇到文本框两者叠加，就滚不动"。成因：列是由 scroller 居中的（§10-36），而竖条长在 scroller 自己的右边缘，也就是 752 列的右边缘 —— 正是消息内 exec / diff / code 卡各自竖条所在的位置。两套条叠在同一列像素上，拖到的那个只滚自己，读者的直觉是"滚不动"。
    - 改法：scroller 的竖向策略 `Automatic → External`，条本身换成一个共享同一条 `vadjustment` 的独立 `GtkScrollbar`。**滚轮没有因为换策略而失效**：GTK3 的 `gtk_scrolled_window_scroll_event()` 走 `may_vscroll()`，它对 `GTK_POLICY_EXTERNAL` 返回真（`gtkscrolledwindow.c`，只有 `NEVER` 会关掉滚动）；`EB_LOAD_TEST` 的两次"甩到顶"仍然触发加载，就是在验这条路径。
    - 位置：**不能**把 scroller 直接当 `GtkOverlay` 的主子控件 —— 主子控件不 expand 时，overlay 子控件是按它的**请求宽**（752）摆的，条会落回列边缘（实测 overlay x=737 ⇒ 窗口 1018–1032，正是报障里那个位置）。所以在中间垫一层会 expand 的 `hbox`（只包 scroller），overlay 子控件于是按整条 pane 定位，`halign=END`。
    - 实测几何（1440×836，60 轮存档）：条 `1425–1439`（宽 14，贴 pane 右缘），转录列 `484–1236`（中心 860，与输入卡同轴 —— 条浮在空白 padding 上，没有把列挤偏），条与列之间 189px 全白、逐行扫描无内容压线。1470 宽时条 `1455–1469`，同样贴边；1000 宽（列已经溢出 pane 的 §10-36 残留态）条仍在 `985–999`。（§10-39 把那 14px 改成了透明槽 + 3px 滑块，滑块实测 `1433–1435`；"贴 pane 右缘"这一点没变。）
    - 短转录不挂死条：`vadjustment` 的 `changed` 里 `upper > page_size + 1` 才可见（实测单轮任务 `upper==page==848` ⇒ 条隐藏，右缘回到纯白）。
    - **自创部分**：产品那侧的转录条是 fade-out overlay，静止帧采不到几何（§10-21），所以"14px 常驻 trough"没有采集依据，只有"贴在窗口右缘"这一点是按用户对照产品的观察实现的。它比消息内卡片自己的条更显眼：独立 `GtkScrollbar` 不在任何 scroller 里，拿不到主题的 overlay 样式类，因此离屏快照里它是实心的（灰值 `#cecece`）；同一快照里消息内 8 个 exec/code 卡的条全部透明（按列宽扫描 `1018–1032` 与 `1200–1235` 两处都无非白像素），这个差异就是上面那条报障能出现的物理原因。（这个"14px 实心槽"的中间态后来被 §10-39 换掉了 —— 用户报"突兀、难看"；换掉的正是这条自创，不是采集值。）
    - **残留**：列铺满 pane 时（pane ≤ 约 766，即 §10-36 那个"列宽超过 pane"的窄窗态；开着结果面板则更早）条仍会压在列的右缘上。此时它在 z-order 的**最上层**（overlay 最后加、且不 pass-through），所以拖那 14px 滚的是转录而不是消息内的卡片 —— 报障的"滚不动"在这两种宽度下都不会再出现。
    - 验证：`EB_LOAD_TEST` / `EB_RESIZE_TEST` 配长会话存档（`docs/make_heavy_state.py 1 30`）出图，PIL 读右缘 15px；`column=752`、`stack=1159`、`drift=0px,0px` 三条不变量不变。

38. **空白 padding 也要能滚转录：事件窗口只覆盖列，catcher 必须是 `GtkEventBox` 并自己申报 expand** —— 报障"鼠标在空白区域，也应该能滚动对话消息"。成因不在滚动策略而在**事件窗口的覆盖范围**：列由 scroller 居中（§10-36），scroller 只有它那 752px 的输入窗口；1440 宽的 pane 里左右两侧各约 204/189px 的空白属于上层的 `GtkOverlay`/`GtkBox`，它们没有输入窗口，滚轮事件到了那儿就没人接 —— 指针必须压在文字上才翻页。
    - 改法：在 scroller 外面垫一层铺满 pane 的 `GtkEventBox`（`conversation.transcript_hits`），`connect_scroll_event` 里用 `scroll.event(&**event)` 把同一个事件原样交给 scroller，让 GTK 自己的 `gtk_scrolled_window_scroll_event()` 去算滚轮单位、smooth delta、kinetic 收尾、overshoot 与 clamp —— 不在这儿复制它那份数学。
    - **为什么不直接把 scroller 拉宽到整条 pane**：`GtkViewport` 给子控件的宽度是视口宽（§10-36），列会被撑到 1159；把列钉在 752 靠的是 scroller 的 `max_content_width`，前提是 scroller 自己就窄。
    - **`GtkEventBox` 没有 `compute_expand`**：expand 提示是从子控件推导出来的（`GtkBox` 会、`GtkEventBox` 不会），所以这层必须显式 `set_hexpand(true)` + `set_vexpand(true)`。漏掉的话提示就断在这里，上面一串（`GtkOverlay` 的主子控件 → stack → paned）把转录读成一条 752 宽、不 expand 的带子，overlay 子控件重新按请求宽定位 —— 结果就是 §10-37 刚修掉的那个"条落回列边缘"。垫的这层包住的是 §10-37 那层 expand 的 `hbox`，转发仍在最外层。
    - 覆盖范围：**光有 catcher 不够，catcher 的 GdkWindow 必须自己申报滚轮掩码**（§10-38 的原始实现就是漏在这一步，用户复测"margin 还是滚不动"）。`GtkEventBox` realize 时只 `attributes.event_mask = gtk_widget_get_events(w) | GDK_EXPOSURE_MASK`，加上按钮与进出掩码，实测它的窗口掩码是 `EXPOSURE | BUTTON_MOTION | BUTTON_PRESS | BUTTON_RELEASE | ENTER_NOTIFY | LEAVE_NOTIFY` —— **没有** `SCROLL_MASK/SMOOTH_SCROLL_MASK`。GDK 在 `gdk_event_translate` 里按窗口掩码就把事件丢了，控件链根本不会启动，所以 `connect_scroll_event` 写得再对也是死代码。修法：`add_events(SCROLL_EVENTS)`（`GtkScrolledWindow` 自己也是取这两位）。**harness 为什么没抓到**：`gtk::propagate_event` 是直接递给控件的，绕过了窗口那一层过滤 —— 凡是"用 propagate 模拟真实输入"的 harness 都有这个盲区，所以 `EB_PAD_SCROLL` 现在先断言 `hits.window().events()` 含这两位，再谈位移。
    - 覆盖范围到整页：转录那一段由 `transcript_hits`（overlay 的主子控件，兼作 §10-37 的 expand 提示）接，composer 那一段另有一个铺满 `body` 的 `page_hits`。输入卡是居中放在远宽于它的 pane 里的，左右两条空白同样没人接滚轮；只垫转录那一段，用户从输入框边上滚就还是没反应。自己有窗口的控件（列的 scroller、输入框的 textview）先吃掉事件，catcher 只接没人要的。顶栏那 64px 不在 `body` 里，不参与。
    - z-order 上 catcher 在 fade 遮罩与转录条之下：那两处仍各自处理自己的事件，遮罩本来 pass-through。
    - 验证 `EB_PAD_SCROLL=1`（配长会话存档）：先断言两个 catcher 的窗口掩码，再上滚 400（pane catcher）、下滚 250（同一个）、上滚 300（**page catcher**，即 composer 那一段），逐步断言 `vadjustment` 的位移，实测 `pad-scroll: ok the pane scrolls 18376 -> 17976 -> 18226 -> 17926 of 18980`；中途还断言转录条已经带上 `eb-bar-active`（§10-39），否则"条变粗"这件事只能靠人眼。断言只咬位移（±8px），绝对 `upper` 不必复现：同一份 2×12 轮存档两次跑分别读到 18980 与 22140，差别只在测量那一刻惰性 attach 已经挂了多少轮，转发路径本身与挂载量无关。两条负向对照都是必要的：把转发摘掉 ⇒ `FAIL … moved the transcript 0px, not -400px`；把 `add_events` 注释掉 ⇒ `FAIL the pane catcher's window selects EventMask(EXPOSURE | BUTTON_* | ENTER/LEAVE), which is missing SCROLL_MASK/SMOOTH_SCROLL_MASK`（实测过，掩码断言不是摆设）。几何不变量另跑复核：`EB_RESIZE_TEST=1440 EB_RESIZE_TASK=1` 给 `column=752`、`stack=1159`，`EB_LOAD_TEST` 给 `drift=0px,0px`。
    - **自创部分**：产品采集里没有"指针停在 padding 上滚滚轮"这一帧，也没有可对照的条行为；"空白处也能滚"是按用户对照 WorkBuddy 的观察实现的，属交互补齐而非像素比对。
    - **调查记录（免得再走一遍）**：这条改动一度被 `EB_RESIZE_TEST` 的 grow 失败拖累，当时假设是"composer 的 margin 把会话最小宽顶到 pane 之外、configure 被当成 undersized 丢掉"，还为它写过 margin 钳制 + `size-allocate` 重绘钩子。用真实的 harness 机制复测后 product 代码原样回退（`refresh` 里仍是从 `pane_width` 算 `input_margin`），grow 仍 3/3 通过 —— 假设不成立。真正的原因记在下面这条 harness 坑里。
    - **harness 坑：`surface.resize()` 只让 GTK *知道*新 frame，不会让树重排**。把 frame 变成新分配的是 `gtk_window_process_updates`，而它挂在 GDK frame clock 上；`cargo run` 起的进程不是活动应用，clock 不 tick，于是 `window.size()` 动了、`window.allocation()` 不动，整棵树停在旧分配上，harness 兜底等到 10s 仍读到 `stack=1159`（假失败）。`gtk::WindowExt::process_updates` 在 gtk-rs 0.19 / gtk-sys 里没有绑定，所以配方 29 现在每个 tick 比较 `size()` 与 `allocation()`，不一致就自己 `size_allocate()` 一次 —— 真实拖拽下一帧做的就是这件事。也试过三条路，都不成立：`window.resize()`（macOS 干脆不回 configure）、在 tick 里 pump `MainContext`（没有 pending 源，pump 了个空）、只测一次 `window.preferred_height()`（靠 measure 的副作用碰运气，时序不稳，只有 3/3 显式分配是确定的）。
    - 另一处 harness 事实：缩放 harness 必须**配长会话存档**（`EB_STATE=<tasks.json>`）才有意义。不带存档时 `EB_RESIZE_TASK` 走"自己新建三个短任务"的分支，转录没有可滚内容，`upper==page` ⇒ 条隐藏、滚轮断言也失去对象。

39. **右缘转录条改成细滑块** —— 报障"右侧滚动条比较突兀、难看"。成因：§10-37 那根条是独立的 `GtkScrollbar`，不在任何 scroller 里，拿不到主题的 overlay 样式，于是整条 14px 都是实心槽（离屏快照右缘逐行扫描 `1426–1439` 全是 `#cecece`，只有滑块那 38 行深一点）。读者看到的不是"一条滚动条"，是窗体右缘一条洗不掉的灰带。
    - 改法（`ui/gtk.css` 的 `.eb-transcript-bar`）：槽与控件自身背景清空、去掉边框/阴影/步进按钮，滑块收成 3px 圆头、`alpha(#000,.13)`、右边留 4px 内缩（实测滑块像素 `1433–1435`，其余全白）；hover 或滚动时升到 5px / `alpha(#000,.26)`，拖动中 `:active` 再到 `alpha(#000,.38)`。控件本身仍占 12px 宽，所以那 12px 还是可以点、可以拖 —— 变细只是不画槽。
    - **"滚动时变粗"是 `vadjustment` 的 `value-changed` 加 `eb-bar-active` 类、900ms 一次性定时器摘掉**。两处刻意的省法：类已经在了就不重复 `add_class`（重复加同名类会让节点样式失效并排一次重绘，流式回答是每个 token 一次 `value-changed`）；每次改动都 `remove()` 旧源再挂新的（微秒级），否则连续滚动会在开始 900ms 后自己先暗下去。这条路径由 `EB_PAD_SCROLL` 的掩码断言之后顺带验一次（滚完立刻查 `has_class`）。
    - **CSS 坑：`>` 在这里不成立**。第一版写的是 `scrollbar.eb-transcript-bar > trough > slider`，结果控件自身那条规则生效（灰带变白）、滑块完全没变（还是主题的 `#7e7e7e`、10px 宽）—— GTK 在 `scrollbar` 与 `trough` 之间还夹了节点，主题的 CSS 用的是后代写法所以不受影响。改成空格后代选择器（`scrollbar.eb-transcript-bar slider`）后实测生效。判据别看错：`background-color` 是不是 `alpha(#000,.13)` 折算的 `#dedede`（222），而不是主题的 127。
    - **自创部分**：12px 槽宽、3/5px 滑块、`.13/.26/.38` 三档灰、900ms 回落全是自创。产品那侧的条是 fade-out overlay，静止帧采不到几何与颜色（§10-21），只有"贴在窗口右缘"（§10-37）有采集依据；这一条是把"常驻 14px 实心槽"这个没有采集依据的中间态改回接近 overlay 观感，方向按用户对照产品的观察。
    - 验证：`EB_LOAD_TEST=1 EB_LOAD_SHOT=…` 出图 + PIL 扫右缘 20px（静止态实测只有 `1433–1435` 三列非白，值 222；`1426–1432`、`1436–1439` 全白），`EB_PAD_SCROLL` 验 `eb-bar-active`；`column=752`/`stack=1159`/`drift=0px,0px` 三条不变量不变，`EB_ASSERT`（含 `EB_ARTIFACT=1`）与 33 步点击审计同批通过。

**下一步**

规划统一维护在 [`docs/status-and-roadmap.md`](../status-and-roadmap.md)：P0 验证加固、M2（真实异步 agent + 会话内容块）、P1–P5 与发布合规闸门都在那里。本节只保留测量、决策、缺口与坑，避免两处计划漂移。


