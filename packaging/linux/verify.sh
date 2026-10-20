#!/bin/bash
# Check the bundle on a host that has nothing: no GTK, no Pango, no Cairo, no
# fontconfig, no fonts. That is the requirement, so it gets a test rather than a
# promise. Run packaging/linux/package.sh first.
#
#   PLATFORM=linux/amd64 ./verify.sh       (default: this machine's own arch)
#
# Same default as package.sh, and for the same reason: an AppImage cannot run
# under qemu or Rosetta (see package.sh), so a native default is the only one
# that can test the AppImage as well as the tree.
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
tag="evobuddy-test-$arch"

if [ ! -x "$root/dist/app/run" ]; then
  echo "no bundle at dist/app — run packaging/linux/package.sh first" >&2
  exit 1
fi

# Test what a user actually downloads, not the staging directory it came from:
# unpack the archive over dist/app. A lost exec bit, a stray path prefix or a
# file left out of the tar would show up here and nowhere else. (No pipe: `ls |
# head` can lose a race with SIGPIPE under `set -o pipefail`.)
archive=""
for f in "$root"/dist/evobuddy-*-linux-*.tar.gz; do
  if [ -f "$f" ]; then archive=$f; fi
done
if [ -n "$archive" ]; then
  echo "unpacking $(basename "$archive")"
  rm -rf "$root/dist/app"
  mkdir -p "$root/dist/app"
  tar -xzf "$archive" -C "$root/dist/app" --strip-components=1
  if [ ! -x "$root/dist/app/run" ]; then
    echo "dist/app/run is not executable after unpacking the archive" >&2
    exit 1
  fi
fi

docker build --platform "$platform" -f "$here/Dockerfile.test" -t "$tag" "$root"

# --user for the same reason as package.sh; HOME because xauth wants one.
docker run --rm --platform "$platform" --user "$(id -u):$(id -g)" -e HOME=/tmp \
  -v "$root/dist:/out" "$tag" \
  bash /src/test.sh evobuddy EB_SELFTEST=1 EB_ASSERT=1
