# Linux bundle

`dist/app` is a directory that runs on a machine with no GTK, no Pango, no
Cairo, no fontconfig and no fonts. It carries all of them plus the app:

```
app/
  run                  # the entry point; sets every path relative to itself
  bin/evobuddy
  lib/                 # GTK 3, GLib, Pango, Cairo, GDK, HarfBuzz, ... (72 .so)
  share/               # fonts, hicolor icons, compiled GSettings schemas
  etc/fonts.conf.template
```

```sh
./package.sh   # build on glibc 2.28, stage dist/app
./verify.sh    # run dist/app on a bare Debian 10 and print what it needed
```

Both need Docker; neither needs GTK on the host. `bundle.sh`, `run.sh` and
`test.sh` run *inside* the containers.

## Why it is built in a container

The glibc symbol versions a binary asks for are those of the environment it was
built in: build on a modern distro and the result refuses to start on the
target. Debian 10 is glibc 2.28, the oldest target, so that is the build image.
The binary currently asks for `GLIBC_2.28` exactly — it runs on 2.28 and newer,
and would need Rust's own target support for anything older.

## What had to be discovered, not assumed

- **The theme is free.** Adwaita is compiled into `libgtk-3-0` as a GResource,
  so it travels with the library. There is no theme directory to stage.
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
- **Debian 10 is EOL.** apt has to point at `archive.debian.org` and tolerate
  expired valid-until dates.

## Not done yet

- Input-method support for ibus (`im-ibus.so` + `libibus-1.0.so.0`).
- A launcher entry, an icon and a versioned artifact name: this is a directory,
  not yet something to hand to a user.
- Windows and macOS packaging.
