# 本地启动与插件验证

这份清单用于在本机启动 EvoBuddy，并分别验证 **UI、四个内置插件、插件边界** 和
**自动化审计**。命令都在仓库根目录执行。

## 1. 准备环境

### macOS

```bash
brew install gtk+3 pkg-config
export PKG_CONFIG_PATH="/opt/homebrew/lib/pkgconfig:$PKG_CONFIG_PATH"
cargo run
```

如果只是验证打包后的自包含 app：

```bash
packaging/macos/bundle.sh
open dist/EvoBuddy.app
# 或直接跑二进制，便于看 stdout
dist/EvoBuddy.app/Contents/MacOS/evobuddy
```

未签名 app 首次启动可能需要右键 → Open，或
`xattr -dr com.apple.quarantine dist/EvoBuddy.app`。

### Linux

```bash
sudo apt install libgtk-3-dev
cargo run
```

打包版：

```bash
packaging/linux/package.sh
dist/app/run
# 或
dist/evobuddy-<version>-linux-<arch>.AppImage
```

`package.sh` 需要 Docker；AppImage 只能在原生架构上执行（qemu/Rosetta 会拒绝
`EI_ABIVERSION='AI'`，见 README）。

## 2. 自动化检查（先跑这一组）

```bash
# 1) 编译、单元测试（26 步期望表 + 负向断言用例）
cargo test --workspace

# 2) 几何 / 颜色断言；失败退出非零
EB_ASSERT=1 cargo run

# 3) 26 步点击审计断言；失败退出非零
EB_CLICK_ASSERT=1 cargo run

# 4) 欢迎态 / 会话态截图（写到 /tmp）
EB_SELFTEST=1 EB_SHOT=/tmp/evobuddy-welcome.png cargo run
EB_DEMO=1 EB_SELFTEST=1 EB_SHOT=/tmp/evobuddy-chat.png cargo run

# 5) 打开模型菜单并截图（检查 popover 内容确实有子控件）
EB_CLICK=1 EB_MENU_SHOT=/tmp/evobuddy-menu.png cargo run

# 6) 查看 widget 可见性/分配，排查“控件在但没显示”
EB_DEBUG=1 EB_SELFTEST=1 cargo run

# 7) fake streaming agent：提交、等流结束、自动退出
EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_DEBUG=1 cargo run

# 8) fake streaming agent + API 取消（应看到 Failed/Cancelled）
EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_DEBUG=1 EB_CANCEL_AFTER_MS=300 \
  EB_FAKE_AGENT_CHUNKS=50 cargo run

# 9) fake streaming agent + UI stop 按钮（点击 send 变体）
EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_DEBUG=1 EB_FAKE_STOP_AFTER_MS=250 \
  EB_FAKE_AGENT_CHUNKS=50 cargo run

# 10) start panic 必须变成 Failed，而不是带崩窗口
EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_DEBUG=1 EB_FAKE_PANIC_START=1 cargo run

# 11) Enter 发送链路
EB_ENTER=1 cargo run
```

期望：

- `cargo test --workspace`：`eb-api` 4 个、`eb-plugin-demo-agent` 4 个、
  `evobuddy` 2 个测试通过，其余 crate 0 测试。
- `EB_ASSERT=1`：最后一行 `selftest: assertions passed`，退出码 0；屏幕小于
  1440×900 时只断言语义不变量（窗口钳到工作区是正常的）。
- `EB_CLICK_ASSERT=1`：最后一行 `click: assertions passed (26 steps)`，退出码 0。
- 截图后可用 `node docs/ui-spec/pngpx.js /tmp/evobuddy-chat.png` 查看像素；
  侧栏 `#f7f7f7`、路由 `#ffffff` 是规格值。
- `EB_FAKE_DEMO`：应打印 `fake-demo: finished`；取消/stop 那两次还要能看到
  `Failed { message: "Cancelled" }`；panic 那次应看到
  `agent: start failed: the agent panicked while starting`。
- `EB_ENTER=1`：应打印 `enter: before=welcome` 和 `enter: after=conversation`。

CI 里的同一条路径：Linux 跑
`packaging/linux/verify.sh`（裸 Debian 10 + `EB_ASSERT` + `EB_CLICK_ASSERT` +
fake streaming/cancel），macOS 跑 `packaging/macos/verify.sh`（藏 Homebrew +
两个断言 + fake streaming/cancel）。

## 3. 手工 UI 检查（重点看插件贡献）

| 区域 | 操作 | 期望 | 来自 |
|---|---|---|---|
| 侧栏 | 看五行导航 | `New Task`、`Assistant BETA`、`Experts · Skills · Connectors`、`Scheduled`、`Library` | nav 插件 |
| 侧栏 | 点 `Assistant` / `Experts…` / `Scheduled` / `Library` | 进入 “not in this build” 路由；点 `New Task` 回欢迎页 | 宿主按 nav 的 `unbuilt_route` 渲染 |
| 欢迎页 | 看六个 chip | `Development`、`Websites`、`Mini Programs`、`Agents`、`Skills`、`Tech Docs` | composer 插件 |
| 欢迎页 | 逐个点 chip | 输入框被填入对应的整句 prompt（不是发送） | composer 插件 |
| 欢迎页 | 点 `Select Workspace` | 菜单 `No workspace` / `Add workspace…` | composer 插件 |
| 欢迎页 | 点 `Default Permissions` | `Default Permissions` / `Auto-approve` / `Read-only` | composer 插件 |
| 欢迎页 | 点 `Auto` | `Auto` / `Fast` / `Deep` | composer 插件 |
| 欢迎页 | 点 `+` | `Add file` / `Add folder` / `Add screenshot` / `Add web page` | composer 插件 |
| 侧栏 | 点搜索 / 小程序 / 筛选 | 搜索与小程序弹 notice；筛选菜单 3 行 | notices / composer |
| 侧栏 | 折叠 / 展开、Tasks 分区折叠 | 可见性与 `▶` 字形切换；任务行隐藏/恢复 | 宿主交互 |
| 会话 | 发送（按钮或 Enter） | 进入会话；第一条 demo 回答含文本 + 工具行 + 表格，并打开 artifact | demo-agent |
| 会话 | 再发送一次（当前实现会新建任务） | 第二条回答是另一种占位文案，且新任务没有 artifact，面板关闭（demo-agent 交替） | demo-agent |
| 会话 | `EB_FAKE_AGENT=1 cargo run` 后点 chip / Enter | 每 120ms 到达一个 `chunk N`，底部显示 streaming footer；流式期间侧栏仍可折叠/切换 | fake agent |
| 会话 | 点 `✎`（Code assist） | `Code assist is not part of this UI-only build.` | notices 插件 |
| 会话 | 顶栏搜索 / 分享 | 各自 notice | notices 插件 |
| 会话 | 消息复制 | tooltip/反馈出现 `Copied` | 宿主 |
| 会话 | 侧栏用户行 | `Settings` / `Keyboard shortcuts` / `Send feedback` / `Sign out` | composer 插件 |
| Artifact | 展开 / 恢复 / 关闭 / 任务行重开 | 面板宽度与可见性变化；重开恢复面板 | 宿主 |
| Artifact | `×` 关闭标签、`+` 新建标签 | 标签关闭；新建标签弹 notice | 宿主 / notices |

注意：`Code assist` 与 workspace/permission 菜单**不在** 26 步自动审计里，手工
检查时重点补这几个。

## 4. 插件边界与运行时验证

### 4.1 依赖边界（不会启动 UI）

```bash
cargo tree -p eb-plugin-nav --edges normal
cargo tree -p eb-plugin-composer --edges normal
cargo tree -p eb-plugin-demo-agent --edges normal
```

每个插件应只有 `eb-api`（再往下是 vendored `rutis`/tokio），**不应出现 `evobuddy`
或 `gtk`**。宿主 `evobuddy` 的依赖里才会同时出现四个插件和 `gtk`。

```bash
cargo metadata --no-deps --format-version 1 |
  python3 -c 'import json,sys; d=json.load(sys.stdin); [print(p["name"], "->", sorted(x["name"] for x in p["dependencies"])) for p in d["packages"] if p["name"].startswith("eb-") or p["name"]=="evobuddy"]'
```

### 4.2 插件不碰 widget

```bash
grep -RIn "gtk" crates/plugins || echo "no gtk references in crates/plugins"
```

应没有任何输出。插件产出的都是 `eb-api` 的纯数据（nav 行、菜单项、notice 文案、
chip、`AnswerSource`）。

### 4.3 贡献缺失会启动失败（可选）

`crates/eb/src/host.rs` 的 `validate()` 会检查 nav、6 个菜单、4 个 notice、
composer copy、chips 和 answer source。想验证这一点，可以临时把
`eb-plugin-notices` 从 `Host::mount()` 的内置插件列表里去掉，或改掉某个
notice 的 id，启动时应直接 panic 并报出缺失的贡献；测试完用
`git checkout -- crates/eb/src/host.rs`（以及对应插件文件）恢复。
**不要在有未提交改动时直接 `git checkout` 整个目录。**

### 4.4 改一条贡献看 UI（可选）

在 `crates/plugins/eb-plugin-nav/src/lib.rs` 临时加一行 `NavRow`，重新
`cargo run`，侧栏应多一行；或在 `eb-plugin-composer` 改一个 chip 的 `prompt`，
点击后输入框内容应跟着变。这能直接证明“插件产出数据、宿主渲染”的边界。
改完用 `git diff` 确认，再按需恢复。

## 5. 自动化审计没有覆盖的部分

以下仍需手工或真实环境：

- `Code assist`、workspace / permission 菜单（见上表）。
- 真实活跃窗口下的 popover 焦点、点击外部关闭（无窗管环境只能看到 inactive）。
- IME 组合态下的 Enter（需要真实输入法）。
- Linux 自带栈的 ibus/fcitx5 输入（缺口 22）。
- 超长 artifact 标签省略、overlay 滚动条观感。
- Windows 产物（无验证环境，仍标“未验证”）。

### M2.0a/b 当前限制

- UI stop 已可用；`EB_FAKE_STOP_AFTER_MS` 点击的就是用户点的那颗按钮。
- `Usage` 事件已定义但模型还没有用量环，暂时忽略。
- `AgentSink` 是容量 64 的有界队列；`AgentRuntime` 已有专用线程，
  fake agent 自己起 worker 线程模拟异步生产者。
- 同一个任务只支持一个在途回答；新提交会把旧回答标成 `Cancelled`。
- `Contribs` 仍是挂载期快照，运行期贡献热更新未做。

## 6. 常见问题

- **`cargo run` 报 GTK 找不到**：确认 `pkg-config --modversion gtk+-3.0` 有输出；
  macOS 设 `PKG_CONFIG_PATH`，Linux 装 `libgtk-3-dev`。
- **命令一闪而过**：带 `EB_*` 标志时应用在报告后自动退出，这是设计行为；想交互
  就裸跑 `cargo run`。
- **点击审计 `window active=false`**：终端启动没有窗管激活窗口，审计会尝试
  `present()`；这不影响断言结果。
- **macOS 自包含 app 打不开**：未签名，右键 Open 或清 quarantine；不要用
  `open` 看日志，直接跑 `Contents/MacOS/evobuddy`。
- **AppImage 执行报 `Exec format error`**：当前在模拟层（qemu/Rosetta），
  AppImage runtime 不能运行；换原生架构。
