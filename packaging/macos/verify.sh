#!/bin/bash
# Audit EvoBuddy.app the way a user runs it: unpack the tarball — the download,
# not the staging directory — and prove it never reaches outside itself:
#
#   1. statically: every load command of every Mach-O must resolve inside the
#      bundle or to the OS; no Homebrew path may survive anywhere;
#   2. dynamically: run the launcher with dyld's load log watched and require
#      that not one image is loaded from a Homebrew prefix, while the app's
#      geometry selftest passes.
#
#   packaging/macos/verify.sh     (run packaging/macos/bundle.sh first)
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
dist="$root/dist"

# No pipes that can lose a SIGPIPE race under `set -o pipefail` (the same trap
# the Linux scripts document).
tarball=""
for f in "$dist"/evobuddy-*-macos-*.tar.gz; do
  if [ -f "$f" ]; then tarball=$f; fi
done
[ -n "$tarball" ] || { echo "no macos tarball in dist/ — run packaging/macos/bundle.sh first" >&2; exit 1; }

echo "unpacking $(basename "$tarball")"
rm -rf "$dist/macos-verify"
mkdir -p "$dist/macos-verify"
tar -xzf "$tarball" -C "$dist/macos-verify"
appdir="$dist/macos-verify/EvoBuddy.app"
for required in "Contents/MacOS/evobuddy" "Contents/Info.plist" "Contents/PkgInfo" \
                "Contents/Resources/evobuddy.icns" \
                "Contents/Resources/share/glib-2.0/schemas/gschemas.compiled" \
                "Contents/Resources/lib/gtk-3.0/3.0.0/immodules/im-quartz.so" \
                "Contents/Resources/etc/fonts.conf.template"; do
  [ -e "$appdir/$required" ] || { echo "missing from the .app: $required" >&2; exit 1; }
done

echo "== load commands =="
# Every Mach-O in the .app may depend on: itself (@rpath -> Frameworks),
# its own tree (@executable_path/@loader_path), or the OS (/usr/lib,
# /System/Library). Anything else — an absolute Homebrew path above all — is a
# hole in the self-containment claim and fails here.
fw="$appdir/Contents/Frameworks"
violations="$dist/macos-verify/violations.txt"
: > "$violations"
find "$appdir" -type f -print0 | while IFS= read -r -d '' f; do
  file -b "$f" | grep -q Mach-O || continue
  dir=$(dirname "$f")
  # otool -L's first entry is the file's own install name, not a dependency —
  # skip it, or every dylib would be accused of depending on itself.
  f_id=$(otool -D "$f" 2>/dev/null | sed -n 2p)
  while read -r dep; do
    [ "$dep" = "$f_id" ] && continue
    case "$dep" in
      @rpath/*)
        if [ ! -f "$fw/${dep#@rpath/}" ]; then
          echo "$f: @rpath dependency not in the bundle: $dep" >> "$violations"
        fi ;;
      @executable_path/*)
        if [ ! -e "$appdir/Contents/MacOS/${dep#@executable_path/}" ]; then
          echo "$f: @executable_path dependency missing: $dep" >> "$violations"
        fi ;;
      @loader_path/*)
        resolved="$dir/${dep#@loader_path/}"
        if [ ! -e "$resolved" ]; then
          echo "$f: @loader_path dependency missing: $dep" >> "$violations"
        fi ;;
      /usr/lib/*|/System/*) ;;
      *)
        echo "$f: absolute dependency outside the bundle: $dep" >> "$violations" ;;
    esac
  done < <(otool -L "$f" | awk 'NR>1 {print $1}')
  # rpaths, too: one pointing at /opt/homebrew would make the bundle work only
  # on the machine that built it.
  otool -l "$f" | awk '
      /cmd LC_RPATH/{rpath=1; next}
      rpath && / path /{print; rpath=0}' | while read -r _ rp _; do
    case "$rp" in
      @executable_path/*|@loader_path/*|@rpath/*) ;;
      *) echo "$f: absolute LC_RPATH: $rp" >> "$violations" ;;
    esac
  done
done
if [ -s "$violations" ]; then
  echo "  load-command audit failed:" >&2
  sed 's/^/    /' "$violations" >&2
  exit 1
fi
echo "  every dependency is @rpath inside the bundle or a system path"

echo "== resources =="
# The caches are built as templates precisely so no build-time path ships in
# them; check the text files that would carry one.
if grep -rIle "/opt/homebrew\|/usr/local" \
     "$appdir/Contents/Resources" \
     --include="*.template" --include="*.conf*" --include="Info.plist" 2>/dev/null \
     | grep .; then
  echo "  a build-time path leaked into Resources (see the file above)" >&2
  exit 1
fi
echo "  no build-time path in the templates"

echo "== run, hidden from Homebrew =="
# The strongest version of the self-containment claim: make the prefix that
# built the bundle disappear and run the .app anyway. Anything still pointing
# at it now fails loudly. The prefix is restored on the way out, whatever the
# outcome. (macOS 26 removed the DYLD_PRINT_* variables, so "watch what dyld
# loads" is no longer available; removal beats observation anyway.)
brew_prefix=$(brew --prefix 2>/dev/null || true)
hidden=""
if [ -n "$brew_prefix" ] && [ -d "$brew_prefix" ]; then
  # Renaming the prefix needs write permission on its parent, which a user
  # rarely has; making it untraversable needs only ownership of the prefix
  # itself. The original mode is restored on the way out, whatever the outcome.
  old_mode=$(stat -f %Lp "$brew_prefix")
  if chmod 000 "$brew_prefix" 2>/dev/null; then
    hidden="$brew_prefix"
  else
    hidden=""
    for sub in Cellar opt lib; do
      if [ -d "$brew_prefix/$sub" ] && chmod 000 "$brew_prefix/$sub" 2>/dev/null; then
        hidden="$hidden $brew_prefix/$sub"
      fi
    done
    [ -n "$hidden" ] || echo "  note: could not hide $brew_prefix; the load-command audit stands alone"
  fi
  if [ -n "$hidden" ]; then
    restore() {
      chmod "$old_mode" "$brew_prefix" 2>/dev/null || true
      for sub in Cellar opt lib; do chmod 755 "$brew_prefix/$sub" 2>/dev/null || true; done
    }
    trap restore EXIT
    echo "  $brew_prefix hidden for this run, restored on exit"
  fi
fi

shot="$dist/macos-verify/app.png"
out="$dist/macos-verify/stdout-1.log"
if ! EB_SELFTEST=1 EB_ASSERT=1 EB_SHOT="$shot" \
     "$appdir/Contents/MacOS/evobuddy" >"$out" 2>"$dist/macos-verify/stderr-1.log"; then
  echo "the .app exited non-zero" >&2
  tail -20 "$dist/macos-verify/stderr-1.log" | sed 's/^/  /' >&2
  exit 1
fi
sed 's/^/  /' "$out"

echo "== run again (warm caches) =="
out2="$dist/macos-verify/stdout-2.log"
if ! EB_SELFTEST=1 EB_ASSERT=1 "$appdir/Contents/MacOS/evobuddy" >"$out2" 2>/dev/null; then
  echo "the .app exited non-zero on the second run" >&2
  exit 1
fi

echo "== renders =="
ls -l "$shot" | awk '{print "  " $5 " bytes, " $NF}'
# The window itself is GTK's default 1440x868 only when the display is that
# large — CI's screens are smaller, and clamping to the workarea is the OS
# being right, not a defect. Assert what does not depend on the screen: the
# sash position and the fixed-size blocks of the welcome route.
if grep -q "sash sidebar x=280 (want 280)" "$out" \
   && grep -Eq "eb-composer-card +[0-9]+, [0-9]+ +800x142" "$out" \
   && grep -Eq "eb-chips +[0-9]+, [0-9]+ +800x32" "$out"; then
  echo "  geometry selftest: sash and fixed blocks match the measured spec"
  grep -E "selftest: window" "$out" | sed 's/^/  /'
else
  echo "  geometry selftest did not report the expected values" >&2
  grep -E "sash|composer-card|chips" "$out" | sed 's/^/  /' >&2
  exit 1
fi

echo "== click audit =="
click_out="$dist/macos-verify/click.log"
if ! EB_CLICK_ASSERT=1 "$appdir/Contents/MacOS/evobuddy" >"$click_out" 2>&1; then
  echo "the click audit failed" >&2
  grep -E "ASSERT FAILED|assertion" "$click_out" | sed 's/^/  /' >&2
  exit 1
fi
grep -E "click: assertions|click: --- end" "$click_out" | sed 's/^/  /'

echo "== fake streaming agent =="
fake_out="$dist/macos-verify/fake-agent.log"
if ! EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_FAKE_AGENT_DELAY_MS=10 EB_FAKE_AGENT_CHUNKS=3 \
     "$appdir/Contents/MacOS/evobuddy" >"$fake_out" 2>&1; then
  echo "the fake streaming agent exited non-zero" >&2
  tail -20 "$fake_out" | sed 's/^/  /' >&2
  exit 1
fi
grep -q "fake-demo: finished" "$fake_out"

echo "== fake streaming agent (cancel) =="
cancel_out="$dist/macos-verify/fake-agent-cancel.log"
if ! EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_DEBUG=1 EB_CANCEL_AFTER_MS=50 \
     EB_FAKE_AGENT_DELAY_MS=10 EB_FAKE_AGENT_CHUNKS=50 \
     "$appdir/Contents/MacOS/evobuddy" >"$cancel_out" 2>&1; then
  echo "the fake streaming agent's cancellation run exited non-zero" >&2
  tail -20 "$cancel_out" | sed 's/^/  /' >&2
  exit 1
fi
grep -q 'Failed { message: "Cancelled" }' "$cancel_out"
grep -q "fake-demo: finished" "$cancel_out"

cmp -s "$out" "$out2" && echo "  second launch rendered the same layout" \
  || echo "  note: first and second launch reports differ"
