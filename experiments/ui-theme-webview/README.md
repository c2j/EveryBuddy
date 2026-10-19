# Experiment: dsh ui-theme in a WKWebView component host

This is the route-1 proof from `docs`/the design discussion:

- take the **real** `@deepseek-ai/dsh-client-ui-theme` client bundle
- run it in a WKWebView with a minimal host page
- provide the services the plugin actually uses: `slots`, `locale`, `configForms`, `shortcuts`, React, and the client store/primitives
- render the plugin's `settings.general.item` registration

The host is deliberately a **stub** around the plugin boundary; it is not the
full dsh client boot. It proves the plugin can render as a component, and shows
exactly which host services must be real for a product integration.

## Run

The built host files are committed, so no npm install is needed for the demo:

```sh
EB_THEME_HOST="$PWD/experiments/ui-theme-webview/index.html" \
EB_THEME_SHOT=/tmp/eb-theme-host.png \
cargo run
```

`theme_view.rs` opens a WKWebView window and, when `EB_THEME_SHOT` is set,
captures the WebKit snapshot to the path.

## Result

Screenshot: `/tmp/eb-theme-host6.png` (or regenerate with the command above).

The plugin's **Appearance** row (Light / Dark / System) and the text-size row
are rendered from the original dsh `ui-theme` bundle, not from a GTK port.
The slider/labels are intentionally minimal: `configForms` and `locale` are
stub services in this proof.

## What is still stubbed / missing

- `configForms` is an in-memory scope, not dsh settings persistence.
- `locale` returns labels from a small map; missing keys are visible.
- The theme is applied inside the WebView; it does not change EvoBuddy's GTK
  CSS.
- This is a separate window, not the right-sidebar slot host.
- The real client boot (`dsh-client-web`, module loader, remotes, full slot
  renderer) is not running; the component host calls the plugin's `apply`
  directly.

## Why this matters

It validates the key route-1 claim: the UI plugin is a React/DOM component and
can be hosted in WebKit without porting or wasm. It also shows that the hard
part is the **host service surface**, not the plugin renderer.
