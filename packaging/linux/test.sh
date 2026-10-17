#!/bin/bash
# Run a bundle on a host that has no GTK, no Pango, no Cairo, no fontconfig and
# no fonts, and report exactly what the host had to provide.
#
#   test.sh <app> [env assignments for the run...]
#
# Runs inside the test image (Dockerfile.test), with the bundle at /out/app.
set -euo pipefail

app=${1:?usage: test.sh <app> [env...]}
shift || true
prefix=/out/app

echo "== the host =="
echo "  debian $(cat /etc/debian_version)"
(dpkg -l 2>/dev/null | awk '/^ii/{print $2}' | grep -E 'gtk|pango|cairo|fontconfig|harfbuzz|^libglib' \
   || echo "  no gtk/pango/cairo/fontconfig/harfbuzz/glib packages")
echo "  fonts on host: $(find /usr/share/fonts -type f 2>/dev/null | wc -l) files"

echo "== what the bundle needs from the host =="
total=$(LD_LIBRARY_PATH="$prefix/lib" ldd "$prefix/bin/$app" | grep -c '=> /' || true)
outside=$(LD_LIBRARY_PATH="$prefix/lib" ldd "$prefix/bin/$app" | awk '/=> \//{print $3}' | grep -v "^$prefix/" | wc -l)
echo "  $total shared libraries, $outside from outside the bundle:"
LD_LIBRARY_PATH="$prefix/lib" ldd "$prefix/bin/$app" | awk '/=> \//{print $3}' | grep -v "^$prefix/" | sed 's/^/    /'

rm -f /out/*.png
echo "== run under Xvfb =="
xvfb-run -a --server-args="-screen 0 1920x1080x24" env "$@" "$prefix/run" 2>&1 | sed 's/^/  /'

echo "== renders =="
ls -la /out/*.png 2>/dev/null | sed 's/^/  /' || echo "  (no png)"
