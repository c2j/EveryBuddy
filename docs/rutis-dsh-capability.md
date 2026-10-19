# rutis 0.12 ↔ dsh 能力验证

Date: 2026-10-19. Upstream: `arcships/rutis` main (`rutis` 0.12.1).
This page records an external experiment; it does not add a runtime dependency to
EvoBuddy.

## What upstream claims

- [README.zh-CN.md](https://github.com/arcships/rutis/blob/main/README.zh-CN.md):
  Rust kernel; TypeScript/Python/Go plugins; the lifecycle model comes from
  Cordis; `rutis-dsh` runs dsh's complete web UI with aimux model calls.
- [Cordis guide](https://github.com/arcships/rutis/blob/main/docs/guide/cordis.md):
  a published Cordis plugin can run in real Cordis from a Rust application;
  Rust gets generated types; plugins can also be rutis-loader rows without
  source changes.
- [rutis-dsh README](https://github.com/arcships/rutis/blob/main/crates/rutis-dsh/README.md):
  dsh runs in a Node process started and managed by rutis; `up` launches the
  full dsh web interface; Unix + Node 24 required.

## Experiments run

Environment: rutis `main` at 0.12.1, Node 24.21.0, rustc 1.98.0, macOS arm64.

### A. Published dsh Cordis plugins mounted in Rust

```sh
cd /tmp/rutis
npm --prefix node/baseline ci
npm --prefix node/rutis-runtime ci
cargo test -p dsh-baseline
```

All four integration tests passed:

- `callbacks`: Cordis → Rust closure and Rust → returned function
- `events`: credential change reaches a rutis listener
- `host`: dsh persona uses the `systemPrompt` provided by the Rust host
- `typed`: published dsh plugins through generated typed bindings

The build emitted warnings for members the binder skips: AsyncIterable
streams, generic methods, `AbortSignal` parameters, function-valued properties,
and some intersections. "Mounts" therefore does not mean every member of every
dsh service is usable from Rust.

### B. Full dsh web UI hosted by rutis

```sh
cd /tmp/rutis
npm --prefix crates/rutis-dsh/dsh ci
DSH_HOME=/tmp/rutis-dsh-home \
  cargo run -p rutis-dsh -- up --no-open --port 3198
```

Observed:

```text
dsh web: http://127.0.0.1:3198/?token=…
[rutis-dsh] dsh is up; model routes: deepseek-official, deepseek-account, aimux
```

Without `DEEPSEEK_API_KEY` the host still booted; model calls reported the
missing key. This is the full dsh web application (including its client plugin
UI), not individual dsh widgets in GTK.

## Capability matrix

| Capability | Result |
|---|---|
| Rust process directly executes dsh JS plugins | No; needs Node/Cordis |
| Mount published dsh host/Cordis plugins, generate Rust types | **Yes** (baseline tests) |
| Services, events, callbacks, host-provided services | **Yes**, with documented boundary rules |
| Run the complete dsh web UI under rutis | **Yes** (`rutis-dsh up`) |
| Render an individual dsh client UI plugin as a GTK widget | No; client plugins need a browser DOM |
| Every dsh API member available through generated bindings | No; unsupported members are skipped with warnings |

## Consequence for EvoBuddy

Two honest paths, not interchangeable:

1. **Host-plugin integration**: upgrade the vendored rutis to 0.12, enable
   `rutis-bridge`'s `cordis` feature, add an npm project, and mount selected dsh
   packages. This gives real tools/services/commands, but no dsh client UI.
2. **Full dsh Web**: run `rutis-dsh` (or the equivalent dsh web host) and use
   its Web UI. This gives the real dsh client UI, but not a native GTK surface.

The earlier native port and the WKWebView wrapper were both rejected as
evidence of support; they remain reverted in `main`.
