# M2.0a 异步 agent 契约

> 状态:M2.0a/b/c 与 P0.4 已实现(2026-10-18)。
> 目的不是先写线程代码,而是把"同步整轮返回"换成可流式、可取消、可测试的接口。
> dsh sidecar 留到协议稳定后再评估。

## 1. 现状与问题

- `eb_api::AnswerSource::answer(&self, prompt) -> (Turn, Option<Artifact>)`
  同步返回一整轮;宿主在 GTK 主线程直接调用它。
- `Host` 的 current-thread tokio runtime 只在挂载期 `block_on`,之后没有驱动;
  `Contribs` 是挂载后的快照。
- `Turn::Agent` 已有 `status` / `streaming` 字段,但没有增量更新通道。

所以 M2.0 的阻塞不是"缺一个线程",而是缺一个**事件契约**:agent 在别的线程
产生开始、增量、块、用量、结束/错误,宿主在主线程消费并重绘。

## 2. surface(已实现)

放在 `crates/eb-api`(插件唯一依赖的 surface),不暴露 GTK/rutis 细节。

```rust
pub enum AgentEvent {
    Started { name: String },
    Status { text: String },
    Delta { text: String },
    Block { block: model::Block },
    Artifact { artifact: model::Artifact },
    Usage { used: u32, total: u32 },
    Finished,
    Failed { message: String },
}

pub struct AgentSink { /* cloneable, Send + Sync */ }
impl AgentSink {
    pub fn channel() -> (AgentSink, AgentStream);
    pub fn send(&self, event: AgentEvent) -> Result<(), AgentClosed>;
}

pub struct AgentStream { /* host side */ }
impl AgentStream {
    pub fn try_recv(&self) -> Result<AgentEvent, AgentStreamError>;
    pub fn recv_timeout(&self, timeout: Duration) -> Result<AgentEvent, AgentStreamError>;
}

pub trait AnswerHandle: Send + Sync {
    fn cancel(&self);
}

pub trait AnswerSource: Send + Sync + 'static {
    /// 非阻塞:启动一次回答,事件经 sink 回传;取消通过 handle。
    fn start(&self, prompt: &str, sink: AgentSink) -> Box<dyn AnswerHandle>;
}
```

约定:

- `AgentEvent` 是**纯数据**,可跨线程;`Block` / `Artifact` 沿用现有模型。
- `Delta` 是文本增量,`Block` 是完整块的增量;两者都允许 0..n 次。
- `Finished` 与 `Failed` 互斥且必须恰好出现一次;`sink` 关闭等价于宿主取消。
- 取消是幂等的;`cancel()` 之后允许仍有已排队事件,但宿主必须丢弃。
- 不在 surface 中放 `gtk::Widget`、`glib` 或 `tokio` 类型。

### 2.1 实现映射

| 位置 | 内容 |
|---|---|
| `crates/eb-api/src/lib.rs` | 事件、sink/stream、取消句柄、`AnswerSource::start` |
| `crates/eb-api/src/model.rs` | `begin_task` / `apply_agent_event`;`Delta` 追加到当前文本块 |
| `crates/eb/src/agent.rs` | `AgentRuntime`(专用线程 + tokio)与 `Runner`(主线程 16ms drain 全部在途流;同任务新提交 supersede,cancel 作用于当前路由任务) |
| `crates/plugins/eb-plugin-demo-agent` | demo agent 同步发射;`EB_FAKE_AGENT=1` 的慢流 fake agent |
| `crates/eb/src/dsh_agent.rs` | dsh 路线的整轮在 `evobuddy-dsh-agent` 线程执行,`start` 只回一个 `NoopHandle` |
| `crates/eb/src/bridge.rs` | 主线程内联的 dsh 桥 HTTP:连接 150ms、写/读 1s(`run_agent` 读 600s),超时即进入 30s 降级冷却 |
| `main.rs` | `EB_FAKE_DEMO=1` 提交并等终态;`EB_CANCEL_AFTER_MS` 取消;`EB_TWO_TASK_TEST=1` 验证后台任务不被第二个提交打断;`EB_HANG_TEST=1` 用黑洞桥验证主线程调用有界 |

当前限制/有意保留:

- `AgentRuntime::start` 在主线程等握手,等待有界(`await_start(.., 10s)`):
  `start` 违约不返回就报一条 `Failed` 而不是永久冻结窗口。dsh 路线因此必须把
  整轮搬到工作线程(`crates/eb/src/dsh_agent.rs`);
- `AgentSink` 是容量 64 的有界队列(满时阻塞 producer),没有丢弃策略;
- `AgentRuntime` 已有专用线程和 current-thread tokio,`start` 在该线程执行;
  fake agent 仍自己起一个 worker 线程,模拟真实 agent 的异步生产;
- UI stop 已可用(send 按钮流式期间变 ■),`EB_FAKE_STOP_AFTER_MS` 覆盖点击路径;
- `start` panic 已转成 `Failed`;producer 线程 panic 通过 sink 断开转成
  `Failed("agent stream closed")`;
- `Usage` 事件已定义,但模型还没有用量环,暂时忽略;
- 每个任务只允许一个在途回答,同一任务的新提交把旧回答标成 `Cancelled`;
  不同任务可并发流式:新任务提交/其他会话发言不影响在途任务,后台事件写回
  各自任务(后台 artifact 不弹前台面板,dsh bridge 投影只在 session 仍为当前
  活动会话时写入)。ACP 的真实 agent 共享一条子进程 session,跨任务 prompt
  在插件内串行等待;
- `Contribs` 仍是挂载期快照,运行期贡献热更新未做(agent 内容走 `AgentEvent`)。

测试:`eb-api` 12 个(事件增量、失败、空 prompt、有界队列背压、Usage、后台 artifact 不弹前台面板、在途任务判定),demo-agent 5 个
(完整事件、M2.1 三种块、fake 流、取消、sink 关闭),`eb-plugin-acp-agent` 10 个(协议/权限/cancel/连接器→MCP 映射/握手探针/响应后的尾流/重试状态行/续流失败),`evobuddy` 39 个(宿主单元测试,含 33 步期望表、负向断言、Markdown 解析、artifacts 累积、artifact 文件卡、有界握手、有界且字符安全的 artifact 读取、窄窗口三段保底的分隔条计算、ACP 槽位枚举与 7 行路由菜单的 round-trip)。

## 3. 线程模型

**当前(M2.0a/b)**:`AgentRuntime` 用专用线程持有 current-thread tokio,
`AnswerSource::start` 在该线程执行;`Runner` 仍在 GTK 主线程用 16ms
`timeout_add_local` drain。demo agent 在 `start` 内同步发射,fake agent 自己起
一个 worker 线程模拟异步生产者。

**M2.0b 已实现的结构**:

```
GTK main thread                      runtime thread
---------------                      --------------
AnswerSource::start(prompt, sink) -> 接收 start 请求,驱动 agent
glib::idle / timeout_add_local  <----  AgentEvent 经 mpsc 回传
更新 App + refresh                 ----> HostCommand(cancel)
```

- **一个**专用 runtime 线程持有 `tokio::runtime::Runtime` 并持续 `block_on`/drive;
  不再只在挂载期跑一下。
- agent 任务不直接碰 widget;所有 UI 更新都回到 GTK 主线程。
- `HostCommand`(至少 `Cancel`)走独立通道;背压用有界 channel,满了就阻塞 agent
  而不是丢事件(丢弃策略要显式记录)。
- agent panic 必须在 runtime 线程被隔离成 `Failed` 事件,不能带崩宿主。
- 退出时先取消所有 handle,再关闭 runtime,保证无悬挂任务。

## 4. 贡献热更新

`Contribs` 继续作为**挂载期声明**的快照;agent 产生的内容走 `AgentEvent` /
`UiEvent`,不修改 `Contribs`。这样"数据插件 + 宿主渲染"的边界不变,也避免把
运行期状态塞进插件注册表。

如果后续需要插件在运行期新增导航/菜单,再单独设计 `UiEvent` 的贡献通道;
M2.0a 不把它和 agent 流混在一起。

## 4.5 ACP 真实 agent ✅

`crates/plugins/eb-plugin-acp-agent` 把外部 ACP agent 接成 `AnswerSource`：
启动子进程、`initialize` → `session/new` → 每次 `session/prompt`，把
`agent_message_chunk` 映射为 `Delta`、`usage_update` 映射为 `Usage`、
`tool_call_update` 的 execute/diff content 映射为 `Exec`/`Diff`，其余为
`Tool`。会话跨 turn 保留；`session/cancel` 由 `AnswerHandle::cancel` 发出；
权限请求默认弹 GTK 模态框让用户 allow/reject，`EVOBUDDY_ACP_AUTO_APPROVE=1` 可绕过。

配置与限制见 [`docs/acp-agent.md`](acp-agent.md)。已验证 fake ACP（`eb-plugin-acp-agent`
10 个测试，含 cancel、`EB_ACP_PROBE` 的握手探针和「响应之后还在干活」的尾流）与真实
opencode 的 initialize/prompt 链路；opencode 用本机认证，dsh 已从 ACP profile 改成路由里的
独立进程内路线（需要本地 `DEEPSEEK_API_KEY`）。

## 5. dsh 边界(已先选 A)

- **A. 进程内 Rust agent**(当前):实现简单,线程/取消可控;dsh 生态另做适配。
- **B. 外部 agent 进程**(已由 ACP 兑现):dsh/opencode 作为 ACP stdio 子进程;
  `AgentEvent` 仍是宿主内部合同,ACP 只是它的一个生产者。

`AgentEvent` 是传输无关的;ACP 只是把同一组事件从子进程翻译进来。M2.0a/P0.4
先用 **A + fake agent** 打通线程与重绘;B 已经由 ACP 插件落地,不把 UI surface
绑到 Node 进程模型上。

## 6. 验收

- ✅ fake agent:按 `Started → Status → Delta* → Block → Usage → Finished` 发射,
  带延迟;`EB_FAKE_DEMO=1` 等终态退出,`EB_CANCEL_AFTER_MS` 得到 `Failed/Cancelled`。
- ✅ 顺序:同一任务的 user turn 先于 agent 事件;同一任务的新提交把旧回答标成 `Cancelled`。
- ✅ 并发:任务 A 流式期间另起任务 B,A 继续在自己的会话里流到终态
  (`EB_TWO_TASK_TEST=1` harness,回归时以退出码 1 报 `two-task: FAIL`)。
- ✅ 测试:`cargo test` 覆盖 sink 关闭、取消、事件顺序;Linux/macOS verify 跑
  fake streaming/cancel。
- ✅ demo agent 同步适配;M2.0a 时截图仍为 `5b0eef6b…`,M2.2/P2 后 baseline 为 `5d42eebb…`。
- ✅ M2.0b:专用 runtime 线程、有界 channel(容量 64)、UI stop、start panic
  隔离、慢流期间 UI 可交互的 harness 已完成。
- ✅ ACP:文本流、usage、exec/diff/tool 块、权限策略、cancel、跨 turn session
  已接;fake ACP 与真实 dsh/opencode 验证。
- ✅ ACP `plan` 已映射为 Plan 卡；⏳ 仍待:权限记忆/历史、thought、session resume/list、图片/
  resource prompt、贡献热更新。

## 7. 待确认

1. `Delta` 的粒度:token 级还是 20–50ms 批量?
2. 工具调用是否也用 `Block` 增量,还是单独 `ToolEvent`?
3. 一个任务是否允许多个并发回答(重试/分支)?M2.0a 先按单回答设计。
4. dsh B 方案的时间点:协议稳定后、M2.1 采集之后,还是真实 agent 接入时?
5. 会话持久化是否属于 M2,还是留给 dsh/真实 agent 接入?
