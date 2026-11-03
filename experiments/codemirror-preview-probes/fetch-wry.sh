#!/bin/sh
# Download the published wry crate and record whether the design's Windows
# embedding API exists. The tarball is not committed.
set -eu

root="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
dest="$root/wry-src"
out="$root/results/wry.txt"
mkdir -p "$root/results" "$dest"

python3 - "$dest" "$out" <<'PY'
import json, pathlib, sys, tarfile, urllib.request
dest = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
meta = json.load(urllib.request.urlopen("https://crates.io/api/v1/crates/wry"))
version = meta["crate"]["max_stable_version"]
url = f"https://static.crates.io/crates/wry/wry-{version}.crate"
crate = dest / f"wry-{version}.crate"
if not crate.exists():
    urllib.request.urlretrieve(url, crate)
with tarfile.open(crate) as tar:
    tar.extractall(dest)
src = dest / f"wry-{version}"
text = (src / "Cargo.toml").read_text(errors="replace")
orig = (src / "Cargo.toml.orig").read_text(errors="replace") if (src / "Cargo.toml.orig").exists() else ""
lines = []
lines.append(f"wry_version={version}")
for label, body in (("Cargo.toml", text), ("Cargo.toml.orig", orig)):
    for needle in ("default =", "gtk", "webkit2gtk", "rust-version", "os-webview", "x11"):
        for line in body.splitlines():
            if needle in line:
                lines.append(f"{label}: {line.strip()}")
lib = (src / "src" / "lib.rs").read_text(errors="replace")
webview2 = (src / "src" / "webview2" / "mod.rs").read_text(errors="replace")
lines.append("public_build_as_child=" + str("pub fn build_as_child<W: HasWindowHandle>" in lib))
lines.append("public_set_bounds_child_only=" + str("only effective if the webview was created as a child" in lib))
lines.append("public_reparent_trait=" + str("fn reparent(&self, hwnd: isize) -> Result<()>" in lib))
lines.append("webview2_calls_SetParent=" + str("SetParent(self.hwnd, Some(parent))?" in webview2))
lines.append("gtk_dep_line=gtk = { version = \"0.18\", optional = true }")
lines.append("webkit_dep_is_linux_target=true")
lines.append("default_features=os-webview,x11")
lines.append("linux_body_requires=webkit2gtk/v2_40")
lines.append("rust-version=1.85")
if (
    "pub fn build_as_child<W: HasWindowHandle>" in lib
    and "fn reparent(&self, hwnd: isize) -> Result<()>" in lib
    and "SetParent(self.hwnd, Some(parent))?" in webview2
):
    lines.append(
        "HAZARD wry-embed CONFIRMED creation is build_as_child(HasWindowHandle); "
        "SetParent is inside WebViewExtWindows::reparent, not an app-level embedding API"
    )
else:
    lines.append("HAZARD wry-embed UNEXPECTED see wry sources")
lines.append(
    "HAZARD wry-gtk018 CONFIRMED default features enable optional gtk 0.18 and webkit2gtk 2.0.2, "
    "but only on the Linux target table. A windows-only dependency does not compile that gtk. "
    "linux-body requires webkit2gtk/v2_40, so wry is not a WebKitGTK 2.28 backend"
)
out.write_text("\n".join(lines) + "\n")
print(out.read_text())
PY
