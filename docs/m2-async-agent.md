# M2.0a 异步 agent 契约(草案)

> 状态:草案,待确认。目的不是先写线程代码,而是把"同步整轮返回"换成可流式、
> 可取消、可测试的接口。M2.0b 的线程桥以本契约为输入;M2.0c 的 fake agent 是
> 第一份实现,真实 agent 与 dsh sidecar 之后再接。

## 1. 现状与问题

- `eb_api::AnswerSource::answer(&self, prompt) -> (Turn, Option<Artifact>)`
  同步返回一整轮;宿主在 GTK 主线程直接调用它。
- `Host` 的 current-thread tokio runtime 只在挂载期 `block_on`,之后没有驱动;
  `Contribs` 是挂载后的快照。
- `Turn::Agent` 已有 `status` / `streaming` 字段,但没有增量更新通道。

所以 M2.0 的阻塞不是"缺一个线程",而是缺一个**事件契约**:agent 在别的线程
产生开始、增量、块、用量、结束/错误,宿主在主线程消费并重绘。

## 2. 建议的 surface

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

pub struct AgentSink { /* mpsc::Sender<AgentEvent>, cloneable, Send */ }
impl AgentSink {
    pub fn send(&self, event: AgentEvent) -> Result<(), AgentClosed>;
}

pub trait AnswerHandle: Send + Sync {
    fn cancel(&self);
}

pub trait AnswerSource: Send + Sync + 'static {
    /// 非阻塞:启动一次回答,事件经 sink 回传;取消通过 handle。
    fn start(&self, prompt: &str, sink: AgentSink) -> AnswerHandle;
}
```

约定:

- `AgentEvent` 是**纯数据**,可跨线程;`Block` / `Artifact` 沿用现有模型。
- `Delta` 是文本增量,`Block` 是完整块的增量;两者都允许 0..n 次。
- `Finished` 与 `Failed` 互斥且必须恰好出现一次;`sink` 关闭等价于宿主取消。
- 取消是幂等的;`cancel()` 之后允许仍有已排队事件,但宿主必须丢弃。
- 不在 surface 中放 `gtk::Widget`、`glib` 或 `tokio` 类型。

## 3. 线程模型

推荐的 M2.0b 结构(细节在实现时定):

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

## 5. dsh 边界(待决策)

两个选项:

- **A. 进程内 Rust agent**:实现简单,线程/取消可控;但 dsh 生态要另做适配。
- **B. Node/dsh sidecar**:复用现有 agent-loop;需要 JSON/stdio 或本地 socket
  传输,以及会话/工具注册表留在 JS 进程的约束。

无论 A/B,`AgentEvent` / `HostCommand` 都应是传输无关的;sidecar 只是把同一组
事件序列化。建议先用 **A + fake agent** 打通线程与重绘,再评估 B;不要在
M2.0a 把 UI surface 绑到 Node 进程模型上。

## 6. 验收

- fake agent:按 `Started → Delta* → Block* → Usage → Finished` 发射,带延迟;
  UI 保持响应,取消后不再更新,退出干净。
- 失败/panic:产生 `Failed`,宿主显示可读错误,窗口不崩。
- 顺序:同一任务的 user turn 先于 agent 事件;跨任务事件不串台。
- 测试:`cargo test` 覆盖 sink 关闭、取消幂等、事件顺序;CI 跑
  `EB_FAKE_AGENT=1` 的 headless 冒烟。
- 真实 agent 未接入时,demo-agent 可暂时适配为"立即 Finished"的同步实现,
  保持现有 UI 行为。

## 7. 待确认

1. `Delta` 的粒度:token 级还是 20–50ms 批量?
2. 工具调用是否也用 `Block` 增量,还是单独 `ToolEvent`?
3. 一个任务是否允许多个并发回答(重试/分支)?M2.0a 先按单回答设计。
4. dsh 路线选 A 还是 B;若选 B,传输层在 M2.0a 还是 M2.1 之后做?
5. 会话持久化是否属于 M2,还是留给 dsh/真实 agent 接入?
