# Linux bundle

Two artifacts come out of `package.sh`, both self-contained:

- `dist/evobuddy-<v>-linux-<arch>.AppImage` — one executable file. `chmod +x`
  and run it; nothing is unpacked by hand, nothing is installed.
- `dist/evobuddy-<v>-linux-<arch>.tar.gz` — the same tree as a directory, for
  `install.sh`, for packagers, and for anyone who would rather not run an
  AppImage.

`dist/app` is that directory: it runs on a machine with no GTK, no Pango, no
Cairo, no fontconfig and no fonts. It carries all of them plus the app:

```
app/
  run                  # the entry point; sets every path relative to itself
  install.sh           # optional: copy under ~/.local, link, register the menu
  README.txt
  bin/evobuddy
  lib/                 # GTK 3, GLib, Pango, Cairo, GDK, HarfBuzz, ... (72 .so)
  share/               # fonts, hicolor + app icons, compiled GSettings schemas
  share/applications/evobuddy.desktop
  etc/fonts.conf.template
```

The AppImage is the same tree with three files added at its root — `AppRun`
(what the runtime executes), `evobuddy.desktop`, and the icon, plus `.DirIcon`.

```sh
./package.sh   # build on glibc 2.28: dist/app, the .tar.gz and the .AppImage
./verify.sh    # run both artifacts on a bare Debian 10 and print what they needed

PLATFORM=linux/amd64 ./package.sh && PLATFORM=linux/amd64 ./verify.sh
```

Both need Docker; neither needs GTK on the host. `bundle.sh`, `run.sh`,
`install.sh` and `test.sh` run *inside* the containers. CI calls exactly these
two scripts, so a developer and the release path cannot drift apart.

`PLATFORM` defaults to this machine's own architecture, and that default is
load-bearing rather than a convenience — see the AppImage notes below.

## Why it is built in a container

The glibc symbol versions a binary asks for are those of the environment it was
built in: build on a modern distro and the result refuses to start on the
target. Debian 10 is glibc 2.28, the oldest target, so that is the build image.
The binary currently asks for `GLIBC_2.28` exactly — it runs on 2.28 and newer,
and would need Rust's own target support for anything older.

## What had to be discovered, not assumed

- **The theme is free.** Adwaita is compiled into `libgtk-3-0` as a GResource,
  so it travels with the library. There is no theme directory to stage.
- **Host GTK/GIO/font modules must not leak in.** `run.sh` unsets host
  `GTK_MODULES`/`GIO_EXTRA_MODULES`, points `GIO_MODULE_DIR` at the bundle, and
  uses only bundled fonts (Noto Sans + DejaVu + WenQuanYi). See
  [`docs/linux-runtime-notes.md`](../../docs/linux-runtime-notes.md).
- **GSettings schemas are not.** `libgtk-3-0` owns `org.gtk.Settings.*` and GTK
  refuses to start without them. They must be *compiled* with
  `glib-compile-schemas`, and every `*.xml` has to be copied, not just the
  `*.gschema.xml` files: the enum types the schemas refer to live in files with
  the shorter suffix, and leaving them out fails with "not (yet) defined".
- **Two caches hold absolute paths.** The gdk-pixbuf loader cache and the GTK
  input-method cache are generated at build time, pointing into the build
  container. They are written with a placeholder and rewritten by `run` into a
  writable cache directory, because a bundle can be moved.
- **The generator tools are off PATH.** Debian installs
  `gdk-pixbuf-query-loaders` and `gtk-query-immodules-3.0` under the multiarch
  libdir, not in `/usr/bin`.
- **A bundle whose shape people ask for is a single file.** A directory tree
  needs unpacking before anything runs, which reads as "this is not a binary".
  An AppImage is one executable: the same tree plus `AppRun`, an embedded
  squashfs payload, and a runtime that mounts it. The runtime is fetched per
  architecture at image build time; `--appimage-extract-and-run` is how it is
  invoked in the build and test containers, since neither has FUSE — and neither
  will every target machine, so that path is the one the test exercises.
- **No AppImage can be built or run under an emulation layer.** Every AppImage
  runtime carries `AI` in the ELF header byte `EI_ABIVERSION` (that marker *is*
  the format). A real kernel ignores it; qemu and Rosetta reject the file with
  "Exec format error". Hence the host-native `PLATFORM` default, and hence
  `test.sh` asking the environment whether it can run an AppImage at all before
  running one — so an emulation limit is reported as one, not as a defect in the
  artifact. CI's runners are the real architecture, so the AppImage runs there.
- **The two artifacts are asserted to be the same program.** `test.sh` renders
  the tree and the AppImage in the same bare container and requires the two PNGs
  to be byte-identical, so "the AppImage is the same app" is checked rather than
  assumed.
- **glibc stays on the host.** libc, libm, the dynamic loader and the NSS
  modules are *not* bundled: they are forward compatible, and a bundled libc
  breaks name resolution. Everything else is.
- **Fonts have to be shipped.** DejaVu covers Latin and WenQuanYi Micro Hei
  covers CJK; both are redistributable. The client's own fonts are not used.
  `fonts.conf.template` points only at the bundle, deliberately: appending the
  system font directories would look better on a desktop but would make text
  metrics — and therefore the measured geometry — depend on the machine.
- **No `im-ibus`.** Debian's GTK ships `im-xim` and friends, but ibus brings its
  own module, so CJK typing inside the bundle currently goes through XIM.
- **The output must belong to the caller.** `docker run --user` is not cosmetic
  here: a root-owned `dist/app` cannot be deleted by the user who built it, and
  the next `package.sh` run starts by deleting it.
- **A report must not be able to fail the build.** The closing `readelf` loop
  finds no `GLIBC_` string in a few libraries (libcairo-gobject, libicudata), so
  `grep` exits 1; under `set -e` with `pipefail` that failed the whole bundle
  *after* printing a truncated answer. It survived several local runs because
  they piped the script through `tail`, which reports tail's exit status, not the
  script's. CI found it on its first tag, which is what CI is for.
- **POSIX `sh` in the bundle.** `install.sh` and `run` run under dash on Debian,
  so the CI step syntax-checks them with `dash -n`; bash-only syntax would
  otherwise only fail on a user's machine.
- **A `[target.'cfg(...)'.dependencies]` header swallows every dependency
  declared after it.** The plugin-surface dependencies were once listed below
  the macOS-only `libc` entry — valid TOML, wrong section — so the crate
  compiled *on macOS* (where the cfg is true) and failed in the Debian build
  container with "unlinked crate `eb_api`". The container caught it on the
  first build after the move, which is the whole argument for building in the
  oldest target's environment instead of trusting a local pass.
- **Debian 10 is EOL.** apt has to point at `archive.debian.org` and tolerate
  expired valid-until dates.

## Not done yet

- Input-method support for ibus (`im-ibus.so` + `libibus-1.0.so.0`).
- Windows: the same treatment, plus Windows 7/8.1. See the top-level README.
