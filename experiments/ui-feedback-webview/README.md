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

The real A2 path is now wired end to end:

```text
rutis-dsh agent turn (scripted aimux)
  → live dsh Session + assistant Message
  → real dsh messageFeedback service
  → HTTP :39472
  → WebKit component host
```

The component host no longer needs localStorage for this path. It loads
`/session` and uses `/feedback`, `/feedback/put`, `/feedback/delete`; when the
bridge is absent it falls back to the snapshot/localStorage fixture.

Start the bridge:

```sh
cd vendor/rutis-0.12
EVOBUDDY_LIVE_BRIDGE=1 cargo test -p rutis-dsh --test feedback_bridge -- --nocapture
```

The test holds the bridge for 300 seconds after seeding one real feedback item.

Serve the component host over HTTP (a file:// page cannot reliably call the
bridge):

```sh
cd experiments/ui-feedback-webview
python3 -m http.server 39473 --bind 127.0.0.1
```

Then:

```sh
EB_THEME_HOST="http://127.0.0.1:39473/index.html" \
EB_THEME_SHOT=/tmp/eb-feedback-live-rating2.png \
cargo run
```

Observed: session `evobuddy-live-…`, assistant `The code is RESULT:m2.`,
rating `positive` loaded from the real service, and Submit wrote the item back
to the live service. Screenshot: `/tmp/eb-feedback-live-rating2.png`.

The bridge is still a test harness using a scripted LLM, not a long-running
production daemon. The next step after A2 is moving the bridge out of the test
binary into a managed EvoBuddy/rutis process.
