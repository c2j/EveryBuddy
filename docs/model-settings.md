# Native model settings

EvoBuddy Settings now has a **Models** section for the dsh path:

- provider id (`AIMUX_PROVIDER`)
- model id (`AIMUX_MODEL`)
- write-only API key, stored under `~/.evobuddy/credentials.json` with mode 0600
- current active provider/model

`Save & Apply` writes `~/.evobuddy/models.json`, mirrors the values into the
process environment, and stops the live dsh bridge. The next turn restarts the
bridge with the new env, so the selected model is used without restarting the
whole app.

The composer model menu is generated from the saved model
(`Auto` + configured model id). Changing the model requires the app to rebuild
that menu, so the selector text refreshes on the next app start; the dsh turn
itself uses the saved env immediately.

![Settings Models](/tmp/settings-model3.png)

## Why not mount the dsh Models plugin directly?

`@deepseek-ai/dsh-client-ui-settings-models` is a **dsh Web client plugin**:
it is React, injects `@deepseek-ai/dsh-client-ui-settings`,
`@deepseek-ai/dsh-client-locale`, and `@deepseek-ai/dsh-api-remotes`, and renders
through dsh's `settings.models.*` slots. EvoBuddy is a Rust/GTK application with
no dsh Web client runtime or slot registry, so the package cannot be mounted
as-is.

A future integration could either:

1. embed the dsh Web client and this plugin in a WebKit view, or
2. call dsh's `settings`/`credentials` remotes from a native adapter.

The Models page now exposes **Open dsh Models**, which opens the real dsh Web
client (and therefore the real `ui-settings-models` plugin) in EvoBuddy's WKWebView
host on macOS, or in the system browser elsewhere. It uses
`EVOBUDDY_DSH_WEB_URL` and defaults to `http://127.0.0.1:3080`; the dsh web host
must already be running.

The current section is the native equivalent, not a reimplementation of the
full provider catalog editor (model lists, reasoning levels, OAuth accounts,
and custom pi-ai routes remain in the dsh Web Models page).
