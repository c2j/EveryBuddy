# LGPL source offer

EvoBuddy's own code is MIT-licensed. The self-contained Linux and macOS
packages dynamically link the GTK 3 stack, which is licensed under the GNU
Lesser General Public License, version 2.1 or later (LGPL-2.1-or-later).

This file is the written offer for the corresponding source code of those
libraries, valid for at least three years from the date of the accompanying
binary release.

## Libraries

| Library | License | Corresponding source |
|---|---|---|
| GTK 3 | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/gtk |
| GLib | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/glib |
| Pango | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/pango |
| ATK | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/at-spi2-core |
| gdk-pixbuf | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/gdk-pixbuf |
| librsvg | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/librsvg |
| Cairo | LGPL-2.1-or-later or MPL-1.1 | https://gitlab.freedesktop.org/cairo/cairo |

The exact versions used by a release are recorded in `SBOM.json`. The build
recipes that select and copy them are:

- `packaging/linux/bundle.sh` (Debian 10 build image; dynamically linked `.so` files)
- `packaging/macos/bundle.sh` (Homebrew GTK 3; rewritten `@rpath` dylibs)

## How to obtain the source

1. Download the release tag or tarball for the version in `SBOM.json` from the
   upstream URL above.
2. For the Linux package, the same source is available from Debian's source
   packages for the corresponding versions (`apt-get source gtk+3.0` etc.).
3. For the macOS package, use the Homebrew formula versions pinned by the
   release's build log; `brew info gtk+3` gives the upstream tarball URL.

If a URL is unavailable, contact the EvoBuddy maintainers through the
repository issue tracker and we will provide the corresponding source on
physical or electronic media for the cost of media and shipping.

## Relinking

The packages keep the LGPL libraries as separate dynamic libraries
(`.so`/`.dylib`), not statically linked into the EvoBuddy binary. The user can
replace them with a compatible build and relink/re-run the application; no
EvoBuddy source changes are required. The launcher scripts do not prevent
replacement.

## No warranty

The LGPL libraries are distributed in the hope that they will be useful, but
WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
FITNESS FOR A PARTICULAR PURPOSE. See `LICENSES/LGPL-2.1.txt`.
