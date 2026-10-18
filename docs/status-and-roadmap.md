# EvoBuddy 状态与路线图

> 截至 2026-10-18(v0.4.0)。本文回答两个问题:现在站在哪里,接下来往哪走。
> 逐条测量的细节以 `ui-spec.md` 为准;本文只保留决策、证据指针与计划。
> **规划只在本文件维护**;spec §10 只记录测量、决策、缺口与坑,不再保留"下一步"清单。

## 一、当前状态

### 1.1 已验证并发布

| 平台 | 产物 | 自包含验证方式 | 状态 |
|---|---|---|---|
| Linux x86_64 / aarch64 | `evobuddy-<v>-linux-<arch>.tar.gz`(自包含目录树)+ `.AppImage`(单文件) | 裸 Debian 10.13(无 GTK/Pango/Cairo/fontconfig/**0 字体**)真跑;AppImage 与目录树渲染**逐字节一致** | ✅ CI 每次打 tag 强制 |
| macOS arm64 / x86_64 | `evobuddy-<v>-macos-<arch>.tar.gz`(内含自包含 `EvoBuddy.app`) | 每条 load command 静态审计 + **把 Homebrew prefix `chmod 000` 藏起来**真跑几何自检 | ✅ CI(macos-15 / macos-15-intel) |
| Windows x86_64 | `evobuddy-<v>-windows-x86_64.zip` | **无验证环境**;只带 DLL 不带 GTK 数据文件 | ⏸ 刻意挂起(软失败,不阻塞发布) |

- 已发布: v0.2.0(首个正式版)→ v0.2.1(+单文件 AppImage)→ v0.3.0(+macOS 双架构)→ v0.4.0(插件化骨架;run 39089801107 六任务全绿,14 个资产已发布)。
- **发布闸门是 Linux 双架构 + macOS 双架构必须绿**;Windows 任务在 workflow 里是 `continue-on-error` 软失败,不阻塞 release。"六任务全绿"是 v0.4.0 的实际结果,不是固定条件。
- 下限:Linux 要求 glibc ≥ 2.28(精确等于,无余量;Rust std 决定);macOS 下限取自 libgtk 的 `LC_BUILD_VERSION`(随 Homebrew 档次浮动);Windows 仅 UCRT64(Win10+)。

### 1.2 UI 复刻状态

- 主路由(欢迎页、任务会话页、artifact 面板)按 CDP 实测几何搭建,几何逐项可测(`EB_SELFTEST` 输出与规格书数值对齐)。
- 文案、图标为**自有占位**(产品文案与品牌资产不复制,法律边界见 spec §9);实测过的产品字体是改名自包含字体,**不得打包**——这是全项目唯一影响所有文本宽度的既知偏差(Linux 带 DejaVu+WQY:hero 151;macOS 用系统栈:hero 137,与规格书 macOS 实测一致)。
- spec §10 共 **24 个编号条目**,不是 24 条同质的"偏差":按性质分,**18 个待办/未验证、3 个记录在案的选择、2 个已完成、1 组 GTK 坑**。完整清单与证据在 spec §10;其中待办项分散在下面的 P0–P5 与远期。

### 1.3 插件化(M1 已完成,v0.4.0)

仓库已 workspace 化,边界由 Cargo 强制:

```
crates/eb           GTK 宿主:渲染器、装配、EB_* 审计(拥有全部 widget 与渲染策略)
crates/eb-api       插件唯一可依赖的 surface:数据模型 + 贡献注册表 + EbPlugin/AnswerSource
crates/plugins/     内置插件:nav / notices / composer / demo-agent
vendor/rutis        vendored 插件运行时(收窄 tokio 特性;原型裁决,见 prototype/probes/README.md)
prototype/          dsh 兼容测量工作区(不进 UI 构建;kernel 引 ../vendor/rutis)
```

两个已定架构决策(spec §2):
1. **数据插件 + 宿主渲染**:插件产出纯值,永不触碰 widget。线程契约一句话;像素纪律单点收敛。
2. **M1 不设内核线程**:内置插件无异步工作,单线程 current-thread tokio 挂载期 `block_on` 即可。**线程桥推迟到第一个异步服务/插件出现时**(demo-agent 的替代者)。

**M2.0a/b 已落地(2026-10-18)**:`AnswerSource` 从同步整轮返回改为 `start(prompt, sink) -> handle`;`AgentEvent` 经 `AgentSink`/`AgentStream` 回传;`AgentRuntime` 用专用线程跑 current-thread tokio,`Runner` 在 GTK 主线程 drain。`AgentSink` 是容量 64 的有界队列(满时阻塞 producer,不阻塞 UI);`start` 的 panic 被 `catch_unwind` 转成 `Failed`;send 按钮在流式期间变成 stop(32×32 几何不变)。demo agent 同步发射,行为与像素不变;`EB_FAKE_AGENT=1` 的 fake agent 产生慢流,`EB_FAKE_DEMO=1`/`EB_CANCEL_AFTER_MS`/`EB_FAKE_STOP_AFTER_MS`/`EB_FAKE_PANIC_START` 覆盖完成、API 取消、UI 取消与启动 panic。

**等值证据**:M1 改造前后 welcome/demo 渲染 PNG SHA 逐字节一致(`ebb489b4…` / `5b0eef6b…`,历史证据);M2.2 有意改了 table 字号、会话宽度与输入卡 2px,当前 baseline 是 welcome `c3736f11…` / demo `fd3fbb89…`（P2 自绘图标/吉祥物后）。点击审计 **26 步(31 行 `click:` 输出)** 的步骤与文案全等。审计有三处**环境敏感**字段:toplevels 计数(macOS 有时预建隐藏 `GtkTooltipWindow`)、popover 绝对坐标(含窗口服务选择的窗口原点)、以及**代码区滚动宽度**(本机连续运行观察到 579/656 与 579/646 两种读数)。同二进制两次运行也可能不同(已用 stash 实验在旧树上证明);比较审计看步骤与文案,数值列不要整行 diff。

### 1.4 验证体系与实际边界

- `EB_SELFTEST` 几何自检 + `EB_SHOT` 离屏截图(`gtk_widget_draw` 前强制重分配,坑已记录)+ `EB_CLICK` 26 步点击审计 + `EB_ENTER` 发送链路;`EB_ASSERT` / `EB_CLICK_ASSERT` 把报告/审计变成退出码断言(P0.1/P0.2)。
- Linux:`packaging/linux/{package,verify}.sh` —— 容器内 glibc 2.28 构建 + 裸机验证 + `install.sh` 实跑 + AppImage≈tree 断言。裸机验证同时跑 `EB_ASSERT`、`EB_CLICK_ASSERT` 与 fake streaming/cancel/stop/panic;断言模式下 CSS 加载失败、几何不符或某一步交互失效都会让 job 变红。
- macOS:`packaging/macos/{bundle,verify}.sh` —— load command 审计 + 藏 Homebrew 真跑;`EB_ASSERT` 断言固定块(屏幕太小时只断言屏幕无关不变量),`EB_CLICK_ASSERT` 跑 26 步交互,fake streaming/cancel/stop/panic 再跑一次。verify 里原有的三条 grep 保留为快速读数。
- CI(tag-only):每次 `v*` tag 构建四个平台产物;Linux/macOS 的开发者与 CI 调用同一对脚本。Windows 只有 CI 脚本,没有等价的开发者/真机验证路径。另新增 `.github/workflows/check.yml`:push/PR 跑 `cargo test --workspace`。
- **剩余缺口**:`EB_ENTER` 仍只在本地手动跑;还没有 PR 级的截图/像素回归;`Contribs` 仍是挂载期快照,没有热更新通道。
- spec:`docs/ui-spec/ui-spec.md` —— 测量、决策、缺口、坑的唯一权威记录。本地启动与插件手工验证清单见 [`docs/local-verification.md`](local-verification.md)。

## 二、本轮(2026-10-17/18)完成的事

1. **v0.2.1**:Linux 发布形态补齐单文件 AppImage;发现并绕过"qemu/Rosetta 拒绝 AppImage runtime 的 `EI_ABIVERSION='AI'` 字节"的环境限制(PLATFORM 默认宿主原生架构;CI 双真实 runner 各建各验)。
2. **v0.3.0**:macOS 双架构自包含 .app。新坑记录于 `packaging/macos/README.md`(CoreText 使 `FONTCONFIG_FILE` 无效故不带字体;re-export 壳 loader 需用 `.dylib` 本体;未签名代码 dlopen 即 SIGKILL 且 `--deep` 不签 Resources 下的 Mach-O;macOS 26 移除 `DYLD_PRINT_*` 改用藏 prefix)。
3. **插件化调研与裁决**:确认 `prototype/` 可作基础——原型验证的 rutis 进程内用法成为 UI 宿主的运行时;wasm/Node 路线保留为 dsh 兼容层(其核心结论"同步返回活对象的注册表必须与插件同进程"对 UI 宿主天然成立)。
4. **v0.4.0(M1)**:workspace 化 + eb-api surface + 4 个内置插件 + 等值迁移(证据见 §1.3)。过程中 CI/容器各抓一坑:CI runner 屏幕小于规格窗口导致几何断言过严(改为断言屏幕无关不变量);SVG 挪目录后构建镜像缺 COPY;TOML 的 `[target.'cfg(...)'.dependencies]` 表吞掉其后所有依赖(macOS 能编、容器报 unlinked crate)。
5. **P0 验证加固**:`EB_ASSERT` / `EB_CLICK_ASSERT` 落地并接入 Linux 裸机与 macOS verify;新增 `.github/workflows/check.yml`(push/PR `cargo test --workspace`);Linux aarch64 与 macOS arm64 均通过。计划文档校正(24 条计数、26 步/31 行、三处环境敏感字段、发布闸门与 spec 计划指针)。
6. **M2.0a + P0.4**:`AnswerSource` 改为事件流契约;M2.0a 时 demo 保持 `5b0eef6b…` 等值(M2.2/P2 后 baseline 变为 `fd3fbb89…`);fake streaming agent、完成/取消 harness、模型/事件/取消测试全部落地;Linux 与 macOS verify 都跑 fake streaming/cancel/stop/panic。
7. **M2.0b**:`AgentRuntime` 专用线程 + current-thread tokio;`AgentSink` 改为容量 64 的有界队列;`start` panic 经 `catch_unwind` 转 `Failed`;send 按钮流式期间变 stop;Linux/macOS verify 增加 stop 与 panic 用例。
8. **M2.1 采集**:WorkBuddy AI 5.8.4 的 8 个状态已落 spec §5.3 与 fixtures;修正了 752/500 会话宽度;`cr-status-chips`/`--summary` 未出现。
9. **P1a + M2.2(进行中)**:token 生成器新增 metrics(json/rs/css)与 CI 防漂移;`Block::Diff`/`Block::Exec`/`Block::Group`、diff/exec/table 渲染器、752/500 会话宽度与 832/580 输入区、usage 环/最小 popover 已落地;`EB_DEMO_BLOCKS=1` 覆盖新块,Linux/macOS verify 通过。四类 usage 分解与贡献面仍待做。
10. **M2.3 ACP 真实对话**:新增 `eb-plugin-acp-agent`(JSON-RPC stdio),接 dsh/opencode 等 ACP agent;文本流、usage、exec/diff/tool、权限策略、cancel 与跨 turn session 已落;fake ACP 5 测试,真实 dsh/opencode 链路已验证。配置见 [`docs/acp-agent.md`](acp-agent.md)。

## 三、后续规划

按依赖关系排序;每个条目标注规模(小=小时级,中=天级,大=周级)与前置条件。

### P0 验证加固(小)✅ P0.1–P0.4 已落地

M2.2 会动消息块、工具卡与结果面板。现有 CI 只能证明"产物自包含",不能证明"像素/交互没退化";在改 UI 之前先把审计变成断言:

- **P0.1 `EB_ASSERT=1`**(已完成):按平台断言几何/颜色,以退出码驱动 CI。Linux(Xvfb 1920×1080)断言精确窗口与固定块;macOS runner 屏幕更小时只断言屏幕无关不变量。
- **P0.2 `EB_CLICK_ASSERT=1`**(已完成):对 26 步的 label 与稳定观察值做断言,忽略 toplevels、popover 坐标、代码区滚动宽度等易变数值;Linux 裸机与 macOS verify 都已接线并通过。
- **P0.3 快速检查**(已完成):新增 `.github/workflows/check.yml`,push/PR 跑 `cargo test --workspace`;tag-only 的发布链保持独立,不把发布条件放宽。测试包含 26 步期望表覆盖与"缺一个观察值必须失败"的负向用例。
- **P0.4 假插件测试**(已完成):`EB_FAKE_AGENT=1` 的慢流 fake agent + `EB_FAKE_DEMO=1`/`EB_CANCEL_AFTER_MS`/`EB_FAKE_STOP_AFTER_MS`/`EB_FAKE_PANIC_START` harness;Linux 与 macOS verify 都验证完成、取消、UI stop 与启动 panic;`eb-api` 5 个、demo-agent 5 个、acp-agent 5 个测试覆盖事件顺序、取消、sink 关闭、有界队列背压、第二 demo 的三种新块与 ACP 协议。
- 验收:P0.1/P0.2 在 Linux 与 macOS 的 `verify` 脚本里执行;P0.4 额外跑 fake streaming/API cancel/UI stop/start panic。正向验证已完成;故意改坏几何/交互的完整负向演练仍可补做。

### M2 真实异步 agent + 会话内容块(大)⭐ 推荐下一步

**M2.0a 异步契约**(中)✅ 已完成:`AnswerSource` 改为 `start(prompt, sink) -> handle`;`AgentEvent`(started/delta/block/artifact/usage/finished/failed)、`AgentSink`/`AgentStream`、`AnswerHandle` 落在 `eb-api`;`App::begin_task`/`apply_agent_event` 增量构建 agent turn;`Runner` 在 GTK 主线程 drain。协议与实现映射见 [`docs/m2-async-agent.md`](m2-async-agent.md)。dsh 边界仍按 A 方案(进程内 Rust)先走,sidecar 留后。

**M2.0b 运行时桥**(中)✅ 已完成:`AgentRuntime` 用专用线程跑 current-thread tokio,`AnswerSource::start` 在该线程执行;`AgentSink` 改为容量 64 的 `sync_channel`(满时阻塞 producer);`start` 的 panic 被 `catch_unwind` 转成 `Failed`;send 按钮流式期间变 stop,点击走同一个 `Runner::cancel`。`Contribs` 仍是挂载期快照,运行期内容只走 `AgentEvent`;真正的贡献热更新留到出现需求时再做。

**M2.0c 假流式插件 + 测试**(小-中)✅ 已完成:fake agent 走 `EB_FAKE_AGENT=1`,分块、带延迟、可取消;`EB_FAKE_DEMO=1` 等流结束退出,`EB_CANCEL_AFTER_MS` 验证取消;Linux/macOS verify 和单元测试覆盖。

**M2.0 验收**(当时):welcome `ebb489b4…`、demo `5b0eef6b…`;当前 baseline 为 welcome `c3736f11…` / demo `fd3fbb89…`。fake 完成/API 取消/UI stop/启动 panic 在 Linux 与 macOS verify 通过;有界队列背压与 sink 关闭有单元测试。

**M2.1 产品补采集**(前置,外部长周期)✅ 已完成(2026-10-18):WorkBuddy AI 5.8.4 / macOS 26.2 / 1440×868 / DPR2 / en-US,只读 CDP 采集了 `cr-collapse` 折叠+完成+展开、`cr-tool-diff` all-add 与 mixed、`cr-tool-exec` 200 行长输出、`cr-table-block` 默认与 artifact 打开、`cr-context-usage` popover、`cr-agent__processing-status` 运行/完成态。数值与类名落在 spec **§5.3**,可复用 fixture 在 [`docs/ui-spec/fixtures/blocks/captured-5.8.4.json`](ui-spec/fixtures/blocks/captured-5.8.4.json)。**采集修正**:无 artifact 时会话 frame 是 752,打开后收窄到 500;`cr-collapse--summary` 与 `cr-status-chips` 在本次 5.8.4 采集中未出现,标为未采集。

**M2.2 内容块模型 + 插件化**(中)🚧 进行中:
- ✅ `Block::Diff` / `Block::Exec` 数据模型;diff 行级 add/remove/context;demo 第二个回答覆盖三种块。
- ✅ diff 卡(浅绿、行级配色、252 视口)、exec 卡(`#f2f2f2`、300 视口、200 行输出)、table 字号/表头样式。
- ✅ 会话 frame 752(无 artifact)/500(有 artifact),输入区 832/580;`EB_ASSERT` 已断言 `eb-messages`/`eb-chat-card` 宽度;Linux/macOS verify 增加 `EB_DEMO_BLOCKS=1`。
- ✅ `cr-collapse` 工具组(收/展,chevron 在标题后)、`cr-context-usage` 环(仅在 agent 上报 Usage 时显示)与最小 popover;demo 第二个回答用 Group 包住 diff/exec/table。
- ✅ 关键运行时修复(2026-10-18):`Views` 在 `build_ui` 返回后被 drop,refresh 的 `Weak` 全部失效;现保留到进程生命周期。另修 primary monitor 定位与 agent 事件直接重绘。真实 opencode 普通运行路径已能显示对话。
- ⏳ 尚未做:usage popover 的五类分解(采集到但事件流没有分类数据)、`ConversationBlock` 贡献面、未知块 fallback、旧 CSS 字面量迁移。

**验收**:每块几何与配色进 spec,并进入 `EB_SELFTEST` / `EB_CLICK_ASSERT` 的断言范围;M2.0c 的 fake agent 在 CI 可跑。

**M2.3 ACP 真实对话**(中)✅ 已完成(2026-10-18):
- 新插件 `crates/plugins/eb-plugin-acp-agent`:JSON-RPC stdio 启动外部 ACP agent,dsh/opencode 等无需改宿主;配置 `EVOBUDDY_ACP_COMMAND` 时替代 demo。
- 映射 `agent_message_chunk`→Delta、`usage_update`→Usage、exec/diff/tool_call→`Block::Exec/Diff/Tool`;session 跨 turn 保留。
- `session/cancel` 接入 UI stop;权限请求走输入框上方的 allow/reject 弹层,`EVOBUDDY_ACP_AUTO_APPROVE=1` 可绕过。
- fake ACP 5 个测试(含 cancel/permission);真实 dsh 的 initialize/session/prompt 链路已验证(缺 `DEEPSEEK_API_KEY` 时错误可换行显示),真实 opencode 已流式输出并渲染 tool/exec 块。
- 配置、限制与截图 harness 见 [`docs/acp-agent.md`](acp-agent.md)。

### P1 像素精修包(小-中,可并行穿插)

**P1a(先于 M2.2)**
- ✅ 缺口 6(部分):生成器新增 `tokens-metrics.json` / `tokens-metrics.rs` / `tokens-metrics.css`(spacing/radius/font-size/line-height/weight),`spec.rs` 已用生成常量,CI 跑 `check_tokens.sh` 防漂移;新内容块用生成的 fs/radius 类与 spacing 常量。**旧 `ui/gtk.css` 的结构性字面量尚未全部迁移**。
- 缺口 3:输入卡 `box-shadow` 被 GTK 裁剪——Cairo 自绘或外层留 padding(需对照产品截图定方案)。
- ✅ 缺口 4:welcome 输入卡改为 800×144,会话卡 531/783×134(产品 532/784×134,1px 为 sash 折中)。

**P1b(M2.1 已采集,可做)**
- ✅ 缺口 16(部分):工具状态/详情、错误状态、diff/exec 长内容已加 ellipsize/width_chars/滚动约束,不再把窗口撑宽;未逐像素比对长标签。
- ✅ 缺口 13/9:接受 1px sash 折中;当前弹层是窗口内 popover,不需要额外 z-index 表,记录为已接受。
- ✅ 缺口 14:会话顶栏左内缩改为 24px(产品 x=304;实现此前 297)。
- 缺口 16:长标签省略实测(Monaco 近似体的行宽/换行差异一并记录)。
- 缺口 13(三栏 1px sash)与缺口 9(z-index 层级表):补进 P1,或明确写成"接受/延后"。

### P2 图标与吉祥物(中)✅ 已完成(2026-10-18)

- 23 个自有 PNG 图标 + 120px 吉祥物,由 `crates/eb/assets/gen_icons.py` 从几何图元生成;`include_bytes!` 嵌入二进制,不依赖图标主题/librsvg/文件路径。
- 资产许可:仓库 MIT,见 [`crates/eb/assets/README.md`](../crates/eb/assets/README.md)。
- 已替换侧栏、顶栏、工具行、文件面板、composer、agent/brand mark;欢迎页吉祥物用 overlay 放置,不改 hero/输入卡几何。

### P3 Linux 包内 CJK 输入(小)

- 缺口 22:带 `im-ibus.so` + `libibus-1.0.so.0`(并验证 `GTK_IM_MODULE=ibus` 可达),否则自带栈里中文输入只有 XIM,现代桌面基本不可用。对日常自用是实打实的提升。除 ibus 外,至少把 fcitx5/XIM 的降级路径写进验证清单。

### P4 人工核验清单(小,需要真窗口)✅ 清单已落地

- 清单见 [`docs/manual-verification.md`](manual-verification.md):ACP 文本/权限、dsh、多屏位置、IME 组合态、失败日志收集。
- 缺口 18/21 仍需真实窗口逐项打勾;自动化无法模拟 IME 组合与真实权限点击。

### P5 内容词汇与文案(中)

- 缺口 8:i18n 分片(`zh-cn-*.js`)可作为屏幕/状态枚举的**键**来源,值得提取成表;只提取 key/状态分类,**不复制产品文案值**。
- 缺口 5:占位文案替换为 EvoBuddy 自己的成套文案(依赖 8 的枚举)。

### 远期/挂起

- **dsh 兼容**(prototype 路线 A):Node+Cordis 宿主、dsh 插件原样装载、Rust 兑现 fs/search 服务——原型已通 read 工具全链路;扩展 write/edit、agent-loop/session 注册表是后续工作。UI 侧在 M2.0a 定 agent 边界时决定 in-process 还是 sidecar,避免异步协议返工。
- **Windows 自包含**(缺口 23/24):需真机;`.cmd` 启动器做路径改写 + Win7/8.1 可行性定论。当前 zip 文档如实标注"未验证"。
- **macOS 公证 / .dmg**:需要 Apple Developer ID;`hdiutil` 一步之遥。
- **缺口 7**:UI 字族在 token 层定义并整体替换重测(影响全部文本宽度,动一次全量回归)。
- **发布合规闸门**(建议随下一个 release 落地):仓库目前没有根 `LICENSE` 文件,发布物也没有 third-party notices / SBOM / LGPL source offer。自包含包包含 GTK(LGPL)、字体、图标;补 `LICENSE`、第三方许可清单与 GTK 对应源码获取方式,P2/P5 新增资产一律先过许可审查。

## 四、风险与已知限制

1. **glibc 2.28 无余量**:任何新依赖不得引入更高符号版本;`bundle.sh` 收尾的 readelf 报告是每次打包的闸门。
2. **字体替换是全量文本宽度变化**(缺口 7):动它必须全量重测几何,一次做完,不要渐进。
3. **审计的数值列不稳定**(见 §1.3):P0.2 只断言 label 与稳定文案,不对数值列做 diff;若要字节级 diff,需先把 toplevels、popover 坐标、代码区滚动宽度稳定化。
4. **CI runner 屏幕 < 1440×868**:macOS/任何真屏验证几何时,窗口会被钳到工作区——断言用屏幕无关不变量(spec 已记录);P0.1 已把这个规则写进退出码。
5. **Windows 无验证环境**:在拿到真机前,不要把 Windows 产物从"未验证"升级为"可用"。
6. **rutis 0.11 是 pre-1.0**:vendored fork 是唯一事实源;上游若发 1.0,评估回迁(收窄 tokio 特性的 3 行补丁值得向上游提)。
7. **M2.0b 已交付,ACP 也已接入**:runtime 线程、有界队列、UI stop、panic 隔离已做;真实 agent 的流式文本、tool/exec/diff、usage 与 cancel 已验证。剩余风险是权限记忆/历史、agent 认证/会话恢复与长输出布局。
8. **发布合规风险**:LGPL 动态库随包分发、字体/图标再分发、产品文案/i18n 的边界,都需要在发布前明确记录。

## 五、近期待办(按序)

1. ~~v0.4.0 CI 收尾确认;结果已记进 spec~~(已完成,2026-10-18)。
2. ~~P0 验证加固:`EB_ASSERT` / `EB_CLICK_ASSERT` + Linux/macOS verify 接线 + 快速检查~~(P0.1–P0.4 已完成,2026-10-18)。
3. ~~M2.1 采集:清单见 [`docs/m2-capture-checklist.md`](m2-capture-checklist.md)~~(已完成,2026-10-18;结果见 spec §5.3 与 fixture)。
4. ~~M2.0b:宿主专用 runtime 线程 + 有界 channel + UI 取消按钮~~(已完成,2026-10-18;贡献热更新留后)。
5. **M2.2/M2.3/P2**:diff/exec/table + 752/500 布局、collapse 组、usage 环、ACP 真实对话与权限弹层、自绘图标/吉祥物均已完成;usage 分类/贡献面待做。
6. **P1a 缺口 6/4 已完成**(生成 metrics + CI 校验;输入卡 2px 已修;旧 CSS 迁移留后);缺口 3 与 P3 可穿插;P1b/P2/P4/P5 按依赖推进。
7. **发布合规闸门**:根 LICENSE、第三方许可、SBOM/LGPL source offer。
