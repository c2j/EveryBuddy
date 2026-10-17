#!/usr/bin/env bash
#
# Builds the Windows x86_64 release with the GNU (MinGW-w64) toolchain and puts
# every required GTK 3 runtime DLL next to the executable.
#
# Runs inside an MSYS2 UCRT64 shell (see .github/workflows/build.yml).
#
# NOT VERIFIED, and not self-contained in the way the Linux bundle is:
#
#   - GTK's *data* is not staged here — GSettings schemas, input-method modules,
#     the gdk-pixbuf loader cache and an icon theme all still come from the
#     MSYS2 installation. On a machine without MSYS2, GTK refuses to start
#     without org.gtk.Settings.*, so a DLL-only zip is not enough.
#   - The loader and immodule caches contain absolute paths, which on Linux the
#     launcher rewrites at run time; Windows would need the same launcher (a
#     .cmd with the same substitution).
#   - UCRT64 targets Windows 10+. Whether GTK 3 can still be built for Windows
#     7/8.1 — a stated requirement — is unresolved (spec §10-23).
#
# Until someone can test this on a real Windows machine, switching the
# packaging to match the Linux one would be guesswork.
set -euo pipefail

target=x86_64-pc-windows-gnu

# The Windows runner's default Rust host is `x86_64-pc-windows-msvc`, so
# building the `*-windows-gnu` target looks like a cross build to the
# `pkg-config` crate, which then refuses to run. Allow it: the pkg-config on
# PATH is MSYS2's MinGW-w64 one and already points at the right prefix.
export PKG_CONFIG_ALLOW_CROSS=1

# MSYS2 sets MSYSTEM_PREFIX to the active environment (e.g. /ucrt64, /mingw64).
prefix="${MSYSTEM_PREFIX:-/ucrt64}"

version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)

echo "=== toolchain ==="
rustc --version
cargo --version
echo "=== gtk version ==="
pkg-config --modversion gtk+-3.0
echo "=== build ==="
cargo build --release --locked --target "$target" --bin evobuddy

exe="target/$target/release/evobuddy.exe"

dist=dist
stage="$dist/evobuddy-$version"
rm -rf "$dist"
mkdir -p "$stage"
cp "$exe" "$stage/"

# Walk the DLL dependency graph, copying every MinGW-w64 DLL into the stage dir.
declare -A seen=()
frontier=("$exe")
while [ "${#frontier[@]}" -gt 0 ]; do
    next=()
    for f in "${frontier[@]}"; do
        while IFS= read -r dll; do
            [ -n "$dll" ] || continue
            base=$(basename "$dll")
            [ -n "${seen[$base]:-}" ] && continue
            seen[$base]=1
            cp -f "$dll" "$stage/"
            next+=("$stage/$base")
        done < <(ldd "$f" 2>/dev/null | grep -oE "${prefix}/[^ ]+\.dll" | sort -u || true)
    done
    frontier=("${next[@]}")
done

cat > "$stage/README.txt" <<EOF
EvoBuddy $version (windows-x86_64)
==================================

A UI study build: the interface of a desktop client, reproduced natively in
GTK 3. Wording and icons are placeholders; no brand assets are reproduced.

Uses the GTK 3 DLLs in this folder. Unlike the Linux bundle, GTK's data files
are NOT included yet, so this build expects the MSYS2 installation those DLLs
came from (or a machine where GTK has been installed some other way). It is
unverified: see .github/scripts/build-windows.sh.

Requires Windows 10 or newer, as built.

Run: evobuddy.exe
EOF

rm -f "evobuddy-windows-x86_64.zip"
(cd "$dist" && zip -r -q "../evobuddy-windows-x86_64.zip" "evobuddy-$version")

echo "=== artifact ==="
ls -l evobuddy-windows-x86_64.zip
