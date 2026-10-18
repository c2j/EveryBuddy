# DSH plugin acceptance: what runs, what is ported

This page records the acceptance check for "rutis hosts a dsh plugin" and the
visible UI demo that follows from it.

## 1. What actually runs unmodified

`deepseek-harness/EvoBuddy/host/demo-02-real-plugin.mjs` loads
`packages/fs/tool-fs/lib` without changing it:

```
loaded packages/fs/tool-fs unmodified (read only)
  registered tools: ["read"]
  read /workspace/src/greet.ts: 7 lines
  window offset=5 limit=2 -> lines [5, 6]
  absent file -> FsError FS_NOT_FOUND
  escape attempt -> FsError FS_PERMISSION_DENIED
```

That plugin is a **Node/Cordis ES module**. The prototype host is Node, with a
Rust/WASM kernel behind the `fs` service. rutis is the Rust plugin runtime in
EvoBuddy; it does not execute JavaScript and has no dsh-plugin ABI. So the
literal claim "rutis loads a dsh plugin" is false; the honest split is:

| Layer | Runtime | What it hosts |
|---|---|---|
| EvoBuddy host | Rust + GTK | `eb-api` plugins (rutis) |
| dsh plugin | Node + Cordis | JS/TS packages such as `tool-fs` |
| Bridge | sidecar / co-kernel | services, not GTK widgets |

A real dsh **client** plugin is React/Cordis UI; rendering it inside this GTK
app would need a WebView or a separate JS client process. That is not in the
current build.

## 2. Visible port: `dsh-client-ui-message-feedback`

To evaluate the *effect* in the running app without pretending to load JS, a
small rutis plugin (`crates/plugins/eb-plugin-dsh-feedback`) ports the
declaration of DSH's Like/Dislike pair onto EvoBuddy's `MessageAction`
contribution surface.

- Source of the effect: `@deepseek-ai/dsh-client-ui-message-feedback`
- Rendered: two 16px thumb buttons at the end of a finalized assistant turn
- Click: a small popover says Liked/Disliked and that it was recorded locally
- This is a **port**, not a JS loader; the tooltip and this page say so
- Screenshot harness: `EB_DEMO=1 EB_SELFTEST=1 EB_SHOT=/tmp/eb-feedback2.png`

The plugin is still useful as a plugin-surface acceptance: the host renderer
only knows `MessageAction { id, icon, tooltip }`; adding/removing the plugin
changes the action strip without touching `conversation.rs`.

## 3. What a faithful JS plugin integration would take

1. Start a Node/Cordis sidecar (the existing `EvoBuddy/host` prototype).
2. Keep dsh's `sessions`/`agents`/`tools` registries in that JS process (they
   return live objects synchronously; a Rust process cannot fabricate them).
3. For client UI, either embed WebKitGTK and mount the dsh client, or define a
   typed bridge that sends declarative contributions to GTK.
4. Package Node + the selected dsh packages, or make Node a documented runtime
   dependency.

None of those are a small UI change; they are a separate integration boundary.
