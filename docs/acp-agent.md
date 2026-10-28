# ACP 对话插件

`crates/plugins/eb-plugin-acp-agent` 是一个 **AnswerSource**：它启动外部
[Agent Client Protocol](https://agentclientprotocol.com) agent，用 JSON-RPC
stdio 驱动它，并把 `session/update` 映射到 EvoBuddy 已有的 `AgentEvent` /
`Block`。宿主、消息渲染器和贡献面都不需要知道后面的 agent 是 dsh、opencode
还是别的实现。

自 0.7.x 起由 `AgentRouter`（`crates/eb/src/router.rs`）选择路由：默认路由即
ACP（opencode），设置界面可切换 ACP / dsh / custom / demo；`EVOBUDDY_ACP_AGENT`
是一次性覆盖。插件本身不再从 `EVOBUDDY_ACP_COMMAND` 读取配置（`AcpConfig::from_env`
仅保留给嵌入方）。会话 cwd 是任务 workspace，不再是 `EVOBUDDY_ACP_CWD`。

## 启动

```bash
# opencode（默认路由；自动探测 ~/.opencode/bin/opencode，PATH 上的 opencode 也可）
cargo run

# Grok Build
EVOBUDDY_ACP_AGENT=grok cargo run

# jcode（原生 ACP：`jcode acp`，自动探测 ~/.local/bin/jcode 与 PATH）
EVOBUDDY_ACP_AGENT=jcode cargo run

# pi（**pi 自己没有 ACP 入口**，只有 `pi --mode rpc`；这一槽位跑第三方适配器
#  pi-acp：PATH 上有 `pi-acp` 就用它，否则 `npx --yes pi-acp@0.0.34`）
EVOBUDDY_ACP_AGENT=pi cargo run

# 任意 ACP agent：把 opencode 槽位指到可执行文件（参数固定为 `acp`，
# 需要别的参数形状时用 wrapper 脚本）
EVOBUDDY_ACP_AGENT=opencode EVOBUDDY_OPENCODE_BIN=/path/to/acp-agent cargo run
```

## 探针：`EB_ACP_PROBE`

槽位接得上不等于 agent 说得通。`EB_ACP_PROBE` 只做 `initialize` + `session/new`
就退出——**不发 `session/prompt`，所以不花 token**，也不建任何窗口（二进制仍链接
GTK，但不进 `main` 的 GUI 分支）：

```bash
EB_ACP_PROBE=1 ./target/debug/evobuddy      # 1 = 当前选中的槽位
EB_ACP_PROBE=jcode ./target/debug/evobuddy  # 或指定 opencode/grok/jcode/pi
```

退出码：0 握手完成、1 agent 拒绝/起不来、2 槽位名不存在。成功时打印 agent 自报名
与 `agentCapabilities`，失败时打印协议错误原文（例：未登录的 grok 报
`no auth method id provided`）。本机 2026-10-26 实测：

| 槽位 | 结果 | agent 自报 | 关键 capability |
|---|---|---|---|
| opencode | ✅ | OpenCode | `loadSession`、`mcpCapabilities.http`、`sessionCapabilities` 含 additionalDirectories/close/delete/fork/list/resume |
| grok | ✅ | Grok Build | 四个槽位里唯一 `mcpCapabilities.sse=true`；`sessionCapabilities` 只有 close/list/resume；顶层多一个 `auth` |
| jcode | ✅ | Jcode | `loadSession`、`promptCapabilities.image/embeddedContext`、`sessionCapabilities.resume/close`；MCP 只支持 stdio（http/sse 均 false） |
| pi | ✅ | pi ACP adapter | `loadSession`、`promptCapabilities.image`；**无** `embeddedContext`，`sessionCapabilities` 只有 list/delete |

grok 那行是登录状态下测的：未登录时同一个命令以 1 退出并报
`no auth method id provided`，登录或配 `EVOBUDDY_ACP_AUTH_METHOD` 后才是正常路径。
四个槽位都还只发 text prompt——`image`/`embeddedContext` 是 agent 侧声明的能力，
宿主这侧尚未接上（见「已知限制」）。


dsh 不再是 ACP profile：它是路由里的独立 `Dsh` 路线（`EB_AGENT=dsh`，进程内
桥），会话持久化在 `~/.dsh`。`DEEPSEEK_API_KEY` 只作为示例：真实值放在本地
环境，不要写进仓库或截图。

Grok Build 的 ACP 入口是 `grok agent stdio`，安装后直接
`EVOBUDDY_ACP_AGENT=grok cargo run`。

## 一轮什么时候算结束

ACP 的 `session/prompt` 响应**不等于** agent 说完了：Grok 的 `deep-research`
这类后台任务会先回 `stopReason=end_turn`（附带"我这就去研究，结论稍后给你"这种
正文），几分钟后才把真正的结论推成同一条 session 的 `session/update`。旧实现读到
响应就 `Finished` 并且再也不读 stdout，那半段结论被整段丢掉——现象就是"任务没完成
就结束了"，而 grok 子进程还在后台烧 CPU（2026-10-27 实测：宿主侧轮已终态，子进程
12s 内 CPU 时间仍 +0.38s）。

现在的规则（`TailPolicy`，四个 env 见下表）：**agent 自己的说法优先于计时器**。

Grok 每次收到 prompt 都会发一条私有通知 `_x.ai/sessions/changed`，把 session 行的
`activity` 置为 `working`，直到这轮真正结束才置 `idle`。两次独立抓包（1403s / 1444s
的 provider 重试风暴，各 60 条 `retry_state`，相邻最大间隔 36.9s）里 `working` 都覆盖
全程，而 `idle`、`_x.ai/session/prompt_complete`、`turn_completed` 与 JSON-RPC 响应
**落在同一毫秒**（t=1444.457 三条一起到）——所以那三个信号只能说明"这一条 prompt 的
RPC 结束了"，不能用来判断后台工作结束。宿主因此按 `activity` 选静默窗口：

1. stdout 由一条常驻线程读，任何时刻都不会把 agent 的输出堆在管道里；
2. agent 说 `working` → 用 120s 安全窗口（必须小于 180s 的静默看门狗，否则先把子进程杀掉），
   这期间多长的沉默都不算结束；
3. agent 说 `idle` → 2s 收尾探针。探针内来了正文就交回 40s 续流窗口（先说 idle
   再流式输出的 agent 不能被探针截断），探针内没东西 → 发 `Finished`；
4. agent 从不报 `activity`（opencode/dsh） → 保持旧行为：响应后 2s 探续流，有续流按
   40s 间隔收，总窗口 900s 封顶。普通问答仍然只多等 2s；
5. 期间 `session/request_permission` 照样回答；用户点 stop 走 `session/cancel` →
   `Failed{Cancelled}`。
6. 状态行 `Agent is still working` 只在续流**正文**到达时出现：一条光秃秃的
   `activity` 通知不算，否则每轮都会虚报一次"还在工作"。
7. `activity` 只在状态**变化**时通知，所以它是"这个 session 现在怎么样"而不是"这一轮怎么
   样"：常驻 reader 在两轮之间也照常读，`activity` 就跨轮保持最新。上一轮放弃时 session 正
   `working`，agent 干完才会翻 `idle`——新 prompt 因此**不会有任何新通知**（状态没变），
   下一轮必须带着 `working` 开始；反过来，`idle` 在没人听的时候落进队列，`discard_backlog`
   丢正文但不丢状态，下一轮就从 `idle` 开始、按 2s 探针收尾（否则一个早答完了的 agent 每轮
   都要白等到 120s）。fixture 的 `resume` / `idle_between` 两个场景各测一半。

每轮开始时还会把上一轮遗留的排队消息丢掉（`discard_backlog`）：上一轮放弃后 agent
继续输出的那些块，之前会被下一轮读成"这一轮的回答"—— fixture 的 `backlog` 场景能稳定
复现（新问题的正文前面挂着上一轮的 `LEFTOVER0 LEFTOVER1 LEFTOVER2`）。遗留的
JSON-RPC **请求**仍照常回答，否则 agent 会卡在一个跨轮调用上。

`stopReason` 不是 `end_turn`/`cancelled` 时（`refusal`、`max_turn_requests`…）
在 `EVOBUDDY_ACP_DEBUG=1` 下打印，不再静默当成功。注意 Grok 在重试耗尽时回的
`session/prompt` 是 `{"result": null}`——**根本没有 `stopReason` 字段**，所以"看响应
判断结果"在这条路上不可用，只能靠私有通知里的 `stop_reason`。

同一场抓包里还有第二种"没结束就结束了"：provider 一直 `Connection reset by peer`，
agent 在 23.4 分钟（1403s）里**不发 `session/prompt` 的响应**，只在私有通知里刷
`retry_state`（1→14 循环三轮），最后以 JSON-RPC error 收尾（`-32603`，
`data` 是 `reqwest error stream: …`，5ms 前刚发过一条
`turn_completed{stop_reason:"error"}`）。旧实现在这种场合并存两个缺陷——180s 墙钟
看门狗先把子进程杀掉（UI 立刻终态，agent 其实还在重试）、重试通知因为没有
`session/update` 信封被全部丢弃（用户看不到任何解释）。现在看门狗按**连续静默**计时，
任何一条通知都算活着；退避以
`Retrying 6/15: …` 状态行显示，用户至少知道卡在哪儿、要不要 stop。

## 环境变量

| 变量 | 必填 | 含义 |
|---|---|---|
| `EVOBUDDY_ACP_AGENT` | 否 | 一次性选择 ACP 实现：`opencode`（默认）、`grok`、`jcode`、`pi`；设置后强制走 ACP 路由（优先于 harness 的 Demo 强制） |
| `EVOBUDDY_OPENCODE_BIN` / `EVOBUDDY_GROK_BIN` / `EVOBUDDY_JCODE_BIN` / `EVOBUDDY_PI_ACP_BIN` | 否 | 覆盖对应槽位的可执行文件（PATH 名字或绝对路径）；前三者参数形状固定（`acp` / `agent stdio` / `acp`），其他参数用 wrapper 脚本。`EVOBUDDY_PI_ACP_BIN` 覆盖的是**适配器**，不是 `pi` 本身 |
| `EVOBUDDY_ACP_AUTH_METHOD` | 否 | `initialize` 后调用 `authenticate` 的 method id |
| `EVOBUDDY_ACP_AUTO_APPROVE` | 否 | `1/true/yes` 时绕过权限弹窗自动 allow；默认弹窗让用户选 |
| `EVOBUDDY_ACP_DEBUG` | 否 | 把 agent 的 stderr 转发到宿主 stderr |
| `EVOBUDDY_ACP_LOG` | 否 | 把 agent 发来的每一条原始 JSON 追加到该文件（前缀是 Unix 秒时间戳）。判断"到底哪个信号说结束"要靠转写本，自己写 driver 复现一遍要花的 token 普通问答本来就花 |
| `EVOBUDDY_ACP_TIMEOUT_SECS` | 否 | **连续静默**多久就判定 agent 死了（默认 180，TERM 后 3s 未收尾补 KILL）。不是整轮墙钟：持续有消息进来的长任务不会被杀 |
| `EVOBUDDY_ACP_TAIL_FIRST_SECS` | 否 | 没有"还在工作"证据时的静默收尾窗口（默认 2s），也是收到 `idle` 后的收尾探针。这段时间轮仍算进行中 |
| `EVOBUDDY_ACP_TAIL_QUIET_SECS` | 否 | 已经收到续流后，容忍的多消息间隔上限（默认 40s；实测 Grok provider 退避时相邻 `retry_state` 间隔 26.6–35.8s，所以不能更小） |
| `EVOBUDDY_ACP_TAIL_BUSY_SECS` | 否 | agent 自己说 `activity=working` 时容忍的静默上限（默认 120s）。必须小于 `EVOBUDDY_ACP_TIMEOUT_SECS`，否则先被看门狗杀掉；也不要大于 `TAIL_QUIET`+`TAIL_MAX` 之类，它只负责"说还在干却一个字不发"这种情况 |
| `EVOBUDDY_ACP_TAIL_MAX_SECS` | 否 | 续流窗口总上限（默认 900s）。agent 明说 `working` 时不受该上限约束 |

`EVOBUDDY_ACP_COMMAND` / `_ARGS` / `_CWD` / `_NAME` 是 0.5 之前的插件级配置
（`AcpConfig::from_env`，仍供嵌入方使用），宿主路由不再读取它们。

## 事件映射

| ACP `session/update` | EvoBuddy |
|---|---|
| `agent_message_chunk`（text） | `AgentEvent::Delta`，由模型追加到文本块 |
| `usage_update` | `AgentEvent::Usage { used, size }`，驱动输入区 usage 环 |
| `tool_call` | 状态行（工具标题） |
| `tool_call_update`（`kind=execute`，completed/failed） | `Block::Exec`（命令 + 输出 + exit code） |
| `tool_call_update`（content 含 `diff`） | `Block::Diff`（path、+/- 计数、行级 add/remove/context） |
| 其他完成的 tool call | `Block::Tool`（状态 + 标题 + 输出首行）；同一 turn 的工具块在文本前聚合为 `Block::Group`（Completed） |
| `plan` | `Block::Plan`：执行计划列表（完成/进行中状态） |
| `retry_state` | `AgentEvent::Status`：`Retrying <attempt>/<max>: <reason>`（reason 截 140 字符）。Grok 在 provider 退避时只发这个，不映射就只剩"界面卡住" |
| `agent_thought_chunk`（text） | `AgentEvent::Thought`，流式合并进尾部 `Block::Thought`；宿主渲染为灰斜体 + 左侧细线的独立思考行（产品未采集思考区外观，刻意做轻） |
| `tool_call_update`（未知 content 类型） | fallback：条目带 `content.text` 或 `text` 时并入工具输出，不再静默丢弃 |

**Grok 私有信封里的 `turn_completed`**：只在续流阶段读。`stop_reason="error"` 时用它
的 `agent_result` 把这一轮判成 `Failed`（否则后台工作死掉后界面会"安静地成功"，用户
只看到一句"稍后发结论"再没有下文）；`"cancelled"` 转成取消；`"end_turn"` 忽略并继续收——
它在 spec 响应旁边几毫秒就会到，认账会把续流窗口提前关掉。

**通知的信封不只是 `session/update`。** 2026-10-27 用独立 stdio 驱动抓真实 Grok
（`grok agent stdio`，provider 指向 aihubmix，当时该端点持续 `Connection reset by
peer`）：23.4 分钟里一条 `session/update` 都没有，`retry_state`×60、`hook_execution`、
`session_summary_generated`、`turn_completed` 全部装在私有方法 `_x.ai/session_notification` 里，
`params` 形状和 spec 一致（`{sessionId, update}`）。所以 `is_update_notification()`
认这两个方法名，其余私有通知（`_x.ai/session/setup`、`_x.ai/queue/changed` 等）继续忽略。
只匹配 spec 方法名时，这 60 条退避通知会被全部丢弃——界面就只剩"卡住"。

JSON-RPC 生命周期：

1. 启动子进程，发 `initialize`（protocolVersion 1，clientInfo evobuddy）。
2. 发 `session/new`（absolute cwd = 任务 workspace，`mcpServers` 来自
   `~/.evobuddy/connectors/*.json`），保存 sessionId。
3. 每次用户 prompt 发 `session/prompt`，同时读 `session/update` 通知，直到收到
   同 id 的 response；`stopReason=cancelled` 转成 `Failed("Cancelled")`。
   注意 provider 故障时 agent **可以永远不发这个 response**：抓到的那一场在
   20 分钟里把 `retry_state` 1→14 循环了三轮，始终没有 stopReason。这类 turn
   靠"沉默看门狗 + 状态行"活着，不靠 response。
4. 进程和 session 跨 turn 保留，所以同一任务里的多轮对话有上下文。
5. 出错时丢弃进程，下次 prompt 重新 initialize/session；用户按 stop 时发
   `session/cancel`。

## 权限与安全

- **默认弹窗**：工具权限请求会走输入框上方的 GTK 弹层，选项来自 ACP 的
  `options[].name`，用户可以逐个 allow/reject。需要无人值守的本地开发可显式设
  `EVOBUDDY_ACP_AUTO_APPROVE=1` 绕过弹窗。
- agent 的读写/命令由 agent 自己执行；本插件只负责协议，不把客户端 fs/terminal
  能力广告出去（`clientCapabilities: {}`），所以 agent 不会反过来调用宿主文件系统。
- agent 进程继承宿主环境；不要把 API key 写进 `EVOBUDDY_ACP_ARGS`（会出现在
  `ps` 里），用本地 export。
- stdout 必须只有 JSON-RPC；agent 的日志应走 stderr（`EVOBUDDY_ACP_DEBUG=1` 可看）。

## 已验证

- `cargo test -p eb-plugin-acp-agent`：fake ACP server 覆盖
  initialize → session/new → 文本流 → usage → permission → exec → diff → finish，
  以及 `session/cancel` → `cancelled`；另有 `probe` 用例（握手成功 + 可执行文件
  缺失必须报错）。共 16 个测试，其中结束条件 6 个：
  `late_updates_stay_in_the_same_turn`（响应后续流不丢）、
  `a_working_session_outlives_the_probe_and_idle_closes_the_turn`（`working` 撑过
  2s 探针、`idle` 2s 内收尾）、`output_that_follows_the_idle_signal_survives_the_probe`
  （先 `idle` 再输出的 agent 不被探针截断）、`a_late_agent_cannot_answer_the_next_turn`
  （上一轮的迟到正文不会读成下一轮的回答）、
  `a_session_still_working_from_an_earlier_turn_keeps_the_next_one_open`（继承上一轮没
  说完的 `working`：那一轮放弃后 agent 不会再发第二条通知）、
  `an_idle_that_arrives_between_turns_starts_the_next_one`（没人听时落进队列的 `idle`
  照样算数，下一轮按 2s 探针收尾而不是白等 busy 窗口）；再加
  `the_transcript_log_records_every_raw_message` 保证 `EVOBUDDY_ACP_LOG` 真的落盘。
  六个结束条件用例在关掉对应修复后都会红（已逐个验证）。
- 真实 Grok 抓包两次（2026-10-27，`/tmp/acp-dump*.jsonl`）：`working` 覆盖 1403s/1444s
  全程、`idle` 与响应同毫秒、错误终态的响应是 `{"result": null}`。deep-research
  **成功**那条路径的 `activity` 顺序仍未经真机确认（provider 当天一直
  `Connection reset by peer`，15 次重试全部失败），见「已知限制」的兜底说明。
- 真实 **dsh** `dsh --profile acp`：initialize/session/new/session/prompt 全链路
  到达模型层；未设置 `DEEPSEEK_API_KEY` 时，错误被回显为可换行的 Failed 状态，
  窗口宽度保持 1440。设置 key 后应直接出文本。
- 真实 **opencode** `opencode acp`：能流式输出、上报 tool_call/usage，并渲染
  exec/tool 块；工具执行会弹权限对话框，或由 `EVOBUDDY_ACP_AUTO_APPROVE=1` 放行。
- 真实 **jcode** `jcode acp`（2026-10-26，v0.86.0-dev）：**原生 ACP**，探针握手通过
  （agent 自报 `Jcode`，`loadSession` + `resume/close` + image/embeddedContext）。
  它是 daemon 形态：第一次 `session/new` 会顺带拉起 jcode 服务进程（stderr 里的
  `Starting server...`），因此进程存活与否由 jcode 自己决定。
- 真实 **pi**（2026-10-26，pi 0.84.3 + `pi-acp@0.0.34` 经 npx）：握手通过，agent
  自报 `pi ACP adapter`。**pi 本体没有 ACP 入口**，只有 `pi --mode rpc` 的私有
  行分隔 JSON；适配器在 stdio 上翻译两者并 spawn `pi --mode rpc`。要求 Node 22+
  与 pi ≥ 0.81，且 provider/key 由 pi 自己配置。
- 走真实 UI 的 fake ACP（local-verification 配方 12）两条路径都跑过（2026-10-27）：
  默认场景（从不报 `activity`）→ `tail closed after 2s of silence (activity=None)`，
  终态时刻和修复前一致；`tail_busy` 场景 → 日志依次
  `session activity working` → `prompt response` → `agent kept working after the
  response` → `session activity idle` → `tail closed after 2s (activity=Some(false))`，
  两段迟到正文渲染在同一轮里。

## 已知限制

- 权限弹层是基础 GTK `Popover`，还没有记住 allow-always 选项或把权限历史写进任务；也没有 `authenticate` 之外的登录 UI。
- 续流（tail）文本直接拼在同一轮正文尾部，没有"（接上）"这类分隔——加了就是自创视觉，且 agent 自己的块边界已经够乱。代价是每一轮普通问答在响应后固定多等 `tail.first`（默认 2s）才落终态。
- 续流只对"**响应之后还往 stdout 发消息**"的 agent 有效。如果 agent 把结论留在
  自己进程里、响应后一条通知都不发，这半段仍然拿不到——协议层没有"把结论再推一次"
  的入口，只能等用户发下一条 prompt 时由 agent 自己补（那条会被 `discard_backlog`
  丢掉，不会再串到下一轮正文里）。`turn_completed` 只有 `error`/`cancelled` 会终结
  续流窗口，`end_turn` 一律忽略：它和 spec 响应同时到达，认账等于把这次修复回退掉。
- `activity` 是 **Grok 私有**通知，别的 agent 不发，于是走 2s/40s/900s 的静默窗口。
  抓包里能确认的是"`working` 覆盖整场重试、`idle` 与真正的结束同毫秒"；**没能**确认
  的是 deep-research 交接给子 agent 后，父 session 会不会提前 `idle` 一次——真机验证
  被 provider 故障挡住（`EVOBUDDY_ACP_LOG` 抓一条成功轮即可判定）。因此这条路径留了
  两层兜底：`idle` 后先等 `tail.first` 再落终态，探针里一旦来了正文就交回
  `tail.quiet`（40s）窗口；最坏情况退回"和修复前一样长"，不会更短。
- 不支持 `session/load`/`session/list`/`session/resume`；一次 app 运行一个活动
  session，重启后由 agent 自己持久化。
- 只发送 text prompt；图片/resource/embedded context 还没接。
- 工具块的进度更新只在完成时落成 Block；进行中主要靠状态行。
- 没有客户端 fs/terminal 能力，agent 必须用自己的工具访问工作区。
- `plan` 已映射为轻量 Plan 卡；`agent_thought_chunk` 已映射为 `Block::Thought`
  （外观未对照产品采集）；`available_commands_update` 仍未映射。
- **pi 拿不到 EvoBuddy 连接器**：`pi-acp@0.0.34` 在 `session/new` 上以 `-32602`
  拒掉我们的 stdio MCP 列表（它的 schema 要 `env`/`headers` 数组与 `type`
  判别式）。插件既有的"拒绝即去掉 MCP 重试"路径（当初为 Grok 加的）让会话照常
  起来，代价是连接器缺失。jcode 同理只声明 `mcpCapabilities.http=false`。
- **选 pi 槽位等于运行第三方适配器**：PATH 上没有 `pi-acp` 时，槽位解析为
  `npx --yes pi-acp@0.0.34`，首次启动会联网下载并执行该包（缓存进 `~/_npx`）。
  不选 pi 槽位就永远不会下载；想彻底避免 npx 路径就 `npm i -g pi-acp`（PATH 上
  的 `pi-acp` 优先）或把 `EVOBUDDY_PI_ACP_BIN` 指到自己的副本。

## 截图 harness

```bash
# fake ACP：确认协议/宿主/渲染链路，不依赖真实模型。
# 路由只接受"槽位 + 可执行文件"，所以用 wrapper 把 fixture 塞进 opencode 槽位。
# 注意：agent 的 cwd 是任务 workspace，fixture 路径必须在创建 wrapper 时展开成
# 绝对路径（heredoc 不加引号，`$@` 转义），不能在 wrapper 运行时求值。
cd /path/to/evobuddy
cat > /tmp/fake-acp-wrapper.sh <<EOF
#!/bin/bash
exec python3 "$PWD/crates/plugins/eb-plugin-acp-agent/tests/fixtures/fake_acp.py" "\\\$@"
EOF
chmod +x /tmp/fake-acp-wrapper.sh
EVOBUDDY_ACP_AGENT=opencode EVOBUDDY_OPENCODE_BIN=/tmp/fake-acp-wrapper.sh \
EVOBUDDY_ACP_AUTO_APPROVE=1 EB_ACP_DEMO=1 EB_NO_SCROLL=1 \
EB_SHOT=/tmp/evobuddy-acp.png EB_SHOT_AFTER_MS=2500 \
cargo run

# 真实 agent：把 OPENCODE_BIN 换掉即可（默认就是本机 opencode）
```

`EB_ACP_DEMO=1` 只提交一次、不提前退出；`EB_SHOT_AFTER_MS` 让截图等到 agent
流稳定后再拍。`EB_FAKE_DEMO=1` 也可以和 ACP 环境变量一起用：它会在终端事件
（`Finished`/`Failed`）后退出，适合 CI 做“能跑完”检查。无人值守测试可用
`EB_PERMISSION_AUTO=allow|reject` 直接回答权限请求，`EB_PERMISSION_CLICK=allow|reject` 自动点击弹层按钮（验证 UI 路径）。
