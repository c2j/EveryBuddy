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

**等值证据**:改造前后 welcome/demo 渲染 PNG SHA 逐字节一致(`ebb489b4…` / `5b0eef6b…`);点击审计 **26 步(31 行 `click:` 输出)** 的步骤与文案全等。审计有三处**环境敏感**字段:toplevels 计数(macOS 有时预建隐藏 `GtkTooltipWindow`)、popover 绝对坐标(含窗口服务选择的窗口原点)、以及**代码区滚动宽度**(本机连续运行观察到 579/656 与 579/646 两种读数)。同二进制两次运行也可能不同(已用 stash 实验在旧树上证明);比较审计看步骤与文案,数值列不要整行 diff。

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
6. **M2.0a + P0.4**:`AnswerSource` 改为事件流契约;demo agent 保持像素等值(`5b0eef6b…` 仍一致);fake streaming agent、完成/取消 harness、模型/事件/取消测试全部落地;Linux 与 macOS verify 都跑 fake streaming/cancel/stop/panic。
7. **M2.0b**:`AgentRuntime` 专用线程 + current-thread tokio;`AgentSink` 改为容量 64 的有界队列;`start` panic 经 `catch_unwind` 转 `Failed`;send 按钮流式期间变 stop;Linux/macOS verify 增加 stop 与 panic 用例。

## 三、后续规划

按依赖关系排序;每个条目标注规模(小=小时级,中=天级,大=周级)与前置条件。

### P0 验证加固(小)✅ P0.1–P0.4 已落地

M2.2 会动消息块、工具卡与结果面板。现有 CI 只能证明"产物自包含",不能证明"像素/交互没退化";在改 UI 之前先把审计变成断言:

- **P0.1 `EB_ASSERT=1`**(已完成):按平台断言几何/颜色,以退出码驱动 CI。Linux(Xvfb 1920×1080)断言精确窗口与固定块;macOS runner 屏幕更小时只断言屏幕无关不变量。
- **P0.2 `EB_CLICK_ASSERT=1`**(已完成):对 26 步的 label 与稳定观察值做断言,忽略 toplevels、popover 坐标、代码区滚动宽度等易变数值;Linux 裸机与 macOS verify 都已接线并通过。
- **P0.3 快速检查**(已完成):新增 `.github/workflows/check.yml`,push/PR 跑 `cargo test --workspace`;tag-only 的发布链保持独立,不把发布条件放宽。测试包含 26 步期望表覆盖与"缺一个观察值必须失败"的负向用例。
- **P0.4 假插件测试**(已完成):`EB_FAKE_AGENT=1` 的慢流 fake agent + `EB_FAKE_DEMO=1`/`EB_CANCEL_AFTER_MS`/`EB_FAKE_STOP_AFTER_MS`/`EB_FAKE_PANIC_START` harness;Linux 与 macOS verify 都验证完成、取消、UI stop 与启动 panic;`eb-api` 4 个、demo-agent 4 个测试覆盖事件顺序、取消、sink 关闭与有界队列背压。
- 验收:P0.1/P0.2 在 Linux 与 macOS 的 `verify` 脚本里执行;P0.4 额外跑 fake streaming/API cancel/UI stop/start panic。正向验证已完成;故意改坏几何/交互的完整负向演练仍可补做。

### M2 真实异步 agent + 会话内容块(大)⭐ 推荐下一步

**M2.0a 异步契约**(中)✅ 已完成:`AnswerSource` 改为 `start(prompt, sink) -> handle`;`AgentEvent`(started/delta/block/artifact/usage/finished/failed)、`AgentSink`/`AgentStream`、`AnswerHandle` 落在 `eb-api`;`App::begin_task`/`apply_agent_event` 增量构建 agent turn;`Runner` 在 GTK 主线程 drain。协议与实现映射见 [`docs/m2-async-agent.md`](m2-async-agent.md)。dsh 边界仍按 A 方案(进程内 Rust)先走,sidecar 留后。

**M2.0b 运行时桥**(中)✅ 已完成:`AgentRuntime` 用专用线程跑 current-thread tokio,`AnswerSource::start` 在该线程执行;`AgentSink` 改为容量 64 的 `sync_channel`(满时阻塞 producer);`start` 的 panic 被 `catch_unwind` 转成 `Failed`;send 按钮流式期间变 stop,点击走同一个 `Runner::cancel`。`Contribs` 仍是挂载期快照,运行期内容只走 `AgentEvent`;真正的贡献热更新留到出现需求时再做。

**M2.0c 假流式插件 + 测试**(小-中)✅ 已完成:fake agent 走 `EB_FAKE_AGENT=1`,分块、带延迟、可取消;`EB_FAKE_DEMO=1` 等流结束退出,`EB_CANCEL_AFTER_MS` 验证取消;Linux/macOS verify 和单元测试覆盖。

**M2.0 验收**:demo 两张截图 SHA 不变(`ebb489b4…` / `5b0eef6b…`);fake 完成/API 取消/UI stop/启动 panic 在 Linux 与 macOS verify 通过;有界队列背压与 sink 关闭有单元测试。

**M2.1 产品补采集**(前置,外部长周期;现在排期):驱动产品产生工具调用的**展开卡**(`cr-collapse` 浅绿 `#e7f9ef`)、`cr-tool-diff`(行级 +42/−0)、`cr-tool-exec`、`cr-table-block`、`cr-status-chips`、`cr-context-usage` 用量环的 DOM/几何捕获,进 spec **§5(组件)/§4(几何)**,不是 §6(屏幕清单)。固定产品版本与采集日期,清洗个人数据,落成可复用 fixtures;需要真实环境配合。采集清单已写在 [`docs/m2-capture-checklist.md`](m2-capture-checklist.md),拿到环境即可执行。

**M2.2 内容块模型 + 插件化**(中):先扩 `Block` 数据模型(diff / exec / table / chips / usage),定义未知块的 fallback;再以 `ConversationBlock` 贡献面实现上述块,宿主渲染器加对应分支。贡献面只传数据,不传 widget。这是"最大可见保真缺口"(spec §10-10)。

**验收**:每块几何与配色进 spec,并进入 `EB_SELFTEST` / `EB_CLICK_ASSERT` 的断言范围;M2.0c 的 fake agent 在 CI 可跑。

### P1 像素精修包(小-中,可并行穿插)

**P1a(先于 M2.2)**
- 缺口 6:间距/圆角/字号进 token 生成器(现在是手写字面量)。先做这个,否则新内容块只会增加更多字面量。
- 缺口 3:输入卡 `box-shadow` 被 GTK 裁剪——Cairo 自绘或外层留 padding(需对照产品截图定方案)。
- 缺口 4:输入卡 800×144 vs 800×142 的 2px(会话态还有 532×134 vs 531×132 的 1px/2px,一并查)。

**P1b(依赖 M2.1 的采集)**
- 缺口 14:会话顶栏 24px 左内缩成因(实测 x=304 vs 实现 297)。
- 缺口 16:长标签省略实测(Monaco 近似体的行宽/换行差异一并记录)。
- 缺口 13(三栏 1px sash)与缺口 9(z-index 层级表):补进 P1,或明确写成"接受/延后"。

### P2 图标与吉祥物(中)

- 文字占位图标在不同平台渲染成不同字形,本身就是跨平台不一致;需要一套**自有/自由许可**图标(位图进 bundle,避免 librsvg 依赖差异)+ 欢迎页吉祥物占位。资产许可记录进发布闸门。

### P3 Linux 包内 CJK 输入(小)

- 缺口 22:带 `im-ibus.so` + `libibus-1.0.so.0`(并验证 `GTK_IM_MODULE=ibus` 可达),否则自带栈里中文输入只有 XIM,现代桌面基本不可用。对日常自用是实打实的提升。除 ibus 外,至少把 fcitx5/XIM 的降级路径写进验证清单。

### P4 人工核验清单(小,需要真窗口)

- 缺口 18:IME 组合态下 Enter(自动化无法提供)。
- 缺口 21:popover 在活跃窗口下的呈现/焦点/点击外部关闭;overlay 滚动条实际观感。
- 建议提供一份 5 分钟清单,按步骤截图回报;同时补一次真实桌面(Linux X11/Wayland + macOS)的 AppImage/.app 冒烟。

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
7. **M2.0b 已交付,但还没接真实 agent**:runtime 线程、有界队列、UI stop、panic 隔离已做;下一块风险转移到真实 agent 的流式语义、工具调用与 dsh sidecar 边界。
8. **发布合规风险**:LGPL 动态库随包分发、字体/图标再分发、产品文案/i18n 的边界,都需要在发布前明确记录。

## 五、近期待办(按序)

1. ~~v0.4.0 CI 收尾确认;结果已记进 spec~~(已完成,2026-10-18)。
2. ~~P0 验证加固:`EB_ASSERT` / `EB_CLICK_ASSERT` + Linux/macOS verify 接线 + 快速检查~~(P0.1–P0.4 已完成,2026-10-18)。
3. **并行启动**:M2.1 采集(清单见 [`docs/m2-capture-checklist.md`](m2-capture-checklist.md));M2.0a 已完成(协议与实现映射见 [`docs/m2-async-agent.md`](m2-async-agent.md);A 方案进程内 Rust)。
4. ~~M2.0b:宿主专用 runtime 线程 + 有界 channel + UI 取消按钮~~(已完成,2026-10-18;贡献热更新留后)。
5. **M2.2**:内容块数据模型 + 渲染器,逐块进 spec 与断言。
6. **P1a**(缺口 6/3/4)与 **P3**(CJK 输入)可穿插;P1b/P2/P4/P5 按依赖推进。
7. **发布合规闸门**:根 LICENSE、第三方许可、SBOM/LGPL source offer。
