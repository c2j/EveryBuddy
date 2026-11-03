# Artifact 预览迁移 CodeMirror 6 —— 设计文档

状态：v2 待评审 · 提案于 v0.7.36 之后 · v2 变更：补 Windows 宿主方案、
统一三平台消息桥矩阵

## 0. 背景与问题

v0.7.34 起点击会话中的 `Read …` 工具行可在 artifact 面板预览文件
（0.7.36 修通了端到端链路）。但当前预览是自绘 `GtkTextView` 实现
（`crates/eb/src/detail.rs` 的 `code()`），体验与工程量都到顶了：

| 问题 | 现状 |
| --- | --- |
| 复制带行号 | 行号在文本缓冲区内（单 buffer 方案），选中即带上，复制粘贴被污染 |
| 行号对齐 | 需要手工保证单 buffer 行盒一致；CJK 字体回退曾导致逐行漂移（v0.7.36 修复） |
| 语法高亮 | 无；md/rs/json/toml 等一律等宽黑白 |
| 多文件 | 状态 `tool_preview` 只支持单文件；面板标签条为原生实现，功能有限 |

调研结论（2026-11-03）：

- `sourceview`（GtkSourceView 3 绑定）停在 0.9.0 / 2020，且不匹配本项目
  gtk 0.19 栈；`sourceview4` 0.5.0 绑定 gtk ^0.18，与本仓库 gtk 0.19 /
  glib 0.22 类型系统不互通。**GTK3 侧没有可用的现成绑定**。
- `syntect` 只是高亮引擎，行号/选中/复制仍需自管，上限即当前手搓水平。
- 本项目已有 WebView 基建：`crates/eb/src/preview_server.rs`
  （`127.0.0.1:39473` 静态服务）与 `crates/eb/src/theme_view.rs`
  （macOS WKWebView 面板 + GTK 锚点 + 主题桥）。

据此选定 **WebView + CodeMirror 6** 方案：页面与交互逻辑三平台一套，
各平台只差 WebView 的宿主方式（§1）与消息桥运输层（§2.3）。

## 1. 目标平台与宿主方案

| 平台 | 引擎 | 宿主方式 | 新增依赖 |
| --- | --- | --- | --- |
| macOS arm64/x86_64 | WKWebView（系统自带） | 原生 NSPanel + GTK 锚点（复用 `theme_view.rs` 已验证模式） | 无 |
| Windows x86_64 | WebView2（Edge，Win10/11 预装） | **`wry` crate 进程内嵌**：WebView2 控件 `SetParent` 到 GTK 部件的原生 HWND | `wry`（Windows-only target）；WebView2 Runtime（系统预装） |
| Linux（Kylin V10 arm64，实机核查） | WebKitGTK **2.28.1**，仅 `webkit2gtk-4.0`（libsoup2） | **C 辅助进程 + XEmbed 内嵌**（§1.2），应用与 helper 统一 `GDK_BACKEND=x11` | 自研 `preview-helper`（C，链接系统 `libwebkit2gtk-4.0`） |

三平台均不改动现有 gtk 0.19 栈；Linux 不引入 webkit2gtk Rust 绑定
（不可用，见 §1.1）；Windows 的 `wry` 后端不经 gtk-rs，无版本冲突。

### 1.1 绑定兼容性审计（2026-11-03，决定性结论）

审核发现 **Linux 侧不存在可用的 webview Rust 绑定**：

- `webkit2gtk` 绑定现由 Tauri 团队维护（2.0.2，2025-12，活跃），但依赖
  **gtk ^0.18 / glib ^0.18**（与本仓库 gtk 0.19 / glib 0.22 类型系统不
  互通），且依赖 **soup3** —— 即只绑定 `webkit2gtk-4.1` API；目标机
  Kylin 仅有 `webkit2gtk-4.0`（libsoup2），**即便 gtk 版本对上也无法
  在目标机运行**。
- 旧 gtk-rs 系 `webkit2gtk` 0.19.x（2023-01）对应 gtk 0.16 栈，同样
  不匹配；特性只到 v2_38。
- 与 GTK3 侧 `sourceview` 绑定（3 停更于 2020 / 4 卡在 gtk 0.18）是
  同一类问题：gtk3-rs 0.19 栈没有配套的卫星绑定。

### 1.2 Linux 落地方案：C 辅助进程 + XEmbed 内嵌（已核实可行）

绑定缺位只影响"从 Rust 调用"，不影响 C 层：目标机系统自带
`libwebkit2gtk-4.0`（2.28.1，已核实）。且目标机 Wayland 会话下
**XWayland 在运行**（`pgrep -a Xwayland` → pid 4567，display `:0`），
因此：

- 新增**轻量 C 辅助进程** `preview-helper`（约 200 行，Linux-only）：
  创建 `GtkPlug` + `WebKitWebView`，加载
  `http://127.0.0.1:39473/preview`，把 X11 窗口 ID 打到 stdout。
  只依赖系统 `-lwebkit2gtk-4.0`，随构建用 pkg-config 编译。
- **Rust 侧**（Linux）：启动时对应用与 helper 统一设置
  `GDK_BACKEND=x11`（两者都作为 XWayland 里的 X11 客户端运行），
  用 `GtkSocket::add_id(plug_id)` 把 helper 窗口内嵌进 detail 面板。
- **约束与回退**：
  - 依赖 XWayland 存在；helper 检测不到 X display 时退出，
    Rust 侧自动回退原生 `code()` 预览并提示；
  - 嵌入窗口的键盘焦点、DPI、IME 需真机验证（§3 步骤 ⑤）；
  - helper 崩溃即退出进程，面板负责检测 `EOF`/退出码并重启或回退。

### 1.3 Windows 落地方案：`wry`（WebView2）进程内嵌

- **引擎**：WebView2 是微软官方 Evergreen 分发，Win10/11 系统预装；
  `wry` 的 Windows 后端 = `webview2-com` + `windows` crate，
  **不依赖 gtk-rs**，与本项目 gtk 0.19 栈零冲突。
- **内嵌**：GTK3 在 Windows 上同为 HWND 体系——在 detail 面板放一个
  占位 `GtkDrawingArea`，取其 GdkWindow 的原生句柄
  （`gdk_win32_window_get_handle`），把 WebView2 控件的 HWND
  `SetParent` 过去；GTK allocation 变化时同步 WebView2 边界
  （`set_bounds`），焦点切换经 `wry::WebView::focus()`。
- **消息桥**：`wry` 自带 `evaluate_script`（Rust→JS）与 IPC handler
  （JS→Rust），走 §2.3 的统一协议。
- **回退**：运行时检测 WebView2 Runtime 缺失（注册表 /
  `GetAvailableCoreWebView2BrowserVersionString`）→ 提示安装
  Evergreen Bootstrapper，或回退原生 `code()`。
- **前提**：仓库路线图标注 Windows 为"无验证环境"——步骤 ⑥ 开工前
  需先落实一台 Windows 验证机（Win10/11 x64）。

## 2. 总体架构

```
┌─ GTK app ──────────────────────────────────┐
│ shell.rs: PreviewDocs 状态（多文件栈）       │
│    │ 打开/追加/切换/关闭（消息桥，双向 JSON） │
│    ▼                                        │
│ ┌─ WebView 预览面板（三平台一套页面）──────┐ │
│ │ index.html + bundle.js（CM6，离线打包）│  │
│ │  · 文件标签条（HTML/CSS，跟随主题）      │  │
│ │  · 单个 CodeMirror EditorView          │  │
│ │    行号 gutter（复制不含）/ 语言高亮 /   │  │
│ │    搜索面板 / 折行切换 / 当前行 / 只读   │  │
│ └───────────────┬────────────────────────┘  │
└─────────────────┼───────────────────────────┘
     preview_server :39473（静态资产 + 按需文件内容）

宿主：macOS = NSPanel + WKWebView（锚点跟随）
      Windows = wry/WebView2 嵌入面板 HWND
      Linux = preview-helper（C）+ GtkSocket/XEmbed
```

### 2.1 前端资产（全离线，三平台同一份）

- 新目录 `experiments/preview-webview/`：源码（TS/JS）+ esbuild 打包为
  **单 `bundle.js` + `index.html`**，以 `include_bytes!` 进二进制
  （与现有 `host.js` 同模式）。运行期零网络、零 CDN。
- 构建约束：esbuild `target: ['es2018']`；不使用 OffscreenCanvas、
  WebLocks 等 WebKitGTK 2.28 缺失的 API。
- 功能清单：
  - 行号 = CM 原生 gutter（不属于文档缓冲，选中/复制天然不含行号）；
  - 多文件标签条：单 `EditorView` 按 tab 切换 `EditorState`，每文档保留
    独立滚动位置；
  - 语言映射（按扩展名）：`@codemirror/lang-markdown`、`-rust`、`-json`、
    `-python`、`-javascript/-html/-css`；`toml` 用 `@codemirror/legacy-modes`
    的 toml mode；未识别扩展为纯文本；
  - `@codemirror/search` 搜索面板、折行切换、当前行高亮；
  - 大文件：CM6 原生虚拟滚动；> 5 MiB 截断并提示（预览读取上限从
    512 KiB 放宽到 5 MiB，经 HTTP 按需拉取，不阻塞 UI 线程）；
  - 只读起步；编辑能力留 feature flag（默认关）。
- 主题：亮/暗变量由 Rust 侧推送（复用 `ui-theme-webview` 的
  host.js → client.js 消息模式），页面 CSS 只用变量。

### 2.2 状态模型（Rust 侧，三平台共享）

`model.rs` 中 `tool_preview: Option<Artifact>` + `tool_preview_task`
改为 `preview_docs: PreviewDocs { task: Option<usize>,
files: Vec<Artifact>, active: usize }`。打开来源（Read 行、artifact 卡）
统一入栈去重；现有清空规则不变（关面板、切任务、真实写入文件到达、
手动关闭胜出）。

### 2.3 消息桥（协议统一，运输层分平台）

协议（平台无关）：

- Rust → JS：`open-docs {files:[{id,name,lang,url}],active}`、
  `activate {id}`、`theme {dark}`；
- JS → Rust：`close`、`ready`（页面加载完成，Rust 收到后才推首个文档）；
- 大文件不整包推送：页面按 `url`（`/file?id=…`）向 preview_server 拉取，
  server 内存缓存当前任务文件表，按 id 校验后回 512 KiB～5 MiB 内容。

运输层：

| 方向 | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Rust → JS | `WKUserContentController`（scriptMessage 风格注入） | `wry::evaluate_script` | `evaluate_script` 直达（④b 不需要：helper 起始 URL 带 `/preview?task=…`，文档更新靠页面拉取 + `activate` 经 server SSE/轮询） |
| JS → Rust | `scriptMessageHandler` | wry IPC handler | HTTP `POST /event` 到 preview_server |

（Linux 的桥以 server 为中介：页面 POST 事件、Rust 侧 server 回调；
避免 helper 进程与主进程另开 IPC。首期页面只读，事件面很小：`ready`
与 `close`。）

### 2.4 回退开关

`EB_PREVIEW=native` 时走旧 `code()` 路径（迁移期保留，回退最终随本
设计收尾移除）。各平台的自动回退（§1.2 / §1.3）共用此路径。

### 2.5 依赖与打包影响

- npm devDependencies：`codemirror`、`@codemirror/*`、`esbuild`
  （仅构建期；产物进仓库，运行期无 Node）。
- Cargo（按 target 分置，主栈零改动）：
  - `[target.'cfg(target_os = "windows")'.dependencies]`：`wry`
    （Windows-only）；
  - macOS / Linux：无新增。
- 打包：
  - macOS：无新增；
  - Windows：依赖系统 WebView2 Runtime（预装）；缺失时提示装
    Evergreen Bootstrapper；
  - Linux：新增**自研** `preview-helper` 二进制（C，链接系统
    `libwebkit2gtk-4.0` ≥ 2.28，pkg-config 构建期检查）。

## 3. 实施步骤（每步独立可验收）

| # | 内容 | 验收标准 |
| --- | --- | --- |
| ① | `preview-webview/` 打包：标签条 + 行号 + md 高亮 + 只读 + 搜索；server 加 `/preview`、`/file`、`/event` 路由 | 桌面浏览器（Chrome/Safari）打开页面全功能可用 |
| ② | shell 状态改 `PreviewDocs`；Read 行 / artifact 卡统一入栈 | 多文件页签；清空规则回归（关面板/切任务/新写入） |
| ③ | macOS：`preview_panel.rs`（NSPanel + 锚点 + WKWebView）+ 消息桥 | app 内点 Read 行 → 面板打开；多文件切换、主题跟随、ESC 关闭 |
| ④ | Linux：`preview-helper`（C）+ `GtkSocket` 内嵌 + `GDK_BACKEND=x11` | Kylin arm64 真机：预览内嵌面板；行号、md 高亮、搜索、中文、HiDPI、键盘焦点全部可用；无 X display 时回退原生 |
| ⑤ | Windows：`wry` 内嵌 + HWND 父子关系 + 运行时检测 | Windows 验证机：同 ③ 的功能面；WebView2 缺失时给出安装引导或回退 |
| ⑥ | 清理：`code()` 转回退路径；`eb-code-gutter` 遗留 CSS 删除；打包清单更新；文档更新 | 三平台回归 + `EB_PREVIEW=native` 回退可用 |

顺序说明：状态模型（②）先于两个平台接入，使 ③④⑤ 只做宿主与桥；
Windows（⑤）受验证机约束可并行后置。

## 4. 风险与对策

| 风险 | 对策 |
| --- | --- |
| macOS 浮层面板与主窗口移动/缩放不同步 | 复用 `theme_view.rs` 的 anchor + monitor 代码；面板随锚点 widget 的 allocation 重算 |
| Linux 强制 `GDK_BACKEND=x11` 改变整个应用的显示路径 | Kylin 生态常态（XWayland 已核实存在）；真机回归主流程 UI；若个别组件异常，可仅对 helper 用 X11、面板降级原生预览 |
| XEmbed 内嵌窗口焦点/DPI/IME（2.28 老引擎 + arm64） | 步骤 ④ 真机验收项；helper 侧处理 `plug-removed`，Rust 侧捕获 helper 退出自动回退原生预览 |
| helper 进程生命周期 | 面板关闭/任务切换即终止 helper；崩溃检测 → 自动重启一次，再失败回退原生并提示 |
| WebKitGTK 2.28 老引擎 | es2018 目标；禁用新 API；§3 步骤 ④ 真机先行验证 |
| Windows 无验证环境（路线图现状） | 步骤 ⑤ 前置条件：落实 Win10/11 x64 验证机；WebView2 Runtime 缺失走安装引导或回退 |
| Windows HWND 嵌套的 DPI/焦点切换 | `set_bounds` 跟随 GTK allocation；WM_DPICHANGED 场景在步骤 ⑤ 验收清单 |
| 三平台桥实现漂移 | 协议（§2.3）以一份 JSON schema 定义，三端按同一 schema 收发；页面端做协议自检日志 |
| 主题双份维护（GTK CSS + 页面变量） | 变量名对齐 `ui-theme-webview` 现有契约；推送时一次到位 |
| CM6 打包体积 | 按语言包按需引入（预计 bundle < 1 MiB gzip 前）；产物进仓库受评审约束 |

## 5. 非目标

- 不做应用内编辑保存（只读预览；编辑能力仅留 flag，不在本期实现）。
- 不迁移 markdown 渲染视图（现有 md 渲染块是会话消息的一部分，与
  文件预览无关）。
- 不在本期升级 Linux 的 WebKitGTK（仓库锁死 2.28.1，按它写前端）。
- 不引入 `webkit2gtk`/`sourceview` Rust 绑定（绑定栈不兼容，见 §1.1；
  将来迁移 GTK4 时由 `sourceview5`/`wry` 原生体系自然替换）。
