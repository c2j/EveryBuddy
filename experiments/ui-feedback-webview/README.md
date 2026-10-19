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
