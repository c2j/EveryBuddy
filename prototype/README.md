# EvoBuddy

度量"用 Rust 内核承载 dsh 插件生态"的可行性工作区。全部脚本只读仓库源码,不修改任何包的**源码**;`kernel/` 与 `host/` 是路线 A(双内核)的可运行原型。

## 目录内容(分析产物)

| 文件 | 作用 |
|---|---|
| `scan.mjs` | 耦合扫描:用 TypeScript 编译器 API 解析每个包的 `src/**`,记录对内核权威状态的直接依赖 |
| `classify.mjs` | 把 `scan.mjs` 的结果按符号级分类,输出"可原样运行 / 被卡住" |
| `unsafe.json` | 12 个"有状态符号"清单。改这个文件即可改变判定,是其唯一调参入口 |
| `inject-surface.mjs` | 统计 `inject` 声明的服务名分布(谁在依赖什么服务) |
| `service-surface.mjs` | **服务成员级契约扫描**:插件实际调用了每个服务的哪些方法 |
| `contract.mjs` | **签名与同步性提取**:定位提供服务的类,读出成员签名/返回类型/是否 `async`,并把每个调用点归类为 awaited / asyncFn / syncFn / fieldInit / topLevel |
| `contract-verdict.mjs` | 把上一步归并为四条出路:asyncOK / syncRegister / syncLive / syncData |
| `report.json` | `scan.mjs` 的完整输出,每个包一条 |
| `service-surface.json` | `service-surface.mjs` 的完整输出,每个 `service.member` 一条 |
| `contract.json`, `contract-verdict.json` | 上两步的完整输出,每个 Host 侧 `service.member` 一条 |
| `scan.out`, `classify.out`, `inject-surface.out`, `service-surface.out`, `contract.out`, `contract-verdict.out` | 六次扫描的原始摘要 |

`scan.out` 里的 `B-viable as-is 91 (40.1%)` 是**分类前的严格读数**(它把每个 authority 符号都当作不安全)。权威判定以 `classify.out` 的 `RUN UNCHANGED ... 208 (91.6%)` 为准,两者的差别就是 `unsafe.json` 里那 12 个有状态符号分类的结果。

## 判定标准

`scan.mjs` 对每个包的 host 源码记录四类信号:

| 信号 | 含义 |
|---|---|
| authority 运行时符号 | 从 `core/*`、`llm/*`、`session/*`、`boot/*`、`api/*` 取**非 type-only** 导入 |
| waterfall | `.waterfall(` 调用点,跨边界不可能转发 |
| `ctx.session` 直访 | 直接操作会话日志(内核权威状态) |
| 有状态类 | `instanceof` / `extends` / `new` 落在内核拥有的类上 |

authority 集合按目录前缀判定,**故意偏宽**,因此"被卡住"的数量只会高不会低。

`service-surface.mjs` 解析这几种形态:`ctx.<svc>.<member>`、`ctx.get('<svc>').<member>`、`const s = ctx.<svc>` 后再 `s.<member>`、`const { m } = ctx.<svc>`。

## 当前快照结论

快照:342 个包、48 个 authority、**227 个插件侧 host 包**、2,484 个 src 文件。

**插件源码的耦合度**

| 桶 | 包数 | 占比 |
|---|---|---|
| 完全不导入 authority 运行时符号 | 144 | 63.4% |
| 导入的符号全是纯函数/错误类/ID 品牌 | 64 | 28.2% |
| **在内核门面下可原样运行** | **208** | **91.6%** |
| 被卡住(去重) | 19 | 8.4% |

被卡住的 19 个包只涉及 **6 个有状态符号**,加上两类模式:waterfall(6 个包)、`ctx.session` 直访(8 个包)。排除 `test-support/*` 后,224 个产品包里 16 个被卡住(7.1%)。

插件从 authority 包取走的运行时符号绝大多数是纯的:`defineTool`(31 个包)、`createUserMessage`(33 个包)、`SessionSeq` / `SessionLogOffset`(品牌构造函数)、`SESSION_FORMAT_VERSION`、`renderPrompt`、`parseCmdline`、以及大量 `*Error` 类。

**内核门面** — 所有包都依赖、且只有约 20 个符号:

```
@deepseek-ai/cordis               Context, Service, Inject, FiberState,
                                  getTraceable, resolveConfig, symbols, *
@deepseek-ai/cordis-plugin-loader EntryGroup, EntryTree, ModuleLoader,
                                  evaluate, interpolate, isJsExpr, default
@deepseek-ai/cordis-plugin-include applyEntryPatches, entryListSchema, default
@deepseek-ai/cordis-plugin-group   default
```

**服务契约** — 118 个服务被触及:74 个 Host 侧、39 个 Client 侧、5 个 Cordis 运行时(`logger`、`fiber`、`events`、`reflect`、`registry`)。共 455 个 `service.member` 组合。

| 侧 | 包→成员边 | 不同成员组合 | 覆盖 50% | 覆盖 80% |
|---|---|---|---|---|
| Host | 718 | 278 | 33 个 | 135 个 |
| Client | 628 | 196 | 15 个 | 72 个 |

Host 侧最重的服务:`sessions`(40 包 / 14 成员)、`tools`(36 / 7)、`agents`(25 / 9)、`sessionProjections`(22 / 4)、`workingDirectory`(22 / 3)、`systemPrompt`(20 / 5)、`subprocess`(15 / 4)、`llm`(13 / 8)。
Client 侧:`slots`(61 包 / 13 成员)、`locale`(55 / 5)、`remote`(31 / 31)。

80% 以上的调用集中在少数成员上:`logger.warn`、`slots.register`、`slots.inject`、`locale.register`、`locale.bind`、`tools.register`、`workingDirectory.ensure`、`remote.$on`、`systemPrompt.section`、`agents.get`。

## 同步性契约(步骤 1+2 的结果)

`service-surface.mjs` 有两件事看不到:**方法是不是 async**,以及**调用点能不能等**。这两步把它们补上。

`contract.mjs` 通过 `super(ctx, '<name>')`、`ctx.provide('<name>', this)` 定位提供服务的类,沿包内继承链读成员声明——参数、返回类型、`async` 修饰符;同时把每个 `ctx.<svc>.<member>` 按所在上下文归类为 `awaited` / `asyncFn` / `syncFn` / `fieldInit` / `topLevel`。

判定规则只有一条:

> 一个成员**必须同步返回** ⟺ 它的声明不是 async,且至少有一个调用点不在 `await` 里。

278 个 Host 侧成员组合的结果:

| 出路 | 数量 | 占比 | 含义 |
|---|---|---|---|
| **asyncOK** | 126 | 45.3% | 声明为 async,或所有调用点都 await。**进程外的桥可以直接承载** |
| syncData | 82 | 29.5% | 同步读,返回普通数据(`systemPrompt.getSectionOrder(): number`、`agents.get(): Agent \| undefined`) |
| syncRegister | 28 | 10.1% | 同步注册,返回 disposer(`tools.register(): () => void`、`systemPrompt.section(): () => void`) |
| syncLive | 4 | 1.4% | 同步读,返回活对象(`sessions.list(): Session[]`、`sessions.get(): Session \| undefined`、`storage.backend`、`otel.createEventReporter`) |
| 未定位到声明 | 38 | 13.7% | 20 个是 vendored Cordis(`logger` 5、`loader` 6、`fiber` 5、`events` 2、`reflect` 1、`registry` 1),5 个是 `ctx.root.*`(根 Context 自身),其余是 0 调用的假阳性与 `ctx.provide` 对象字面量 |

同步性不是边缘情况,而是常态:**276 个插件入口里 256 个是 `export function apply`**,只有 20 个是 `export async function apply`(92.8% 同步);类式插件的构造函数必然同步。所以 `ctx.effect(() => ctx.tools.register(...))` 这类注册必须在 `apply` 返回前完成。

**结论:仓库存量的 async/同步划分,本身就是 Rust 内核的边界。**

| 已 async —— 进程外可承载 | 必须同步 —— 要与调用方同进程 |
|---|---|
| `workingDirectory` 3/3、`ssh` 3/3、`credentials` 6/6、`sandbox` 1/1、`sessionQuery` 10/10、`schedule` 5/5、`userQuestions` 2/2、`workspaceFiles` 2/2、`fs` 10/13、`subprocess` 3/4、`llm` 3/8、`sessionPersistence` 3/4、`sessionQuery` 10/10 | `systemPrompt` 5/5、`sessionProjections` 4/4、`goals` 9/9、`sandboxPolicy` 3/3、`connection` 5/5、`tokenMeter` 2/2、`commands` 1/1、`storage` 2/2、`shellEnv` 2/2、`tools` 5/7、`agents` 5/9、`sessions` 3/5 |

同步成员分布在 55 个包里,但真正无解的是"同步 + 返回活对象"的那 4 个:`sessions.list`、`sessions.get`、`storage.backend`、`otel.createEventReporter`。`Session`、`Agent` 是带方法标识的活对象,Rust 内核无法凭空产出——**会话 / 智能体 / 工具这三张注册表必须与插件同进程**。

于是两条路线的裁决变成一个可证伪的命题:B 不成立的原因不是插件耦合,而是**内核必须同步返回 `Session | undefined`**。要让 B 成立,只能把注册表留在 JS——那 B 就退化成了 A。

### 怎么验证

```sh
# 92.8% 的插件入口是同步的
grep -rn "export function apply"       packages/*/*/src --include=*.ts | wc -l   # 256
grep -rn "export async function apply" packages/*/*/src --include=*.ts | wc -l   # 20

# 必须同步返回的三个典型声明
sed -n '1360,1362p' packages/core/session/src/index.ts   # list(): Session[] -> [...this.store.values()].map(e => e.session)
sed -n '568,570p'   packages/core/agent/src/index.ts     # get(id: SessionId): Agent | undefined
sed -n '1071,1073p' packages/core/tools/src/index.ts     # register(definition: ToolDefinition): () => void

# 非 async 函数体里直接取用返回值,所以不能改成 await
grep -rn "ctx.sessions.get(" packages/*/*/src --include=*.ts | head
```

## 盲区

扫描只见**直接 import** 与**成员调用**。它看不到语义:同步与异步、调用顺序、对象身份(`instanceof` 之外的身份假设)、错误类型与错误路径、以及事件分发顺序。真正的迁移成本在这些语义里,不在符号数量里。

## 重跑

```sh
cd <repo-root>
node EvoBuddy/scan.mjs .                 # 耦合扫描,产出 report.json
node EvoBuddy/classify.mjs               # 应用 unsafe.json 判定
node EvoBuddy/inject-surface.mjs         # inject 服务名分布
node EvoBuddy/service-surface.mjs .      # 服务成员级契约,产出 service-surface.json
node EvoBuddy/contract.mjs .             # 签名与同步性,产出 contract.json
node EvoBuddy/contract-verdict.mjs .     # 四条出路归并,产出 contract-verdict.json
```

`contract.mjs` 之后再跑 `contract-verdict.mjs` 的顺序是必需的:后者读前者的输出。

## 原型(路线 A:双内核)

`probes/README.md` 记录的测量裁决了技术选型:**rutis + `wasm32-wasip1`**。实现如下:

```
EvoBuddy/
├── kernel/                     Rust 核心 → 同一份代码,rlib(原生)或 cdylib(wasm)
│   ├── src/bin/evobuddy.rs     Rust 侧驱动器:tour(巡礼)/ shell(交互)/ bench(基准)
│   ├── vendor/rutis/           已裁决的 rutis 分支(tokio 特性收窄)
│   └── src/
│       ├── abi.rs              C ABI:kernel_version/alloc/free/boot/request/shutdown
│       ├── mem.rs              帧式堆边界(4 字节小端长度 + JSON)
│       ├── protocol.rs         双向 JSON 消息(Request/Response/HostCall)
│       ├── host.rs             唯一出站通道 env.host_call(日志;未来回叫)
│       ├── walk.rs / search.rs Rust 独有能力:进程内多模式检索(aho-corasick)
│       ├── fs.rs               受限文件系统能力(WASI preopen 限定)
│       ├── policy.rs           SearchGate waterfall:检索前策略链(可否决)
│       ├── plugins.rs          rutis 插件:provider(注册)+ consumer(依赖重启)
│       └── kernel.rs           rutis 根上下文 + 请求路由└── host/                       Node 宿主
    ├── src/wasm-kernel.js      装载 wasm、WASI、帧式读写、host_call 分发
    ├── src/co-kernel.js        真 vendored Cordis + 服务适配(fs/evoSearch ← 内核)
    ├── contracts/              契约插件(dsh 插件形态,由 Rust 服务)
    ├── demo-00-kernel.mjs      ABI 端到端(含热插拔与结构化拒绝)
    ├── demo-01-contract.mjs    契约插件跑在真 Cordis 上,注入由 Rust 兑现
    ├── demo-02-real-plugin.mjs 未改动的 packages/fs/tool-fs,fs 由 Rust 兑现
    ├── demo-03-chat.mjs        存量 dsh 栈接入大模型:最小对话闭环(需 DEEPSEEK_API_KEY)
└── chat/                       纯 Rust 对话智能体(无 Node/wasm/JS)
    ├── Cargo.toml              依赖:evobuddy-kernel(path)+ ureq(SSE)+ serde_json
    └── src/
        ├── main.rs             智能体循环 + REPL;工具 = read/search,经内核请求面执行
        └── anthropic.rs        Anthropic Messages 流式客户端(与 dsh llm-deepseek 同一线协议)
```

### 架构裁决的落地形态

| 设计点 | 实现 | 证据 |
|---|---|---|
| 插件框架 = rutis | vendor 分支,`Ctx::root_with(handle)` + `block_on` 驱动 | 28 个原生 `cargo test` |
| Rust 插件也走 WASM | 内核本身是 cdylib;服务由 rutis 插件注册 | demo-00 的 4 个 fiber |
| 热插拔 | 替换 provider → rutis 依依赖驱逐 consumer → 重新扫描 | demo-00 reload、demo-02 热切模式 |
| Rust 独有能力 | 进程内多模式检索,无子进程 | demo-01:51 匹配/0.4ms |
| 真正的 dsh 插件 | `packages/fs/tool-fs/lib` 原样装载 | demo-02:read 工具全链路 |
| 双内核分工 | Cordis 管插件生命周期;内核管检索+fs | demo-02:真 ToolRuntime + 内核 fs |

### 已验证的三个开放项

1. **rutis waterfall 越界语义** — `policy.rs` 的 SearchGate 在 `search.find` 前跑完整 waterfall:监听器可在链中咨询内核服务(`FsService`)、可否决(不调 `next` 即截断,后续监听器与终态都不再执行)、可注册/注销(`Disposer::dispose()` 是显式 await,rustis 的 drop 不触发清理)。demo-00 里 `/outside` 请求被以 `search_denied` 拒绝。
2. **内核 panic 行为** — release profile `panic = "abort"`:panic hook 先经 `host_call` 报日志,然后模块整体 trap。宿主侧的对策是实例级重建(载入新实例),而不是假装半初始化的内核可用。此为设计决定,未做故障注入演示。
3. **Node-Cordis ↔ Rust-rutis 服务互通** — demo-01/02 即证明:宿主侧 Cordis `reflect.provide('fs', …)` 把内核请求包装成 `FileSystem` 形状,真实 tool-fs 的 `ctx.fs.resolve/stat/readText/processPath` 全部由 Rust 兑现;错误经 `FsError` 码(`FS_NOT_FOUND`/`FS_PERMISSION_DENIED`)还原 dsh 错误分类。

### 实现中发现的事实(反直觉的记下来)

- **rutis 的 `require` 是插件界面的严格读**:校验调用方 fiber 是否声明了依赖;嵌入方(请求处理器)不是插件,必须用非严格的 `get`。这不是缺陷——它使"provider 被替换后无人能持旧引用"成为可验证性质。
- **rutis 的 `Snapshot.generation` 是父上下文装载代数,不是 fiber 重启次数**;观察热插拔要看 `PluginId`(每次替换都变)与扫描报告序列,不能看 generation。
- **WASI 在能力层就拒绝逃逸**(`ENOTCAPABLE`, errno 76):`canonicalize` 逃出 preopen 时根本到不了内核自己的前缀检查。两层防御返回同一个 `fs_outside_root` 码。
- **macOS 上 root 必须先规范化**:`/var/...` 是 `/private/var/...` 的别名,不规范化则所有解析都被前缀检查误杀。
- **`Uint8Array.prototype.toString()` 是十进制列表拼接**,不是 UTF-8 解码;只有 `Buffer` 的 toString 会解码。宿主读 guest 内存必须过 `Buffer.from`。
- **Node 的 `WebAssembly.instantiate(buffer)` 返回 `{module, instance}`**,必须解构。
- **包的构建产物分两层**:tsc 出 `lib/types/`(类型),tsdown 出 `lib/index.js`(运行时)。只跑 `tsc -b` 不会刷新运行时包——demo-02 曾因此装载到陈旧的 tool-fs(无条件注册全部工具的旧版)。刷新用 `pnpm run build:artifacts`。

### 怎么运行

```sh
cd <repo-root>

# ① Rust 侧驱动内核(原生,无 wasm、无 Node)—— 三档体验
cd EvoBuddy/kernel
cargo test                                       # 28 个纯逻辑测试,约 1 秒
cargo run --release -- tour .                    # 巡礼:启动/检索/策略否决/热插拔/fs
cargo run --release -- shell .                   # 交互式外壳:现场敲命令驱动内核
cargo run --release -- bench ../.. "waterfall" "hot-plug" "provide("   # 全树基准
#   shell 命令面:find / read / use / state / reports / stat / bench / quit
#   基准读数(2026-10,本机):19,945 文件 / 168 MB / 三模式单遍 ≈ 440 ms ≈ 380 MB/s
cargo build --release --target wasm32-wasip1     # ② wasm 构建(同一份内核,加 abi 薄壳)

# ①′ 纯 Rust 对话智能体:无 Node / 无 wasm / 无 JS
cd ../chat
DEEPSEEK_API_KEY=… cargo run --release -- --root ../..                     # 交互 REPL
DEEPSEEK_API_KEY=… cargo run --release -- --root ../.. --ask "一句话问题"    # 单轮
#   工具 read/search 经内核请求面执行(fs.resolve/readText/search.find + 策略闸门);
#   换检索模式 = 一次内核热插拔(provider 换代、sweep 插件重扫),这是架构裁决。
#   EVO_MODEL 覆盖模型(默认 deepseek-chat),DEEPSEEK_BASE_URL 覆盖端点。

# ③ Node 宿主:四个演示(依赖 ② 的 wasm)
cd ../host
node demo-00-kernel.mjs        # ABI 端到端 + 热插拔 + 策略拒绝
node demo-01-contract.mjs      # 契约插件(真 Cordis,搜索由 Rust 兑现)
node demo-02-real-plugin.mjs   # 真 tool-fs(fs 由 Rust 兑现;需先 build:artifacts)
DEEPSEEK_API_KEY=… node demo-03-chat.mjs                # 与大模型对话(存量 dsh 栈)
DEEPSEEK_API_KEY=… node demo-03-chat.mjs "一次性提问"     # 单轮模式
```

demo-02 要求仓库构建产物是最新的(`pnpm run build:artifacts`);demo-03 需要
`DEEPSEEK_API_KEY`(缺失则按仓库 e2e 约定自跳过)。

**demo-03 接入的存量栈**:dsh-llm(LlmRuntime/消息词汇/BlockAssembler)、
llm-deepseek(协议适配器,Anthropic Messages 线协议)、system-prompt、tools
(真 ToolRuntime)、tool-fs(read 工具,fs 由 Rust 兑现)。演示自带的胶水只有
两块且已标注:`settings` 桩(适配器只调用 `configure` 一个方法)与凭据直读
环境变量(真实部署走 llm-deepseek-api-key 插件 + credentials 服务)。

**为什么演示在 Node 侧**:路线 A 的主张是"dsh 插件一行不改,服务由 Rust 兑现"——插件与 Cordis 是 JS,命题的另一端必须有 JS 运行时;且 wasm cdylib 没有 `main`,永远是宿主实例化模块。Node 是路线 A 里的宿主,不是演示布景。反过来,**内核本身不依赖 Node**:它既是 cdylib 也是 rlib,① 与 ①′ 都是从 Rust 进程直接驱动的同一个 `Kernel`。`chat/` 更进一步:线协议(Anthropic Messages,与 dsh llm-deepseek 同一格式)、工具循环、能力全部原生——它证明内核单独就能构成完整智能体;而"纯 Rust 进程跑 dsh 插件"仍需嵌 JS 引擎,那是另一条架构(注册表须与插件同进程的结论不变)。两个对话演示的关系:demo-03 验证"dsh 存量栈可用",`chat/` 验证"内核自足"。

### 边界(诚实记录)

- demo-02 只验证 read 工具;write/edit 需要 `fs.writeText/editText`(内核侧未实现)与 `sandboxPolicy` 等宿主服务。
- `workingDirectory` 是最小桩(`ensure → undefined`);无 Agent 的工具调用不会触达它。
- `fs.streamText` 是单块实现;`readStreamMinSize` 默认 10 MiB,演示文件不触达流式路径。
- 内核请求全部同步推进(wasm 内没有事件循环),回复到达即工作完成——这对宿主是简化的好事,但意味着长扫描会阻塞宿主调用线程。
- demo-03 未挂 dsh 的 agent-loop/session/agent 注册表:对话循环是演示自带的最小驱动(约百行),用的全是 dsh 的消息词汇与装配器。这意味着"会话持久化、子代理、审批"等 agent-loop 能力尚不在演示范围内——它们正是 `syncLive` 清单所指向的宿主侧注册表,挂进来是后续工作而非架构障碍。

## 下一步候选

1. 对 19 个被卡住的包做逐包裁决:改写、门面兜底、还是保持原样。
2. 把 `syncRegister` 的 28 个成员按"注册即忘记(可异步转发)"与"注册后必须立即可查(需要本地镜像)"再分一层——这一层决定 Rust 侧要不要维护镜像注册表。
3. 验证 `ctx.root.*` 那 5 个调用点(反射进根 Context):它们是扩展自省工具,还是产品路径上的必需能力。
4. 内核侧补 `fs.writeText/editText`,让 demo-02 的 write/edit 工具也跑通(需要 `fs/write-intent` waterfall 对接 Rust 的 SearchGate 同款模式);`chat/` 的工具面同步扩展。
5. panic 故障注入演示:宿主捕获 trap 后重建实例的完整路径。
6. 插件动态装载:用户写的 rutis 插件编译成独立 wasm 模块,内核运行时实例化/卸载——热插拔从"换模式"升级为"换插件"。
