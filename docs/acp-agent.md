# ACP 对话插件

`crates/plugins/eb-plugin-acp-agent` 是一个 **AnswerSource**：它启动外部
[Agent Client Protocol](https://agentclientprotocol.com) agent，用 JSON-RPC
stdio 驱动它，并把 `session/update` 映射到 EvoBuddy 已有的 `AgentEvent` /
`Block`。宿主、消息渲染器和贡献面都不需要知道后面的 agent 是 dsh、opencode
还是别的实现。

配置了 `EVOBUDDY_ACP_COMMAND` 时，`Host::mount` 选这个插件；否则仍选内置
demo agent。两者只提供一个 `AnswerSource`，不会同时应答。

## 启动

```bash
# opencode（本机已安装时）
EVOBUDDY_ACP_COMMAND="$HOME/.opencode/bin/opencode" \
EVOBUDDY_ACP_ARGS="acp" \
EVOBUDDY_ACP_CWD="$PWD" \
cargo run

# Grok Build（本机已安装时）
EVOBUDDY_ACP_AGENT=grok cargo run
# 等价显式配置：
EVOBUDDY_ACP_COMMAND="$HOME/.grok/bin/grok" \
EVOBUDDY_ACP_ARGS="agent stdio" \
EVOBUDDY_ACP_CWD="$PWD" \
cargo run

# dsh：从 deepseek-harness checkout 启动 ACP profile
EVOBUDDY_ACP_COMMAND="$DSH/node_modules/.bin/pnpm" \
EVOBUDDY_ACP_ARGS="dsh --profile acp" \
EVOBUDDY_ACP_CWD="$DSH" \
DEEPSEEK_API_KEY="…" \
cargo run
```

`DEEPSEEK_API_KEY` 只作为示例：真实值放在本地环境，不要写进仓库或截图。
dsh 的 ACP profile 会把会话持久化到 `~/.dsh`；在普通桌面用户下可写。

Grok Build 的 ACP 入口是 `grok agent stdio`。安装后可直接：

```sh
EVOBUDDY_ACP_AGENT=grok cargo run
```

也可以用 `EVOBUDDY_ACP_COMMAND` + `EVOBUDDY_ACP_ARGS` 接其他 ACP agent；
不需要改 Rust 代码。

## 环境变量

| 变量 | 必填 | 含义 |
|---|---|---|
| `EVOBUDDY_ACP_AGENT` | 否 | 本机自动探测的选择：`opencode`（默认）或 `grok` |
| `EVOBUDDY_ACP_COMMAND` | 是 | ACP agent 可执行文件；设置后即启用本插件 |
| `EVOBUDDY_ACP_ARGS` | 否 | 参数，空白分隔或 JSON 字符串数组，如 `["dsh","--profile","acp"]` |
| `EVOBUDDY_ACP_CWD` | 否 | 会话工作区，必须是绝对路径；默认启动进程的 cwd |
| `EVOBUDDY_ACP_AUTH_METHOD` | 否 | `initialize` 后调用 `authenticate` 的 method id |
| `EVOBUDDY_ACP_AUTO_APPROVE` | 否 | `1/true/yes` 时绕过权限弹窗自动 allow；默认弹窗让用户选 |
| `EVOBUDDY_ACP_NAME` | 否 | agent 未上报名称时的显示名，默认 `ACP agent` |
| `EVOBUDDY_ACP_DEBUG` | 否 | 把 agent 的 stderr 转发到宿主 stderr |

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
| `agent_thought_chunk` | 暂不渲染（保留给后续状态区） |

JSON-RPC 生命周期：

1. 启动子进程，发 `initialize`（protocolVersion 1，clientInfo evobuddy）。
2. 发 `session/new`（absolute cwd，`mcpServers: []`），保存 sessionId。
3. 每次用户 prompt 发 `session/prompt`，同时读 `session/update` 通知，直到收到
   同 id 的 response；`stopReason=cancelled` 转成 `Failed("Cancelled")`。
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
  以及 `session/cancel` → `cancelled`。共 5 个测试。
- 真实 **dsh** `dsh --profile acp`：initialize/session/new/session/prompt 全链路
  到达模型层；未设置 `DEEPSEEK_API_KEY` 时，错误被回显为可换行的 Failed 状态，
  窗口宽度保持 1440。设置 key 后应直接出文本。
- 真实 **opencode** `opencode acp`：能流式输出、上报 tool_call/usage，并渲染
  exec/tool 块；工具执行会弹权限对话框，或由 `EVOBUDDY_ACP_AUTO_APPROVE=1` 放行。

## 已知限制

- 权限弹层是基础 GTK `Popover`，还没有记住 allow-always 选项或把权限历史写进任务；也没有 `authenticate` 之外的登录 UI。
- 不支持 `session/load`/`session/list`/`session/resume`；一次 app 运行一个活动
  session，重启后由 agent 自己持久化。
- 只发送 text prompt；图片/resource/embedded context 还没接。
- 工具块的进度更新只在完成时落成 Block；进行中主要靠状态行。
- 没有客户端 fs/terminal 能力，agent 必须用自己的工具访问工作区。
- `plan` 已映射为轻量 Plan 卡；`agent_thought_chunk`、`available_commands_update`
  仍未映射。

## 截图 harness

```bash
# fake ACP：确认协议/宿主/渲染链路，不依赖真实模型
EVOBUDDY_ACP_COMMAND=python3 \
EVOBUDDY_ACP_ARGS="$PWD/crates/plugins/eb-plugin-acp-agent/tests/fixtures/fake_acp.py" \
EB_ACP_DEMO=1 EB_PERMISSION_AUTO=allow \
EB_SHOT=/tmp/evobuddy-acp.png EB_SHOT_AFTER_MS=1500 \
cargo run

# 真实 agent：把上面的 COMMAND/ARGS/CWD 换成 dsh 或 opencode
```

`EB_ACP_DEMO=1` 只提交一次、不提前退出；`EB_SHOT_AFTER_MS` 让截图等到 agent
流稳定后再拍。`EB_FAKE_DEMO=1` 也可以和 ACP 环境变量一起用：它会在终端事件
（`Finished`/`Failed`）后退出，适合 CI 做“能跑完”检查。无人值守测试可用
`EB_PERMISSION_AUTO=allow|reject` 直接回答权限请求，`EB_PERMISSION_CLICK=allow|reject` 自动点击弹层按钮（验证 UI 路径）。
