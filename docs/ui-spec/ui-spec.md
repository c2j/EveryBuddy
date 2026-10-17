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

**本程序已实测**：`cargo build --release --bin evobuddy` 在 `debian:10` + GTK 3.24.5 上零错误（gtk-rs 0.19，58 秒），打包成的 74MB / 72 `.so` 目录树在**另一个裸机上**用 `EB_SELFTEST=1 EB_SHOT=` 跑通：`css: parsed 72598 bytes`、`window 1440x868 (want 1440x868)`、`sash sidebar x=280`，几何逐项与 macOS 一致（`eb-sidebar 280x868`、`eb-content-topbar 281,0 1159x64`、`eb-composer-card 460,333 800x142`、`eb-detail <hidden>`）。渲染出的 PNG 采样为 `#f7f7f7` 侧栏 / `#ffffff` 路由，与 §4.3 实测值一致；与 macOS 的唯一差异是**文本宽度**（hero 151 vs 137、声明 330 vs 311），因为字体换成了自带的 DejaVu + WenQuanYi——这是"字体必须换"的必然结果（缺口 7）。

**下限没有余量**：整套产物要求的最小符号版本 = `GLIBC_2.28`，正好等于要求的底线（C 探针只需 2.25，多出来的来自 Rust std）。即"2.28 及以上可跑、以下不行"；若要下探到更老的发行版，需要 Rust 侧更低的 target baseline。

打包脚本已进仓库 `packaging/linux/`（`package.sh` 构建+打包、`verify.sh` 裸机验证，细节见该目录 README）。

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
2. **顶栏只有两个操作按钮**（搜索、分享），此前按 3 个实现（多了一个结果面板开关）属于**虚构，已删除**。重新打开已关闭的结果面板改由点击侧栏任务行完成（`open_task` 同时把 `detail_visible` 置为 `artifact.is_some()`）。
3. **分段控件**：`wb-scene-tabs` 163.74×36 @778.1,185，`wb-segmented` 159.74×32，滑块 `wb-segmented__thumb--measured` 79×32，选中类 `wb-segmented__item--active`——全部与 §4.3 的实现尺寸一致。

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
- 实测校验通过（欢迎态）：`teams-container` 1440×868、侧栏 280、居中列 800、分段控件高 36、chip 行高 32、输入卡 800×142、选择器行高 36；hero / 分段 / chip / 输入卡的 y（127 / 185 / 285 / 333）与产品**逐项一致**；8 个关键色解析值与实测完全吻合。
- 实测校验通过（会话态）：会话顶栏 64 高、**操作组 68×32 @776,16（逐像素一致）**、消息列表 579×636、输入区 579×168、输入卡 531×132、**结果面板 861,0 579×868（一致）**、**结果面板头部 579×64（一致）**、标签页 240×32 @877,15.5（本轮修正后一致）、加号 1121、头部操作 68×32 @1356（一致）。
- **交互逐控件审计**（`EB_CLICK=1`，26 步「点一下 → 读回状态」，每步打印可观测变化）：导航、chip、分段开关、侧栏折叠 / 恢复、Enter 发送、发送按钮、结果面板展开 / 恢复 / 关闭、任务行重开面板、复制按钮（`Copied` 反馈）**全部有响应**。审计顺带查出并修掉三个**真实缺陷**（产品侧同样会看到的问题，不是测试环境问题）：
  1. **`set_no_show_all(true)` 必须放在 `show_all()` 之后**。`gtk_widget_show_all()` 对 `no_show_all` 控件**直接 return，也不向下递归**，所以此前所有下拉 popover 的内容、以及所有"隐藏待显"按钮（侧栏展开 `▶`、Code assist）的**子控件从未被 show 过**：下拉弹出来是个**空盒子**（requisition 恒为 66×42），按钮是个**空方块**。修法：新增 `widgets::hide_from_show_all()`（先 `show_all()`，再置 `no_show_all` + `set_visible(false)`），7 处调用点全部改掉。修后 popover requisition 回到按内容计算（筛选菜单 93×114、模型菜单 71×114、提示 294–324×48–80），审计打印 `3 of 3 menu rows visible`；离屏渲染 `menu.png` 里能直接看到模型下拉「Auto / Fast / Deep」。
  2. **侧栏分区标题点了没反应** —— 现在接真动作：折叠 / 展开（状态在 `App.collapsed`；因为 `show_all` 会把两个分区都显示出来，折叠只能在 `show_all()` **之后**施加），chevron `⌄` / `›` 随之切换。审计打印 `shown → hidden → shown` 的可见性链。
  3. **结果面板代码区横向被裁且无处可滚** —— 非换行 `GtkTextView` 上报的是"已排版行的宽度"，而它永远不会超过被分配到的宽度，请求宽度于是收敛到分配宽度、长行被裁掉。修法：用 Pango 量出最长行并 `set_size_request`，滚动区 `hadjustment` 由 `579/579` 变为 `579/646`（`upper > page_size`）⇒ 可横向滚动。

**缺口**

1. **吉祥物未实现** —— `wb-home-route__growth-buddy`（120×120，居于输入区右上）。产品用的是自有图片资产，不得复用，需要自绘或换图。
2. **图标全是字形占位** —— 产品图标是其自有资产，不得复用。另外 **macOS Homebrew 的 gtk+3 只装了 `hicolor`，没有 Adwaita 图标主题**，所以 `GtkImage::from_icon_name()` 在本机会渲染成空；要支持多平台必须随包附带一套自有图标（如 MIT 许可图标集）。
3. **GTK 3 的 `box-shadow` 会被裁剪到控件内** —— 输入卡的 `0 16px 32px -8px rgba(0,0,0,.03)` 在产品里会铺到下方选择器行背后（这正是那里看起来发灰的原因），GTK 侧只能画出卡片自身范围内的部分。要完全还原需要自绘（Cairo `draw` 信号）或在外层留出 padding。
4. **输入卡高/宽差 2px / 1px**（欢迎态实测 800×144，GTK 800×142；会话态实测 532×134，GTK 531×132）—— 2px 来源未查清（疑似 `cr-resize-handle` 或 1px 边框）。当前忽略。
5. **文案是占位** —— 产品文案（品牌标语、空态、AI 声明、输入框提示）未复制，各模块顶部的文本常量用自有措辞替代；需替换为 EvoBuddy 自己的文案。
6. **间距 / 圆角 / 字号未纳入代码生成** —— `ui/gtk.css` 里目前是手写字面量（带实测注释），应一并由生成器产出。
7. **UI 字族未在 token 层定义** —— 由基础 CSS 直接声明；产品的 `"Segoe UI (Custom)"` / `"Open Sans (Custom)"` 是改名后的自包含字体，**不得打包**，必须换自有字体并重测字宽（会影响上述所有文本宽度）。
8. **i18n 文案清单**未提取（`zh-cn-*.js` / `zh-tw-*.js` 分片，可作为屏幕/状态枚举的权威来源）。
9. **z-index 层级表**未提取。
10. **工具调用只做了折叠态** —— 产品还有 `cr-collapse` 展开卡（浅绿 `#e7f9ef`）、`cr-tool-diff` 差异视图（`+42 / −0` 行级配色）、`cr-tool-exec` 命令输出、`cr-table-block` 表格块、`cr-status-chips`、`cr-context-usage` 用量环。当前实现把结果面板正文渲染成单块静音文本、工具行只保留折叠行，属**记录在案的偏差**。
11. **Linux 完全自包含：已验证** —— 见 §2.1：C 探针与本程序都在裸 glibc 2.28 上跑通，自带 GTK 3.24.5 + schema + 图标 + 字体，几何与 macOS 一致（唯一差异是文本宽度）。脚本在 `packaging/linux/`。剩余：产物命名 / desktop entry / 图标（当前是个目录而不是能给用户的东西），以及 ibus 输入法（缺口 22）。
12. **CI 仍然只打包 `notebook`** —— `.github/scripts/build-{linux,windows}.sh` 里写死了 `target/release/notebook`，主程序 `evobuddy` 不会被产出。等壳能替代 demo 后，两者需要一起切换（包括去掉 `notebook` bin）。另外：**Linux 产物必须在 glibc 2.28 的镜像里构建**（GitHub 的 ubuntu-latest runner 会带来过新的符号版本，产物在目标机上起不来），且 CI 需要跑打包与"裸机启动"测试，而不只是 `cargo build`。
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
19. **"本构建未实现"提示 popover 是自创 chrome** —— 侧栏搜索 / 小程序、会话顶栏搜索 / 分享、结果面板新建标签、Code assist 这 6 个控件，产品的行为都没采集到（而且产品里全局搜索是**独立窗口** `global-search` 800×640，不是本栏内的东西）。实现侧的选择是：点击弹一个 `eb-notice` popover 明说缺什么，而不是静默无反应——静默会被读成"坏了"（这正是本轮用户反馈"部分能点，部分失效"的来源之一）。这些 popover、`eb-missing-label` 样式与文案都属自创，记录备案。
20. **结果面板标签体与分区的折叠行为也属实现侧选择** —— 标签体不再是按钮（唯一的标签就是当前标签，点它没有动作），标签里的 `×` 变成真的关闭按钮 `eb-tab-close`（14×14，靠 `button.eb-tab` → `.eb-tab` 的类名改动保持几何不变）；`eb-section-label` 的折叠是照着我写的 tooltip 实现的，产品实测里没抓到过这类点击。
21. **popover 与 overlay 滚动条在"无头 / 非活跃窗口"环境下的呈现无法完全验证** —— 审计环境没有窗管激活窗口（`window active=false`，还伴随 `Failed to initialize CVDisplayLink`），下拉在修好缺陷 1 后能按内容 sizing、也能在离屏渲染里看到（`menu.png`），但**真实活跃窗口下的呈现与焦点**未验证。同理代码区的横向滚动条在 macOS 上是 GTK 的 **overlay 指示条，静止时透明**，离屏截图看不到，只能用 `hadjustment` 的 `upper > page_size`（579/646）证明可滚动。
22. **自带 GTK 栈下的 CJK 输入法** —— Debian 10 的默认 immodules 只有 `im-xim` / `im-cedilla` / `im-ipa` 等，**没有 `im-ibus`**（它随 `ibus` 包分发），而 Linux 桌面上的中文输入绝大多数走 ibus。要在自带栈里打中文，得把 `im-ibus.so` 与 `libibus-1.0.so.0` 一并打包（并保证 `GTK_IM_MODULE=ibus` 时能找到），否则只剩已自带的 XIM。目前两者都未做，探针不带输入法。
23. **Windows 7 / 8.1 未验证，且文档自相矛盾** —— 硬要求里有 Win7/8.1，而 §2 表原先写"GTK 3（MSYS2 构建）无 Win7 门槛"，README「Prebuilt binaries」却写"MSYS2 自 2024-05 起不再支持 Win7/8.0，因此没有可维护的 GTK 3 运行时"。二者必有一错。要分清**构建环境**与**目标机**：Cygwin/msys2-runtime 3.5 不支持 Win7 只影响构建侧；MinGW-w64 产物理论上仍可声明 `_WIN32_WINNT` 兼容 Win7，但新版 GLib 可能已调用 Win8+ API（如 `GetSystemTimePreciseAsFileTime`），这只能真机实测。需要一台 Win7/8.1 虚拟机才能定论，目前无法验证。

**下一步（建议顺序）**

1. 在真实活跃窗口下人工复核下拉 popover 的呈现、焦点与点外关闭（缺口 21）。
2. 补上工具调用展开卡、差异视图、表格块与 `cr-context-usage` 用量环（缺口 10）。
3. 人工复核 IME 组合态下的 Enter 行为（缺口 18，自动化无法提供输入法）。
4. 补上吉祥物、图标集与真实文案，把欢迎态收尾（缺口 1、2、5）。
5. 解决输入卡投影：用 Cairo `draw` 自绘阴影，或确认 GTK 能否通过外层留白渲染出卡片外投影（缺口 3）。
6. 修正结果面板标签的超长省略（缺口 16），并核对会话顶栏那 8px 内缩（缺口 14）。
7. 把间距 / 圆角 / 字号纳入生成器，`ui/gtk.css` 只留结构规则（缺口 6）。
8. Debian 10 容器验证自带 GTK 栈的可行性（缺口 11）。
9. 切换 CI 到 `evobuddy` 并去掉 `notebook` bin（缺口 12）。


