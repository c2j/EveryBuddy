# Experiment: dsh ui-message-feedback in a component host

This is the second route-1 plugin, chosen because it exercises the parts
`ui-theme` did not:

- a `conversation.chat.assistant-actions` slot
- an assistant `messageId`
- `remote.messageFeedback.list/put/delete`
- `remote.sessionFeedback.record`
- a slot store (`useFeedback`) and a per-session feedback controller

The plugin bundle is the **unmodified**
`@deepseek-ai/dsh-client-ui-message-feedback/lib/client.js`.

## Run

```sh
EB_THEME_HOST="$PWD/experiments/ui-feedback-webview/index.html" \
EB_THEME_SHOT=/tmp/eb-feedback-host.png \
cargo run
```

`index-like.html` is a deterministic click harness. It clicks **Like** after
1.5 seconds and the page log becomes:

```text
dialog opened: positive
```

Screenshot: `/tmp/eb-feedback-like.png`.

## What is real / fake

| Part | State |
|---|---|
| plugin bundle and React components | real |
| message-feedback controller (plugin code) | real |
| assistant message/session | fake `session-demo` / `m1` |
| `remote.messageFeedback` | in-memory fake list/put/delete |
| `remote.sessionFeedback` | in-memory fake record |
| feedback dialog overlay | not rendered; `openDialog` only logs |
| EvoBuddy right sidebar | not integrated; native window |

This proves the plugin can run and react without a full dsh client boot, but it
also shows what a product host must provide: a real session/message context and
real remotes. The Like button currently opens the plugin's dialog path; the
dialog itself is intentionally not part of this proof.

## Dialog overlay (second slice)

The host now registers the second slot, `conversation.input.overlay`, and passes
the plugin's own `FeedbackDialog` component. Clicking Like opens the real
plugin dialog state:

```json
{"kind":"message","messageId":"m1","rating":"positive"}
```

The dialog's category chips, textarea and Submit button render from the plugin.
The Modal/Toast primitives are minimal stubs; the dialog is not persisted to a
real backend. Screenshot: `/tmp/eb-feedback-dialog8.png`.

## Snapshot + persistent remote (intermediate A)

The message/session are now taken from the real dsh snapshot
`snapshots/session/empty-response-retry-current` (message id
`snapshot-message-4`, text `Recovered.`), not a made-up session.

The fake remote now persists entries in `localStorage` under
`evobuddy-feedback:<sessionId>`, so the Like/Dislike state survives reopening
the preview. This is still **not** a live dsh host/remote: it is a real session
fixture plus a real persistence boundary, used to validate the bridge shape.

Screenshot: `/tmp/eb-feedback-snapshot.png`.

## Live A2 bridge

The real A2 path is wired end to end:

```text
rutis-dsh generic agent composition (scripted aimux)
  → live dsh Session + assistant Message
  → real dsh messageFeedback service
  → EvoBuddy bridge crate (Rust HTTP :39472)
  → WebKit component host
```

The component host loads `/session` and uses `/feedback`, `/feedback/put`,
`/feedback/delete`; when the bridge is absent it falls back to the
snapshot/localStorage fixture.

The bridge lives in EvoBuddy, not in the rutis submodule:

```sh
cd experiments/rutis-live-bridge
cargo build
./target/debug/evobuddy-rutis-bridge
```

It prints a live session and listens on `http://127.0.0.1:39472`.

Serve the component host over HTTP (a file:// page cannot reliably call the
bridge):

```sh
cd experiments/ui-feedback-webview
python3 -m http.server 39473 --bind 127.0.0.1
```

Then:

```sh
EB_THEME_HOST="http://127.0.0.1:39473/index.html" EB_THEME_SHOT=/tmp/eb-feedback-live-rating2.png cargo run
```

Settings → **Live dsh bridge** starts the same binary and shows
`running · http://127.0.0.1:39472`.

## Boundary

Kept in the rutis submodule (generic, candidate upstream issue/PR):

- `AgentProbe.runPersistent/liveSnapshot/disposePersistent`
- `session-persistence-jsonl` and `message-feedback` in the generic agent
  composition
- the test helper config for those services

Kept in EvoBuddy:

- `experiments/rutis-live-bridge/` (scripted LLM, HTTP server, session choice)
- EvoBuddy-specific `/tmp/evobuddy-rutis-sessions` root
- the WebKit component host and its fake/native services

## Right-side panel

The artifact panel header has a **◐ dsh feedback panel** button. It opens the
component host as a borderless native panel anchored over the artifact pane,
so WebKit input works. Closing the artifact panel closes the panel. The panel
uses the embedded localhost preview server (`127.0.0.1:39473`), not `file://`,
and ensures the live bridge is healthy before opening.

Harness:

```sh
EB_PANEL=1 EB_PANEL_SHOT=/tmp/eb-panel3.png cargo run
```

Screenshot: `/tmp/eb-panel3.png`.

The bridge process is now owned by `crates/eb/src/bridge.rs`: Settings
Start/Stop, preview/panel Ensure, and app shutdown all use the same process.
`bridge::start()` waits for `/session` on 39472 before returning, so a panel
opened immediately after startup does not fall back to the snapshot fixture.

## First-click validation

With Accessibility granted, the panel was tested with a real CGEvent click:

- first click on Like opened the real feedback dialog
- Submit wrote the item to the live rutis messageFeedback service
- the bridge returned `rating: positive`

Harness: `EB_PANEL=1 cargo run`, then click Like in the 420-wide panel.
Screenshot: `/tmp/eb-e2e18.png`.

The panel is a 420-wide native drawer anchored to the artifact body; the
underlying artifact body is hidden while it is open.
