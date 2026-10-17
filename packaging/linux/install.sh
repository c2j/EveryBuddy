#!/bin/sh
# Install this bundle for the current user: copy it under <prefix>/lib/evobuddy,
# put a launcher on PATH, and register the menu entry. Nothing outside <prefix>
# is touched; to undo it, delete those two paths.
#
#   ./install.sh [prefix]      prefix defaults to ~/.local
#
# The bundle needs no installation to run — ./run works from wherever it sits.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# Spelled out rather than "${1:-${HOME:?…}}": the nested form is easy to get
# subtly wrong, and a stray character in the word makes dash report the error at
# end of file instead of at the line.
if [ "$#" -gt 0 ]; then
  prefix=$1
else
  prefix=${HOME:-/tmp}/.local
fi

lib="$prefix/lib/evobuddy"
apps="$prefix/share/applications"
icons="$prefix/share/icons/hicolor"

# Copying the tree onto itself would delete the source mid-copy.
if [ "$lib" != "$here" ]; then
  rm -rf "$lib"
  mkdir -p "$lib"
  cp -a "$here/." "$lib/"
fi

mkdir -p "$prefix/bin" "$apps" "$icons/256x256/apps" "$icons/scalable/apps"
ln -sf "$lib/run" "$prefix/bin/evobuddy"
cp "$lib/share/applications/evobuddy.desktop" "$apps/"
cp "$lib/share/icons/hicolor/scalable/apps/evobuddy.svg" "$icons/scalable/apps/"
[ -f "$lib/share/icons/hicolor/256x256/apps/evobuddy.png" ] &&
  cp "$lib/share/icons/hicolor/256x256/apps/evobuddy.png" "$icons/256x256/apps/"

echo "installed:  $lib"
echo "launcher:   $prefix/bin/evobuddy"
echo "menu entry: $apps/evobuddy.desktop"
echo "remove:     rm -rf $lib $prefix/bin/evobuddy $apps/evobuddy.desktop"
