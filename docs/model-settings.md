# Native model settings

EvoBuddy Settings now has a sidebar and a **Models** page.

### ACP agents

Choose the external agent mounted at startup:

- `opencode` (`opencode acp`)
- `Grok Build` (`grok agent stdio`)

Installed state and the resolved command are shown. Saving writes
`~/.evobuddy/acp.json`; the host reads it at startup (or
`EVOBUDDY_ACP_AGENT` overrides it).

Each agent uses its default location (`~/.opencode/bin/opencode` or
`~/.grok/bin/grok`). Override it when the binary is elsewhere:

```sh
EVOBUDDY_OPENCODE_BIN=/opt/opencode/bin/opencode
EVOBUDDY_GROK_BIN=/opt/grok/bin/grok
```

A bare command name is resolved on `PATH`:

```sh
EVOBUDDY_OPENCODE_BIN=opencode
```

The default agent is still selected with `EVOBUDDY_ACP_AGENT=opencode|grok` or
the Settings radio buttons; the row shows the resolved path.

### Custom Model

For the dsh path:

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

Settings has a sidebar with General / Models / Built-in plugins / Agent presets /
About. Models is a grouped list: Default route, ACP agents, Provider keys, and
Custom Model — no tabs, so the current default and every option stay visible in
one scroll.

![Settings Models](/tmp/settings-group.png)

The host now mounts an `AgentRouter` as its single answer source. Every new turn
resolves the current route, so switching ACP / dsh / custom model does not restart
EvoBuddy. A running turn keeps its old source and finishes; the next message uses
the new route.

Settings' Default route is global. The composer model menu also offers:

- `Default` — follow the current global route
- `ACP · opencode`
- `ACP · Grok Build`
- `dsh`
- `Custom Model`

Selecting one writes `Task.agent_route`, so that conversation keeps its own route
across sends and restarts while other conversations keep the global default.

When an existing conversation switches routes (or a task without an override
follows a changed global default), EvoBuddy injects a bounded plain-text
transcript of the prior visible turns into the next prompt. ACP cannot export its
session, so this is a best-effort context migration: text and tool names are
preserved, but the old agent's internal session, token cache, and exact tool
state are not.

### Custom Model

The custom route takes a display name, an OpenAI-compatible Base URL, model id,
and write-only API key. It posts to `<base>/v1/chat/completions` through `curl`
(one non-streaming request per turn).

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
