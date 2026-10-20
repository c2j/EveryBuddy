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
