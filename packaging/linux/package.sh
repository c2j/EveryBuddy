#!/bin/bash
# Build the Linux bundle: compile on glibc 2.28, stage a self-contained tree in
# dist/app, and write dist/<name>-<version>-linux-<arch>.tar.gz. Needs Docker;
# the host's own libc and GTK are irrelevant.
#
#   PLATFORM=linux/arm64 ./package.sh      (default linux/amd64)
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
platform=${PLATFORM:-linux/amd64}

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
  bash -c 'rm -rf /out/app /out/evobuddy-*.tar.gz*'
docker run --rm --platform "$platform" --user "$(id -u):$(id -g)" \
  -v "$root/dist:/out" "$tag" \
  bash /src/packaging/linux/bundle.sh /src/target/release/evobuddy evobuddy "$version" "$arch"

echo
echo "bundle:  $root/dist/app"
echo "archive: $root/dist/evobuddy-$version-linux-$arch.tar.gz"
echo "run:     $root/dist/app/run"
echo "verify:  $here/verify.sh"
