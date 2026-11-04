#!/bin/sh
# Rebuild results/host.txt, results/wry.txt, and results/linux.txt.
set -eu

root="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
repo="$(CDPATH= cd -- "$root/../.." && pwd)"
mkdir -p "$root/results"

{
  echo "## preview server"
  cargo run --quiet --manifest-path "$root/preview-server/Cargo.toml"
  echo
  echo "## sources"
  sh "$root/check-sources.sh"
  echo
  echo "## state"
  (cd "$repo" && cargo test -p eb-api read_preview_is_a_separate_pile_from_written_files --offline -- --nocapture)
  echo
  echo "## gdk macos"
  cc -o /tmp/gdk_backend "$root/gdk-backend/gdk_backend.c" $(pkg-config --cflags --libs gtk+-3.0)
  /tmp/gdk_backend
  echo "--- GDK_BACKEND=x11 before init ---"
  GDK_BACKEND=x11 /tmp/gdk_backend || true
  echo
  echo "## objc"
  (cd "$root/objc-as-eb" && cargo metadata --format-version 1 --offline) > /tmp/objc-as-eb-meta.json
  python3 - <<'PY'
import json
meta = json.load(open("/tmp/objc-as-eb-meta.json"))
for node in meta["resolve"]["nodes"]:
    if "objc2-web-kit" in node["id"]:
        feats = node["features"]
        print(node["id"])
        for name in ["WKUserContentController", "WKScriptMessageHandler", "WKSnapshotConfiguration", "WKWebView"]:
            print(f"{name}={name in feats}")
print("HAZARD objc-feature-missing REFUTED current eb dependency enables WKUserContentController via default features")
PY
  echo "no-defaults:"
  set +e
  (cd "$root/objc-no-defaults" && cargo check --offline --message-format=short) > /tmp/objc-no-defaults.txt 2>&1
  set -e
  grep WKUserContentController /tmp/objc-no-defaults.txt | head -n 3
  echo "HAZARD objc-without-defaults CONFIRMED dropping default features hides WKUserContentController"
} | tee "$root/results/host.txt"

sh "$root/fetch-wry.sh" >/dev/null

docker run --rm \
  -v "$root:/probes" \
  --entrypoint bash \
  evobuddy-build-aarch64:latest \
  -lc 'bash /probes/linux/run.sh /probes/results/linux.txt'
