# Experiment: Settings → Plugins (runtime dsh host plugins)

This branch adds an experimental Settings window to EvoBuddy:

- open it from the account menu → **Settings**
- the **Plugins** section lists a small catalog of dsh host packages that can
  run through `rutis-loader` without a host-provided service
- selecting rows and pressing **Apply & Reload** writes a `rutis.json` and
  drives a managed `rutis-host run --watch-config` process
- adding/removing rows is reconciled by `--watch-config`; **no Rust rebuild**

It is a sidecar integration: EvoBuddy does not link `rutis-loader` in-process
yet. The panel proves the runtime-loading path and gives a place to iterate on
the real embedded loader.

## Prerequisites

- `rutis-host` 0.12 binary (the experiment was run with a debug build at
  `/tmp/rutis/target/debug/rutis-host`)
- Node 24 (`EVOBUDDY_NODE_BIN`)
- an npm project containing the dsh packages, e.g. the `rutis-dsh` npm project
  (`EVOBUDDY_DSH_NODE_PROJECT`)

## Run

```sh
EVOBUDDY_RUTIS_HOST=/tmp/rutis/target/debug/rutis-host \
EVOBUDDY_DSH_NODE_PROJECT=/tmp/rutis/crates/rutis-dsh/dsh \
EVOBUDDY_NODE_BIN=/path/to/node24 \
cargo run
```

Open the account menu → Settings. Select **Invariants** and **Credentials**,
then **Apply & Reload**; statuses become `running` and the log shows the
`rutis-host` output.

Automated screenshot harness:

```sh
EVOBUDDY_RUTIS_HOST=... \
EVOBUDDY_DSH_NODE_PROJECT=... \
EVOBUDDY_NODE_BIN=... \
EB_SETTINGS=1 EB_SETTINGS_SELECT=invariants,credentials \
EB_SETTINGS_SHOT=/tmp/eb-settings-panel.png \
cargo run
```

## Observed result

```text
settings: apply selected={"credentials", "invariants"}
settings: started /tmp/rutis/target/debug/rutis-host with .../evobuddy-dsh-plugins-....json
settings: credentials: running
settings: invariants: running
```

Screenshot: `/tmp/eb-settings-panel.png`.

## Limits

- Rows are limited to host/Cordis packages; dsh client UI plugins are not
  loadable here (they need the dsh Web client).
- The panel drives an external `rutis-host`; process lifetime is tied to the
  Settings window close in this spike.
- The catalog is intentionally small and curated; a real catalog needs
  dependency/`check` discovery.
- No in-app services are exposed to the GTK conversation yet; this validates
  dynamic plugin rows and status, not GTK consumption of the services.

## Plan B: the Files panel only exists with `fs`

When **Filesystem** is selected, Apply also loads a small local `fs-bridge.mjs`
plugin into the same Node/Cordis runtime. It exposes `/list` and `/read` on
`127.0.0.1:39471` using the real `ctx.fs` service.

The Settings panel then shows **Open Files**. The panel only appears while
`fs` is selected; with no `fs` row there is no Files button and no panel. The
window lists the workspace, navigates directories, and shows file text.

Screenshot: `/tmp/eb-files-panel.png`.

This is the Plan B behavior in a small window: no fs plugin → no file panel;
loading fs → panel appears. Moving the same panel into EvoBuddy's right sidebar
is the next UI step.
