#!/bin/sh
# Runs inside the Debian 10 image that builds the Linux bundle
# (evobuddy-build-aarch64). The image already has GTK 3 and does not have
# WebKitGTK. xvfb is installed here if the package index allows it.
set -eu

src="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
work=/tmp/codemirror-preview-probe
rm -rf "$work"
mkdir -p "$work"
cp "$src"/*.c "$work"/
cd "$work"
out="${1:-/tmp/linux-probe.txt}"
: > "$out"

log() {
  printf '%s\n' "$1" | tee -a "$out"
}

log "image=$(grep PRETTY_NAME= /etc/os-release) arch=$(uname -m)"

if pkg-config --exists webkit2gtk-4.0; then
  log "HAZARD webkit-pkgconfig REFUTED webkit2gtk-4.0 is installed ($(pkg-config --modversion webkit2gtk-4.0))"
else
  log "HAZARD webkit-pkgconfig CONFIRMED pkg-config webkit2gtk-4.0 is absent in the bundle build image"
fi
dpkg -l 'libwebkit2gtk*' 2>/dev/null | awk 'NR==1 || /webkit/ {print}' | tee -a "$out" || true
if ! dpkg -l 'libwebkit2gtk*' 2>/dev/null | grep -q '^ii'; then
  log "installed_webkit_packages=none"
fi

if ! command -v Xvfb >/dev/null 2>&1; then
  apt-get update
  DEBIAN_FRONTEND=noninteractive apt-get install -y xvfb
fi

log "--- apt-cache webkit ---"
apt-cache policy libwebkit2gtk-4.0-dev | tee -a "$out" || log "apt-cache webkit failed"
candidate=$(apt-cache policy libwebkit2gtk-4.0-dev | awk '/Candidate:/ {print $2}')
log "HAZARD webkit-version CONFIRMED build image has no WebKitGTK installed; Debian 10 archive candidate is ${candidate:-unknown}"
log "NOTE target desktop WebKitGTK is 2.28 (webkit2gtk-4.0 / libsoup2). The archive candidate is not the acceptance version."
log "socket_header=$(grep -n GDK_WINDOWING_X11 /usr/include/gtk-3.0/gtk/gtksocket.h | head -n 1)"
log "gdkconfig=$(grep GDK_WINDOWING /usr/include/gtk-3.0/gdk/gdkconfig.h | tr '\n' ' ')"

cc -o gdk_backend gdk_backend.c $(pkg-config --cflags --libs gtk+-3.0)
if cc -o xembed xembed.c $(pkg-config --cflags --libs gtk+-3.0) -pthread; then
  xembed_built=1
else
  xembed_built=0
  log "HAZARD xembed-plug-id BLOCKED xembed.c did not compile"
fi
cc -shared -fPIC -o stub_gtk.so stub_gtk.c -Wl,-soname,libgtk-3.so.0
mkdir -p bundle/lib
cp stub_gtk.so bundle/lib/libgtk-3.so.0
cc -o victim victim.c $(pkg-config --cflags --libs gtk+-3.0)
cc -o parent_exec parent_exec.c

log "--- gdk backend, set after init ---"
xvfb-run -a ./gdk_backend | tee -a "$out"
log "--- gdk backend, wayland requested before init ---"
if xvfb-run -a env GDK_BACKEND=wayland ./gdk_backend > /tmp/gdk-wayland.txt 2>&1; then
  log "HAZARD gdk-backend-early UNEXPECTED wayland init succeeded"
else
  log "HAZARD gdk-backend-early CONFIRMED GDK_BACKEND=wayland before init cannot open a display here"
fi
cat /tmp/gdk-wayland.txt | tee -a "$out"

log "--- xembed ---"
if [ "$xembed_built" -eq 1 ]; then
  xvfb-run -a ./xembed > /tmp/xembed.txt 2>&1 || true
  cat /tmp/xembed.txt | tee -a "$out"
else
  log "xembed skipped"
fi

log "--- ld library path ---"
log "ldd_without_override=$(ldd ./victim | awk '/libgtk-3/{print $1, $2, $3}')"
set +e
xvfb-run -a ./victim > /tmp/victim-clean.txt 2>&1
clean_status=$?
set -e
log "victim_without_override status=$clean_status"
cat /tmp/victim-clean.txt | tee -a "$out"

log "ldd_with_bundle=$(LD_LIBRARY_PATH=./bundle/lib ldd ./victim | awk '/libgtk-3/{print $1, $2, $3}')"
set +e
xvfb-run -a env LD_LIBRARY_PATH=./bundle/lib ./victim > /tmp/victim-bundle.txt 2>&1
bundle_status=$?
set -e
log "victim_with_bundle_ld_path status=$bundle_status"
cat /tmp/victim-bundle.txt | tee -a "$out"

set +e
xvfb-run -a ./parent_exec keep ./victim > /tmp/victim-inherit.txt 2>&1
inherit_status=$?
xvfb-run -a ./parent_exec clean ./victim > /tmp/victim-cleaned.txt 2>&1
cleaned_status=$?
set -e
log "child_inherits status=$inherit_status"
cat /tmp/victim-inherit.txt | tee -a "$out"
log "child_unsets status=$cleaned_status"
cat /tmp/victim-cleaned.txt | tee -a "$out"

if grep -q 'after_setenv type=GdkX11Display' "$out" && grep -q 'GDK_BACKEND=wayland' "$out"; then
  log "HAZARD gdk-backend-late CONFIRMED display stayed GdkX11Display after g_setenv(GDK_BACKEND, wayland)"
else
  log "HAZARD gdk-backend-late UNEXPECTED see gdk output above"
fi

if grep -q 'plain_xid_equals_plug_id=0' "$out" \
  && grep -q 'add_id_0 plug_window=null' "$out" \
  && grep -q 'add_id_plug_id plug_window=non-null' "$out"; then
  log "HAZARD xembed-plug-id CONFIRMED gtk_plug_get_id returns a live id, that id is not a toplevel XID, add_id(0) does not embed, add_id(plug id) does"
else
  log "HAZARD xembed-plug-id UNEXPECTED see xembed output above"
fi

if grep -q "Can't add non-GtkPlug" "$out"; then
  log "HAZARD xembed-plain-xid CONFIRMED GtkSocket refuses a normal toplevel XID"
fi

if grep -q 'STUB_LIBGTK_LOADED' /tmp/victim-inherit.txt && [ "$inherit_status" -ne 0 ] \
  && grep -q 'victim_gtk_ok' /tmp/victim-cleaned.txt && [ "$cleaned_status" -eq 0 ]; then
  log "HAZARD ldpath-inherit CONFIRMED a child that keeps LD_LIBRARY_PATH loads the bundle libgtk and fails; unsetting it loads the system GTK"
else
  log "HAZARD ldpath-inherit UNEXPECTED inherit=$inherit_status cleaned=$cleaned_status"
fi
