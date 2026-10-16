#!/usr/bin/env bash
#
# Builds the Windows x86_64 release with the GNU (MinGW-w64) toolchain and
# bundles every required GTK 3 runtime DLL next to the executable, so the zip
# is self-contained.
#
# Runs inside an MSYS2 MINGW64 shell (see .github/workflows/build.yml).
# Requires Windows 10 or newer at runtime.
set -euo pipefail

target=x86_64-pc-windows-gnu

echo "=== toolchain ==="
rustc --version
cargo --version
echo "=== gtk version ==="
pkg-config --modversion gtk+-3.0
echo "=== build ==="
cargo build --release --locked --target "$target"

exe="target/$target/release/notebook.exe"
dist=dist

rm -rf "$dist"
mkdir -p "$dist"
cp "$exe" "$dist/"

# Walk the DLL dependency graph, copying every MinGW-w64 DLL into dist/.
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
            cp -f "$dll" "$dist/"
            next+=("$dist/$base")
        done < <(ldd "$f" 2>/dev/null | grep -oE '/mingw64/[^ ]+\.dll' | sort -u || true)
    done
    frontier=("${next[@]}")
done

cat > "$dist/README.txt" <<'EOF'
notebook (windows-x86_64)
=========================

Requires Windows 10 or newer.

All required GTK 3 runtime DLLs are bundled in this archive next to
notebook.exe, so no separate GTK installation is needed.

Run: double-click notebook.exe, or from a terminal: notebook.exe
EOF

rm -f notebook-windows-x86_64.zip
zip -r -q notebook-windows-x86_64.zip "$dist"

echo "=== artifact ==="
ls -l notebook-windows-x86_64.zip
