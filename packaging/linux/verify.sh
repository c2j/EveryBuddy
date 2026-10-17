#!/bin/bash
# Check the bundle on a host that has nothing: no GTK, no Pango, no Cairo, no
# fontconfig, no fonts. That is the requirement, so it gets a test rather than
# a promise. Run packaging/linux/package.sh first.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
tag=evobuddy-test-amd64

if [ ! -x "$root/dist/app/run" ]; then
  echo "no bundle at dist/app — run packaging/linux/package.sh first" >&2
  exit 1
fi

docker build --platform linux/amd64 -f "$here/Dockerfile.test" -t "$tag" "$root"
docker run --rm --platform linux/amd64 -v "$root/dist:/out" "$tag" \
  bash /src/test.sh evobuddy EB_SELFTEST=1 EB_SHOT=/out/app.png
