#!/bin/bash
# Build the Linux bundle: compile on glibc 2.28, then stage a self-contained
# tree in dist/app. Needs Docker; the host's own libc and GTK are irrelevant.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
tag=evobuddy-build-amd64

docker build --platform linux/amd64 -f "$here/Dockerfile.build" -t "$tag" "$root"

rm -rf "$root/dist/app"
mkdir -p "$root/dist"
docker run --rm --platform linux/amd64 -v "$root/dist:/out" "$tag" \
  bash /src/packaging/linux/bundle.sh /src/target/release/evobuddy evobuddy

echo
echo "bundle: $root/dist/app"
echo "run:    $root/dist/app/run"
echo "verify: $here/verify.sh"
