#!/bin/bash
# Stage a fully self-contained GTK 3 tree under /out/app around an already built
# binary, and tar it up as /out/<name>-<version>-linux-<arch>.tar.gz.
#
#   bundle.sh <binary> [name] [version] [arch]
#
# Runs inside the Debian 10 build image (Dockerfile.build). package.sh supplies
# the version and arch; on their own they are read from the source and the
# current machine.
set -euo pipefail

src_bin=${1:?usage: bundle.sh <binary> [name] [version] [arch]}
app=${2:-evobuddy}
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
version=${3:-$(sed -n 's/^version *= *"\(.*\)"/\1/p' /src/Cargo.toml | head -1)}
arch=${4:-$(uname -m)}
prefix=/out/app
stage=/tmp/stage/$app-$version

rm -rf "$prefix" "$stage"
mkdir -p "$prefix"/{bin,lib,etc,share/fonts,share/glib-2.0/schemas,share/icons,share/applications}
mkdir -p "$prefix/share/icons/hicolor/scalable/apps" "$prefix/share/icons/hicolor/256x256/apps"

cp "$src_bin" "$prefix/bin/$app"
strip --strip-unneeded "$prefix/bin/$app" 2>/dev/null || true

# Icon and menu entry. The desktop file's Exec is the bare name, resolved
# through PATH, which install.sh arranges.
cp "$here/evobuddy.svg" "$prefix/share/icons/hicolor/scalable/apps/$app.svg"
if command -v rsvg-convert >/dev/null 2>&1; then
  rsvg-convert -w 256 -h 256 -o "$prefix/share/icons/hicolor/256x256/apps/$app.png" \
    "$here/evobuddy.svg"
fi
cp "$here/evobuddy.desktop" "$prefix/share/applications/$app.desktop"
install -m755 "$here/install.sh" "$prefix/install.sh"
sed "s|@VERSION@|$version|; s|@ARCH@|$arch|" "$here/README.txt.in" > "$prefix/README.txt"

echo "== GTK data =="
# The default theme (Adwaita) is compiled into libgtk-3-0 as a GResource, so it
# travels with the library and needs no staging here. Icons are not: without an
# icon theme GTK finds no icon at all.
cp -r /usr/share/icons/hicolor "$prefix/share/icons/" 2>/dev/null || true

# GSettings schemas: libgtk-3-0 owns org.gtk.Settings.*, and GTK refuses to
# start without them. Every *.xml, not just *.gschema.xml: the enum types the
# schemas refer to live in files with the shorter suffix, and compiling without
# them fails with "not (yet) defined". They have to be compiled, not copied raw.
cp /usr/share/glib-2.0/schemas/*.xml "$prefix/share/glib-2.0/schemas/"
glib-compile-schemas "$prefix/share/glib-2.0/schemas"

# Input-method modules (CJK typing) and gdk-pixbuf loaders (any image at all,
# SVG icons included). Debian's immodules have no im-ibus: GTK ships xim and
# friends, ibus brings its own, so XIM is what works out of the box.
immodules=$(find /usr/lib -path '*gtk-3.0/3.0.0/immodules' -type d | head -1)
loaders=$(find /usr/lib -path '*gdk-pixbuf-2.0*loaders' -type d | head -1)
mkdir -p "$prefix/lib/gtk-3.0/3.0.0" "$prefix/lib/gdk-pixbuf-2.0/2.10.0/loaders"
cp -r "$immodules" "$prefix/lib/gtk-3.0/3.0.0/immodules"
cp "$loaders"/*.so "$prefix/lib/gdk-pixbuf-2.0/2.10.0/loaders/"

# Debian keeps both cache generators in the multiarch libdir, off PATH.
query_loaders=$(find /usr/lib -name 'gdk-pixbuf-query-loaders' -type f | head -1)
query_immodules=$(find /usr/lib -name 'gtk-query-immodules-3.0' -type f | head -1)

# Those two caches list absolute paths, and they are built in *this* container:
# the build prefix becomes a placeholder that the launcher resolves at run time,
# because where the bundle ends up is not knowable here.
"$query_loaders" "$prefix/lib/gdk-pixbuf-2.0/2.10.0/loaders"/*.so \
  | sed "s|$prefix|@BUNDLE@|g" > "$prefix/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache.template"
"$query_immodules" "$prefix/lib/gtk-3.0/3.0.0/immodules"/*.so \
  | sed "s|$prefix|@BUNDLE@|g" > "$prefix/lib/gtk-3.0/3.0.0/immodules.cache.template"

echo "== fonts =="
# The bundle carries its own fonts, because the host is assumed to have none.
# DejaVu covers Latin, WenQuanYi Micro Hei covers CJK. Both are freely
# redistributable; the client's own fonts are not, and are not used.
find /usr/share/fonts/truetype -name 'DejaVuSans.ttf' -exec cp {} "$prefix/share/fonts/" \;
find /usr/share/fonts -name 'wqy-microhei*.ttc' -exec cp {} "$prefix/share/fonts/" \;
mkdir -p "$prefix/share/fonts/licenses"
cp /usr/share/doc/fonts-dejavu-core/copyright "$prefix/share/fonts/licenses/dejavu.txt" 2>/dev/null || true
cp /usr/share/doc/fonts-wqy-microhei/copyright "$prefix/share/fonts/licenses/wqy-microhei.txt" 2>/dev/null || true
ls "$prefix/share/fonts" | sed 's/^/  /'

echo "== shared-library closure =="
# Repeat over everything staged until a pass adds nothing: a library pulled in
# by ldd can itself pull in more, and the dlopened loaders and input-method
# modules were not in the binary's own list.
seen=" "
while :; do
  added=0
  for f in "$prefix/bin/$app" $(find "$prefix/lib" -type f -name '*.so*'); do
    while read -r lib; do
      case " $seen " in *" $lib "*) continue;; esac
      # glibc, the dynamic loader and the NSS modules stay on the host: they are
      # forward compatible, and a bundled libc breaks name resolution.
      case "$(basename "$lib")" in
        libc.so*|libm.so*|libdl.so*|libpthread.so*|librt.so*|libresolv.so*|\
        libnsl.so*|libutil.so*|libanl.so*|libcrypt.so*|libnss_*|ld-linux*) continue;;
      esac
      seen="$seen$lib "
      cp -L "$lib" "$prefix/lib/"
      added=1
    done < <(ldd "$f" 2>/dev/null | awk '/=> \//{print $3}')
  done
  [ "$added" = 1 ] || break
done
echo "  $(find "$prefix/lib" -maxdepth 1 -name '*.so*' | wc -l) libraries"

cp "$here/fonts.conf.template" "$prefix/etc/fonts.conf.template"
sed "s|@@APP@@|$app|" "$here/run.sh" > "$prefix/run"
chmod +x "$prefix/run"

echo "== archive =="
mkdir -p "$stage"
cp -a "$prefix/." "$stage/"
rm -f "/out/$app-$version-linux-$arch.tar.gz" "/out/$app-$version-linux-$arch.tar.gz.sha256"
tar -C /tmp/stage -czf "/out/$app-$version-linux-$arch.tar.gz" "$app-$version"
(cd /out && sha256sum "$app-$version-linux-$arch.tar.gz" > "$app-$version-linux-$arch.tar.gz.sha256")

echo "== appimage =="
# One executable file holding the whole tree: nothing to unpack, nothing to
# install, no system GTK. An AppDir is that same tree with three files at its
# root — AppRun (what actually gets executed), the desktop entry, and the icon.
appdir=/tmp/appimg/$app.AppDir
out_appimage="/out/$app-$version-linux-$arch.AppImage"
rm -rf "$appdir" "$out_appimage" "$out_appimage.sha256"
mkdir -p "$appdir"
cp -a "$prefix/." "$appdir/"
cp "$prefix/share/applications/$app.desktop" "$appdir/$app.desktop"
if [ -f "$prefix/share/icons/hicolor/256x256/apps/$app.png" ]; then
  cp "$prefix/share/icons/hicolor/256x256/apps/$app.png" "$appdir/$app.png"
  cp "$prefix/share/icons/hicolor/256x256/apps/$app.png" "$appdir/.DirIcon"
fi
printf '#!/bin/sh\nexec "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/run" "$@"\n' > "$appdir/AppRun"
chmod +x "$appdir/AppRun"

# ARCH selects the runtime to embed. --runtime-file is the runtime fetched when
# this image was built, so packaging never reaches the network.
# --no-appstream because there is no AppStream metadata to validate.
ARCH=$arch appimagetool --appimage-extract-and-run --no-appstream \
  --runtime-file "/usr/local/lib/appimage/runtime-$arch" \
  "$appdir" "$out_appimage"
(cd /out && sha256sum "$(basename "$out_appimage")" > "$(basename "$out_appimage").sha256")

echo "== result =="
du -sh "$prefix" | sed 's/^/  tree: /'
ls -lh "/out/$app-$version-linux-$arch.tar.gz" | awk '{print "  archive: " $5}'
ls -lh "$out_appimage" | awk '{print "  appimage: " $5}'
file -b "$out_appimage" | sed 's/^/  appimage is: /'

echo "== glibc symbol versions the bundle needs =="
# readelf finds nothing in a few libraries (libcairo-gobject, libicudata), and
# under `set -e` that would abort the loop part-way, print a truncated answer and
# fail the whole bundle. A report must not be able to fail the build.
for f in "$prefix/bin/$app" "$prefix/lib"/*.so*; do
  readelf --version-info "$f" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' || true
done | sort -uV | tail -3 | sed 's/^/  /'
