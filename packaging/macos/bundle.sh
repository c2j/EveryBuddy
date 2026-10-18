#!/bin/bash
# Build EvoBuddy.app: the app plus every Homebrew dylib it needs, with all
# load commands rewritten so the .app runs on a Mac that has no Homebrew, no
# pkg-config and no GTK — the macOS counterpart of packaging/linux.
#
#   packaging/macos/bundle.sh
#
# Produces dist/EvoBuddy.app (the staging copy) and the download itself,
# dist/evobuddy-<version>-macos-<arch>.tar.gz + .sha256, holding EvoBuddy.app
# and README.txt. verify.sh audits the unpacked tarball, not this staging copy.
#
# Written for the /bin/bash that Apple ships (3.2): no associative arrays, no
# bash-4 syntax. CI's macOS runners have the same bash.
#
# What the .app deliberately does NOT carry: fonts. On macOS pango selects
# fonts through CoreText (see fonts.conf.template and README.md), so the system
# font stack renders text on every Mac by definition — and the spec's measured
# geometry is macOS-based to begin with.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
app=evobuddy
case "$(uname -m)" in
  arm64) arch=arm64 ;;
  *)     arch=x86_64 ;;
esac
version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)

die() { echo "bundle.sh: $*" >&2; exit 1; }
for tool in brew pkg-config cargo otool install_name_tool codesign strip \
            rsvg-convert iconutil glib-compile-schemas; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is required on PATH"
done
pkg-config --exists gtk+-3.0 || die "GTK 3 missing: brew install gtk+3 pkg-config"
prefix=$(brew --prefix)
dist="$root/dist"
appdir="$dist/EvoBuddy.app"

echo "== build =="
# -headerpad_max_install_names: the load commands are rewritten after linking,
# and the linker must leave room for the replacement strings.
RUSTFLAGS="-C link-arg=-Wl,-headerpad_max_install_names" \
  cargo build --release --locked --manifest-path "$root/Cargo.toml" --bin "$app"
exe="$root/target/release/$app"

rm -rf "$appdir" "$dist"/evobuddy-*-macos-*.tar.gz* "$dist/README.txt"
mkdir -p "$appdir/Contents/MacOS" "$appdir/Contents/Resources"

# run_nt: install_name_tool, with its always-true "invalidates the code
# signature" warning filtered out and real errors kept.
run_nt() {
  local out status
  out=$("$@" 2>&1) && status=0 || status=$?
  if [ "$status" -ne 0 ]; then
    printf '  %s\n' "$out" >&2
    die "install_name_tool failed on $*"
  fi
  printf '%s' "$out" | grep -v "will invalidate the code signature" || true
}

echo "== resources (loaders and immodules first: the closure walks them) =="
res="$appdir/Contents/Resources"
mkdir -p "$res"/{etc,share/glib-2.0/schemas,share/icons} \
         "$res/lib/gdk-pixbuf-2.0/2.10.0/loaders" \
         "$res/lib/gtk-3.0/3.0.0/immodules"

# gdk-pixbuf loaders (any image at all, SVG included) and input-method modules
# (im-quartz is what makes CJK typing work on macOS). Where a module has a
# .dylib twin (librsvg's loader does), the twin is the real module and the .so
# merely re-exports it — a load command install_name_tool cannot rewrite — so
# the twin is what gets bundled and cached. loader_files/immodule_files record
# the *originals* behind each copy, for the caches below.
loaders_src="$prefix/lib/gdk-pixbuf-2.0/2.10.0/loaders"
immodules_src="$prefix/lib/gtk-3.0/3.0.0/immodules"
[ -d "$loaders_src" ] || die "no gdk-pixbuf loaders under $prefix/lib"
[ -d "$immodules_src" ] || die "no GTK immodules under $prefix/lib"

bin_dir="$appdir/Contents/MacOS"

loader_files=()
for src in "$loaders_src"/*.so; do
  twin="${src%.so}.dylib"
  if [ -f "$twin" ]; then
    cp "$twin" "$res/lib/gdk-pixbuf-2.0/2.10.0/loaders/$(basename "$twin")"
    loader_files+=("$twin")
  else
    cp "$src" "$res/lib/gdk-pixbuf-2.0/2.10.0/loaders/$(basename "$src")"
    loader_files+=("$src")
  fi
done

immodule_files=()
for src in "$immodules_src"/*.so; do
  twin="${src%.so}.dylib"
  if [ -f "$twin" ]; then
    cp "$twin" "$res/lib/gtk-3.0/3.0.0/immodules/$(basename "$twin")"
    immodule_files+=("$twin")
  else
    cp "$src" "$res/lib/gtk-3.0/3.0.0/immodules/$(basename "$src")"
    immodule_files+=("$src")
  fi
done

echo "== dylib closure =="
# Walk otool -L from the binary, the loaders and the input-method modules —
# the latter two are dlopened, so their dependencies never appear in the
# binary's load commands. Three facts about otool -L shape this loop:
#   - its first entry is the file's own install name, not a dependency
#     (otool -D says which);
#   - a Homebrew library may depend through @rpath plus an LC_RPATH of its own
#     (libwebp does: @rpath/libsharpyuv.0.dylib, @loader_path/../lib), so an
#     @rpath dependency is resolved against the scanning file's rpaths;
#   - copies are named after the name in the *load command* (libsharpyuv.0.dylib),
#     not after the realpath (libsharpyuv.0.2.1.dylib), so the rewritten
#     @rpath matches the name the dependency was written under.
# One name mapping to two files is an error; one file with two names is
# copied twice. (Linear scans: ~40 entries, and Apple's bash 3.2 has no
# associative arrays.)
fw="$appdir/Contents/Frameworks"
mkdir -p "$fw"
seen_real=()   # the realpath each name came from
seen_name=()   # names already present in Frameworks
n_seen=0
queue=("$exe" "$res"/lib/gdk-pixbuf-2.0/2.10.0/loaders/* \
       "$res"/lib/gtk-3.0/3.0.0/immodules/*)
while [ "${#queue[@]}" -gt 0 ]; do
  f=${queue[0]}
  queue=("${queue[@]:1}")
  f_id=$(otool -D "$f" 2>/dev/null | sed -n 2p)
  while read -r dep; do
    [ -n "$dep" ] || continue
    case "$dep" in /usr/lib/*|/System/*) continue ;; esac
    [ "$dep" = "$f_id" ] && continue
    case "$dep" in
      @rpath/*)
        name=${dep#@rpath/}
        real=""
        while read -r _ rp _; do
          case "$rp" in
            @loader_path/*)     cand="$(dirname "$f")/${rp#@loader_path/}/$name" ;;
            @executable_path/*) cand="$bin_dir/${rp#@executable_path/}/$name" ;;
            @*)                 continue ;;
            *)                  cand="$rp/$name" ;;
          esac
          cand=$(realpath "$cand" 2>/dev/null) || continue
          case "$cand" in /usr/lib/*|/System/*) continue ;; esac
          if [ -f "$cand" ]; then real=$cand; break; fi
        done < <(otool -l "$f" | awk '
            /cmd LC_RPATH/{r=1; next}
            r && / path /{print; r=0}')
        # librsvg's loader module carries an @rpath dependency and *no* rpath of
        # its own: dyld falls back to the main executable's rpaths, which on a
        # Homebrew machine covers it and here is our Frameworks rpath. Resolve
        # the build-time name against brew's flat lib directory instead.
        if [ -z "$real" ] && [ -e "$prefix/lib/$name" ]; then
          real=$(realpath "$prefix/lib/$name")
        fi
        [ -n "$real" ] || die "cannot resolve $dep (referenced by $f) against its own rpaths"
        ;;
      @*) continue ;;   # @executable_path: already bundle-relative
      *)
        name=$(basename "$dep")
        real=$(realpath "$dep")
        ;;
    esac
    [ -f "$real" ] || die "dependency does not exist: $dep (referenced by $f)"
    i=0
    while [ "$i" -lt "$n_seen" ]; do
      if [ "${seen_name[$i]}" = "$name" ]; then
        if [ "${seen_real[$i]}" != "$real" ]; then
          die "one name, two libraries: $name is both ${seen_real[$i]} and $real"
        fi
        continue 2   # already bundled under exactly this name
      fi
      i=$((i + 1))
    done
    seen_real+=("$real")
    seen_name+=("$name")
    n_seen=$((n_seen + 1))
    cp -L "$real" "$fw/$name"
    queue+=("$fw/$name")
  done < <(otool -L "$f" | awk 'NR>1 {print $1}')
done
echo "  $(find "$fw" -name '*.dylib' | wc -l | tr -d ' ') libraries"

echo "== rewrite load commands =="
# Every reference to a Homebrew path becomes @rpath/<basename>, and every
# Mach-O in the bundle gets an rpath that finds Frameworks from the
# executable's location. System dependencies are left exactly as they are:
# they are the OS.
bin_dir="$appdir/Contents/MacOS"
install -m755 "$exe" "$bin_dir/$app-bin"
strip -x "$bin_dir/$app-bin" 2>/dev/null || true

ensure_rpath() {
  if otool -l "$f" | grep -qF "@executable_path/../Frameworks"; then return 0; fi
  run_nt install_name_tool -add_rpath "@executable_path/../Frameworks" "$f"
}
rewrite() {
  local f=$1 dep rp f_id
  case "$f" in
    "$fw"/*) run_nt install_name_tool -id "@rpath/$(basename "$f")" "$f" ;;
  esac
  f_id=$(otool -D "$f" 2>/dev/null | sed -n 2p)
  while read -r dep; do
    case "$dep" in /usr/lib/*|/System/*) continue ;; esac
    [ "$dep" = "$f_id" ] && continue   # the file's own install name, not a dep
    case "$dep" in @*) continue ;; esac
    run_nt install_name_tool -change "$dep" "@rpath/$(basename "$dep")" "$f"
  done < <(otool -L "$f" | awk 'NR>1 {print $1}')
  # Brew libraries can carry absolute rpaths of their own (jpeg-turbo does);
  # after the -change pass they point at nothing that exists on a bare Mac.
  while read -r _ rp _; do
    case "$rp" in
      @*) ;;
      *) run_nt install_name_tool -delete_rpath "$rp" "$f" ;;
    esac
  done < <(otool -l "$f" | awk '
      /cmd LC_RPATH/{r=1; next}
      r && / path /{print; r=0}')
  ensure_rpath
}
rewrite "$bin_dir/$app-bin"
for f in "$fw"/*.dylib; do rewrite "$f"; done
for f in "$res"/lib/gdk-pixbuf-2.0/2.10.0/loaders/* \
         "$res"/lib/gtk-3.0/3.0.0/immodules/*; do
  rewrite "$f"
done

echo "== resources (data) =="

# GSettings schemas: libgtk owns org.gtk.Settings.* and GTK refuses to start
# without them. Compiled, with every *.xml — same trap as on Linux.
cp "$prefix/share/glib-2.0/schemas"/*.xml "$res/share/glib-2.0/schemas/"
glib-compile-schemas "$res/share/glib-2.0/schemas"

cp -r "$prefix/share/icons/hicolor" "$res/share/icons/" 2>/dev/null || true

# The caches are generated from Homebrew's *originals* (recorded by
# copy_modules), not the rewritten copies beside them: on Apple Silicon,
# executing anything whose signature install_name_tool invalidated gets the
# whole process killed. Substituting the whole prefix — not just the module
# directories, which the caches quote alongside locale dirs and a "created
# by" tool path — maps every absolute path into the bundle's mirrored layout;
# what the bundle does not ship (locales) resolves to a path inside it that
# simply does not exist, which is exactly what a bare machine may offer.
"$prefix/bin/gdk-pixbuf-query-loaders" "${loader_files[@]}" \
  | sed "s|$prefix|@RES@|g" > "$res/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache.template"
"$prefix/bin/gtk-query-immodules-3.0" "${immodule_files[@]}" \
  | sed "s|$prefix|@RES@|g" > "$res/lib/gtk-3.0/3.0.0/immodules.cache.template"

# No font files (CoreText provides the system stack); a config so the bundled
# libfontconfig, a hard dependency of cairo, initialises without a host.
cp "$here/fonts.conf.template" "$res/etc/fonts.conf.template"

echo "== icon =="
iconset="$(mktemp -d)/$app.iconset"
mkdir -p "$iconset"
for s in 16 32 128 256 512; do
  rsvg-convert -w "$s" -h "$s" -o "$iconset/icon_${s}x${s}.png" "$here/../evobuddy.svg"
  rsvg-convert -w $((s * 2)) -h $((s * 2)) -o "$iconset/icon_${s}x${s}@2x.png" "$here/../evobuddy.svg"
done
iconutil -c icns "$iconset" -o "$res/$app.icns"

echo "== bundle metadata =="
# The oldest macOS the bundled libraries themselves ask for, read from
# libgtk's own build version — claimed from evidence, not from a guess.
minos=$(otool -l "$fw/libgtk-3.0.dylib" \
        | awk '/LC_BUILD_VERSION/{f=1} f && / minos /{print $2; exit}')
minos=${minos:-10.15}
sed "s|@VERSION@|$version|g; s|@MINOS@|$minos|g" "$here/Info.plist.in" \
  > "$appdir/Contents/Info.plist"
printf 'APPL????\n' > "$appdir/Contents/PkgInfo"
sed "s|@@APP@@|$app|g" "$here/launcher.sh.in" > "$bin_dir/$app"
chmod +x "$bin_dir/$app"

echo "== sign (ad-hoc) =="
# codesign --deep signs nested *bundles* but treats Mach-O files under
# Resources as resources: they stay unsigned, and on Apple Silicon dlopening
# unsigned code is answered with SIGKILL — which is exactly how the first
# launch of the first bundle died. Sign every Mach-O explicitly, inside-out,
# then seal the .app so the resource seal covers the (now signed) files.
find "$appdir" -type f -print0 | while IFS= read -r -d '' f; do
  file -b "$f" | grep -q Mach-O || continue
  codesign --force --sign - "$f"
done
codesign --force --sign - "$appdir"

echo "== archive =="
sed "s|@VERSION@|$version|g; s|@ARCH@|$arch|g; s|@MACOS@|$minos|g" "$here/README.txt.in" > "$dist/README.txt"
tarball="$dist/$app-$version-macos-$arch.tar.gz"
tar -C "$dist" -czf "$tarball" "EvoBuddy.app" "README.txt"
(cd "$dist" && shasum -a 256 "$(basename "$tarball")" > "$(basename "$tarball").sha256")

echo "== result =="
du -sh "$appdir" | sed 's/^/  app: /'
ls -lh "$tarball" | awk '{print "  archive: " $5}'
echo "  min macOS: $minos"
echo "  verify:    $here/verify.sh"
