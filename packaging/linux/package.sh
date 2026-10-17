#!/bin/bash
# Build the Linux bundle: compile on glibc 2.28, stage a self-contained tree in
# dist/app, and write dist/<name>-<version>-linux-<arch>.tar.gz plus the
# single-file .AppImage. Needs Docker; the host's own libc and GTK are
# irrelevant.
#
#   PLATFORM=linux/amd64 ./package.sh      (default: this machine's own arch)
#
# The default is the host architecture because no AppImage can be *built* under
# an emulation layer: every AppImage runtime carries "AI" in the ELF byte
# EI_ABIVERSION, which a real Linux kernel ignores but qemu and Rosetta reject
# with "Exec format error". Staying native keeps one pipeline for the developer
# and CI, where each architecture builds on its own runner.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
case "$(uname -m)" in
  arm64|aarch64) host_platform=linux/arm64 ;;
  *)             host_platform=linux/amd64 ;;
esac
platform=${PLATFORM:-$host_platform}

case "$platform" in
  *arm64*|*aarch64*) arch=aarch64 ;;
  *) arch=x86_64 ;;
esac

version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)
tag="evobuddy-build-$arch"

docker build --platform "$platform" -f "$here/Dockerfile.build" -t "$tag" "$root"

# --user so the output belongs to whoever ran this, not to root. Any earlier
# root-owned tree is cleared first, from inside a container, because the user
# could not delete it from the outside.
mkdir -p "$root/dist"
docker run --rm --platform "$platform" -v "$root/dist:/out" debian:10 \
  bash -c 'rm -rf /out/app /out/evobuddy-*'
docker run --rm --platform "$platform" --user "$(id -u):$(id -g)" \
  -v "$root/dist:/out" "$tag" \
  bash /src/packaging/linux/bundle.sh /src/target/release/evobuddy evobuddy "$version" "$arch"

echo
echo "bundle:   $root/dist/app"
echo "archive:  $root/dist/evobuddy-$version-linux-$arch.tar.gz"
echo "appimage: $root/dist/evobuddy-$version-linux-$arch.AppImage"
echo "run:      $root/dist/app/run"
echo "verify:   $here/verify.sh"
