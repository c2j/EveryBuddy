# EvoBuddy 状态与路线图

> 截至 2026-10-18(v0.4.0)。本文回答两个问题:现在站在哪里,接下来往哪走。
> 逐条测量的细节以 `ui-spec.md` 为准;本文只保留决策、证据指针与计划。

## 一、当前状态

### 1.1 已验证并发布

| 平台 | 产物 | 自包含验证方式 | 状态 |
|---|---|---|---|
| Linux x86_64 / aarch64 | `evobuddy-<v>-linux-<arch>.tar.gz`(自包含目录树)+ `.AppImage`(单文件) | 裸 Debian 10.13(无 GTK/Pango/Cairo/fontconfig/**0 字体**)真跑;AppImage 与目录树渲染**逐字节一致** | ✅ CI 每次打 tag 强制 |
| macOS arm64 / x86_64 | `evobuddy-<v>-macos-<arch>.tar.gz`(内含自包含 `EvoBuddy.app`) | 每条 load command 静态审计 + **把 Homebrew prefix `chmod 000` 藏起来**真跑几何自检 | ✅ CI(macos-15 / macos-15-intel) |
| Windows x86_64 | `evobuddy-<v>-windows-x86_64.zip` | **无验证环境**;只带 DLL 不带 GTK 数据文件 | ⏸ 刻意挂起(软失败,不阻塞发布) |

- 已发布: v0.2.0(首个正式版)→ v0.2.1(+单文件 AppImage)→ v0.3.0(+macOS 双架构)→ v0.4.0(插件化骨架;run 39089801107 六任务全绿,14 个资产已发布)。
- 下限:Linux 要求 glibc ≥ 2.28(精确等于,无余量;Rust std 决定);macOS 下限取自 libgtk 的 `LC_BUILD_VERSION`(随 Homebrew 档次浮动);Windows 仅 UCRT64(Win10+)。

### 1.2 UI 复刻状态

- 主路由(欢迎页、任务会话页、artifact 面板)按 CDP 实测几何搭建,几何逐项可测(`EB_SELFTEST` 输出与规格书数值对齐)。
- 文案、图标为**自有占位**(产品文案与品牌资产不复制,法律边界见 spec §9);实测过的产品字体是改名自包含字体,**不得打包**——这是全项目唯一影响所有文本宽度的既知偏差(Linux 带 DejaVu+WQY:hero 151;macOS 用系统栈:hero 137,与规格书 macOS 实测一致)。
- 已知偏差共 24 条,完整清单与证据在 spec §10"缺口"。其中多数是**记录在案的选择**(如"未实现"提示 popover 是自创 chrome),少数是待办(见下文规划)。

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
2. **M1 不设内核线程**:内置插件无异步工作,单线程 current-thread tokio 挂载期 `block_on` 即可。**线程桥推迟到第一个异步插件出现时**(demo-agent 的替代者)。

**等值证据**:改造前后 welcome/demo 渲染 PNG SHA 逐字节一致(`ebb489b4…` / `5b0eef6b…`);点击审计 31 行步骤与文案全等。审计有两处**环境敏感**字段(toplevels 计数——macOS 有时预建隐藏 `GtkTooltipWindow`;popover 绝对坐标——含窗口服务选择的窗口原点),同二进制两次运行也会不同(已用 stash 实验在旧树上证明),比较审计看步骤与文案、勿整行 diff。

### 1.4 验证体系(现有资产,迁移后全部沿用)

- `EB_SELFTEST` 几何自检 + `EB_SHOT` 离屏截图(`gtk_widget_draw` 前强制重分配,坑已记录)+ `EB_CLICK` 26+ 步点击审计 + `EB_ENTER` 发送链路。
- Linux:`packaging/linux/{package,verify}.sh` —— 容器内 glibc 2.28 构建 + 裸机验证 + `install.sh` 实跑 + AppImage≈tree 断言。
- macOS:`packaging/macos/{bundle,verify}.sh` —— load command 审计 + 藏 Homebrew 真跑。
- CI(tag-only):每次 `v*` tag 六任务全绿才发布;开发者与 CI 调用**同一对脚本**,不许分叉。
- spec:`docs/ui-spec/ui-spec.md` —— 测量、决策、缺口、坑的唯一权威记录。

## 二、本轮(2026-10-17/18)完成的事

1. **v0.2.1**:Linux 发布形态补齐单文件 AppImage;发现并绕过"qemu/Rosetta 拒绝 AppImage runtime 的 `EI_ABIVERSION='AI'` 字节"的环境限制(PLATFORM 默认宿主原生架构;CI 双真实 runner 各建各验)。
2. **v0.3.0**:macOS 双架构自包含 .app。新坑记录于 `packaging/macos/README.md`(CoreText 使 `FONTCONFIG_FILE` 无效故不带字体;re-export 壳 loader 需用 `.dylib` 本体;未签名代码 dlopen 即 SIGKILL 且 `--deep` 不签 Resources 下的 Mach-O;macOS 26 移除 `DYLD_PRINT_*` 改用藏 prefix)。
3. **插件化调研与裁决**:确认 `prototype/` 可作基础——原型验证的 rutis 进程内用法成为 UI 宿主的运行时;wasm/Node 路线保留为 dsh 兼容层(其核心结论"同步返回活对象的注册表必须与插件同进程"对 UI 宿主天然成立)。
4. **v0.4.0(M1)**:workspace 化 + eb-api surface + 4 个内置插件 + 等值迁移(证据见 §1.3)。过程中 CI/容器各抓一坑:CI runner 屏幕小于规格窗口导致几何断言过严(改为断言屏幕无关不变量);SVG 挪目录后构建镜像缺 COPY;TOML 的 `[target.'cfg(...)'.dependencies]` 表吞掉其后所有依赖(macOS 能编、容器报 unlinked crate)。

## 三、后续规划

按依赖关系排序;每个条目标注规模(小=小时级,中=天级,大=周级)与前置条件。

### M2 会话内容块插件化 + 真实异步插件(中-大)⭐ 推荐下一步

- **M2.0 线程桥**(小):内核线程 + UiEvent/HostCommand 双通道;"插件代码不碰 widget"契约落到运行时。这是 demo-agent 被替换前的最后一块基础设施。
- **M2.1 产品补采集**(前置,需要真实环境):驱动产品产生工具调用的**展开卡**(`cr-collapse` 浅绿 `#e7f9ef`)、`cr-tool-diff`(行级 +42/−0)、`cr-tool-exec`、`cr-table-block`、`cr-status-chips`、`cr-context-usage` 用量环的 DOM/几何捕获,进 spec §6。
- **M2.2 内容块插件**(中):以 `ConversationBlock` 贡献面实现上述块;宿主渲染器加对应分支。这是"最大可见保真缺口"(spec §10-10)。
- 验收:每块的几何与配色进 spec 并被 `EB_SELFTEST`/截图审计覆盖。

### P1 像素精修包(小-中,可并行穿插)

- 缺口 3:输入卡 `box-shadow` 被 GTK 裁剪——Cairo 自绘或外层留 padding(需对照产品截图定方案)。
- 缺口 4:输入卡 800×144 vs 800×142 的 2px(疑似 `cr-resize-handle`/边框,未查清)。
- 缺口 6:间距/圆角/字号进 token 生成器(现在是手写字面量)。
- 缺口 14:会话顶栏 24px 左内缩成因(实测 x=304 vs 实现 297)。
- 缺口 16:长标签省略实测(Monaco 近似体的行宽/换行差异一并记录)。

### P2 图标与吉祥物(中)

- 文字占位图标在不同平台渲染成不同字形,本身就是跨平台不一致;需要一套**自有/自由许可**图标(位图进 bundle,避免 librsvg 依赖差异)+ 欢迎页吉祥物占位。

### P3 Linux 包内 CJK 输入(小)

- 缺口 22:带 `im-ibus.so` + `libibus-1.0.so.0`(并验证 `GTK_IM_MODULE=ibus` 可达),否则自带栈里中文输入只有 XIM,现代桌面基本不可用。对日常自用是实打实的提升。

### P4 人工核验清单(小,需要真窗口)

- 缺口 18:IME 组合态下 Enter(自动化无法提供)。
- 缺口 21:popover 在活跃窗口下的呈现/焦点/点击外部关闭;overlay 滚动条实际观感。
- 我出 5 分钟清单,按步骤截图回报即可。

### P5 内容词汇与文案(中)

- 缺口 8:i18n 分片(`zh-cn-*.js`)是屏幕/状态枚举的权威来源,值得提取成表。
- 缺口 5:占位文案替换为 EvoBuddy 自己的成套文案(依赖 8 的枚举)。

### 远期/挂起

- **dsh 兼容**(prototype 路线 A):Node+Cordis 宿主、dsh 插件原样装载、Rust 兑现 fs/search 服务——原型已通 read 工具全链路;扩展 write/edit、agent-loop/session 注册表是后续工作。UI 侧等真实 agent 接入时再启用。
- **Windows 自包含**(缺口 23/24):需真机;`.cmd` 启动器做路径改写 + Win7/8.1 可行性定论。当前 zip 文档如实标注"未验证"。
- **macOS 公证 / .dmg**:需要 Apple Developer ID;`hdiutil` 一步之遥。
- **缺口 7**:UI 字族在 token 层定义并整体替换重测(影响全部文本宽度,动一次全量回归)。

## 四、风险与已知限制

1. **glibc 2.28 无余量**:任何新依赖不得引入更高符号版本;`bundle.sh` 收尾的 readelf 报告是每次打包的闸门。
2. **字体替换是全量文本宽度变化**(缺口 7):动它必须全量重测几何,一次做完,不要渐进。
3. **审计的两处环境敏感字段**(见 §1.3):比较审计看步骤与文案;若要字节级 diff,需先把这两个字段从输出中稳定化(小改动,建议与 P1 一起做)。
4. **CI runner 屏幕 < 1440×868**:macOS/任何真屏验证几何时,窗口会被钳到工作区——断言用屏幕无关不变量(spec 已记录)。
5. **Windows 无验证环境**:在拿到真机前,不要把 Windows 产物从"未验证"升级为"可用"。
6. **rutis 0.11 是 pre-1.0**:vendored fork 是唯一事实源;上游若发 1.0,评估回迁(收窄 tokio 特性的 3 行补丁值得向上游提)。

## 五、近期待办(按序)

1. ~~v0.4.0 CI 收尾确认;结果已记进 spec~~(已完成,2026-10-18)。
2. M2.0 线程桥 + 单测。
3. M2.1 产品补采集(需要产品环境配合,建议约一次联调)。
4. M2.2 内容块插件(diff → exec → table → chips → 用量环,逐块进 spec 与审计)。
5. P1 像素精修包(与 M2 穿插)。
6. P3 CJK 输入(小,随时可插队)。
7. P4 人工核验清单(约一次真窗口时间)。
