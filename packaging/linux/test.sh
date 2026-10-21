#!/bin/bash
# Run a bundle on a host that has no GTK, no Pango, no Cairo, no fontconfig and
# no fonts, and report exactly what the host had to provide. The AppImage in
# /out is run too, and has to render exactly what the unpacked tree renders.
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
LD_LIBRARY_PATH="$prefix/lib" ldd "$prefix/bin/$app" | awk '/=> \//{print $3}' | grep -v "^$prefix/" | sed 's/^/    /' || true

rm -f /out/*.png
echo "== run under Xvfb: the unpacked tree =="
xvfb-run -a --server-args="-screen 0 1920x1080x24" env "$@" EB_SHOT=/out/render-tree.png \
  "$prefix/run" 2>&1 | sed 's/^/  /'

# The AppImage is the artifact most people will download: one file, run without
# FUSE (--appimage-extract-and-run) on a host that has nothing. Whatever it
# renders must equal what the tree renders, byte for byte, or one of the two
# paths is wrong about something.
appimage=""
for f in /out/*.AppImage; do
  if [ -f "$f" ]; then appimage=$f; break; fi
done

# A type-2 AppImage is an ELF whose bytes 8-9 are the marker "AI". That claim is
# checked here, where nothing can stop it from running.
if [ -n "$appimage" ]; then
  magic=$(head -c 10 "$appimage" | od -An -tx1 | tr -d ' \n')
  if [ "$magic" != 7f454c46020101004149 ]; then
    echo "  $(basename "$appimage") is not a type-2 AppImage (first bytes $magic)" >&2
    exit 1
  fi
  echo "  $(basename "$appimage"): type-2 AppImage, $(stat -c %s "$appimage") bytes"
fi

# ...but an AppImage can only be *executed* where nothing emulates the machine:
# qemu and Rosetta reject the EI_ABIVERSION byte ("AI") that every AppImage
# runtime carries, with "Exec format error". So ask this environment whether it
# can run any AppImage at all, instead of reporting that as a defect in ours.
# On the architecture itself — CI's runners, and this test run natively — the
# probe succeeds and the run below happens.
runs_appimage=no
if [ -n "$appimage" ] && "$appimage" --appimage-version >/dev/null 2>&1; then
  runs_appimage=yes
fi

if [ "$runs_appimage" = yes ]; then
  echo "== run under Xvfb: the AppImage ($(basename "$appimage")) =="
  xvfb-run -a --server-args="-screen 0 1920x1080x24" env "$@" EB_SHOT=/out/render-appimage.png \
    "$appimage" --appimage-extract-and-run 2>&1 | sed 's/^/  /'
elif [ -n "$appimage" ]; then
  echo "== the AppImage is not executed here =="
  echo "  This environment refuses to run AppImage runtimes (an emulation layer:"
  echo "  qemu or Rosetta). Run this on the architecture itself to execute it."
fi

echo "== renders =="
ls -la /out/*.png 2>/dev/null | sed 's/^/  /' || echo "  (no png)"
if [ "$runs_appimage" = yes ]; then
  if cmp -s /out/render-tree.png /out/render-appimage.png; then
    echo "  identical: the AppImage renders exactly what the tree does"
  else
    echo "  DIFFERENT: the AppImage and the tree do not render the same pixels" >&2
    exit 1
  fi
fi

# The click audit is the only check that the controls are *wired*, not merely
# drawn. It asserts the stable observations (route, visibility, notice copy),
# not the environment-sensitive numbers; see `EB_CLICK_ASSERT` in selftest.rs.
echo "== click audit (tree) =="
xvfb-run -a --server-args="-screen 0 1920x1080x24" env "$@" EB_CLICK_ASSERT=1 \
  "$prefix/run" 2>&1 | sed 's/^/  /'

# The fake agent is the only end-to-end check of the streaming contract: a
# worker thread produces events, the GTK main loop drains them, and the app
# quits on the terminal event. The second run cancels before the stream ends.
echo "== M2.2 content blocks (752px column) =="
xvfb-run -a --server-args="-screen 0 1920x1080x24" \
  env EB_DEMO_BLOCKS=1 EB_ASSERT=1 "$prefix/run" 2>&1 | sed 's/^/  /'

echo "== fake streaming agent =="
fake_out=/tmp/fake-agent.log
xvfb-run -a --server-args="-screen 0 1920x1080x24" \
  env EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_FAKE_AGENT_DELAY_MS=10 EB_FAKE_AGENT_CHUNKS=3 \
  "$prefix/run" >"$fake_out" 2>&1
sed 's/^/  /' "$fake_out"

echo "== fake streaming agent (cancel) =="
cancel_out=/tmp/fake-agent-cancel.log
xvfb-run -a --server-args="-screen 0 1920x1080x24" \
  env EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_DEBUG=1 EB_CANCEL_AFTER_MS=50 \
  EB_FAKE_AGENT_DELAY_MS=10 EB_FAKE_AGENT_CHUNKS=50 \
  "$prefix/run" >"$cancel_out" 2>&1
grep -q "fake-demo: finished" "$fake_out"
grep -q 'Failed { message: "Cancelled" }' "$cancel_out"
grep -q "fake-demo: finished" "$cancel_out"

echo "== fake streaming agent (stop button) =="
stop_out=/tmp/fake-agent-stop.log
xvfb-run -a --server-args="-screen 0 1920x1080x24" \
  env EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_DEBUG=1 EB_FAKE_STOP_AFTER_MS=150 \
  EB_FAKE_AGENT_DELAY_MS=10 EB_FAKE_AGENT_CHUNKS=50 \
  "$prefix/run" >"$stop_out" 2>&1
grep -q 'Failed { message: "Cancelled" }' "$stop_out"
grep -q "fake-demo: finished" "$stop_out"

echo "== fake streaming agent (start panic) =="
panic_out=/tmp/fake-agent-panic.log
xvfb-run -a --server-args="-screen 0 1920x1080x24" \
  env EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_DEBUG=1 EB_FAKE_PANIC_START=1 \
  "$prefix/run" >"$panic_out" 2>&1
grep -q "agent: start failed: the agent panicked while starting" "$panic_out"
grep -q "fake-demo: finished" "$panic_out"

# install.sh is only exercised here: it copies the tree, links the launcher and
# registers the menu entry, and a typo in it would otherwise ship unnoticed.
echo "== compliance files =="
for f in LICENSE THIRD_PARTY_NOTICES.md SBOM.json LICENSES/MIT.txt lgpl-source-offer.md; do
  test -f "$prefix/share/doc/evobuddy/$f" || {
    echo "missing compliance file: $f" >&2
    exit 1
  }
done

echo "== install.sh =="
rm -rf /tmp/inst-prefix
"$prefix/install.sh" /tmp/inst-prefix 2>&1 | sed 's/^/  /'
ls -l /tmp/inst-prefix/bin/evobuddy | sed 's/^/  /'
ls /tmp/inst-prefix/share/applications/ /tmp/inst-prefix/share/icons/hicolor/*/apps/ | sed 's/^/  /'
echo "  resolved Exec: $(sed -n 's/^Exec=//p' /tmp/inst-prefix/share/applications/com.evobuddy.app.desktop)"
