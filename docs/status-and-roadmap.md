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
- spec §10 共 **31 个编号条目**(2026-10-26 清点),不是同质的一揽子"偏差":按性质分,**15 个待办/未验证、9 个记录在案的自创选择(19/20/25–31)、6 个已完成 + 1 个部分完成(条目 6)**。完整清单与证据在 spec §10;其中待办项分散在下面的 P0–P5 与远期。**§10-32–40 是之后的补记(转录条/滚动/侧栏行),尚未并入上面那份分类。**

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

**等值证据**:M1 改造前后 welcome/demo 渲染 PNG SHA 逐字节一致(`ebb489b4…` / `5b0eef6b…`,历史证据);M2.2 有意改了 table 字号、会话宽度与输入卡 2px,当前 baseline 是 welcome `c3736f11…` / demo `5d42eebb…`（P2 自绘图标/吉祥物后）。点击审计 **33 步** 的步骤与文案全等(含多文件标签切换与单标签关闭,`EB_ARTIFACT=1` 种子下断言 `tabs=2 → 1`)。审计有三处**环境敏感**字段:toplevels 计数(macOS 有时预建隐藏 `GtkTooltipWindow`)、popover 绝对坐标(含窗口服务选择的窗口原点)、以及**代码区滚动宽度**(本机连续运行观察到 579/656 与 579/646 两种读数)。同二进制两次运行也可能不同(已用 stash 实验在旧树上证明);比较审计看步骤与文案,数值列不要整行 diff。

### 1.4 验证体系与实际边界

- `EB_SELFTEST` 几何自检 + `EB_SHOT` 离屏截图(`gtk_widget_draw` 前强制重分配,坑已记录)+ `EB_CLICK` 33 步点击审计 + `EB_ENTER` 发送链路;`EB_ASSERT` / `EB_CLICK_ASSERT` 把报告/审计变成退出码断言(P0.1/P0.2)。
- Linux:`packaging/linux/{package,verify}.sh` —— 容器内 glibc 2.28 构建 + 裸机验证 + `install.sh` 实跑 + AppImage≈tree 断言。裸机验证同时跑 `EB_ASSERT`、`EB_CLICK_ASSERT` 与 fake streaming/cancel/stop/panic;断言模式下 CSS 加载失败、几何不符或某一步交互失效都会让 job 变红。
- macOS:`packaging/macos/{bundle,verify}.sh` —— load command 审计 + 藏 Homebrew 真跑;`EB_ASSERT` 断言固定块(屏幕太小时只断言屏幕无关不变量),`EB_CLICK_ASSERT` 跑 33 步交互,fake streaming/cancel/stop/panic 再跑一次。verify 里原有的三条 grep 保留为快速读数。
- CI(tag-only):每次 `v*` tag 构建四个平台产物;Linux/macOS 的开发者与 CI 调用同一对脚本。Windows 只有 CI 脚本,没有等价的开发者/真机验证路径。另新增 `.github/workflows/check.yml`:push/PR 跑 `cargo test --workspace`。
- **剩余缺口**:`EB_ENTER` 仍只在本地手动跑;还没有 PR 级的截图/像素回归;`Contribs` 仍是挂载期快照,没有热更新通道。
- spec:`docs/ui-spec/ui-spec.md` —— 测量、决策、缺口、坑的唯一权威记录。本地启动与插件手工验证清单见 [`docs/local-verification.md`](local-verification.md)。

## 二、本轮(2026-10-17/18)完成的事

1. **v0.2.1**:Linux 发布形态补齐单文件 AppImage;发现并绕过"qemu/Rosetta 拒绝 AppImage runtime 的 `EI_ABIVERSION='AI'` 字节"的环境限制(PLATFORM 默认宿主原生架构;CI 双真实 runner 各建各验)。
2. **v0.3.0**:macOS 双架构自包含 .app。新坑记录于 `packaging/macos/README.md`(CoreText 使 `FONTCONFIG_FILE` 无效故不带字体;re-export 壳 loader 需用 `.dylib` 本体;未签名代码 dlopen 即 SIGKILL 且 `--deep` 不签 Resources 下的 Mach-O;macOS 26 移除 `DYLD_PRINT_*` 改用藏 prefix)。
3. **插件化调研与裁决**:确认 `prototype/` 可作基础——原型验证的 rutis 进程内用法成为 UI 宿主的运行时;wasm/Node 路线保留为 dsh 兼容层(其核心结论"同步返回活对象的注册表必须与插件同进程"对 UI 宿主天然成立)。
4. **v0.4.0(M1)**:workspace 化 + eb-api surface + 4 个内置插件 + 等值迁移(证据见 §1.3)。过程中 CI/容器各抓一坑:CI runner 屏幕小于规格窗口导致几何断言过严(改为断言屏幕无关不变量);SVG 挪目录后构建镜像缺 COPY;TOML 的 `[target.'cfg(...)'.dependencies]` 表吞掉其后所有依赖(macOS 能编、容器报 unlinked crate)。
5. **P0 验证加固**:`EB_ASSERT` / `EB_CLICK_ASSERT` 落地并接入 Linux 裸机与 macOS verify;新增 `.github/workflows/check.yml`(push/PR `cargo test --workspace`);Linux aarch64 与 macOS arm64 均通过。计划文档校正(24 条计数、26 步/31 行、三处环境敏感字段、发布闸门与 spec 计划指针)。
6. **M2.0a + P0.4**:`AnswerSource` 改为事件流契约;聊天发送现在续接当前 task(只有欢迎页/New Task 才新增 sidebar task);M2.0a 时 demo 保持 `5b0eef6b…` 等值(M2.2/P2 后 baseline 变为 `5d42eebb…`);fake streaming agent、完成/取消 harness、模型/事件/取消测试全部落地;Linux 与 macOS verify 都跑 fake streaming/cancel/stop/panic。
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
- **P0.4 假插件测试**(已完成):`EB_FAKE_AGENT=1` 的慢流 fake agent + `EB_FAKE_DEMO=1`/`EB_CANCEL_AFTER_MS`/`EB_FAKE_STOP_AFTER_MS`/`EB_FAKE_PANIC_START` harness;Linux 与 macOS verify 都验证完成、取消、UI stop 与启动 panic;`eb-api` 12 个、demo-agent 5 个、acp-agent 5 个测试覆盖事件顺序、取消、sink 关闭、有界队列背压、第二 demo 的三种新块与 ACP 协议。
- 验收:P0.1/P0.2 在 Linux 与 macOS 的 `verify` 脚本里执行;P0.4 额外跑 fake streaming/API cancel/UI stop/start panic。正向验证已完成;故意改坏几何/交互的完整负向演练仍可补做。

### M2 真实异步 agent + 会话内容块(大)⭐ 推荐下一步

**M2.0a 异步契约**(中)✅ 已完成:`AnswerSource` 改为 `start(prompt, sink) -> handle`;`AgentEvent`(started/delta/block/artifact/usage/finished/failed)、`AgentSink`/`AgentStream`、`AnswerHandle` 落在 `eb-api`;`App::begin_task`/`apply_agent_event` 增量构建 agent turn;`Runner` 在 GTK 主线程 drain。协议与实现映射见 [`docs/m2-async-agent.md`](m2-async-agent.md)。dsh 边界仍按 A 方案(进程内 Rust)先走,sidecar 留后。

**M2.0b 运行时桥**(中)✅ 已完成:`AgentRuntime` 用专用线程跑 current-thread tokio,`AnswerSource::start` 在该线程执行;`AgentSink` 改为容量 64 的 `sync_channel`(满时阻塞 producer);`start` 的 panic 被 `catch_unwind` 转成 `Failed`;send 按钮流式期间变 stop,点击走同一个 `Runner::cancel`。`Contribs` 仍是挂载期快照,运行期内容只走 `AgentEvent`;真正的贡献热更新留到出现需求时再做。

**M2.0c 假流式插件 + 测试**(小-中)✅ 已完成:fake agent 走 `EB_FAKE_AGENT=1`,分块、带延迟、可取消;`EB_FAKE_DEMO=1` 等流结束退出,`EB_CANCEL_AFTER_MS` 验证取消;Linux/macOS verify 和单元测试覆盖。

**M2.0 验收**(当时):welcome `ebb489b4…`、demo `5b0eef6b…`;当前 baseline 为 welcome `c3736f11…` / demo `5d42eebb…`。fake 完成/API 取消/UI stop/启动 panic 在 Linux 与 macOS verify 通过;有界队列背压与 sink 关闭有单元测试。

**M2.1 产品补采集**(前置,外部长周期)✅ 已完成(2026-10-18):WorkBuddy AI 5.8.4 / macOS 26.2 / 1440×868 / DPR2 / en-US,只读 CDP 采集了 `cr-collapse` 折叠+完成+展开、`cr-tool-diff` all-add 与 mixed、`cr-tool-exec` 200 行长输出、`cr-table-block` 默认与 artifact 打开、`cr-context-usage` popover、`cr-agent__processing-status` 运行/完成态。数值与类名落在 spec **§5.3**,可复用 fixture 在 [`docs/ui-spec/fixtures/blocks/captured-5.8.4.json`](ui-spec/fixtures/blocks/captured-5.8.4.json)。**采集修正**:无 artifact 时会话 frame 是 752,打开后收窄到 500;`cr-collapse--summary` 与 `cr-status-chips` 在本次 5.8.4 采集中未出现,标为未采集。

**M2.2 内容块模型 + 插件化**(中)🚧 进行中:
- ✅ `Block::Diff` / `Block::Exec` 数据模型;diff 行级 add/remove/context;demo 第二个回答覆盖三种块。
- ✅ diff 卡(浅绿、行级配色、252 视口)、exec 卡(`#f2f2f2`、300 视口、200 行输出)、table 字号/表头样式。
- ✅ 会话 frame 752(无 artifact)/500(有 artifact),输入区 832/580;`EB_ASSERT` 已断言 `eb-messages`/`eb-chat-card` 宽度;Linux/macOS verify 增加 `EB_DEMO_BLOCKS=1`。
- ✅ `cr-collapse` 工具组(收/展,chevron 在标题后)、`cr-context-usage` 环(仅在 agent 上报 Usage 时显示)与最小 popover;demo 第二个回答用 Group 包住 diff/exec/table。
- ✅ 对话 Markdown:段落/标题/有序无序列表/行内 code/围栏 code 卡(语言、复制、Java/Bash 轻量高亮、横向滚动、高度上限、24px 正文行高/20px 代码行高);Quine 提示已截图验证。
- ✅ ACP tool_call 聚合为 `Block::Group`(Completed,紧凑 26px 工具行),与产品展开态一致。
- ✅ ACP `plan` 映射为 Plan 卡(完成/进行中);`agent_thought_chunk` 已映射为
  `Block::Thought`(灰斜体思考行,2026-10-24);session resume 待做。
- ✅ 侧栏筛选补全(2026-10-24):`task_status` 从 turns 推导 Working/Completed/
  Failed/Pending/Planning 五态,Failed/Pending/Planning 不再恒空;显式
  `EVOBUDDY_ACP_AGENT` 优先于 harness 的 Demo 强制(ACP 截图配方恢复可用)。
- ✅ 消息快捷按钮(2026-10-24):用户气泡下方补 编辑重发(回填输入框),已完成的
  EvoBuddy 回答下方补 复制回答/点赞/点踩/重新生成(把该轮提问按正常链路再发一次);
  流式期间不画这一行。复制/编辑/重新生成有真实行为并进 33 步点击审计,点赞/点踩按
  §10-19 纪律弹自创 notice。整行属自创 chrome,记录在 ui-spec §10-25。
- ✅ Artifact 侧栏宽度自适应(2026-10-24):面板出现时不再把 sash 钳到
  `窗口-面板自然宽`(1470 窗口实测 728/741 宽)。根因是 GtkPaned
  `gtk_paned_calc_position` 每帧按 `resize=false` 的第二子件重写 position;
  现把面板改为 outer paned 的 resize 子件(`pack1(inner,false,false)` +
  `pack2(detail,true,true)`),sash 由 `outer_sash()` 统一计算:面板恒占
  `ARTIFACT_PANEL_WIDTH=580`,对话列吸收余量,窄窗口下对话保底 500+24 才轮到
  面板压缩。1440 基线逐像素不变(860/579),宽窗口 890/579;configure 事件
  触发重排,`EB_WINDOW_WIDTH/HEIGHT` 测试钩子进 local-verification 配方 17。
- ✅ Artifact 头部按钮不再被裁(2026-10-24):窄面板(如 1200 窗口下约 395px)
  时 ◐/✦/⤢/▥ 四个按钮只露两个。原因是 `.eb-tab` 的 CSS `min-width:224` 把
  tab 钉死,头部放不下时从尾部裁掉动作按钮。现在 240px 自然宽改由
  `label.set_width_chars(24)` 提供,CSS 地板去掉,面板变窄先缩 tab 名
  (中间省略),四个按钮恒可见;1440 基线截图逐像素不变。
- ✅ Artifact 识别放宽(2026-10-24):工具标题里的路径此前含空格即被丢弃
  (macOS "Application Support" 一类路径永不弹面板)。现把"整段剩余"和"首个
  token"都作为候选,由 `resolve_file_artifact` 的存在性读盘裁决:猜错的候选
  无害,带空格路径与"Write /x.py created"式尾注都能命中(agent.rs
  `collect_written_paths` + 扩展单测)。
- ✅ Artifact 多文件标签(2026-10-24):一个 task 写多个文件时面板顶部出现标签
  条,每文件一签、最新默认选中,`×` 只关当前那一个(关到空才收起面板且不留永久
  dismiss),切换走 `rebuild` 重渲染选中文件。`Task.artifacts` 上限 12
  (`MAX_TABS`)。三个坑:①tab 条若直挂 header,其天然宽会把 GtkPaned 的 resize
  子件(面板)钉到 776——包进 `propagate_natural_width=false` 的 ScrolledWindow
  封顶;②`show_all` 之后新增的子件可见但未分配,而 GtkViewport 吞掉子件的
  resize 传播,须在 `rebuild` 里对 window `queue_resize()`;③tab 盒自带
  BUTTON_PRESS_MASK 拥有 input-only(Temp)窗口,合成事件必须带它,且不能在
  press handler 里同步 rebuild(会在 release 前销毁自己)——延迟到
  `idle_add_local_once`。审计 +1 步(30 步),`EB_ARTIFACT` 种子下断言
  `tabs=2→1`、选中文件 `demo.py→notes.md`;记录在 ui-spec §10-26。
- ✅ 筛选弹层重做(2026-10-24):按用户提供的 5.8.4 截图实测重排(spec §5.4)
  ——240 宽圆角卡、37px 灰色分组标题带、31px 行高/34px 间距、选中行
  `#f7f7f7` 圆角 8 药丸 + 加粗 + 行尾 `#57bf9c` 绿勾、`#f2f2f2` 分隔线、
  灰色 Reset Filters;替换旧"✓ 前缀"临时样式。两个坑:①`flat_button` 的
  class 参数不能塞空格分隔的多类名(add_class 按整串处理),要逐个 `class()`;
  ②卡片底色/边框必须画在子 widget(`.eb-filter-body`)上,`> contents` 节点
  背景在离屏快照里不渲染(§10-27)。悬停底色为自创。`EB_FILTER_SHOT` 快照
  钩子进 local-verification 配方 19;审计观察值 `11 of 11 menu rows` 不变。
- ✅ 会话内 artifact 文件卡(2026-10-24):按 5.8.4 截图实测(spec §5.5)——
  已完成回答下最多两张 370×64 圆角 10 灰卡(`#f2f2f2`,24px 蓝徽章 + 文件名
  15px/600 + 字节数 13px),>2 个文件才有 `View all artifacts (N) ›` 链接;
  点卡/链接展开面板并选中对应标签(`App.artifact_select` 一次性消费,
  `detail::select` 落焦)。卡行折进最后一个已完成 agent 轮的 sig,新文件会
  重排卡片;流式期间不画。徽章 `artifact-code/artifact-unknown` 由
  `gen_icons.py` 自绘(扩展名表、首二文件、链接阈值、悬停均为自创,§10-28)。
  审计 +1 步(31 步),种子下断言 `cards=2` 且点第一卡 `demo.py→notes.md`。
- ✅ 顶栏三件套(2026-10-24):按 5 张 5.8.4 截图实测(spec §5.6)——搜索 /
  Share / 右面板开关三枚 32px 按钮常驻(4px 间距),悬停底 `#f0f0f0`(新
  `@eb_icon_hover_bg`,`button.eb-icon:hover` 改用实测值)。点搜索原地展开
  300×32 `#f2f2f2` 药丸(放大镜 14px `#797979`、`^` `v` `×` 24px 步进、
  右内缩 12),标题保持可见;`×`/Esc 收起并清查询。Share tooltip 改采集值
  `Share`、图标重绘为弯曲前向箭头;面板开关用新自绘 `panel-right`(圆角矩形
  + 实心右导轨),开合右面板、tooltip 随状态换 `Open/Close right panel
  ⇧⌘B`,撤掉旧的"有文件才显示"门控(`Wiring.open_artifact`→`toggle_artifact`)。
  ⌘F / ⇧⌘B 真实接入窗口 key-press。审计 +2 步(33 步):药丸展开/收起、
  面板开关关闭;`EB_SEARCH_SHOT` 快照钩子进 local-verification 配方 20。
  自创细节(Esc、无文件时行为、开态 tooltip 措辞、聚焦时占位不渲染)记 §10-29。
- ✅ 后台并发流(2026-10-25):修复"正在执行的 task 在另起新任务/其他会话发言后被
  结束"。`Runner.active` 由单槽 Option 改为 Vec,每个任务各自一条在途流:submit 只
  supersede 同一任务的旧回答,`cancel()`/UI stop 只作用于当前路由任务,drain 轮询全部
  流并写回各自 Task;后台任务的 artifact 不再弹前台面板(`open_task` 切回时按
  `artifact.is_some()` 恢复),dsh bridge 投影仅在该任务仍是活动 session 时写入。
  新增 `EB_TWO_TASK_TEST=1` harness(local-verification 配方 21)与 `eb-api`
  后台 artifact 单测;33 步审计、EB_ASSERT、`cargo test --workspace` 全绿。
  ACP 真实 agent 的子进程 session 仍跨任务串行(插件内锁,属已知限制)。
- ✅ 侧栏在途任务光圈(2026-10-25):运行中任务(最后 agent 轮 `streaming`,判定
  走新 `task_is_running`)在行尾时间戳位置显示 16px 转动灰弧,普通任务行与
  工作区嵌套行共用 `task_row` 同时生效;相位取全局时钟,侧栏整表重建也不跳帧
  (样式自创,记 spec §10-30;capture 仅证"运行行显示 spinner")。
  `EB_SHOT` 流式中途截屏验证(配方 22),`eb-api` +1 单测(12 个)。
- ✅ 偶发界面 hang(2026-10-25):四条主线程阻塞路径全部收口。① `DshAgent::start`
  过去在 AgentRuntime 线程里同步跑完整轮 dsh,违反"`start` 必须尽快返回"的契约,
  于是 `AgentRuntime::start` 的握手 `recv()` 把 GTK 主线程无限挂住——现在整轮搬到
  `evobuddy-dsh-agent` 工作线程。② `bridge.rs` 每个会话调用都是裸 TCP 连接、
  无 socket 超时,桥进程活着但不回包时主线程永久卡在 `read_to_string`:现在统一走
  `exchange()`(连接 150ms、写 1s、普通读 1s、`run_agent` 读 600s),读到超时即进入
  30s 降级冷却,后续调用直接短路;桥地址支持 `EVOBUDDY_BRIDGE_ADDR` 覆盖。
  ③ 握手等待改为 `await_start(.., 10s)`,契约违约表现为一条错误气泡而不是冻结窗口。
  ④ 另外两处:`bridge::ensure()` 的冷启动等待 2s→300ms(`create_session` 只对
  "连接被拒"重试到同一预算,超时类错误立刻返回),Connectors 卡的 `Test` 按钮原本在
  主线程等 MCP 握手最长 10s,现在搬到工作线程并显示 `testing…`;顺带修掉 artifact
  读取先整文件读入再 `truncate(MAX)` 的写法(大文件卡住主线程、切在多字节字符中间
  会 panic)。提交路径在桥往返之前先 `refresh_now()`,用户消息不再被 1s 超时压住。
  新增 `EB_HANG_TEST=1` 黑洞桥 harness(配方 23)、`evobuddy` +2 单测(35 个:
  握手有界、artifact 读取有界且字符安全);33 步审计、EB_ASSERT、两任务并发、
  cancel/stop 全绿。
- ✅ 极窄窗口的三栏保底(2026-10-26):面板开着把窗口拉窄时,分隔条会被 GtkPaned
  clamp 到面板内容最小值,而消息列/输入区的 margin 仍按我们设定的位置算,两者一旦
  不一致气泡就越过分隔条压进面板(1082px 窗口实测:设定 804、实得 750)。现在
  `shell::outer_sash(width, sidebar, open, maximised, panel_floor)` 是唯一的分隔条
  来源,`panel_floor` 取 `detail.root.preferred_width().0` 的**实时**最小值(不是写死
  常量:§10-26 后标签条按内容收缩,标签一变最小值就变),消息列与输入区的 margin 一律
  从返回值推。三段保底:≥1384 保持实测 580 份额;1136–1384 会话先让到 `500+24`;
  <1136 面板保住自己的最小宽、会话降到新常量 `spec::CONVERSATION_MIN_WIDTH=240`。
  窗口有效最小宽实测 1082px(`EB_WINDOW_WIDTH=900` 也只会得到它),窄态下面板头部
  ×/+/◐/⤢ 与顶栏 ⇧⌘B 都仍可达。规则属自创,记 spec §10-31;`evobuddy` +1 单测
  `the_outer_sash_holds_the_captured_share_then_the_two_floors`(36 个,含 640–1920
  区间不变量),`EB_DEBUG=1` 打印 `layout: width=… pos=… detail_min=…` 供核对
  (local-verification 配方 17)。
- ✅ ACP 槽位加 jcode 与 pi(2026-10-26):先确认协议支持再动手。**jcode 原生支持**
  (`jcode acp`,v0.86 实测握手通过,agent 自报 `Jcode`,capability 含
  `loadSession`/`resume`/`close` 与 image+embeddedContext);**pi 本体不支持**——它
  只有私有的 `pi --mode rpc` 行分隔 JSON,ACP 由第三方适配器 `pi-acp@0.0.34` 提供
  (Zed ACP Registry 同款;适配器 spawn `pi --mode rpc`,要求 Node 22+、pi ≥0.81)。
  `agents::AcpAgent` 因此是四槽位:程序+参数统一走新 `launch()`(pi 槽位 PATH 上有
  `pi-acp` 就用,否则 `npx --yes pi-acp@0.0.34`——**不选 pi 就永远不会下载第三方
  代码**),`EVOBUDDY_JCODE_BIN`/`EVOBUDDY_PI_ACP_BIN` 可覆盖;路由的 `acp:<id>` 解析、
  配置读写与 `ACP_MENU` 表一并改成按 id 驱动,composer 路由菜单 5 项 → 7 项(审计
  期望值同步)。新增 `AcpAgent::probe()` 与 `EB_ACP_PROBE=<id>`:只做
  `initialize`+`session/new` 就退出,**不花 token**,退出码 0/1/2,顺带能看出 grok
  未登录(`no auth method id provided`)。实测限制:pi-acp 以 `-32602` 拒我们的 stdio
  MCP 连接器,靠既有"拒绝即去掉 MCP 重试"路径起会话 ⇒ pi 拿不到连接器。
  `evobuddy` +3 单测(39 个)、acp-agent +1(7 个);33 步审计、EB_ASSERT 全绿。
- ✅ **ACP 修复「任务没完成就结束了」(2026-10-27)**:用户用 Grok Build 跑
  「研究 DuckDB 适用场景」,界面只留下"结论稍后给你"就终态。独立 stdio 驱动抓真实
  Grok 会话(23.4 分钟、零 `session/update`、60 条退避)定位到三个各自独立的缺陷:
  ① `prompt()` 读到 `session/prompt` 响应就 `Finished` 且不再读 stdout,后台
  (`deep-research`/子 agent)续流被整段丢弃 → 常驻读线程 + `TailPolicy`
  (2s 探测 / 40s 静默 / 900s 封顶)把续流收进**同一条**回答;② 180s **墙钟**看门狗
  杀掉还在重试的子进程 → 改成按**连续静默**计时,任何一条通知都算活着;③ Grok 把
  `retry_state`/`turn_completed` 装在私有 `_x.ai/session_notification` 里,只匹配
  spec 方法名时 60 条全丢 → `is_update_notification()` 两个信封都认,退避显示为
  `Retrying 6/15: …`,续流阶段的 `stop_reason="error"` 用 `agent_result` 判
  `Failed`(不再"安静地成功")。acp-agent 10 个测试(尾流、双信封重试、续流失败),
  33 步审计/EB_ASSERT/EB_HANG_TEST 全绿;零 token 端到端见 local-verification 配方 25。
- ✅ **流式期间不再抢滚动**(2026-10-27):读长任务时往上翻历史会被每条 delta 拽回
  底部。改成"只在读者已经贴底时跟随"(离底 ≤24px 算在看),重新钉住只由用户动作触发:
  轮数变多(提交/重新生成)、切换任务、自己滚回底部;宿主跳转用 `auto_scroll` 旗标
  与用户滚动区分,搜索跳转不包旗标所以跳完就不再跟随。规则属自创,记 spec §10-33。
  新增 `EB_FOLLOW_TEST` harness(配方 26),负向旁路复现原症状 `yanked the view +348px`。
- ✅ **修多任务切换几次后窗口卡死**(2026-10-27):在几个流式任务间来回点,越点越卡直到
  不动。`EB_SWITCH_TEST=40` 配 3×20 轮长会话存档实测:每次切换都走 `fill_messages` 全量
  分支把整段转录重建(markdown/代码卡/表格/diff 全来一遍,20 轮约 40ms),收尾那句
  `messages.show_all()` 再走全树约 1100 个控件(均值 9.5ms、峰值 25.1ms);三个流叠加上
  16ms drain + 33ms 合并刷新,主循环彻底追不上 → `the main loop starved by 1233ms`。
  两处改动:① 离屏会话的帧按任务索引缓存(`Conversation::detached`,上限
  `DETACHED_CONVERSATIONS=4`,超了丢最久未用的),切回来只逐帧 `pack_start`;② `show_all`
  下沉到刚建出的那一帧。结果(同一存档复测 30 次切换)`worst_late=0ms`
  `worst_switch=0ms`、29/30 次走 re-attach(只有冷缓存那次全量重排)、`conv` 均值
  23.3ms→8.5ms;harness 补第三条不变量(帧必须可见且有非零分配)
  与 `EB_SWITCH_SHOT` 出图人工核对。取舍记 spec §10-34,配方 27 含长会话存档生成器
  `docs/make_heavy_state.py`。顺带把配方 26 的 harness 改成自带高度(原先依赖存档里
  恰好存在的长会话)。
- ✅ **修"切到轮数很多的任务仍然卡"**(2026-10-27):上一条修的是每次切换都重建,剩下的
  成本仍按**总轮数**算——`docs/make_heavy_state.py … 3 120` 的 122 轮任务实测
  `remove 18–24ms + attach 4ms + reveal 68ms`(全树 6289 个控件),刷新内外合计约 150ms ⇒
  `the main loop starved by 3334ms during 20 switches`。改成**只渲染最近 `ATTACH_TURNS=20` 轮**
  的窗口(`Conversation::first` 记窗口下界),更早的轮次靠**滚到顶自动加载**(用户选的方案,
  不加"加载更早"按钮这类自创 chrome):`value_changed` 只置旗标并请求重绘,重绘里插
  `ATTACH_STEP=10` 帧,一次手势一片。补偿锚在"视口顶边下那一帧 + 距顶边像素"上,内容高度每变
  一次就把视图放回那条线(预算 16 次)——只按 `upper` 增量补一次在冷加载上仍偏 1580px,因为
  reveal 之后上方内容还会再长高。结果(同一 120 轮存档):30 次切换 `window=(100, 22)`、
  `worst_late=0ms worst_switch=0ms`、树 1087→1139、`conv` 均值 8.0ms/峰值 41.8ms、reveal
  均值 10.6ms;滚到顶一片 29–47ms,两次甩顶的视线漂移实测 0px/0px。行为与参数属自创,
  记 spec §10-35;新增 `EB_LOAD_TEST`/`EB_LOAD_SHOT` harness(配方 28),`EB_SWITCH_TEST`
  补窗口不变量与 `window=` 输出。
- ✅ 关键运行时修复(2026-10-18):`Views` 在 `build_ui` 返回后被 drop,refresh 的 `Weak` 全部失效;现保留到进程生命周期。另修 primary monitor 定位与 agent 事件直接重绘。真实 opencode 普通运行路径已能显示对话。
- ✅ **修"切换到某个任务时,侧栏拉满整窗、没有会话区"**(2026-10-27):触发点看着是切任务,
  实际是**窗口缩放**。`shell` 里挂的 `connect_configure_event` 返回了 `true`,而 configure-event
  是 RUN_LAST——返回 true 会跳过 GTK 自己的 `gtk_window_configure_event`,那是 toplevel 记录新
  frame 尺寸的唯一地方,于是窗口服务器把 280px 的旧 surface 铺满 1470px 的 frame。改成返回
  `false`,并且**比较事件里的宽度**(处理器跑在默认处理器之前,`window.size()` 还是上一帧的值)。
  同一份报障还牵着两个"窗口拉不小"的伴生缺陷:消息列 scroller 的横向策略 `NEVER` 把列的最小宽
  (实测 1158)原样抬进窗口最小宽;居中用的 margin 在 `GtkViewport` 里会自我放大(视口给子控件
  `max(子最小宽+margin, 视口宽)`,而**换行后的中文段落最小宽等于它上次折断时的宽度**),一次 grow
  后列停在 1220px 挂在 1389px 的 pane 里再也回不来。改成由 scroller 居中(`halign` +
  `max_content_width` + 列自身的宽度请求),列宽在 1200/1470/1670 三档双向稳定 752。新增
  `EB_RESIZE_TEST`/`EB_RESIZE_TASK`/`EB_RESIZE_SHOT` harness(配方 29):直接 resize GdkWindow
  复现"服务器改了 frame 而 GtkWindow 不知情",断言 `toplevel == surface`、侧栏恒 280、内容栈吃
  余量;把处理器改回 `true` 会被它抓住(`FAIL grow: the content stack got 1159px of a 1670px
  window`)。`EB_ASSERT` 里 `eb-messages` 的期望从 `MESSAGE_WIDTH - 1` 改成定值。全量回归:
  `cargo test --workspace` + 15 个 GUI harness 全绿(assert/click/artifact×3/artifact 出图/
  follow/switch40/switch-heavy/load/two-task/hang/resize×2)。残留:窗口窄于约 1030 时旧转录会
  被裁掉右侧(重折代价见 spec §10-36),已进下面的待办。
- ✅ **把 ACP 的"这轮结束了"从计时器改成协议信号**(2026-10-27):上一条修的是"响应后的
  续流被丢",这次是**结束条件本身在猜**。用户再跑同一个 Grok deep-research(研究 DuckDB
  适用场景),界面又只留下"结论稍后给你":同一 prompt 三次实测一次截断(5 块)、两次拿到
  完整续流(53 / 774 块),差别只在续流第一条有没有赶上响应后那 2s 静默探针。抓包两次
  (1403s / 1444s 的 provider 重试风暴,各 60 条 `retry_state`)确认:`_x.ai/sessions/changed`
  的 `activity` 全程 `working`,而 `idle`、`_x.ai/session/prompt_complete`、`turn_completed`
  与 JSON-RPC 响应**落在同一毫秒**;错误终态的响应甚至是 `{"result": null}`,连
  `stopReason` 字段都没有——所以那两个 complete 通知只能说明"这条 RPC 答完了",判不了后台。
  改法四条:① `AcpSession` 记 `activity` 且**跨轮保持**(常驻 reader 两轮之间也在读;
  `activity` 只在状态变化时通知,所以上一轮放弃时正在 `working` 的 session 下一轮不会有新
  通知,而没人听时落进队列的 `idle` 也必须算数——否则答完的 agent 每轮白等到 120s),
  `drain_tail` 按它选窗口——
  `working` 用新增的 `tail.busy`(120s,必须小于 180s 静默看门狗)且不受 900s 总上限约束,
  `idle` 用 2s 收尾探针、探针里一旦来了正文就交回 40s 续流窗口(先说 idle 再输出的 agent
  不能被截断),从不报 activity 的 agent(opencode/dsh)完全保持旧行为、普通问答仍只多等 2s;
  ② 光一条 `activity` 通知不算"还在工作",否则每轮虚报一次状态行;③ 每轮开始
  `discard_backlog()` 丢掉上一轮放弃后迟到的正文(关掉即红:新问题正文前挂着
  `LEFTOVER0 LEFTOVER1 LEFTOVER2`),遗留的 JSON-RPC 请求仍回答,免得 agent 卡在跨轮调用;
  ④ 新增 `EVOBUDDY_ACP_LOG` 把 agent 发来的每条原始 JSON 带时间戳落盘——判断结束条件要靠
  转写本,而自己写 driver 复现一轮要花的 token 普通问答本来就花。acp-agent 测试 10→16,
  六个结束条件用例逐个验证过"关掉对应修复会红";配方 12 走真实 UI 跑通两条路径
  (`activity=None` 与 `working`→`idle`),迟到正文渲染在同一条回答里。残留:Grok deep-research
  **成功**那条路径的 `activity` 顺序仍未经真机确认(当天 provider 一直 `Connection reset by
  peer`,15 次重试全失败),兜底是探针 + 40s 交回,最坏退回修复前一样长;并行子 agent 的正文
  仍混排成一段,见下面的待办。
- ✅ **把转录滚动条移到窗口右缘**(2026-10-28):报障"WorkBuddy 的转录条在窗口右侧,evobuddy
  的和输入框右缘对齐,正好与消息里引用/代码框自己的条平齐,遇到文本框两者叠加就滚不动"。
  列由 scroller 居中(§10-36),竖条于是长在 752 列的右缘 —— 即消息内 exec/diff/code 卡各自
  竖条的那一列像素。改法:scroller 竖向策略 `Automatic → External`,条换成共享同一条
  `vadjustment` 的独立 `GtkScrollbar`,挂在 pane 的 overlay 上 `halign=END`(直接挂在
  scroller 的 overlay 上不行:不 expand 的主子控件会让 overlay 子控件按 752 的请求宽定位,
  实测条落回窗口 1018–1032)。滚轮路径未变 —— GTK3 `may_vscroll()` 只对 `NEVER` 关滚动,
  `EXTERNAL` 仍算可滚,`EB_LOAD_TEST` 两次甩顶依旧触发加载。实测(1440 宽、60 轮存档):
  条 `1425–1439`、转录列 `484–1236`(中心 860,与输入卡同轴)、条与列之间 189px 全白;
  短转录(`upper==page`)条自动隐藏。取舍与"14px 常驻 trough 属自创"记 spec §10-37。
- ✅ **空白处也能滚转录**(2026-10-28):报障"鼠标在空白区域,也应该能滚动对话消息"。列由
  scroller 居中(§10-36),scroller 的输入窗口只有它那 752px,1440 宽 pane 里左右各约
  204/189px 的空白属于上层没有输入窗口的 box,滚轮事件没人接 —— 指针得压在文字上才翻页。
  改法:scroller 外面垫一层铺满 pane 的 `GtkEventBox`,把 scroll 事件原样交给 scroller
  (`scroll.event()`),滚轮单位、smooth delta、kinetic、overshoot、clamp 全走 GTK 自己的
  路径。两个必要细节:① `GtkEventBox` 没有 `compute_expand`,必须显式 `set_hexpand/vexpand`,
  否则 expand 提示断在这里,§10-37 的条会按 752 的请求宽重新落回列边缘;② 不能把 scroller
  直接拉宽到 pane —— `GtkViewport` 会把子控件拉到视口宽,列就散了。验证 `EB_PAD_SCROLL=1`,
  harness 只断言位移(±8px),绝对 `upper` 随惰性 attach 当刻挂了多少轮而变。同一
  批回归各自复核不变量:`EB_RESIZE_TEST=1440 EB_RESIZE_TASK=1` 给 `column=752`、
  `stack=1159`→1910 档 `stack=1629`;`EB_LOAD_TEST` 给 `drift=0px,0px`。顺带修了
  `EB_RESIZE_TEST` 的 grow 假失败:根因在 harness(`surface.resize()` 之后 frame clock 不
  tick,树不会拿到新分配),不是布局;调查过程中为"margin 顶高最小宽"写的钳制与重绘钩子已
  原样回退。细节与取舍记 spec §10-38。
- ✅ **margin 滚轮真正落到 catcher**(2026-10-28,上一条的用户复测):"体验并未优化 …… 只有在消息对话区域才能滚动,margin 区域无法滚动"。上一版的转发逻辑没问题,问题在**事件根本没进 GTK**:catcher 是 `GtkEventBox`,它 realize 时的窗口掩码只有 `EXPOSURE | BUTTON_* | ENTER/LEAVE`,不含 `SCROLL_MASK/SMOOTH_SCROLL_MASK`,GDK 按窗口掩码在翻译阶段就把滚轮丢了,`connect_scroll_event` 是死代码。补 `add_events(SCROLL_EVENTS)`;同时把覆盖范围从"转录那一段"扩到整页(再垫一个铺满 `body` 的 `page_hits`,输入卡左右两条空白同样能滚)。**harness 的盲区也补了**:`gtk::propagate_event` 直接递给控件、绕过窗口过滤,所以旧断言全绿而真机无效 —— 现在 `EB_PAD_SCROLL` 先断言两个 catcher 的 `window().events()` 含那两位,再测三格位移(400/250/300)。把 `add_events` 注释掉会红在掩码那行(实测过)。记 spec §10-38。
- ✅ **右缘滚动条收成细滑块**(2026-10-28):报障"右侧滚动条比较突兀、难看"。§10-37 那根独立 `GtkScrollbar` 拿不到主题的 overlay 样式,14px 全是实心槽(快照右缘 `1426–1439` 整列 `#cecece`)。`.eb-transcript-bar` 清空槽与控件背景、去步进按钮,滑块 3px / `alpha(#000,.13)`、内缩 4px(实测只剩 `1433–1435` 三列非白),hover 与滚动时 5px 加深,拖动再深一档;控件仍占 12px 宽,点与拖不受影响。CSS 坑:`scrollbar.x > trough > slider` 不匹配(GTK 在中间还夹了节点,主题用的是后代写法),必须用空格后代选择器 —— 第一版就是控件规则生效、滑块没变。宽度与灰度全是自创(产品的条会淡出,静止帧采不到),记 spec §10-39。
- ⏳ 尚未做:usage popover 的五类分解(采集到但事件流没有分类数据)、`ConversationBlock` 贡献面、旧 CSS 字面量迁移。未知 tool content 类型已有 fallback(带 text 即并入工具输出,2026-10-24);`available_commands_update` 仍按噪声忽略。
- ⏳ ACP 并行子 agent 正文分块(2026-10-27 记):deep-research 的几路子 agent 共用一条
  `agent_message_chunk` 流,现在全部拼进同一段文本,读起来是互相穿插的半句。要按流分块得先
  知道 Grok 拿什么键区分子 session(`_meta`?`toolCallId`?),而这只在**成功**的 deep-research
  转写本里看得到——`EVOBUDDY_ACP_LOG` 抓一条即可开工。
- ⏳ 窄窗口转录重折(2026-10-27 记):窗口拉到约 1030 以下时,已经按 752 折断过的中文段落最小宽仍是 752,列超出 pane、右侧被裁(`EB_RESIZE_TEST=800` 的 `note` 就是它)。要修必须在列变窄时重折每个 `GtkTextView`(切 `wrap-mode` 再切回),而那是 §10-34/§10-35 刚消掉的按帧遍历开销——先记着,等有真实报障再动。

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
- 缺口 3(已调查,暂接受):GTK 将 `box-shadow` 裁剪到控件分配区;试过 832px 外层 gutter,阴影仍不出现。完全还原需 Cairo 自绘或改布局,当前记为已知限制。
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

- **dsh 兼容**:rutis 0.12 上游已验证两条真实路径——`rutis-bridge` 的 Cordis mount(已发布 dsh host 插件原样挂到 Rust,4 个 baseline 测试通过)与 `rutis-dsh`(完整 dsh Web UI 在 rutis 管理的 Node 中启动)。本仓库 vendored rutis 仍是 0.11;native GTK 不渲染 dsh client UI。证据见 [rutis-dsh-capability.md](rutis-dsh-capability.md)。
- **Windows 自包含**(缺口 23/24):需真机;`.cmd` 启动器做路径改写 + Win7/8.1 可行性定论。当前 zip 文档如实标注"未验证"。
- **macOS 公证 / .dmg**:需要 Apple Developer ID;`hdiutil` 一步之遥。
- **缺口 7**:UI 字族在 token 层定义并整体替换重测(影响全部文本宽度,动一次全量回归)。
- **发布合规闸门**✅ 已落地(2026-10-18):根 `LICENSE`(MIT)、`THIRD_PARTY_NOTICES.md`、`SBOM.json`(CycloneDX)、`LICENSES/` 标准文本、[`docs/lgpl-source-offer.md`](lgpl-source-offer.md);Linux/macOS 包都把 `LICENSE`/notices/SBOM/LGPL offer 复制到 `share/doc/evobuddy/`,verify 脚本检查存在。P2 资产为自有 MIT 图。

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
7. ~~发布合规闸门:根 LICENSE、第三方许可、SBOM/LGPL source offer~~(已完成)。
8. **侧边栏逐功能对齐**(用户要求,2026-10-28 起)。第一轮=任务行状态与时间(spec §10-40,`EB_ROWS` 配方 32):时间梯、running/planning spinner、未见失败徽标(按 `TaskFailedIcon` 坐标重绘)、未读 6px 绿点、hover/菜单清空尾槽。同轮采集到但**未实现**的三处:行首 `headerIcon` 状态图标、`pending` 的行内态、section 头部状态旗(`resolveTaskFlag` 优先级 `pending>unread>failed>working` + 点击定位 + 五句 tooltip)。后续按 B–L 排:独立 Pinned 段、归档语义与还原入口、View More (n)/Show Less、⌘K 搜索弹窗(480×560)、多选状态筛选与指示点、底部账号菜单与小程序弹层、48px 窄栏与 ⌘B、任务菜单文案对齐、Workspaces 头部与行对齐、产品快捷键、顶部标签条。
