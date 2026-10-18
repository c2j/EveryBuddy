# macOS .app

`dist/EvoBuddy.app` — and the download, `dist/evobuddy-<v>-macos-<arch>.tar.gz`
holding it plus a README — runs on a Mac with no Homebrew, no pkg-config, no
GTK, no fontconfig and no fonts installed. It carries GTK 3, every library it
needs, an icon theme, compiled GSettings schemas and GTK's input-method modules
(im-quartz included, so CJK typing works).

```sh
./bundle.sh    # build + assemble dist/EvoBuddy.app + the .tar.gz + .sha256
./verify.sh    # audit the unpacked tarball, then run it with Homebrew hidden

PLATFORM   # not used; the .app targets this machine's own architecture
```

Both scripts want the Homebrew GTK stack (`brew install gtk+3 pkg-config
librsvg`) and Apple's toolchain (`otool`, `codesign`, `iconutil`). CI calls
exactly these two scripts on `macos-15` (arm64) and `macos-15-intel` (x86_64),
so a developer and the release path cannot drift apart.

## Why there are no fonts in the .app

On macOS pango selects fonts through **CoreText**, not fontconfig — verified,
not assumed: pointing `FONTCONFIG_FILE` at a directory of DejaVu+WenQuanYi and
re-running the geometry selftest changed nothing (hero stays 137×42, the
spec's macOS-measured value). The system font stack renders text on every Mac
by definition, so the .app ships none; the Linux bundle, where a bare host has
*zero* fonts, ships its own for deterministic metrics instead. The one
fontconfig file in the .app exists because bundled `libfontconfig` is a hard
dependency of cairo and must initialise without a host; it configures no font
directories at all.

## What had to be discovered, not assumed

- **`otool -L`'s first entry is the file's own install name**, not a
  dependency. Treat it as one and every dylib depends on itself; the closure
  then copies the loader module into `Frameworks` under the name of the file
  it re-exports, and the audit accuses random files. `otool -D` tells them
  apart.
- **Copy names must follow the load command, not the filesystem.** realpath
  resolves `libsharpyuv.0.dylib` to `libsharpyuv.0.2.1.dylib`; copy under the
  latter and the rewritten `@rpath/libsharpyuv.0.dylib` points at nothing.
- **Brew libraries increasingly use `@rpath` plus an `LC_RPATH` of their own**
  (libwebp → `@rpath/libsharpyuv.0.dylib` via `@loader_path/../lib`), and one
  module (librsvg's loader) has an `@rpath` dependency and *no* rpath at all —
  dyld falls back to the main executable's rpaths there. The closure walk
  resolves `@rpath` against the scanning file's own rpaths and finally against
  brew's flat `lib/`.
- **`install_name_tool -change` cannot rewrite `LC_REEXPORT_DYLIB`.** It
  succeeds silently and changes nothing — librsvg's `.so` loader re-exports a
  `.dylib` twin, and the twin is the real module. Where a module has a `.dylib`
  twin, the twin is what gets bundled and cached.
- **Executing anything whose signature `install_name_tool` invalidated is
  answered with SIGKILL** on Apple Silicon — including *query tools*: run
  `gdk-pixbuf-query-loaders` on the rewritten copies and the generator itself
  is killed. The caches are generated from the originals and rewritten into
  the bundle's mirrored layout by prefix substitution, exactly like the Linux
  caches.
- **`codesign --deep` does not sign Mach-O files under `Resources`** — it
  signs nested bundles and treats loose files as resources. The first bundle
  therefore died on its first `dlopen`. Every Mach-O is signed explicitly,
  inside-out, and the .app is sealed last. Ad-hoc, hence not notarized; see
  README.txt for what Gatekeeper says on first launch.
- **`DYLD_PRINT_*` is gone on macOS 26**, so "watch what dyld loads" is not
  available. verify.sh hides the Homebrew prefix instead (`chmod 000` — a
  rename would need write permission on `/opt`, which a user does not have)
  and runs the .app without it, which beats observation anyway. The original
  mode is restored on exit.
- **The link must leave room for rewritten load commands**: the app builds
  with `-Wl,-headerpad_max_install_names`.
- **The minimum macOS version is read, not guessed**: from libgtk's own
  `LC_BUILD_VERSION` `minos`, and written into `LSMinimumSystemVersion`. What
  it means in practice: the .app's floor tracks the Homebrew tier of the image
  that built it (macOS 26 locally, the macos-15 tier on CI).
- **The window is only 1440x868 when the display is.** GTK clamps the default
  size to the workarea, and CI's screens are smaller than the spec's measured
  window (the arm64 runner gave 1222x778). verify.sh therefore asserts the
  screen-independent invariants — sash x=280, composer card 800x142, chips
  800x32 — and reports the window size as information.
- **Apple's `/bin/bash` is 3.2.** Both scripts avoid associative arrays and
  everything else 3.2 lacks; CI's lint parses them with `dash -n` too, since
  the launcher runs under `/bin/sh`.
- **Text differs from the Linux bundle by design**: CoreText/system fonts
  here, bundled DejaVu+WenQuanYi there. Each platform's numbers match its own
  spec measurements.

## Not done yet

- Notarization (needs an Apple Developer ID; until then, first launch needs a
  right-click Open or `xattr -dr com.apple.quarantine`).
- A `.dmg` (the tar.gz keeps parity with the Linux artifacts; `hdiutil` makes
  this a small step when wanted).
- A universal binary would mean fattening every bundled dylib; Homebrew
  bottles are single-arch, so it is two artifacts, like Linux.
