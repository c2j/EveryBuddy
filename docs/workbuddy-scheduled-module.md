# WorkBuddy Scheduled（定时任务）模块研究

研究时间：2026-10-31。目标：**WorkBuddy AI 5.8.4**（macOS，Electron 37.10.3）。

证据来源（均为 shipped-artifact 静态观察，未观察运行时执行）：

- 应用包：`/Applications/WorkBuddy AI.app/Contents/Resources/app.asar`
  SHA-256 `3f77e5cb036d0066fa6afa67bc9498d7fddd432eed927d4b576bfe91ddb23a49`
  （REA 证据 `ev_15a98a3031b82a4ac7f863cf300cb9cc58d75b80d2ffd5a6d91c45120aacdfad`，
  父级 `ev_4a1f74105965e4692ffefb3f069861f6ae313d583450ae8483bd2169de580632`；
  部分原生二进制与 asar 声明哈希不符，按 untrusted 记录，与本模块无关）
- 主进程 automation 代码位于打包后被改名的 bundle：
  `main/server.js`（3.6 MB，含 `LocalAutomationScheduler`、run 存储层、投递 outbox worker、`executeViaCli`）
  与 `main/log-acl-guard.js`（1.9 MB，含 drizzle schema、`computeNextRunAt`/RRule 解析、模板定义）。
- 渲染层 chunk（与 Resources/ 下的散装文件同内容）：
  `renderer/assets/automation-Dd9Xg1V1.js`（编辑表单 + RRule 构造，132 KB）、
  `automation-Bw-IWOjm.js`、`automation-run-group-BKBmFTeI.js`（44 KB）、
  `automation-conversation-navigation-IBaMU6MA.js`、`schedule-host-PIXWbPKW.js`。
- 本机数据：`~/.workbuddy-ai/workbuddy.db`（SQLite，automation 相关表当前为空）。

## 1. 数据模型（SQLite，`workbuddy.db`）

四张表，drizzle migration 管理：

### automations —— 任务定义
| 列 | 说明 |
|---|---|
| id, name, prompt | 任务 ID / 名称 / 提示词 |
| status | `ACTIVE` / `PAUSED`（另有 deleted_at 软删） |
| schedule_type | `recurring` / `once` |
| rrule | RFC 5545 子集：`FREQ=HOURLY/DAILY/WEEKLY/MONTHLY/YEARLY`，`INTERVAL`、`BYDAY`、`BYHOUR`、`BYMINUTE`、`BYMONTH`、`BYMONTHDAY` |
| scheduled_at | `once` 的一次性时间（ISO） |
| valid_from / valid_until | 生效日期区间（可只填一侧 / 长期） |
| next_run_at / last_run_at | 毫秒时间戳，由调度器维护 |
| cwds / workspace_scope | 运行目录列表；`task` / `workspace` 两种作用域 |
| model_id, model_is_thinking, context_window, reasoning_effort | 模型选择（含思考模式与上下文窗口） |
| skills_json, expert_id, expert_marketplace, connector_ids_json | 附加技能 / 专家 / 连接器 |
| permission_mode | 运行权限模式（默认 `fullAccess`） |
| push_to_wechat, push_to_wecom_bot, wecom_bot_source, wecom_bot_chat_id | 结果推送目标 |
| owner_user_id, owner_status, owner_source | 多账户归属（`confirmed` / `inferred` / `legacy_unassigned`） |
| created_at / updated_at / deleted_at | 时间戳 |

### automation_runs —— 运行记录
`thread_id`（主键，即会话 ID）、`automation_id`、`status`（`QUEUED`/`IN_PROGRESS`/`ACCEPTED`/`ARCHIVED`，失败态 FAILED 由 UI 层表达）、`thread_title`、`source_cwd`、`runs_json`（每次子运行的明细：startedAt/finishedAt/success/cwd/artifacts）、`result_success`、`metadata_json`（含 `runKind: scheduled|manual|missed`）、`failure_code`、`reason_code`。

### automation_runtime_state —— 运行时状态
每任务一行：`last_run_at`、`last_error`、`running`、`running_started_at`、`running_conversation_id`、`metadata_json`。

### automation_delivery_outbox —— 结果推送发件箱
`channel`（默认 `wechatmp`，微信公众号）、`dedupe_key`（唯一索引）、租约字段（`lease_owner`/`lease_expire_at`）、`attempt_count`/`max_attempts=5`、`next_run_at`、`last_error_code/message`、`delivery_id`、`finished_at`。索引按 `(status,next_run_at)`、`(channel,status,next_run_at)`、归属者查询。

## 2. 调度器（`LocalAutomationScheduler`）

关键参数（构造默认值）：`tickMs=30s`（空闲轮询），`activeTickMs=5s`（有任务在跑时加速），`missedWindowMs=24h`，`concurrency=3`，`deferralFreshnessMs=tick×3`，`maxDeferralExtensionMs=24h`。网络重试用退避：`30s × 2^(n-1)`，上限 5 min，最多推迟 5 次后判失败。

tick 流程：

1. `reconcileTerminalRunRecords`：把残留 `IN_PROGRESS` 的运行记录按存活判定收敛为终态（中断原因枚举：`interrupted` / `restart_interrupted` / `session-ended` / `finalization-timeout` / `session-stuck`）。
2. 清理孤儿 runtime state；`computeCanonicalAutomationIds` 去重（同 ID 多视图时只调度 canonical 一份）。
3. 对每个 ACTIVE 任务执行 `decide(automation, runtimeState, now)`：
   - `running=1` → `skip`；`next_run_at` 缺失 / 未到 / 已被 `last_run_at≥next_run_at` 覆盖 → `skip`；
   - 过期超过 24h 错过窗口 → `skip_missed`（写入一条 `runKind:missed` 的失败记录 `ErrorAutomation-MissedWindow`，并推进到下一个周期）；
   - 网络退避期内 / 加了确定性抖动（对 `id:nextRunAt` 做 FNV 哈希取 0–1800s 内的分散延迟 `computeDispatchDelayMs`）未到 → `skip`；否则 `run`。
   - 中断恢复熔断：连续中断次数达到阈值（`consecutiveInterruptCount >= gd`，超过错过窗口时）→ `skip_missed`（`recovery_circuit_broken`）。
4. **同源串行**：`sourceKey = ownerUserId::首个cwd`，同一来源同时最多 1 个运行；冲突任务记 deferral（新鲜期内不判错过），留在队列等下一 tick。
5. 按 `next_run_at` 升序（同值按 id 字典序）派发，超过 concurrency=3 只排队。
6. 运行结束 `finishRun`：清 runtime state、`advanceNextRunAt`（按当前时间重算下一次；PAUSED 则置空）。

冷启动恢复：`restorePersistedQueuedRuns` 把数据库里遗留的 `QUEUED` 运行补跑；不可恢复的（任务已删 / 已暂停 / 超过 24h 窗口）直接标失败并写原因。网络未就绪时 `sweepLocalOnly()` 只做本地收敛、延迟派发。支持 `setSchedulingSuspended`（登录/切换账户期间挂起）。

## 3. 一次运行如何执行（`executeViaCli`）

- 每次运行 = 一个完整 agent 会话：预分配 UUID 会话 → 授予权限 → `renameSession(automationName)`。
- **连接器保障**：运行前对 `connector_ids` 逐个确保连接，全部失败则仅告警并继续。
- **权限**：取任务 `permissionMode`（默认 `fullAccess`），运行期使用 `background-automation-no-approval-ui`（后台执行、无审批 UI）；禁用 sandbox CLI。
- **模型覆盖**：modelId / thinking / reasoningEffort / context_window / 思考等级逐项写入会话配置。
- 失败码枚举（遥测/记录用）：`NETWORK`、`PARTIAL`、`RATE_LIMIT`、`RECOVERY`、`REFUSAL`、`SESSION_ENDED`、`SESSION_START(_TIMEOUT)`、`SESSION_STUCK`、`TIMEOUT` 等。
- `trigger(automation)` API 支持"立即运行一次"（runKind=manual），由调用方传入 executor。
- 会话取消：`assertRunNotCancelled` 在会话创建前/连接器准备后/会话创建后多个检查点中止；可 `killConversationSession`。

## 4. 结果推送（delivery outbox worker）

运行终态后 `onRunFinalized → enqueueAutomationDeliveryOutbox`（失败时还会把失败原因追加到会话 transcript）。worker 特性：

- 租约式抢占（`lease_owner`+`lease_expire_at`），卡死回收 `reclaimStuck`；
- 最多 5 次重试，`next_run_at` 退避；`dedupe_key` 唯一防重；
- 发现更新一代的 outbox 记录时把旧的标记 `skipped`（去重/取代）；
- 低频清理：成功记录保留 30 分钟级、失败记录按 `failedRetainLimit` 限量清理；
- 失败分类 `SUPERSeded` 等，错误码/消息落库。

推送目标即任务上的 `push_to_wechat`（channel `wechatmp`）与 `push_to_wecom_bot`（企业微信机器人，`wecomBotSource: assistant|standalone` + `wecomBotChatId`；助手模式在 websocket 重连后 chat id 失效，由 `invalidateWecomBotTargetChats` 置空并提示重新绑定）。

## 5. UI 能力（渲染层 i18n 键 + 代码）

侧边栏 `Scheduled` 页签两个：**Scheduled Tasks（定时任务）** 与 **Run Records（运行记录）**。

任务列表/卡片：搜索、过滤器（全部/成功/失败/运行中；状态：启用/暂停）、批量管理（多选、批量删除、退出批量）、卡片操作（立即运行、暂停/恢复、打开会话、查看记录）、过期提示、上次失败提示、来源徽章（local / cloud / project，云端与项目来源为只读，离线时只读提示）。

新建入口：`New`（手动）、`New From Template`（模板库）、`New From Task`（从已有会话生成）、批量进入。模板共 11 个：每日 AI 新闻、每日 5 个英语单词、每日儿童睡前故事、每周工作报告、经典电影推荐、历史上的今天、每日一个为什么、父母联系提醒、体检预约提醒、面试准备提醒（每两小时）、会前准备（支持 `{date}`/`{time}` 占位符插值）。

创建/编辑弹窗：
- 名称 + 提示词（必填，长度校验）；
- **调度**三模式：`once`（一次性，必须选未来时间）、`periodic`（天/周/双周/月/年，多选星期、多选月内日期、时间点 HH:mm）、`interval`（每 N 小时 1–24，可选仅工作日 BYDAY）；全部编译为 RRule 字符串；表单 ↔ RRule 双向解析（`getScheduleMode/getPeriodicPreset/getScheduleTime/getExplicitWeekdays/...`）；
- **有效期** validFrom/validUntil 日期区间（长期开关、清除）；
- **执行配置**：模型（含 thinking、reasoningEffort、contextWindow）、专家（expertId/marketplace，可 Summon）、技能（canSelectSkills + skillStorageScope）、连接器多选、权限模式（改回 fullAccess 需二次确认 `confirmFullAccessOnChange`）、工作区作用域；
- **推送**：推送到微信（wechatmp）、推送到企业微信机器人（assistant/standalone 两源，含未连接引导与 websocket 重绑定弹窗）。

运行记录页：按日期分组（今天/昨天/明天/具体日期）、折叠组、`viewAll`；列：时间/名称/类型（scheduled|manual|missed|interrupted）/时长/状态/操作；筛选（任务、类型、状态）；操作：查看会话、归档/取消归档、删除（含批量）；行状态：queued/inProgress/paused/onceOnly/expired/startsIn/nextRun。

## 6. 与 EvoBuddy 现状对照（`crates/eb` scheduled.json）

| 能力 | WorkBuddy | EvoBuddy 现状 |
|---|---|---|
| 存储 | SQLite 4 表（定义/记录/运行态/发件箱） | 单文件 `~/.evobuddy/scheduled.json` |
| 调度 | RRule 子集 + once + 24h 错过窗口 + 抖动 + 熔断 + 同源串行 + 并发 3 | 5s 轮询取 due 即跑 |
| 执行 | 独立后台会话（无审批 UI、模型/专家/技能/连接器/权限随任务） | 复用 `Runner::submit`，新工作区 |
| 运行历史 | runs 表 + UI 记录页（按日分组、归档、失败原因） | 无持久运行记录 |
| 推送 | 微信公众号 / 企业微信机器人，outbox 重试 | 无 |
| 恢复 | 重启补跑 QUEUED、中断原因收敛、暂停期间挂起 | 无 |
| 模板 | 11 个内置模板 | 无 |

若要补齐，优先级建议：① 运行记录持久化 + 记录页；② 错过窗口/暂停/软删语义与 next_run 推进；③ 任务级模型与权限配置；④ once 模式与有效期区间；⑤ 模板库。
