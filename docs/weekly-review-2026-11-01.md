# 周报与下周计划(2026-10-26 ~ 2026-11-01)

范围:EvoBuddy 仓库主线 + 跨工作区 Agent 会话。本周共 **75 个 commit**、**6 个发布 tag**
(v0.7.20 → v0.7.29),工作重心是 **Scheduled「计划-审批-派发」自动化** 与 **Custom Model 直连**。

## 一、本周各线工作总结

### 1. Scheduled / 计划-审批-派发自动化(本周最大主线,约 37 commits)
- **前置研究**:[workbuddy-scheduled-module.md](workbuddy-scheduled-module.md) —— 逆向 WorkBuddy 5.8.4
  调度模块(SQLite 4 表、RRule 子集、错过窗口 24h、并发 3、同源串行、11 个模板、微信/企微 outbox),
  对照出 EvoBuddy 的 6 项差距与补齐优先级。
- **设计**:[scheduled-approval-design.md](scheduled-approval-design.md) +
  [UI mockup](scheduled-approval-ui-mockup.html) —— 定义第 4 种调度模式「计划-审批-派发」:
  每天 8:30 汇总昨日 → 生成计划 → 审批守门 → 按模板派发 → 结果回流次日汇总。
- **落地(PR #39 + 调度迭代)**:汇总/待审批队列/审批卡片(角标、昨日 diff、倒计时、风险标注)、
  批准所选/全部/Skip、超时合并明日、桌面通知(纯文本)、`改`微调 + 保存为模板偏好。
- **调度迭代(10/31–11/1,22 commits)**:
  - 创建对话框升级为**四宫格**(一次性/周期/间隔/计划);周期档支持 每天/每周/双周/每月/每年
    + 星期多选 + 月内日 + 月份(`Periodic` 数据模型)。
  - 四屏 UI 按 audit 补齐(screens 1/3/4),mode tiles 真正切换内容;三处视觉修正。
  - **headless**:`evobuddy schedule --run-due` 无 GUI tick、`schedule.lock` 单写者、
    bounded lock wait + busy 退出码;离窗跑计划后由 GUI `pump_dispatches` 补拉。
  - paused 标志跨兄弟认领持久化;bridge 冷启动期间重试 session resume。
  - **跨工作区**:所有模式支持 Runs in = 来源/跨工作区;cross 计划条目检索全部会话、
    提示词任务注入 Agent×工作区分组摘要;修复 dispatch 时 task 级 cross 标志丢失
    (提示词任务曾落回最近工作区);cross 运行不再创建/绑定工作区;Sidebar 给 Scheduled 派发任务打标。
  - `EB_SCHEDULE_FLOW` selftest:走通 create → edit → run → dispatch 并出截图。

### 2. Custom Model 直连(12 commits,10/30–31,v0.7.24–0.7.25)
- chat-completions 真发、OpenAI SSE 流式、reasoning 模型的思考流进 thoughts。
- 每轮携带会话历史;无工具时上下文如实说明;内置工具走 OpenAI function calling,工具注册表化。
- 搜索改用 ripgrep 库(SBOM/第三方声明同步再生成)。
- bigmodel coding-plan 指向 coding endpoint、`/v4` 端点修复、显示模型自称、
  `api_key` 不落 router.json 以活过重启。
- 工具类 Always-allow 同意 + auto 模式;**工具权限卡内联渲染进会话**(修 double borrow);
  composer 权限模式持久化;env auto-approve 检查在 gate 重构中找回。

### 3. ACP / 会话体验(10/28–29,v0.7.20–0.7.23)
- ACP 协议边界迁移到官方 `agent-client-protocol 3.6.0` 并锁定依赖树。
- turn 按协议信号闭合,transcript 围绕 turn 重建;turn 时间戳 hover 显示;
  ACP client 侧 fs/terminal 方法;长行换行 + 产品 logo;Debian 10 裸包在 search/artifact 卡下存活。

### 4. 打包 / 桌面集成 / 稳定性(10/31,v0.7.26–0.7.29)
- 修复 search 工具 panic 导致的 "agent stream closed"(v0.7.26)。
- Linux AppImage 任务栏图标 + 宽屏自适应消息列(v0.7.27);desktop entry 自装、
  直跑图标存活、集成结果记 stderr(v0.7.28/29)。
- terminal exit code 按 schema 放宽 i32→u32;失败徽标 pixbuf 解码失败回退文本字形;
  Linux verify 失败时先打印 cancel 日志;CI live mock-server 时限放宽。

### 5. 会话 UI 与视觉
- run_command 输出默认折叠、点击展开;去重 jump-to-latest 改浮动回底箭头;
  thinking 结束后折叠 thought;应用图标/app mark 按 macOS 网格重做;
  folder 图标软石墨描边;侧栏 workspace 文件夹图标折叠任务组。

### 6. 跨工作区 Agent 会话(cross 搜索)
本周 7 组会话:Artifact test(10 个,含 C++ Quine 验证可运行)、duckdb 使用场景调研、
桌面小工具规划、EvoBuddy 项目功能分析、每周任务 2 / Test8 / Test9。多为实验与调研,
结论已沉淀在各自任务里;工程主线仍全部落在 EvoBuddy 仓库。

## 二、质量与文档
- 新增 `EB_SCHEDULE_FLOW` selftest + 截图走查;screenshots 8 张 + flow 6 张。
- 三份文档落地/更新:scheduled-module 研究、scheduled-approval 设计(逐条标注实施状态)、
  status-and-roadmap(§10-40 侧栏行状态)。
- 已知债(设计文档明确标注):通知无动作按钮、汇总范围固定「昨日全天」、派发串行且等当前
  ACP 回合、低风险免审批未做、微信/企微推送未做、runs 明细未落库、侧栏 nav 待审批角标未做、
  **升级前创建的旧任务需手动补 `"scope":"cross"`**(scheduled.json)。

## 三、下周计划(按优先级与依赖排序)

| # | 事项 | 规模 | 说明 / 验收 |
|---|---|---|---|
| 1 | **Scheduled 收口:运行记录持久化 + 记录页** | 中(1–2 天) | runs 落库到本地文件/SQLite;运行记录页按时长/状态/类型展示;审批卡「时长」列与次日汇总的数据闭环依赖此项 |
| 2 | **侧栏「待审批(N)」角标** | 小 | nav 徽章当前是插件静态字段,需 nav 插件改造;验收=计划生成后侧栏实时计数 |
| 3 | **cross scope 旧任务迁移** | 小 | 打开时检测缺 `scope` 的旧任务并自动补默认值或弹一次性提示,替代手动改 scheduled.json |
| 4 | **通知动作按钮** | 中 | 桌面通知上「全部批准 / 查看并挑选 / 跳过」;先 Linux(libnotify)后 macOS/Windows |
| 5 | **汇总范围可配置** | 中 | 时间窗口(昨日/24h/自定义)+ 会话来源(全部后台/仅 Scheduled/按工作区排除),落 plan.json |
| 6 | **派发并发上限 + 模板字段固定** | 中 | 并发默认 3(取消严格串行);模板可固定执行 Agent/工作区;依赖 #1 的 runs 记录 |
| 7 | **超时策略与低风险免审批** | 中 | 超时后 合并/跳过/自动执行 三选;低风险任务(仅文档产出)免审批直派,配审批记录审计 |
| 8 | **质量面** | 小(穿插) | `EB_SCHEDULE_FLOW` 接入 tag verify;Custom Model 多轮历史+工具调用回归;ACP 权限记忆/历史与长输出布局(roadmap 风险 7 的剩余面) |
| 9 | **观察项(按判定条件再启动)** | — | P4 外部数据源(Git/飞书)、微信/企微 outbox、§9.2 常驻 daemon —— 等 P1 headless 用一周后的真实反馈再定 |

建议节奏:周一 #1–#3(收口 + 升级体验),周二–周四 #4–#7(能力补齐),周五集中回归
(#8)并出一版 tag(候选 v0.7.30:包含 Scheduled 记录页与角标)。

## 四、风险与提醒
- **glibc 2.28 无余量**:ripgrep 家族入包后,readelf 报告仍是每次打包闸门。
- **旧任务 scope 升级**:跨工作区标志此前会丢失,发布说明需写明「升级后旧定时任务建议删除重建」
  (docs 已记,发布时同步到 release note)。
- **审批会话摘录注入面**(`AllowPlan` 预批准)仍是已知风险面,接入更多来源前先补一段脱敏。
