#!/bin/sh
# The bundle's entry point. Installed as <bundle>/run by bundle.sh, which
# substitutes @@APP@@ with the binary's name.
#
# Every path the app needs is derived from this script's own location, so the
# tree can be moved anywhere and needs nothing from the host except a glibc
# 2.28+ dynamic loader and an X display.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
app=@@APP@@

# glibc, the dynamic loader and the NSS modules deliberately stay on the host:
# they are forward compatible, and a bundled libc breaks name resolution.
LD_LIBRARY_PATH="$here/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export LD_LIBRARY_PATH

# Make GTK look for its data, themes, schemas and modules inside the bundle
# instead of /usr, which on a bare host has none of them.
export GTK_DATA_PREFIX="$here"
export GTK_EXE_PREFIX="$here"
export GTK_PATH="$here/lib/gtk-3.0"
export GSETTINGS_SCHEMA_DIR="$here/share/glib-2.0/schemas"
: "${LC_ALL:=C.UTF-8}"
export LC_ALL

# Caches that hold absolute paths (gdk-pixbuf loaders, GTK input-method modules)
# and configs that need one (fontconfig) are rendered into a writable directory
# on first run, because the bundle's location is not knowable at build time.
cache="${XDG_CACHE_HOME:-${HOME:-/tmp}/.cache}/evobuddy"
mkdir -p "$cache/fontconfig"

if [ -f "$here/etc/fonts.conf.template" ]; then
  sed "s|@FONTS@|$here/share/fonts|; s|@CACHE@|$cache/fontconfig|" \
      "$here/etc/fonts.conf.template" > "$cache/fonts.conf"
  FONTCONFIG_FILE="$cache/fonts.conf"
  export FONTCONFIG_FILE
fi

for spec in \
    "lib/gdk-pixbuf-2.0/2.10.0/loaders.cache.template:GDK_PIXBUF_MODULE_FILE" \
    "lib/gtk-3.0/3.0.0/immodules.cache.template:GTK_IM_MODULE_FILE"
do
  template="$here/${spec%%:*}"
  variable="${spec##*:}"
  [ -f "$template" ] || continue
  target="$cache/$(basename "$template" .template)"
  sed "s|@BUNDLE@|$here|g" "$template" > "$target"
  export "$variable=$target"
done

exec "$here/bin/$app" "$@"
