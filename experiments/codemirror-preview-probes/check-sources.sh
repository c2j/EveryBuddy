#!/bin/sh
# Source checks against the repository. Each line is either evidence or a
# HAZARD verdict. Exits non-zero when a check cannot find the code it names.
set -eu

root="$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)"
cd "$root"

have() {
  if grep -F -q "$2" "$1"; then
    return 0
  fi
  echo "MISSING $2 in $1" >&2
  return 1
}

have crates/eb/src/preview_server.rs 'TcpListener::bind("127.0.0.1:39473")'
have crates/eb/src/preview_server.rs 'else { return };'
have crates/eb/src/preview_server.rs 'for stream in listener.incoming()'
have crates/eb/src/preview_server.rs 'let mut request = [0u8; 2048];'
echo "HAZARD preview-server-shape CONFIRMED production server is one thread, one 2048-byte read, fixed port, bind error returned"

if grep -q evaluateJavaScript crates/eb/src/theme_view.rs; then
  echo "HAZARD theme-no-eval REFUTED theme_view.rs calls evaluateJavaScript" >&2
  exit 1
fi
have crates/eb/src/theme_view.rs 'loadRequest'
have crates/eb/src/theme_view.rs 'Validation-only'
have crates/eb/src/theme_view.rs 'activateIgnoringOtherApps(true)'
if grep -q 'size-allocate\|configure-event' crates/eb/src/theme_view.rs; then
  echo "HAZARD theme-geometry REFUTED a geometry signal was added" >&2
  exit 1
fi
have crates/eb/src/shell.rs 'EB_PANEL_OVERLAY'
echo "HAZARD theme-geometry CONFIRMED theme_view is validation-only, loads a URL, steals focus, and has no size-allocate or configure-event handler"

have crates/eb/Cargo.toml 'features = ["WKSnapshotConfiguration", "block2"]'
if grep -F 'objc2-web-kit' crates/eb/Cargo.toml | grep -F -q 'default-features = false'; then
  echo "HAZARD objc-defaults REFUTED the objc2-web-kit line now disables default features" >&2
  exit 1
fi
echo "HAZARD objc-cargo-line CONFIRMED eb names WKSnapshotConfiguration and block2 and does not turn default features off"

have docs/ui-spec/ui-spec.md '不引入任何自带 Web 引擎'
echo "HAZARD ui-spec-no-web CONFIRMED ui-spec §2 still forbids shipping a web engine"

if grep -R -n --include='*.css' --include='*.rs' eb-code-gutter crates >/dev/null 2>&1; then
  echo "HAZARD gutter-class REFUTED eb-code-gutter is back in crates" >&2
  exit 1
fi
have crates/eb/src/detail.rs 'eb-gutter-fg'
have crates/eb/src/selftest.rs 'no code view'
have crates/eb/src/selftest.rs 'eb-code-scroll'
echo "HAZARD gutter-and-selftest CONFIRMED line numbers are a TextTag named eb-gutter-fg; eb-code-gutter is not in crates; observe_detail reports no code view without eb-code-scroll"

have crates/eb/src/shell.rs 'const PREVIEW_CAP: u64 = 512 * 1024;'
have crates/eb/src/shell.rs 'file.take(PREVIEW_CAP)'
echo "HAZARD preview-cap CONFIRMED the click path reads up to 512 KiB on the caller before storing the whole string in tool_preview"

if grep -q webkit packaging/linux/Dockerfile.build; then
  echo "HAZARD build-image-webkit REFUTED the build image dockerfile mentions webkit" >&2
  exit 1
fi
have packaging/linux/Dockerfile.build 'libgtk-3-dev'
have packaging/linux/run.sh 'LD_LIBRARY_PATH="$here/lib'
have packaging/linux/run.sh 'exec "$here/bin/$app"'
echo "HAZARD bundle-ld-path CONFIRMED run.sh points LD_LIBRARY_PATH at the bundle lib and execs the app with that environment; the build image installs GTK and not WebKit"

if grep -q GDK_BACKEND crates/eb/src/main.rs; then
  echo "HAZARD gdk-backend-callsite REFUTED main.rs now sets GDK_BACKEND" >&2
  exit 1
fi
have crates/eb/src/main.rs 'gtk::Application::new'
echo "HAZARD gdk-backend-callsite CONFIRMED main.rs never sets GDK_BACKEND before gtk::Application::new"

python3 - <<'PY'
from pathlib import Path
text = Path("crates/eb/src/shell.rs").read_text()
start = text.find("let close_tab:")
end = text.find("let filter_state:", start)
body = text[start:end]
if "tool_preview" in body:
    raise SystemExit("HAZARD close-tab REFUTED close_tab now touches tool_preview")
print("HAZARD close-tab CONFIRMED closing one artifact tab edits task.artifacts and does not clear tool_preview")
PY
