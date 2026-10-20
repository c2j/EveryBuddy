# Linux AppImage runtime notes

The AppImage intentionally carries its own GTK/GLib/fontconfig and fonts. On a
desktop host this changes what the launcher must sanitize:

## Console messages

- `Failed to load module "ukuireload-gtk-module"` — the host exported
  `GTK_MODULES=ukuireload-gtk-module` (Kylin/UKUI). The bundle does not ship that
  module. `run.sh` unsets `GTK_MODULES`/`GTK3_MODULES`; it is only a warning.
- `dbind-WARNING ... org.a11y.Bus` — the host has no AT-SPI accessibility bus.
  The application still runs; this is an accessibility-availability warning, not
  a crash. Setting `NO_AT_BRIDGE=1` hides it but disables the accessibility
  bridge, so the launcher leaves it alone.
- `libgvfsdbus.so: undefined symbol: g_date_time_format_iso8601` — the host GIO
  module was built against a newer GLib than the bundle's GLib. `run.sh` points
  `GIO_MODULE_DIR` at an empty bundled directory and unsets
  `GIO_EXTRA_MODULES`; this prevents loading host GIO modules. GVfs/remote
  locations are therefore unavailable in the AppImage; local files still work.

## Fonts

The bundle used only `DejaVuSans.ttf` (regular), so GTK had no real bold face and
fell back to host-dependent metrics. The bundle now ships:

- Noto Sans regular + bold (primary UI face)
- DejaVu Sans regular + bold (fallback)
- WenQuanYi Micro Hei (CJK)

`fonts.conf` aliases `Sans`/`sans-serif` to Noto Sans first, and the stylesheet
sets the same family stack on the window. Only bundled fonts are used, keeping
text metrics deterministic.

Build:

```sh
cd packaging/linux
./package.sh
./verify.sh
```


## Chinese input / fcitx

The AppImage ships GTK's XIM immodule (`im-xim.so`), but not the host's
`im-fcitx.so` / `im-fcitx5.so` / `im-ibus.so`. The launcher therefore:

- preserves the user's locale instead of forcing `LC_ALL=C.UTF-8`;
- maps a requested `GTK_IM_MODULE=fcitx`, `fcitx5`, or `ibus` to `xim`, because
  XIM is the only GTK input-method backend inside the bundle.

Fcitx still serves XIM when its XIM frontend is running. The desktop normally
exports:

```sh
GTK_IM_MODULE=fcitx
XMODIFIERS=@im=fcitx
LANG=zh_CN.UTF-8
```

For the already-released 0.4.3, the same can be tested without rebuilding:

```sh
GTK_IM_MODULE=xim XMODIFIERS=@im=fcitx LC_ALL=zh_CN.UTF-8 ./evobuddy-0.4.3-linux-aarch64.AppImage
```

If that still does not work, check that the fcitx XIM server is running and that
the locale is installed (`locale -a` contains `zh_CN.utf8`).
