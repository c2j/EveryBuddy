#!/bin/bash
# Check the bundle on a host that has nothing: no GTK, no Pango, no Cairo, no
# fontconfig, no fonts. That is the requirement, so it gets a test rather than a
# promise. Run packaging/linux/package.sh first.
#
#   PLATFORM=linux/arm64 ./verify.sh       (default linux/amd64)
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
platform=${PLATFORM:-linux/amd64}

case "$platform" in
  *arm64*|*aarch64*) arch=aarch64 ;;
  *) arch=x86_64 ;;
esac
tag="evobuddy-test-$arch"

if [ ! -x "$root/dist/app/run" ]; then
  echo "no bundle at dist/app — run packaging/linux/package.sh first" >&2
  exit 1
fi

docker build --platform "$platform" -f "$here/Dockerfile.test" -t "$tag" "$root"

# --user for the same reason as package.sh; HOME because xauth wants one.
docker run --rm --platform "$platform" --user "$(id -u):$(id -g)" -e HOME=/tmp \
  -v "$root/dist:/out" "$tag" \
  bash /src/test.sh evobuddy EB_SELFTEST=1 EB_SHOT=/out/app.png
