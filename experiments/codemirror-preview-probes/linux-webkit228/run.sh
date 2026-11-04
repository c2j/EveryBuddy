#!/bin/sh
# Step ④a only. Links and runs the Kylin archive build of WebKitGTK 2.28.1.
# Refuses any other WebKit, including the 2.38.6 package on the 2403 image
# and the Debian 10 archive candidate. Does not build preview-helper.
set -eu

src="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
results="$(CDPATH= cd -- "$src/../results" && pwd)"

if ! grep -q '^ID=kylin$' /etc/os-release 2>/dev/null; then
  exec docker run --rm \
    --security-opt seccomp=unconfined \
    --security-opt apparmor=unconfined \
    -v "$src:/src:ro" \
    -v "$results:/results" \
    --entrypoint /bin/sh \
    kylin-arm64:v10sp1 \
    /src/run.sh --inside
fi

if [ "${1:-}" != "--inside" ]; then
  echo "REFUSE this script's install step only runs inside the Kylin container" >&2
  exit 2
fi

out=/results/linux-webkit228.txt
work=/tmp/eb-webkit228
log() { printf '%s\n' "$1" | tee -a "$out"; }

: > "$out"
mkdir -p "$work"
cd "$work"

log "image=$(grep PRETTY_NAME= /etc/os-release) release=$(grep KYLIN_RELEASE_ID= /etc/os-release) arch=$(uname -m)"
log "glibc=$(ldd --version | awk 'NR==1{print}')"
log "container_seccomp=unconfined container_apparmor=unconfined"
log "note=ephemeral downgrade to the Kylin archive package 2.28.1-1kylin1k7. The 2403 image's installed WebKit is 2.38.6 and is not the acceptance library."

before=$(dpkg-query -W -f '${Version}' libwebkit2gtk-4.0-37 2>/dev/null || true)
log "webkit_before=${before:-absent}"

export DEBIAN_FRONTEND=noninteractive
printf 'Acquire::Retries "8";\n' > /etc/apt/apt.conf.d/80-retries
apt_status=1
attempt=1
while [ "$attempt" -le 4 ]; do
  set +e
  apt-get install -y --allow-downgrades --fix-missing \
    -o Dpkg::Options::=--force-confdef \
    -o Dpkg::Options::=--force-confold \
    libwebkit2gtk-4.0-37=2.28.1-1kylin1k7 \
    libjavascriptcoregtk-4.0-18=2.28.1-1kylin1k7 \
    gir1.2-javascriptcoregtk-4.0=2.28.1-1kylin1k7 \
    gir1.2-webkit2-4.0=2.28.1-1kylin1k7 \
    libjavascriptcoregtk-4.0-dev=2.28.1-1kylin1k7 \
    libwebkit2gtk-4.0-dev=2.28.1-1kylin1k7 \
    pkg-config libgtk-3-dev libsoup2.4-dev libxtst-dev fcitx-pinyin \
    > /tmp/apt-webkit228.out 2>&1
  apt_status=$?
  set -e
  tee -a "$out" < /tmp/apt-webkit228.out
  if [ "$apt_status" -eq 0 ]; then
    break
  fi
  log "apt attempt $attempt failed status=$apt_status"
  attempt=$((attempt + 1))
  sleep 8
done
if [ "$apt_status" -ne 0 ]; then
  log "REFUSE apt-get install failed status=$apt_status"
  exit "$apt_status"
fi

apt-mark hold \
  libwebkit2gtk-4.0-37 \
  libjavascriptcoregtk-4.0-18 \
  gir1.2-javascriptcoregtk-4.0 \
  gir1.2-webkit2-4.0 \
  libjavascriptcoregtk-4.0-dev \
  libwebkit2gtk-4.0-dev \
  >>"$out" 2>&1

pkg=$(pkg-config --modversion webkit2gtk-4.0)
deb=$(dpkg-query -W -f '${Version}' libwebkit2gtk-4.0-37)
jsc=$(dpkg-query -W -f '${Version}' libjavascriptcoregtk-4.0-18)
gtk=$(pkg-config --modversion gtk+-3.0)
soup=$(pkg-config --modversion libsoup-2.4)
log "pkg-config webkit2gtk-4.0=$pkg"
log "dpkg libwebkit2gtk-4.0-37=$deb"
log "dpkg libjavascriptcoregtk-4.0-18=$jsc"
log "pkg-config gtk+-3.0=$gtk"
log "pkg-config libsoup-2.4=$soup"
log "soname=$(readlink -f /usr/lib/aarch64-linux-gnu/libwebkit2gtk-4.0.so.37)"

if [ "$pkg" != "2.28.1" ] || [ "$deb" != "2.28.1-1kylin1k7" ] || [ "$jsc" != "2.28.1-1kylin1k7" ]; then
  log "REFUSE WebKit is not Kylin 2.28.1-1kylin1k7"
  exit 2
fi

set +e
cc -O2 -Wall -Wextra -o "$work/probe" /src/probe.c \
  $(pkg-config --cflags --libs gtk+-3.0 webkit2gtk-4.0 xtst x11) \
  > /tmp/cc-webkit228.out 2>&1
cc_status=$?
set -e
tee -a "$out" < /tmp/cc-webkit228.out
if [ "$cc_status" -ne 0 ]; then
  log "REFUSE compile failed status=$cc_status"
  exit "$cc_status"
fi

log "--- ldd -r ---"
ldd -r "$work/probe" >>"$out" 2>&1 || {
  log "REFUSE ldd -r reported unresolved symbols"
  exit 2
}
if ! ldd "$work/probe" | grep -q 'libsoup-2.4.so.1'; then
  log "REFUSE probe is not linked to libsoup-2.4"
  exit 2
fi
if ldd "$work/probe" | grep -q 'libsoup-3'; then
  log "REFUSE probe is linked to libsoup-3"
  exit 2
fi

if ! command -v Xvfb >/dev/null 2>&1; then
  log "REFUSE Xvfb is not installed"
  exit 2
fi

Xvfb :98 -screen 0 1400x900x24 -ac +extension XTEST >/tmp/xvfb.log 2>&1 &
xvfb_pid=$!
sleep 1
if ! kill -0 "$xvfb_pid" 2>/dev/null; then
  log "REFUSE Xvfb failed to start"
  cat /tmp/xvfb.log | tee -a "$out"
  exit 2
fi

export DISPLAY=:98
export GDK_BACKEND=x11
export GDK_SCALE=2
unset LD_LIBRARY_PATH || true
export GTK_IM_MODULE=xim
export XMODIFIERS=@im=fcitx
export QT_IM_MODULE=xim

if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
  eval "$(dbus-launch --sh-syntax)"
fi
export DBUS_SESSION_BUS_ADDRESS
log "dbus=${DBUS_SESSION_BUS_ADDRESS}"

mkdir -p "$HOME/.config/fcitx/addon" /tmp/fcitx-disabled
cat > "$HOME/.config/fcitx/profile" << 'EOF'
[Profile]
EnabledIMList=pinyin:True
IMName=pinyin
EOF
# Kylin's fcitx-vk crashes during module load on this headless Xvfb
# (strcmp in fcitx-vk.so, signal 11). Sogou modules miss cairo symbols.
# Move only those modules aside so the XIM frontend can start.
for so in fcitx-vk.so fcitx-sogouimebshxm.so fcitx-sogouimebshdm.so fcitx-sogouimebs.so; do
  if [ -e "/usr/lib/aarch64-linux-gnu/fcitx/$so" ]; then
    mv "/usr/lib/aarch64-linux-gnu/fcitx/$so" /tmp/fcitx-disabled/
  fi
done
fcitx -d >>"$out" 2>&1 || log "fcitx_start_failed=$?"
sleep 1
log "fcitx_state=$(fcitx-remote 2>&1 || true)"
log "fcitx_im=$(fcitx-remote -n 2>&1 || true)"
if [ -d /usr/share/fcitx/pinyin ]; then
  log "pinyin_data=present"
else
  log "pinyin_data=absent"
fi
log "LD_LIBRARY_PATH=${LD_LIBRARY_PATH-<unset>}"
log "GDK_BACKEND=$GDK_BACKEND GDK_SCALE=$GDK_SCALE GTK_IM_MODULE=$GTK_IM_MODULE XMODIFIERS=$XMODIFIERS"

set +e
"$work/probe" >>"$out" 2>&1
probe_status=$?
set -e
log "PROBE_EXIT $probe_status"

kill "$xvfb_pid" 2>/dev/null || true
if command -v fcitx-remote >/dev/null 2>&1; then
  fcitx-remote -e >/dev/null 2>&1 || true
fi

if ! grep -q '^WEBKIT 2\.28\.1$' "$out"; then
  log "REFUSE runtime webkit_get_version was not 2.28.1"
  exit 2
fi
exit "$probe_status"
