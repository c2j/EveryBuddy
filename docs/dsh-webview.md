# DSH WebView acceptance (real dsh plugin UI)

This is the first slice of the WebView route: EvoBuddy starts the **real DSH
Web client** in a WKWebView window instead of porting its React plugins.

## Why a WebView

dsh client plugins are Cordis/React modules registered into browser slots
(`slots.register`, `locale.bind`, ...). They need a DOM and the dsh client host.
rutis is a Rust plugin runtime and cannot execute them. The only way to see the
unmodified plugin UI is to keep the dsh JS host and render its Web client.

## Run the spike

```sh
# 1. Start the real dsh web client from the dsh checkout (temporary home)
cd /path/to/deepseek-harness
DSH_HOME=/tmp/dshhome node --import tsx/esm apps/cli/src/bin.ts \
  web --no-open --port 3199

# 2. Point EvoBuddy at the printed URL and start it
cd /path/to/EvoBuddy
EB_DSH_WEB_URL='http://127.0.0.1:3199/?token=...' cargo run
```

In a conversation, the topbar gets a book icon (**Open DSH plugins**). Clicking
it opens a native WKWebView window containing the actual DSH Web UI; the
Plugins page is the dsh plugin manager itself (built-in, experimental and
installed plugins are the real dsh code).

Harness screenshot of the WebView (no screen-recording permission needed):

```sh
EB_DSH_WEB_URL=... \
EB_DSH_JS='(function(){document.querySelectorAll("[role=dialog],[class*=modal]").forEach(e=>e.remove());const p=[...document.querySelectorAll("button,a")].find(x=>(x.textContent||"").trim()==="插件");if(p)p.click();})()' \
EB_DSH_SNAPSHOT=/tmp/dsh-plugins.png cargo run
```

## What is real / what is still missing

| Item | State |
|---|---|
| dsh Web client and plugin manager | real, loaded from `dsh web` |
| dsh client plugin UI (React/Cordis) | real, rendered by WebKit |
| WKWebView host on macOS | yes, `src/dsh_web.rs` |
| Embedded in the GTK content area | no, currently a separate native window |
| Auto-start of `dsh web` | no, URL is supplied by `EB_DSH_WEB_URL` |
| Linux WebKitGTK | no; `webkitgtk` has no bottle for this macOS and the code is macOS-only for now |
| Packaging Node + dsh | no; this is a developer spike, not a release dependency |

## Next slice

1. Move the WKWebView from the separate window into a `GtkStack` page (the
   content area), keeping the GTK sidebar/header.
2. Start `dsh web --port 0 --no-open` as a managed sidecar, parse its printed
   URL, and stop it on window close.
3. Add a Linux WKWebView/WebKitGTK implementation and package the runtime.
4. Keep the `dsh web` URL/token out of logs; pass it only through the child
   environment/pipe.
