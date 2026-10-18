#!/bin/bash
# Fail when the generated token artefacts drift from tokens.css.
#
# build_gtk_tokens.js writes gtk/tokens-{light,dark}.css, tokens.resolved.json,
# tokens-metrics.json and tokens-metrics.rs. They are committed, so CI must
# prove they were regenerated after the last token change.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

node "$here/build_gtk_tokens.js" "$here/tokens.css" "$tmp" >/dev/null

status=0
for file in \
  gtk/tokens-light.css \
  gtk/tokens-dark.css \
  gtk/tokens.resolved.json \
  gtk/tokens-metrics.json \
  gtk/tokens-metrics.rs \
  gtk/tokens-metrics.css
do
  if ! diff -q "$here/$file" "$tmp/$file" >/dev/null; then
    echo "token artefact is stale: $file" >&2
    diff -u "$here/$file" "$tmp/$file" | sed -n '1,40p' >&2 || true
    status=1
  fi
done

if [ "$status" -eq 0 ]; then
  echo "token artefacts are up to date"
fi
exit "$status"
