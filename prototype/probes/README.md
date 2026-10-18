# probes

"Rust 侧的插件框架该用哪个"这个问题的实测证据。这些 crate 不参与 EvoBuddy 的正式实现,留着是因为结论依赖它们。

## 结论

**rutis 可以用,但要满足三个条件。** 最初的判断("rutis 进不了 wasm")是错的——它源自上游过宽的 tokio 特性表,不是能力缺失。

| 条件 | 内容 |
|---|---|
| 1. 目标 | 必须是 `wasm32-wasip1`,不是 `wasm32-unknown-unknown` |
| 2. 依赖 | 必须收窄 rutis 的 tokio 特性:去掉 `net` / `process` / `rt-multi-thread` |
| 3. 宿主 | 必须提供 WASI preview1;Node 自带的 `node:wasi` 正好覆盖 |

## 证据链

### 第一关:编译

上游 `rutis` 0.11.0 的依赖表是:

```toml
[dependencies.tokio]
version = "1"
features = ["rt-multi-thread", "sync", "macros", "time", "net", "process", "io-util"]
```

`wasm32-unknown-unknown` 上 49 个编译错误,源头是 `mio` 落到 `std::net` 与 `sys::event`。换 `wasm32-wasip1` 也一样:tokio 直接

```
compile_error!("Only features sync,macros,io-util,rt,time are supported on wasm.");
```

但**核对源码后发现 `net` 和 `process` 从未被使用**。rutis 库代码用到 tokio 的地方只有:

| 位置 | 用法 |
|---|---|
| `src/effect.rs` | `tokio::sync::Notify` |
| `src/fiber.rs` | `tokio::sync::{mpsc, watch}`,`tokio::task::JoinHandle`,`tokio::time::timeout` |
| `src/ctx.rs` | `tokio::runtime::Handle`,`tokio::time::timeout` |
| `src/bus.rs` | `tokio::task::{JoinHandle, JoinSet}` |
| `src/error.rs` | `tokio::task::JoinError` |

全部落在 `sync` / `rt` / `time` / `macros` 里。`rt-multi-thread` 是单线程 wasm 上不可能存在的特性;`net`、`process` 是纯粹的过度声明。收到 `["sync","rt","time","macros","io-util"]` 后,**编译通过,零源码改动**(`vendor/rutis/` 只改了 Cargo.toml)。

### 第二关:运行时

编译过之后在 `wasm32-unknown-unknown` 上一跑就炸:

```
PANIC: library/std/src/sys/time/unsupported.rs:13:9:
       time not implemented on this platform
```

`wasm32-unknown-unknown` 的 std 没有时钟,而 rutis 的 fiber 驱动要走 tokio 的时间驱动。这不是 rutis 的缺陷——该目标上**任何**依赖时间的运行时都活不了。WASI 提供时钟,所以换 `wasm32-wasip1`。

### 第三关:宿主

`wasm32-wasip1` 产物只需要 8 个导入,7 个是标准 WASI preview1,`node:wasi` 全覆盖:

```
env.host_log
wasi_snapshot_preview1.{random_get, environ_get, environ_sizes_get,
                        clock_time_get, fd_write, poll_oneoff, proc_exit}
```

模块无 `_start`,是 WASI reactor:宿主 `instantiate` → `wasi.initialize(instance)` → 调导出函数。

跑 rutis 自己的招牌场景(provider 换代 → 消费者被驱逐并自动重载):

```
  [wasm] supplier started: English
  [wasm] consumer ran -> hello from English
  [wasm] supplier started: Esperanto
  [wasm] consumer ran -> hello from Esperanto
result: lifecycle ok, consumer state=Snapshot { generation: 2, state: Active, error: None }
```

### 第四关:常驻运行时

内核需要一个活到进程结束的运行时,由宿主每次调用驱动。三次连续调用复用同一个 `OnceLock<Runtime>` + `block_on`,结果确定:

```
--- host call #1 --- ... generation: 2, state: Active
--- host call #2 --- ... generation: 2, state: Active
--- host call #3 --- ... generation: 2, state: Active
```

### 附:内核自己能读文件

WASI preopens 让内核直接用 `std::fs`,不需要为文件操作造自定义 syscall:

```js
new WASI({ version: "preview1", preopens: { "/host": process.cwd() } })
```

```
read_dir OK: 6 entries, first: Cargo.toml, run.mjs, target, Cargo.lock, vendor, src
```

## 复现

```sh
cd EvoBuddy/probes/rutis-wasm
cargo build --release --target wasm32-wasip1
node run.mjs wasm32-wasip1
```

`run.mjs` 也接受 `wasm32-unknown-unknown`,用来看时钟那一步的失败。

对照实验在 `cordis-core-wasm/`:它能在 `wasm32-unknown-unknown` 上跑通生命周期,但需要绕过 tokio 的守卫。

```sh
cd EvoBuddy/probes/cordis-core-wasm
RUSTFLAGS='--cfg tokio_unstable' cargo build --release --target wasm32-unknown-unknown
node run.mjs
```

## 设计影响

选 rutis 不是只换一个库,它把内核目标从 `wasm32-unknown-unknown` 推到 `wasm32-wasip1`,而这**改善了整体设计**:

| | `wasm32-unknown-unknown` | `wasm32-wasip1` |
|---|---|---|
| 时钟 | 无(`Instant::now()` panic) | WASI `clock_time_get` |
| 文件 | 只能自造 syscall 桥 | WASI preopens,`std::fs` 直接用 |
| 随机 | 需自造 | WASI `random_get` |
| 宿主成本 | `WebAssembly.instantiate` | `WebAssembly.instantiate` + `node:wasi` |
| 能力边界 | 由自定义 syscall 列表定义 | 由显式 preopens 定义 |

## 代价

1. **必须 vendored/fork rutis 改那 3 行依赖表**。上游用的是 `tokio = { workspace = true }` 加 workspace 里的 `net`/`process`/`rt-multi-thread`。在收窄前我们背着这个补丁;值得向上游提。
2. **`node:wasi` 是实验特性**,每个进程会打 `ExperimentalWarning`。
3. **WASI preview1 是冻结的旧形态**,Node 只支持 preview1,不支持 component model。
4. **内核内只有一个单线程 tokio 运行时**,由宿主调用驱动;没有真并发,长任务会阻塞整次调用。
5. rutis 0.11.0 是 pre-1.0,文档与注释以中文为主。

## rutis 与 cordis-core 的取舍

| 维度 | rutis 0.11.0 | cordis-core 0.6.1 |
|---|---|---|
| wasm 目标 | `wasm32-wasip1`(需 WASI 宿主) | `wasm32-unknown-unknown`(可直接实例化) |
| 让 wasm 成立的手段 | 收窄**自己的**依赖表(诚实:只声明真正用到的) | `--cfg tokio_unstable` 绕过 tokio 的守卫(`rt-multi-thread` 实际不可用) |
| 时钟依赖 | 有(fiber 驱动用 `tokio::time::timeout`),因此必须 WASI | 基础生命周期无(已验证 spawn/dispose 不需时钟) |
| 依赖 | tokio + tokio-util + thiserror | `default = []`,零必需依赖 |
| 定位 | "Cordis 核心范式的 Rust 惯用实现" | "the semantic runtime contract of Cordis v3" |
| 事件 | 四分发语义(emit / parallel / serial / waterfall) | `Routing`(含 waterfall) |
| 生态 | 另有 rutis-bridge / rutis-loader 等 | cordis-rs / -loader / -hmr / -timer / -group / -include,与 `vendor/` 一一对应 |

两者都能用。**按"让 wasm 成立的手段是否诚实"看,rutis 更好**:它收窄的是自己真实用到的特性,cordis-core 则是绕过 tokio 的守卫继续声明一个单线程目标上不可用的特性。**按宿主成本看,cordis-core 更省**:不需要 WASI。一旦采用 WASI(时钟、文件、随机都要用),这个差别就不重要了。
