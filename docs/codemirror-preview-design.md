# Artifact 预览迁移 CodeMirror 6 —— 设计文档

状态：v9 通过设计评审（八轮复核）· 提案于 v0.7.36 之后 ·
ui-spec §2 确认仍阻塞 ③④⑤⑥
评审记录：v2 收 13 项（[codemirror-preview-review-1.md](codemirror-preview-review-1.md)），
v3 收 11 项、v4 收 9 项、v5 收 5 项（[codemirror-preview-design-review.md](codemirror-preview-design-review.md)，
文件随轮次更新），v6 收 6 项（5 major / 1 minor），v7 收 5 项
（1 major / 2 minor / 2 nit），v8 收 17 项（4 major / 5 moderate /
4 minor / 4 nit）。处置映射见 §7。

## 0. 背景与问题

v0.7.34 起点击会话中的 `Read …` 工具行可在 artifact 面板预览文件
（0.7.36 修通端到端链路）。当前预览是自绘 `GtkTextView`（`detail.rs`
的 `code()`），行号写进同一 buffer（选中/复制带行号）、无语法高亮、
`tool_preview` 只支持单文件。

引擎选型结论（复核过仓库与 crates.io）：GTK3 侧 `sourceview`（3 停更
2020）/ `sourceview4` 0.5.0（gtk ^0.18）/ `webkit2gtk`（Tauri 2.0.2 绑定
gtk ^0.18 + soup3；0.19.2 绑定 gtk ^0.16 + soup3）均与本仓库
gtk 0.19 / glib 0.22 不兼容——**不引入这些绑定**。`syntect` 只是高亮
引擎，行号/选中/复制仍需自管。

据此选定 **WebView + CodeMirror 6**：页面三平台一套，宿主按平台分治。

## 1. 平台宿主方案

| 平台 | 引擎 | 宿主方式 | 前置条件 |
| --- | --- | --- | --- |
| macOS | WKWebView（系统自带） | `preview_panel.rs`，锚点 = `detail.body`，几何合同见 §3 步骤 ③ | 无 |
| Windows x64 | WebView2（Win10/11 预装） | `wry` `WebViewBuilder::build_as_child`，父句柄 = realize 后的**专用 DrawingArea**（§2.6）的 HWND 包成 `HasWindowHandle`（raw-window-handle 0.6） | 一台 Win10/11 验证机（§3 步骤 ⑤） |
| Linux（Kylin V10 arm64） | WebKitGTK 2.28.1（系统，仅 4.0/soup2） | C 辅助进程 `preview-helper` + `GtkSocket`/XEmbed；应用与 helper 统一 `GDK_BACKEND=x11` | 嵌入探针（步骤 ④a）通过；打包闸门 §2.7 |

三平台均不改动现有 gtk 0.19 栈；Linux 不引入 webkit2gtk Rust 绑定
（§1.1）；Windows 的 `wry` Windows 后端不经 gtk-rs。

### 1.1 绑定兼容性审计（复核成立）

`webkit2gtk` 绑定：Tauri 维护的 2.0.2 依赖 gtk ^0.18/glib ^0.18 +
soup3（= 4.1 API，目标机没有）；0.19.2（2023-01）依赖 gtk ^0.16 +
soup3。`sourceview4` 0.5.0 依赖 gtk ^0.18。与 gtk 0.19 / glib 0.22
均不互通，不引入。soup3 意味着两条旧线都不是 libsoup2 的退路。

### 1.2 Linux：C 辅助进程 + XEmbed（**最小收养已验证；键盘/IME/HiDPI 未做**）

机制：`preview-helper`（C，Linux-only）创建 `GtkPlug` + `WebKitWebView`
加载预览页，`gtk_plug_get_id()` 在 realize 之后取，stdout 打一行
`PLUG <xid>`；Rust 侧 `GtkSocket::add_id()` 收养。`GDK_BACKEND=x11`
必须在**任何 GTK/GDK 调用之前**生效（GDK 在 `gtk_init` 时才初始化，
`gtk::Application::new` 本身不初始化；所以锚点是 `main` 的**第一批
语句**，不是某一行 GTK 调用），且写在 `#[cfg(target_os = "linux")]`
内（探针证实 macOS 上 init 前设置会使 `gtk_init_check` 失败），仅步骤
④a 探针通过且本次会话启用 helper 时设置；探针失败不设，预览回退原生。

**spawn 入站契约**（评审三轮 Issue 21，七轮 Issue 43）：helper 无法发现
父进程刚 bind 的随机端口。父进程在同一次 spawn 后、helper 读配置之前，
往 helper 的 **stdin 写一行** `PLUG_URL http://127.0.0.1:<端口>/preview?token=…`；
helper 读入后只回 stdout 一行 `PLUG <xid>`。**token 不进 argv、不进
环境**——`/proc/<pid>/cmdline` 同机可读，argv 等于把 token 广播给所有
本地用户。环境使用 §2.7 的 helper 专用环境（在 `exec` 前组装——包内
假 `libgtk-3.so.0` 会让继承方在进 `main` 之前就退出 127，helper 的
`main` 里再清已经晚了）。

约束与回退：
- **只对 helper 强制 X11 不构成嵌入**——主窗口不是 X11 客户端时
  `GtkSocket` 无法收养 plug；整应用 X11 是不可回退的产品决策。
- **WebProcess 崩溃 ≠ helper 进程退出**：helper 监听
  `web-process-crashed` 信号，以**非零退出码**退出并在退出前打
  `CRASHED` 一行（与 `PLUG` 行同通道分路，前缀区分）；Rust 侧另有
  兜底超时（§2.5）。
- IME：helper 环境只保留系统 XIM（`XMODIFIERS`），不指向包内
  `im-xim.so`（§2.7）。

### 1.3 Windows：`wry`（WebView2）进程内嵌

- 依赖：`[target.'cfg(target_os = "windows")'.dependencies]`
  `wry = { version = "0.57.1", default-features = false }`。仅 Windows
  target——wry 的 gtk/webkit2gtk/soup3 可选依赖挂在 Linux target 表，
  Windows 构建不会编入（评审二轮 Issue 18）。**默认 features 是
  `os-webview` + `x11`**：前者打开 webkit/soup3 一组，后者打开
  `x11-dl/gdkx11/tao/x11`；WebView2 后端在 Windows target 表里，不受
  `os-webview` 控制，所以关默认 features 不影响 ⑤，也不能靠
  "Windows-only" 兜底 gnu 链接（见步骤 ⑤ 的门禁）。`os-webview` 只在
  需要 `webview_version()` 时才开，并注明它在 Windows 上是空的
  （评审七轮 Issue 49）。
- **MSRV**（评审三轮 Issue 24）：wry 0.57.1 是 `edition = 2024` /
  `rust-version = "1.85"`，workspace `Cargo.toml` 写
  `rust-version = "1.85"` 表示**下限**；**不在仓库根加**
  `rust-toolchain.toml`（那会把三平台钉死在 1.85，而不是"不低于"）。
- 嵌入：realize 后取 §2.6 专用 `DrawingArea` 的 `gdk::Window`，用
  自声明的 `extern "C" gdk_win32_window_get_handle` 取 HWND（`gdk-sys
  0.19` 只绑 GTK 3.24 核心，不导出它；`gdk` 0.19 也没有
  `raw-window-handle` 实现），包成 `Win32WindowHandle`（raw-window-handle
  0.6）交给 `WebViewBuilder::build_as_child`；`set_bounds` 的 `Rect`
  用 `LogicalPosition/LogicalSize`（wry 按 webview HWND 的 DPI 自行
  换算，GTK allocation 已是逻辑像素），在 controller 异步创建完成后
  **首次**调用，并在锚点每次 `size-allocate` 时**再次**调用（评审三轮
  Issue 20、七轮 Issue 44）。不做手动 `SetParent`；隐藏/显示同时驱动
  `webview.set_visible`。
- 回退：`GetAvailableCoreWebView2BrowserVersionString` 检测 Runtime
  缺失 → 安装引导（Evergreen Bootstrapper）或原生回退。
- 门禁：Windows target **gnu 目标的 `cargo build`**（`x86_64-pc-windows-gnu`，
  与 `build-windows.sh` 一致）+ `WebView2Loader` 来源声明进 CI；现有
  windows job 是 `continue-on-error: true`，软失败不能当门禁（评审七轮
  Issue 48）。真机验收在**步骤 ⑤**。

## 1.4 关键决策（Key Decisions）

| # | 决策 | 否决的备选 | 否决理由 |
| --- | --- | --- | --- |
| KD1 | 页面用 CM6，三平台一份 | 各平台原生编辑器控件 | 绑定不可用（§1.1）；自管行号/复制是问题本身 |
| KD2 | Linux 用 C helper + XEmbed，先探针后实施 | 浏览器页签；独立覆盖窗口；webkit2gtk 绑定 | 唯一真内嵌路径；探针失败即终止 Linux 部分 |
| KD3 | Windows 用 wry `build_as_child` | 手动 `SetParent`；独立 WebView2 窗口 | 破坏 controller 父窗口管理与 `set_bounds` 前提；独立窗口非内嵌 |
| KD4 | 内容经**独立预览服务**按需拉取（随机端口 + token） | `evaluate_script` 整包推正文；复用现役 39473 服务 | 正文两次过桥；现服务承载 theme/feedback 且 `url()` 是 `&'static str` |
| KD5 | 原生 `code()` 是产品级回退契约 | 仅迁移期临时开关 | 三平台自动回退都落它；正文来源见 §2.4 |
| KD6 | **`preview_docs` 非空**时页面独占标签条 | 页内标签与原生 `eb-tab` 并存；`webview` 模式一律藏原生标签 | 两套标签互相覆盖；已写文件仍走原生 `fill`，那时 `eb-tab` 必须可见（§2.0） |
| KD7 | 传输统一走 `127.0.0.1` + token 的 localhost | 自定义 URI scheme | 三端都有 scheme API（评审二轮 Issue 16 已更正），但 Linux 页面在 helper 进程里，scheme 解决不了"字节从主进程到 helper"；统一 localhost 保持一份传输实现。绑定**仅 `127.0.0.1`**（或附加 `::1`，绝不听未指定地址），token 不落访问日志（评审三轮 Issue 22） |

## 1.5 与既有规格的决策变更（评审一轮 Issue 8）

- `ui-spec.md` §2 定线"GTK 3 + gtk-rs，不引入任何自带 Web 引擎"。
  本设计在**预览面板**引入**系统** WebView（WKWebView / WebView2 /
  WebKitGTK 均为系统组件），改变了"纯 GTK 绘制"的承诺。**该决策
  变更需 ui-spec 维护者确认**（阻塞步骤 ③④⑤⑥）。
- Windows 下限按 **Win10+**（WebView2 前提）执行；Win7 是否正式
  放弃列入 §6 待决（ui-spec 与路线图本就冲突）。
- Linux 预览**不再完全自包含**：helper 链接系统 `libwebkit2gtk-4.0`；
  无系统 webkit 的环境自动回退原生（§2.7 保证 `verify.sh` 仍通过）。

## 2. 状态、协议与服务

### 2.0 预览模式（评审四轮 Issue 27）

显式三态，决定所有路径分支；**模式与子控件绑定，步骤 ② 只引入枚举
（读入上限按 §2.4 统一到 5 MiB 除外），模式随步骤 ③（macOS）/④b
（Linux）/⑤（Windows）落地才切到 `webview`**：

| 模式 | 进入条件 | 行为 |
| --- | --- | --- |
| `native` | 默认；`EB_PREVIEW=native` 时锁定 | 同现状：`open_tool_file` 写 `tool_preview`，`fill`/`rebuild` 照旧；读入上限按 §2.4 统一到 5 MiB |
| `webview` | 该平台宿主落地（③/④b/⑤） | `open_tool_file` 写 `preview_docs`（不写 `tool_preview`）；**`preview_docs` 非空**时面板承载 WebView 子控件并跳过 `code()` |
| `sticky-native` | `webview` 下发生自动回退（§2.5） | 同 `native` 行为；回退的同一临界区把正文写进 `tool_preview`/`tool_preview_task`，走现有 refresh/`fill`（§2.5） |

打开谓词（改 `shell.rs:1563-1567`，完整布尔式）：

```
detail_visible && ( task.artifact.is_some()
    || (mode == webview ? 当前任务的 preview_docs 非空
                        : tool_preview 属于当前任务) )
```

- **面板手续只有一段**（评审六轮 Issue 35；§2.6、步骤 ③④b⑤、§4
  与此同一句）。宿主控件各平台只创建一次：Linux `GtkSocket`、Windows
  专用 `DrawingArea` 放在 `detail.body` 里，macOS NSPanel 不在 `body`
  里。`widgets::clear`（`widgets.rs:157-160`）是 `remove`，不是
  `set_visible(false)`；`detail::rebuild`（`detail.rs:66-72`）今天对
  `tabs` 和 `body` 都调用它。先藏再 `clear` 仍会把宿主控件摘掉，所以
  **`clear(body)` 必须按控件身份跳过 `GtkSocket` / `DrawingArea`**。
  宿主控件的显隐与 `built` 只走 `detail.rs` 的 API：`detail::detach_native_body`
  （清 `built`、移除原生 `toolbar`/`eb-code-scroll`、显示宿主——先清
  `no_show_all` 再 `set_visible(true)`）与
  `detail::show_host(&detail, bool)`；`shell.rs` 不直接碰 `built`
  （私有字段，`detail.rs:50/53/63`）。另一点：`rebuild` 结尾的
  `body.show_all()`（`detail.rs:89-90`）会把 `hide` 过的孩子重新 `show`
  （GTK 3.24 实测；只有 `no_show_all` 挡得住），所以**隐藏必须发生在
  `fill` 之后**，不能只靠"先藏再 fill"（评审七轮 Issue 40）。
  分支写在调用 `detail::fill` 之前（`shell.rs` 刷新里今天约
  1568–1589 行组装 `files` 再 `fill` 的位置），不要写进 `fill` 的空列表
  早退（`detail.rs:193-195`）或 `built` 比较（`204-206`）之后：
  - **`mode == webview` 且当前任务的 `preview_docs` 非空**：本帧不调用
    `rebuild`、也不调用 `detail::select`（`select` 内部同样 `rebuild`，
    `detail.rs:231-240`）。保留宿主控件并显示；从 `body` 移除原生
    `toolbar` 与 `eb-code-scroll`，不 `remove` 宿主控件。藏 `eb-tab`
    与头部的 `+`（`eb-detail-add`，页面自带新建入口），关闭/展开按钮
    留着，跳过 `code()`。把 `built` 置为 `None`（经
    `detail::detach_native_body`），不要写成"已装好 code"的键。
  - **其余情况**（`native`、`sticky-native`，以及 `webview` 但
    `preview_docs` 为空，含点 artifact 卡清空 `preview_docs` 之后）：
    走现有 `fill` 画 `task.artifacts` 或 `tool_preview`
    （`shell.rs:1572-1589`），原生 `eb-tab` 与 `+` 保持可见；只有真正
    装上 `code()` 之后才写 `built`（今天在 `detail.rs:226`）。
    **宿主控件在 `fill` 之后隐藏**（`detail::show_host(&detail, false)`：
    `set_visible(false)` + `set_no_show_all(true)`；NSPanel 收起），
    不 `remove`、不销毁。顺序不能反（评审七轮 Issue 40）。从 WebView
    回到同一批已写文件时，跳过分支已经把 `built` 置为 `None`，不得再
    因旧键相同而在 `204-206` 早退。下一次 Read 复用同一个 socket /
    DrawingArea / NSPanel，不再 `add_id`、不再 `build_as_child`（只有
    控件尚未创建，或 plug 已死必须重嵌，才是首次嵌入，不是这条切换）。
- **宿主控件的第一次安装不依赖 `fill`**。`detail::fill` 在
  `detail.rs:193-195` 对空列表早退，`GtkSocket` / `DrawingArea`
  不能靠它装上。装好之后无文档时只隐藏，不销毁。
- 复制按钮按**当前画面**取数（评审五轮 Issue 34、六轮 Issue 36）。
  §2.6 用下面同一句。不抓 `toolbar` 建闭包时的 `String`（今天
  `detail.rs:319-326` 的 `let code = code.to_string()` 不再当点击
  来源）：
  - 画面是 WebView（`mode == webview` 且 `preview_docs` 非空）：
    点击时读该 id 的已鉴权缓存（与 `/file` 同源）。
  - 画面是原生 `fill`（其余情况：`native`、`sticky-native`，以及
    `webview` 但 `preview_docs` 为空）：点击时读 `fill` 正在显示的
    那份正文（当前 `tool_preview`，或选中的 `task.artifacts` 项）。
    不读缓存。点 artifact 卡会清空 `preview_docs`，§2.4 又随栈清空
    缓存，这时复制的是原生正文。sticky 之后即使栈里还留着文档，画面
    已经是原生，也走这一支。
- 步骤 ② 在 WebView 子控件落地之前**保持今天的写入与谓词**（读入上限
  按 §2.4 统一到 5 MiB 除外）。

### 2.1 前端资产（全离线，三平台同一份）

- `experiments/preview-webview/`：esbuild 打包为单 `bundle.js` +
  `index.html`，`include_bytes!` 进二进制。
- 构建约束：`target: ['es2018']`；禁用 OffscreenCanvas/WebLocks；
  `ResizeObserver` 依赖列入步骤 ④a 探针。
- 产物治理：提交 esbuild metafile；核对语言包许可证（MIT）；bundle
  内禁止本机绝对路径（构建期断言）。
- 功能：CM 原生行号 gutter（复制不含）、按扩展名映射语言包
  （md/rust/json/python/js/html/css；toml 走 legacy-modes；未知扩展
  纯文本）、搜索面板、折行切换、当前行高亮、读取**上限改为 5 MiB**
  ——**超长文件截断并在页内提示**（与现截断提示同风格，不拒绝），
  只读（编辑留 flag 默认关；改动不落盘、也不假装可保存）。
  语言按扩展名**在页面**判定，`PreviewDoc` 不带 `lang`（避免两处映射
  不一致，评审七轮 Issue 52）。
- 标签：显示 `name`；两个同名 `basename`（不同目录的 `lib.rs`）用
  tooltip（必要时加父目录副标题）消歧，不能出现两个看起来一样的标签
  （评审七轮 Issue 52）。
- 主题：现无 GTK→WebView 主题契约。首期页面内置固定亮/暗两套
  CM6 theme，`dark` 取
  `gtk::Settings::default().is_gtk_application_prefer_dark_theme()`
  （若日后加主题设置，从同一处取值），由 `/session` 的 `theme {dark}`
  下发；主题切换也走 `session_version`（评审七轮 Issue 55）。高亮配色
  后续从 `ui-spec/tokens.css` 的 `--eb-code-syntax-*` 生成，不在本期。

### 2.2 状态模型：三份状态 + 一张操作表（评审三轮 Issue 6 修订）

字段在 `crates/eb-api/src/model.rs` 的 `App`。分三份：

| 状态 | 内容 | 生命周期 |
| --- | --- | --- |
| `task.artifacts`（既有） | 任务已写文件，≤ `MAX_TABS`(12)。**仅本次进程——`state.rs` 的 `save`/`load` 不含 artifacts** | 进程内 |
| `preview_docs`（新，运行时） | 预览栈 `Vec<PreviewDoc>`，`PreviewDoc { id: [u8;16], name, path, file_id }`；**去重按 `path`，命中复用已有 `id`**（不生成新 id——否则该条目不在栈里，挤出逻辑永远碰不到它的缓存，评审七轮 Issue 51），**但复用前丢弃该 id 的旧字节并重读**（同一文件可能已被改写，否则第二次 Read 会显示旧内容）；**上限 12**，超出时挤掉最旧并同锁删字节 | 随面板/任务/写入清空 |
| `tool_preview`（既有） | **`native`/`sticky-native` 模式下 `open_tool_file` 继续写入它**——它是刷新打开面板谓词（`shell.rs:1563-1567`）与 `code()` 的输入；仅 `webview` 模式不写（§2.0），改认 `preview_docs` | 见操作表 |
| 页内选中 + 滚动/选区 | 页面 `EditorState`；`active` 由页面经 `select-tab` 上报，主机只镜像 | 页面持有 |

`preview_docs` 只在本次进程内：`state.rs::save` 是手写 JSON（只挑字段，
`state.rs:39-70`），`App` 也没有 `Serialize`。若将来改为序列化 `App`，
该字段必须 `#[serde(skip)]`，否则 `PreviewDoc.path` 的绝对路径会落盘
（评审七轮 Issue 56）。

操作表（点名函数；多轮评审逐格校对）：

| 操作（函数） | artifacts | preview_docs | `tool_preview` | 面板旗标 |
| --- | --- | --- | --- | --- |
| 点 Read 行（`open_tool_file`） | 不动 | **仅 `webview` 模式**：按 path 去重追加并激活（命中复用 id）；`native`/`sticky-native` 不动 | **`native`/`sticky-native`：写入读到的正文**（读入规则见 §2.4 的 `read_preview`：5 MiB、字符边界）；`webview`：不写，登记进 server 缓存 | `detail_visible=true`、`artifact_dismissed=false`（现状） |
| 点 artifact 卡 | 选中对应项 | 清空 | 清空（现状） | `detail_visible=true`、**`artifact_dismissed=false`**（`shell.rs:719/744` 现状，两处都要）。画面改走 §2.0「其余情况」：**先 `fill` 画已写文件，再隐藏**宿主控件（不销毁） |
| 关面板（`hide_detail`） | 不动 | 清空 | 清空 | `detail_visible=false`、`artifact_dismissed=true` |
| `open_task` / `new_task` | 换任务的 | 清空 | 清空 | 随函数现状 |
| `begin_task`（新建任务） | 不动 | **清空（新字段，运行时不跨任务）** | **不动**——`model.rs:418` 只设 `route`/`detail_visible=false`，且 `read_preview_is_a_separate_pile_from_written_files`（`eb-api/model.rs` 约 1013–1016）锁住"begin_task 之后 `tool_preview` 仍在、`tool_preview_task` 仍指原任务"；评审五轮 Issue 33 确认这不是漏清 | `detail_visible=false`（函数已有）、`artifact_dismissed` 不动（现状） |
| `AgentEvent::Artifact` 写入 | 追加 | 清空 | 清空 | 面板弹出逻辑现状 |
| 页内关单个标签 | 不动 | 从栈移除 | 不动 | **栈空时只 `detail_visible=false`，不设 `artifact_dismissed`**——维持"下一次写入仍弹出"的现状（`shell.rs:340-343` 同款语义，评审三轮 Issue 25） |
| 页内切标签（`select-tab`） | 不动 | 只改 `active`（含 id 校验） | 不动 | 不动（bump，§2.3） |
| 页内 ESC（`close`） | 不动 | 清空 | 清空 | 与 `hide_detail` 同：`detail_visible=false`、`artifact_dismissed=true` |
| 页面 `error`（已 ready 后） | 不动 | 不动 | **按 §2.4 写回当前正文** | 模式切 `sticky-native`（§2.5） |
| 流式 refresh | 照常 | content-hash 短路 | 照常 | 不受影响 |

**每处改变 `preview_docs`（含 `active`）或缓存可见性的行，都要在缓存
就绪后 bump `session_version`**（§2.3）；只改页面滚动/选区的交互不 bump，
不刷新面板。

### 2.3 消息桥与预览服务

**服务**：新建独立 `preview_web_server.rs`（不动现役 39473
`preview_server.rs` 及其全部调用点）。

- **绑定地址：显式 `127.0.0.1:0`（随机端口）**；如需 IPv6 再附加
  `::1`，**绝不听未指定地址**（评审三轮 Issue 22）。token 128-bit
  （`/dev/urandom`），**不写入任何落盘访问日志，也不进 argv/env**
  （helper 走 stdin，§1.2）；每个请求校验 `Host == 127.0.0.1:<port>`
  或 `[::1]:<port>`，不符 400。`POST /event` 只收
  `Content-Type: application/json`，带 `Origin` 时必须是自身 origin，
  `Sec-Fetch-Site: cross-site` 拒绝（评审七轮 Issue 43）。
- 共享状态 `Arc<Mutex<PreviewSession>>`：token、doc 表（id→{path,
  file_id, 元数据}）、正文字节缓存、`session_version: u64`。UI 的
  `Rc<RefCell<App>>` 只在主线程；事件经 channel + 主线程
  `glib::idle_add_local_once` 消费。
- server 线程模型：每连接一线程，上限 32（超限 503）；请求按头 +
  `Content-Length` 收齐正文再解析；**`Connection: close`**、读超时 5 s、
  请求头上限 16 KiB、`/event` 正文 ≤ 64 KiB（评审七轮 Issue 54）。
- **文件读入缓存的写入方**（评审三轮 Issue 6，七轮 Issue 42/51）：
  主线程点击时 `File::open` 取**文件身份** `file_id`（unix：`dev`+`ino`；
  Windows：卷序列号+文件索引）并 `canonicalize` 路径，把 id→(path,
  file_id) 登记进 `PreviewSession`（此时**不 bump**）。命中已有 path 时
  复用 id，但**先删该 id 的旧字节再重读**（文件可能已被改写）；入栈与
  登记在主线程同一段内完成，再交给 **server 线程**读进缓存，**读完才**
  `session_version += 1`；`/file` 从缓存回字节，对"已登记但未就绪"的
  id 回 **503 + `Retry-After: 1`**（页面重试，不当成文件消失）。读盘后
  比对 `file_id`，不一致按"文件已变"处理（不显示另一份文件）。读 IO
  不在主线程。
- **栈挤出释放缓存**：`preview_docs` 按 12 上限挤掉最旧文档时，同一把
  锁内删除该 id 的正文字节——否则只点 Read 不关面板，缓存按文件数
  线性涨（评审四轮 Issue 28）。**清空、挤出、切换 `active` 也都要让
  `session_version` 前进**（同一把锁内改完再 bump，评审七轮 Issue 42）。
- 路由与响应头：

| 路由 | 内容 | Content-Type |
| --- | --- | --- |
| `/preview?token=…` | `index.html`——**响应时改写模板**：把脚本地址换成带同一 token 的 `/bundle.js?token=…`。静态 HTML 里的相对 `<script src="bundle.js">` 不会继承页面 query，不改写就是 404、CM6 不加载（评审四轮 Issue 26）；磁盘上的产物仍是静态字节 | `text/html; charset=utf-8` |
| `/bundle.js?token=…` | CM6 bundle | `application/javascript` |
| `/session?token=…&since=<version>` | 客户端带已知版本；`since == session_version` → **HTTP 204 空体**（明确"无变化"，与"无文档"区分靠 open-docs 数组）；有变 → `{version, open-docs: [{id, name}], active: <id|null>, theme: {dark}}` | `application/json`（204 无） |
| `/file?token=…&id=…` | 文件字节（≤ 5 MiB，经 §2.4 的 `read_preview`）；**截断时带 `X-Preview-Truncated: 1`**——没有该标记页面无法区分"恰好 5 MiB"与"被截断"（评审四轮 Issue 28）；**截断落在 UTF-8 字符边界**（按字节 `take` 后回退到最近字符边界，保证剩余字节合法，现 `shell.rs:879-882` 的 `read_to_string` 失败即拒绝会把本应截断展示的大文件整份拒掉），之后才套"非 UTF-8 拒绝"；**已登记但未就绪 → 503 + `Retry-After: 1`** | **`text/plain; charset=utf-8` + `X-Content-Type-Options: nosniff`（仅此路由）** |
| `POST /event?token=…` | `{ready, close, close-tab, select-tab, error}`，语义见下表 | — |

**事件表**（页面 → 主线程；每行都要 refresh，改 `preview_docs` 的行
还要按上文 bump，评审七轮 Issue 47）：

| 事件 | 主线程动作 | 备注 |
| --- | --- | --- |
| `ready` | 记录页面已就绪，开始接受 `/session` 轮询 | 不 bump |
| `close` | 与 `hide_detail` 同：清 `preview_docs`/缓存与 `tool_preview`、`artifact_dismissed=true`、`detail_visible=false` | bump |
| `close-tab {id}` | 从栈移除该 id、删缓存；栈空只 `detail_visible=false`（不设 `artifact_dismissed`）；**关的是 `active` 时把 `active` 接到相邻项**（页面随后仍可用 `select-tab` 校正） | bump |
| `select-tab {id}` | 只改 `active`，不改栈 | bump |
| `error {message}` | 切 `sticky-native` 并按 §2.4 写回当前正文 | 见 §2.5 |

- **页面 CSP**（评审三轮 Issue 22，写成具体指令）：响应
  `/preview` 时带 `Content-Security-Policy: default-src 'none';
  script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src
  'self'; img-src 'self' data:`——脚本源限自身，杜绝工作区文件注入
  执行。
- **轮询参数**（评审三轮 Issue 23，七轮 Issue 41）：`/session` 500 ms；
  **空闲降频**（连续两次 204 后转 2 s，一有变化立即回 500 ms），
  **不暂停**。暂停与否只能由页面按自己的 `document.visibilitychange`
  判定；GTK 应用是否失焦对 helper 里的页面不可见，**不作为暂停条件**。
  恢复轮询时必须立即补拉一次。
- `ready`：三端统一 `POST /event {ready}`；页面发出 `ready` 后才开始
  轮询 `/session`（服务端不设 ready 门）。`open-docs`/`theme` 一律走
  拉取；平台桥只用于**唤醒**（macOS/Windows `evaluate_script`，Linux
  在面板重新显示时重新 `load_uri`），不承载数据（评审七轮 Issue 41）。

### 2.4 原生回退的正文来源（评审二轮 Issue 6 + 三轮 Issue 6 + 五轮 Issue 31）

- 字节缓存在 `Arc<Mutex<PreviewSession>>`，**写入方 = server 线程**
  （按 §2.2 登记的 id→path 表读磁盘）；主线程点击只登记并取
  `file_id`。
- **按 §2.0 模式分派**：`native`/`sticky-native` 模式下
  `open_tool_file` 继续写 `tool_preview`，`code()` 输入不断流；仅
  `webview` 模式改走 `preview_docs`（§3 步骤 ② 只引入枚举，谓词与
  子控件随步骤 ③④b⑤ 生效）。
- **读入规则只有一份**：`read_preview(path, cap) -> PreviewText`
  （`File::open` → 截断到 `cap` 字节后回退到字符边界 → 非 UTF-8 判定 →
  截断标记），`cap = 5 MiB`，`/file`、sticky 回退、`open_tool_file`
  三处共用；`shell.rs` 的 `PREVIEW_CAP = 512 KiB` 随之上调（评审七轮
  Issue 46）。截断提示由调用方加：原生走现有"… (truncated at 5 MiB)"，
  页面走页内提示 + `X-Preview-Truncated`。
- **sticky 回退写回 `tool_preview`，不另装 `code()`**（评审五轮
  Issue 31）：切到 `sticky-native` 的**同一临界区**内——①缓存已有该
  id 的字节就直接拷出；②否则按登记的 path 用 `read_preview` **在主线程
  同步读**（与 `/file` 同一规则），并比对登记时的 `file_id`；③把
  字符串写入 `tool_preview` 与 `tool_preview_task`。之后走 §2.0
  「其余情况」：先 refresh/`fill`、再隐藏宿主控件（不销毁），让谓词与
  `built` 键看见**同一份字符串**；不要在 refresh 旁边单独装一次
  `code()`（下一帧会被谓词判空隐藏或换成 artifacts）。缓存未就绪
  **不得**当成"文件消失"。
- 跳过 `code()` 时把 `built` 置为 `None`；只有真正装上 `code()`
  之后才写 `built`（`detail.rs:226`）。从 WebView 回到同一内容不得
  靠 `detail.rs:204-206` 早退。手续与 §2.0/§2.6 同一段。
- 缓存随 `preview_docs` 清空而清空；单文件 ≤ 5 MiB。

### 2.5 回退契约

`EB_PREVIEW=native` 强制 `native`；未设置时默认 `webview`（宿主落地
后）。自动回退场景：Windows Runtime 缺失；Linux 无 X display / 探针
失败 / helper 两次启动失败；`ready` 5 s 超时；helper 挂死；**页面
`POST /event {error}`（已 ready 之后）**。回退 =
模式切到 `sticky-native`，并按 §2.4 在同一临界区写回
`tool_preview`/`tool_preview_task` 后由现有 refresh/`fill` 画原生
视图（缓存或同步读提供字节）；首次失败后 sticky。`code()` 不删除。
`EB_SELFTEST` 在 `webview` 模式跳过 `observe_detail` 的
`eb-code-scroll`/`eb-detail-file` 断言，`native`/`sticky-native`
保持。

### 2.6 面板结构衔接与 `rebuild` 隔离（评审三轮 Issue 20）

- 三平台锚点统一 `detail.body`；关闭/展开按钮留在 GTK 头部。
- **面板手续与 §2.0 同一段**（评审六轮 Issue 35、七轮 Issue 40），不另写一套：
  - `mode == webview` 且 `preview_docs` 非空：在调用 `fill` 之前
    分支，本帧不跑会 `clear(body)` 的 `rebuild`、也不跑
    `detail::select`。`clear(body)` 必须跳过 `GtkSocket` /
    `DrawingArea`（`widgets::clear` 是 `remove`）。保留并显示它们；
    macOS 显示 NSPanel。从 `body` 移除原生 `toolbar` 与
    `eb-code-scroll`。藏 `eb-tab` 与头部 `+`，跳过 `code()`，
    `built = None`（经 `detail::detach_native_body`）。
  - 其余情况：先走现有 `fill`/`rebuild` 装 `code()`，**装完再**隐藏
    socket / DrawingArea / NSPanel（`detail::show_host(&detail, false)`：
    `set_visible(false)` + `set_no_show_all(true)`；NSPanel 收起），
    不 `remove`、不销毁。顺序不能反：`rebuild` 结尾的
    `body.show_all()`（`detail.rs:89-90`）会把隐藏过的孩子重新显示。
    原生标签与 `+` 可见；只有装上 `code()` 之后才写 `built`。回到
    同一批已写文件不得因 `detail.rs:204-206` 的旧键早退。
  - 下一次 Read 复用同一宿主控件，不再 `add_id`、不再
    `build_as_child`（首次创建，或 plug 已死需要重嵌，才再嵌入）。
- Linux：`body` 内放专用 `GtkSocket`（不用 `Box` 自身做嵌入对象）。
  Windows：`body` 内放专用 `DrawingArea`（**不用 `Box` 自身的窗口
  句柄**——`gtk::Box` 没有可交给 `build_as_child` 的子 HWND；
  `GtkDrawingArea` 有自己的 GdkWindow，实测 `has_window=1`、realize
  后与顶层不同，HWND 经 `gdk_win32_window_get_handle` 取得，§1.3）。
  macOS：浮层 NSPanel 不在 `body` 里，`clear(body)` 藏不住它，必须
  按上面显隐收起或显示；不改 NSPanel 父子。
- Windows `set_bounds`：controller 创建完成后首调，此后**锚点每次
  `size-allocate` 都再调**（宿主控件隐藏时不为不可见矩形重嵌）；
  `Rect` 用 `Logical*`（wry 按 webview HWND 的 DPI 换算），隐藏/显示
  同时驱动 `webview.set_visible`。
- toolbar 复制按钮与 §2.0 同一句：画面是 WebView（`mode == webview`
  且 `preview_docs` 非空）时，点击读该 id 的已鉴权缓存；画面是原生
  `fill`（其余情况，含 `webview` 但 `preview_docs` 为空，以及 sticky
  之后）时，点击读 `fill` 正在显示的那份正文。不抓
  `detail.rs:319-326` 闭包里的 `String`，也不在原生画面读缓存。
- Linux 承载 `GtkSocket` 的容器写在 `detail.rs` 的 `body`。

### 2.7 打包闸门（Linux）

- **构建基线**（评审三轮 Issue 1）：**删除 v4 的"版本脚本"替代**——
  GNU 版本脚本限制的是本二进制**导出**符号，不改写链接时记下的
  未定义符号版本，2.28/2.38 soname 都是 `.so.37`，照做等于还在用
  2.38 链接。helper **只链接从目标机（或同等 2.28.1 SDK）拷来的
  头文件与库**；**禁止在 Debian 10 镜像安装/链接归档的 2.38.6**。
  `Dockerfile.build` 写明 sysroot 来源与 helper 的编译、安装命令
  （安装到主程序 `current_exe` 同目录）。
- 运行时验收：目标机 `ldd -r preview-helper` **或实际启动**——
  soname 一致不等于符号版本一致。
- **环境由父进程在 `exec helper` 前组装**（评审三轮 Issue 1：继承方
  在进 `main` 前就完成动态链接，helper 的 `main` 里再清无效）：
  去掉包内 `LD_LIBRARY_PATH`（可恢复 `EVOBUDDY_HOST_LD_LIBRARY_PATH`）、
  `GTK_PATH`、`GTK_DATA_PREFIX`、`GTK_EXE_PREFIX`、
  `GTK_IM_MODULE_FILE`、`GDK_PIXBUF_MODULE_FILE`、
  `GSETTINGS_SCHEMA_DIR`、`GIO_MODULE_DIR`、`FONTCONFIG_FILE`；
  保留系统 `XMODIFIERS`，不指向包内 `im-xim.so`。**不改主进程的
  `run.sh`**（主进程捆绑 GTK 依赖这些变量），也**不只改
  `external_env()`**（它只包外部 ACP 子进程，不包 helper）。
- `bundle.sh`：不得把 helper 纳入 ldd 闭包扫描；helper 按
  `current_exe` 同目录查找，AppImage 挂载内同路径。
- 裸 Debian 10 `verify.sh` 必须仍通过：无系统 webkit → helper 退出 →
  自动回退原生（不判失败）。

## 3. 实施步骤（每步 = 一个 PR 的范围）

| # | 内容 | 改动文件 | 验收 |
| --- | --- | --- | --- |
| ① | `preview-webview/` 打包（静态 fixture 驱动）；新建独立 `preview_web_server.rs`（并发 + 随机端口 `127.0.0.1` + token + §2.3 路由含 CSP/since/204/事件表/连接上限）+ 共享 `read_preview`（5 MiB、字符边界、非 UTF-8 判定） | `experiments/preview-webview/*`、新 `crates/eb/src/preview_web_server.rs` | 浏览器带 token：页面可用；`/session?since` 204/200 与 `open-docs` 形状正确；无 token 404、`Host` 不符 400；`/file` 头正确、未就绪 503；**会话表（fixture 驱动）**的清空/替换/重读后 bump 正确；POST 正文收齐、事件按表生效；连接上限/读超时生效；`read_preview` 单测（截断落字符边界、5 MiB、非 UTF-8 拒绝）；theme/feedback 面板不受影响 |
| ② | 状态模型：`eb-api/model.rs` 增 `preview_docs`（按 path 去重、命中复用 id 并重读、上限 12）+ 模式枚举（§2.0，**本步只引入枚举，不切换谓词**）；`begin_task`/`open_task`/`new_task` 清 `preview_docs`；`open_tool_file` 改用 ① 的 `read_preview`（**本步唯一有意的行为变化**：上限 512 KiB → 5 MiB、截断落字符边界）。**本步不改这三处函数今天对 `tool_preview` 的行为：`begin_task` 仍不清（`model.rs:418` 只设 `detail_visible=false`），`open_task`（`357-358`）与 `new_task`（`343-344`）仍清**（评审六轮 Issue 37） | `crates/eb-api/src/model.rs`、`crates/eb/src/shell.rs` | 单测锁定 `open_tool_file` 写入、`shell.rs:1567` 谓词、`begin_task` 后 `tool_preview` 仍在且 `tool_preview_task` 仍指原任务（约 1013–1016 行）、`open_task`/`new_task` 后 `tool_preview` 仍为空（约 1019–1021、1029–1030 行）；`preview_docs` 去重命中复用 id 且**丢弃旧字节重读**（改写后再 Read 不显示旧内容）；挤出时 bump 且字节释放；页内关最后一个标签不设 `artifact_dismissed`。**不在本步新增谓词用例**（评审五轮 Issue 29/33、七轮 Issue 51） |
| ③ | macOS：`preview_panel.rs` + 接线（锚点 `detail.body`；面板手续用 §2.0/§2.6 同一段——**仅 `preview_docs` 非空**才藏 `eb-tab` 与 `+`、显示 NSPanel、`built = None`；为空则先 `fill` 再收起 NSPanel 不销毁，原生标签可见；复制按钮按当前画面取数）。**几何合同**（评审四轮 Issue 19 + 五轮 Issue 32 + 七轮 Issue 45，完整矩形、双触发重算）：`let (wx, wy) = toplevel.position()`；`let (ax, ay) = detail.body.translate_coordinates(&toplevel, 0, 0)`；`X = wx + ax`；`Y = primary_screen_height − (wy + ay) − height`；**`primary_screen_height` 取主显示器高度**（`gdk::Display::primary_monitor().geometry().height()` 或 `NSScreen.screens[0]`；GDK quartz 以主屏底边为翻转原点，**不要**用锚点所在屏的高度）；**宽、高取本次 allocation**（不加 `max(420)`）；坐标保持 **point**（**不乘 HiDPI scale**；将来拿到设备像素才**除以** scale）。**重算两处**：`detail.body` 的每次 `size-allocate`（拖 sash、`detail_maximised` 走 `outer.set_position`，顶层不换尺寸、无 `configure-event`），以及主窗口的 `configure-event`（窗口移动，`wx/wy` 用 `toplevel.position()`）；**不听设置窗口的 `configure-event`**、不改 NSPanel 父子；禁 `CGEventTap`、禁 `activateIgnoringOtherApps`。**取焦契约**：非激活面板（`NonactivatingPanel` + `becomesKeyOnlyIfNeeded`，或 `orderFrontRegardless` + 点击取焦），显示时成为 key、ESC 后把 key 还给主窗口；**不复用** `theme_view.rs` 的取焦代码；ESC 新行为 | 新 `crates/eb/src/preview_panel.rs`、`crates/eb/src/shell.rs`、`crates/eb/src/detail.rs` | 真机（Retina + 副屏在主屏左侧/上方）：打开/多文件/主题/ESC；拖 sash（含收窄到很窄）、`detail_maximised`、移动窗口、设置窗口开关后 frame 正确；搜索面板/⌘F/复制选区可用；点 artifact 卡后是原生 `code()` 且 NSPanel 收起、socket 不显示，再点同一文件的 Read 复用同一 NSPanel |
| ④a | Linux **嵌入探针**（falsifiable）：socket/plug 收养、鼠标、键盘、fcitx(XIM) 中文、HiDPI、`ResizeObserver`、`web-process-crashed` | `preview-helper` 原型 + 探针脚本 | **任一失败 = 放弃 XEmbed，Linux 保持原生，本设计 Linux 部分终止** |
| ④b | ④a 通过后：helper 产品化（§1.2 spawn 契约：**URL 走 stdin** + §2.7 环境/构建闸门）+ `#[cfg(linux)]` 的 `GDK_BACKEND` 前置 + `detail.rs` `body` 内 `GtkSocket` + `rebuild` 隔离（宿主控件跳过 + `no_show_all`）+ 回退链路。**`run.sh` 不在本步改动文件内**——helper 环境只在 spawn helper 的那段代码里组装；验收含"`run.sh` 的导出列表与现在逐字相同"（评审四轮 Issue 1） | `preview-helper`、`crates/eb/src/main.rs`（cfg linux）、`crates/eb/src/detail.rs`、`crates/eb/src/shell.rs`、`packaging/linux/Dockerfile.build`、`bundle.sh` | Kylin 真机：内嵌可用；`ldd -r` 核对；裸 Debian 10 `verify.sh` 通过；`/file` 拒绝非 UTF-8；token 不在 `ps`/`/proc` 里出现；`preview_docs` 非空时拖 sash/refresh 不摘 socket；为空时隐藏 socket 不销毁、切回原生后 `eb-code-scroll` 可见且 socket 不可见，下一次 Read 复用、不再 `add_id`；`run.sh` 无 diff |
| ⑤ | Windows：`wry`（仅 Windows target，`default-features = false`）+ `build_as_child`（HWND 经 `gdk_win32_window_get_handle`）+ §2.6 DrawingArea + `size-allocate` 重设 **Logical** bounds + `webview.set_visible` + 运行时检测 + 回退。面板手续同 §2.0：隐藏 DrawingArea 不销毁，切回 Read 不再 `build_as_child` | `Cargo.toml`（Windows target + workspace `rust-version = "1.85"`）、新 `crates/eb/src/preview_web.rs`、`crates/eb/src/detail.rs`、`crates/eb/src/shell.rs` | Windows 验证机：同 ③ 功能面；Runtime 缺失引导；DPI 切换（150%）；handle 非空且矩形与锚点重合；**gnu 目标 `cargo build` 通过并声明 `WebView2Loader` 来源**（CI 软失败不算门禁） |
| ⑥ | 清理：`code()`/`tool_preview` 保留为回退路径；`EB_PREVIEW` 语义文档化；`EB_SELFTEST` 按 §2.5 调整；`ui-spec` §2 回写 | `crates/eb/src/detail.rs`、`selftest.rs`、`docs/ui-spec/*` | 三平台回归 + native 回归 + selftest 两模式绿 |

错误表（三平台统一）：`ready` 5 s 超时 / 页面 `error` → sticky 回退
原生（按 §2.4 写回正文）；helper 挂死 → kill + 重启一次 + 再失败回退；
文件消失 → 空态提示；`/file` 503 → 页面重试（不当成消失）；端口绑定
失败 → 回退 + 状态栏提示；非 UTF-8 → 拒绝并提示；`file_id` 变化 →
按"文件已变"提示，不显示另一份文件。

## 4. 风险与对策（严重度 + 终止判据）

| 风险 | 严重度 | 对策 / 终止判据 |
| --- | --- | --- |
| Linux XEmbed 嵌入不成立（焦点/IME/HiDPI/黑屏） | **高（有出口）** | 步骤 ④a 探针；任一失败即终止 Linux WebView，保持原生 |
| 全应用 X11 改变 Wayland 会话下缩放/装饰/输入法 | 中 | 真机回归主流程；不可接受则 Linux 整体保持原生（同上出口） |
| helper 环境清理不全或时机错误（`exec` 前未清） | **高** | §1.2/§2.7：父进程 `exec` 前组装环境；逐项清单进验收 |
| 构建基线错位（2.38 头文件/库；版本脚本救不了） | **高** | §2.7：仅目标机 2.28.1 sysroot，禁用 Debian 归档 2.38.6；`ldd -r` + 实际启动验收 |
| `rebuild`/`clear` 摘掉嵌入子控件 | **高** | §2.0/§2.6：`clear(body)` 跳过 socket/DrawingArea；`preview_docs` 非空时不跑会摘掉它们的 `rebuild`；为空时先 `fill` 再只 `set_visible(false)` + `no_show_all`。步骤 ④b/⑤ 验收：非空时拖 sash + 流式 refresh 不掉 socket；切到已写文件只隐藏，下一次 Read 复用，不再 `add_id` / `build_as_child` |
| `fill` 之后的 `show_all` 把宿主控件顶回原生画面 | **高** | §2.0/§2.6：隐藏放在 `fill` 之后，`set_no_show_all(true)`；`rebuild` 内按控件身份跳过宿主。③④b⑤ 验收：点 artifact 卡切回原生后 `eb-code-scroll` 可见、socket/DrawingArea 不可见（评审七轮 Issue 40） |
| `session_version` 失效契约漏 bump / `/file` 未就绪被当"文件消失" | **高** | §2.3：所有改 `preview_docs`（含 `active`）与缓存可见性的操作都在同一把锁内 bump；`/file` 未就绪回 503 + `Retry-After`（评审七轮 Issue 42） |
| macOS `GDK_BACKEND` 泄漏致启动失败 | **高（探针证实）** | `set_var` 仅 `#[cfg(linux)]` 且 ④a 通过后设置 |
| server 线程触碰 UI 状态 | **高** | `Arc<Mutex>` 只放会话数据；UI 仅主线程；事件 channel + `idle_add_local_once` |
| 文件字节经 localhost 暴露 | **高** | §2.3（`127.0.0.1` 显式绑定、token 不落日志、CSP 具体指令、`/file` 头、`Host` 校验、POST 的 `Content-Type`/`Origin`）为合并硬条件 |
| token 经 argv/env 泄露给同机进程 | **高** | §1.2/§2.3：URL 走 helper stdin，argv/env 只留非敏感参数；④b 验收"token 不在 `ps`/`/proc` 里出现"（评审七轮 Issue 43） |
| 点击路径被写成清 `tool_preview` 致面板打不开 | **高** | §2.2 操作表：`native`/`sticky-native` 保留写入；**谓词与写入随步骤 ③/④b/⑤ 与模式一起生效**（步骤 ② 不切），步骤 ② 单测锁定"`open_tool_file` 仍写 `tool_preview`"与 `shell.rs:1567` 谓词（评审五轮 Issue 29） |
| 回退无正文来源 | **高** | §2.4：sticky 临界区内缓存/同步读 → 写 `tool_preview` + `tool_preview_task`，由现有 refresh/`fill` 绘制；缓存未就绪不算文件消失 |
| `webview` 下已写文件画不出来（空面板） | **高** | §2.0/§2.6：只有 `preview_docs` 非空才跳过 `code()` 并把 `built` 置 `None`；为空时隐藏宿主控件（不销毁）再 `fill`。回到同一批已写文件不因 `detail.rs:204-206` 的旧键早退 |
| sticky 当帧手装 `code()` 被下一帧拆掉 | **高** | §2.4：写回 `tool_preview` 让谓词与 `built` 看见同一份字符串，不旁路装控件 |
| macOS 面板几何/抢焦点 | 中 | §3 ③：point 坐标、X 相对锚点、`Y` 用**主显示器**高度；非激活面板 + 点击取焦的取焦契约；禁 `CGEventTap`/`activateIgnoringOtherApps` |
| WebProcess 崩溃检测缺失 | 中 | helper 监听 `web-process-crashed`（`CRASHED` 行 + 非零退出码）；Rust 兜底超时重启一次再回退 |
| helper 拿不到预览 URL（随机端口 + token） | 中 | §1.2 spawn 契约：完整 URL 经 stdin 一行 `PLUG_URL` 交 helper |
| Windows 取不到子 HWND / bounds 单位错 / WebView2 不停 | 中 | §1.3/§2.6：`gdk_win32_window_get_handle` 自声明取 HWND；`Rect` 用 `Logical*`；隐藏时 `webview.set_visible(false)` |
| Windows gnu 链接 WebView2Loader 失败 | 中 | ⑤ 门禁：`x86_64-pc-windows-gnu` 的 `cargo build` + 加载器来源声明；不可行则明示切 MSVC 并改 `build-windows.sh` |
| Windows 无验证机 | 中 | 步骤 ⑤ 前置；CI 软失败（`continue-on-error`）不算门禁 |
| wry MSRV 钉成 channel | 低 | workspace `rust-version = "1.85"`（下限），不加根 `rust-toolchain.toml` |
| `/session` 轮询空转耗电 | 低 | `since` 参数 + 204；500 ms、空闲降频 2 s、**不暂停**；恢复轮询立即补拉（§2.3） |
| 页内关最后一页误设 `artifact_dismissed` | 低 | §2.2：只 `detail_visible=false`，维持"下次写入仍弹出" |
| 预览栈/缓存无上限 | 低 | 栈上限 12、按 path 去重时复用 id；清空/挤出同锁删字节 |
| CM6 在 WebKitGTK 2.28 缺 `ResizeObserver` | **高（有出口）** | ④a 探针；失败 = Linux 保持原生 |
| bundle 体积/许可证/绝对路径 | 低 | metafile 进评审；许可证核对；构建期断言 |
| 两套标签并存冲突 | 中 | KD6：仅 `preview_docs` 非空时页面独占标签条；已写文件路径原生 `eb-tab` 可见 |
| ui-spec §2 决策变更未确认 | **高（阻塞）** | 确认前不进入步骤 ③④⑤⑥（①② 不受影响） |

## 5. 非目标

- 不做应用内编辑保存（只读；编辑留 flag 默认关）。
- 不迁移会话消息内的 markdown 渲染块。
- 不升级 Linux WebKitGTK（锁死 2.28.1，按它写前端）。
- 不引入 webkit2gtk/sourceview Rust 绑定（§1.1）；GTK4 迁移时由
  `sourceview5`/`wry` 体系自然替换。
- 不改动现役 39473 静态服务及其调用点。

## 6. 待决问题（Open Questions）

1. **ui-spec §2 决策变更确认**（阻塞 ③④⑤⑥）。
2. **Windows 下限**：正式放弃 Win7/8.1？
3. **Windows 验证机**（阻塞 ⑤ 真机验收）。
4. **XEmbed 探针结果**（决定 Linux 是否有 WebView 预览）。
5. 页内标签与已写文件的长期关系（待 macOS 落地后评估）。

## 7. 评审意见处置映射

- 一轮 13 项（[review-1](codemirror-preview-review-1.md)）：全部落实；
  二轮"Resolved"清单确认维持。
- 二轮 11 项（[design-review](codemirror-preview-design-review.md)）：
  v3→v4 落实；三轮"Resolved"段确认维持。
- 三轮 9 项（design-review v4 版）：v4→v5 落实；四轮"Resolved"段确认
  维持。
- 四轮 5 项（design-review v5 版）：v5→v6 落实；五轮"Resolved"段确认
  维持。映射：Issue 1 → §3 ④b；Issue 19 → §3 ③；Issue 26 → §2.3；
  Issue 27 → §2.0 + §2.2/§2.4/§2.5/§2.6；Issue 28 → §2.3。
- 五轮 6 项（design-review v6 版）：Issue 29 → §4 风险行（谓词随
  ③④b⑤ 生效，步骤 ② 只锁定今天行为）；Issue 30 → §2.0/§2.2/§2.6
  （`webview` 下 `preview_docs` 为空走现有 `fill`，不出现空面板）；
  Issue 31 → §2.4/§2.5（sticky 在临界区内写回 `tool_preview`，
  不旁路装 `code()`）；Issue 32 → §3 ③（`size-allocate` +
  `configure-event` 双触发，`wx/wy` 用 `toplevel.position()`）；
  Issue 33 → §2.2/§3 ②（`begin_task` 不清 `tool_preview`，与既有
  测试一致）；Issue 34 → §2.0（复制按钮按模式取数）。
- 六轮 5 项（design-review v7 版）：Issue 35 → §2.0/§2.6/§3 ③④b⑤/§4
  （同一段面板手续：非空则在 `fill` 之前分支、`clear` 跳过宿主控件、
  藏 `eb-tab`、`built = None`；为空则隐藏不销毁再 `fill`，旧键不得
  早退）；Issue 36 → §2.0 与 §2.6 同一句，按当前画面取数；Issue 37 →
  §3 ②（`begin_task` 仍不清 `tool_preview`，`open_task`/`new_task`
  仍清）；Issue 38 → 文首「v6 收 6 项」；Issue 39 → §2.4 写成
  步骤 ③④b⑤。
- 七轮 17 项（design-review v8 版）：Issue 40 → §2.0/§2.6/§3 ③④b⑤/§4
  （隐藏放到 `fill` 之后 + `no_show_all`，`rebuild`/`select` 的宿主
  豁免）；Issue 41 → §2.3（空闲降频、不暂停、恢复补拉、唤醒通道）；
  Issue 42 → §2.3（bump 契约 + `/file` 503）；Issue 43 → §1.2/§2.3
  （URL 走 stdin、`Host`/`Origin` 校验）；Issue 44 → §1.3/§2.6/§3 ⑤
  （`gdk_win32_window_get_handle`、`Logical*`、`webview.set_visible`）；
  Issue 45 → §3 ③（主显示器高度 + 取焦契约 + 多屏验收）；Issue 46 →
  §2.1/§2.3/§2.4/§3 ①②（共享 `read_preview`，5 MiB/字符边界统一）；
  Issue 47 → §2.2/§2.3/§2.5（事件表 + 页面 `error` 触发 sticky）；
  Issue 48 → §1.3/§3 ⑤/§4（gnu `cargo build` + `WebView2Loader` 来源，
  CI 软失败不算门禁）；Issue 49 → §1.3（`default-features = false`，
  更正 `os-webview` 说明）；Issue 50 → §2.0/§2.6（`detail::detach_native_body`
  / `show_host`，webview 分支跳过 `select`）；Issue 51 → §2.2/§2.3
  （复用 id、登记时取 `file_id`、读盘比对；命中复用要重读）；Issue 52 → §2.1/§2.2/
  §2.0（删 `lang`、标签消歧、藏 `+`）；Issue 53 → §1.2（`GDK_BACKEND`
  锚点改为 `main` 开头）；Issue 54 → §2.3（连接模型数值）；Issue 55 →
  §2.1/§2.3（`is_gtk_application_prefer_dark_theme`，主题切换 bump）；
  Issue 56 → §2.2（`save` 手写字段；若序列化 `App` 则 `serde(skip)`）。
- 八轮复核（design-review v9 版）：v8 的 17 项逐条验证通过，**无
  blocking**。复核中自查并修正 7 处：§1.2 标题（最小收养已验证，
  键盘/IME/HiDPI 未做）；§2.2/§2.3 复用 id 时丢弃旧字节重读；
  `dev/ino` 抽象为跨平台 `file_id`；§2.3 的 `open-docs` 元素形状；
  `close-tab` 关掉 `active` 时的交接；§2.1 补标签同名消歧；§3 ①② 的
  bump 验收按步骤归属。
