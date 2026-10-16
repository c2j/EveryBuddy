#!/usr/bin/env bash
#
# Builds the Linux release inside a Debian 10 (buster) container.
#
# Debian 10 ships glibc 2.28, so the resulting binary only requires glibc
# symbols up to 2.28 and therefore runs on any glibc >= 2.28. It also ships
# GTK+ 3.24, which is new enough for the `gtk` crate (minimum 3.22).
#
# This script runs *inside* the container as root, with the repository mounted
# at /src. `ARCH` (x86_64/aarch64) and the host uid/gid are passed in via env.
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive

# Debian 10 is archived, so point apt at archive.debian.org and disable the
# (long expired) release-file validity check.
printf '%s\n' \
    'deb http://archive.debian.org/debian buster main' \
    'deb http://archive.debian.org/debian-security buster/updates main' \
    > /etc/apt/sources.list
rm -f /etc/apt/sources.list.d/*

apt-get -o Acquire::Check-Valid-Until=false -o Acquire::Retries=3 update
apt-get -y -o Acquire::Check-Valid-Until=false install --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    pkg-config \
    libgtk-3-dev

export CARGO_HOME=/root/.cargo
export RUSTUP_HOME=/root/.rustup

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh
sh /tmp/rustup.sh -y --profile minimal --default-toolchain stable
export PATH="$CARGO_HOME/bin:$PATH"

cd /src

echo "=== toolchain ==="
rustc --version
echo "=== gtk version ==="
pkg-config --modversion gtk+-3.0
echo "=== build ==="
cargo build --release --locked

arch="${ARCH:-$(uname -m)}"

# Everything the release needs: the stripped binary plus a short note.
out=/tmp/notebook-release
rm -rf "$out"
mkdir -p "$out"
strip -o "$out/notebook" target/release/notebook

cat > "$out/README.txt" <<EOF
notebook (linux-${arch})
=======================

Built on Debian 10 against glibc 2.28: runs on glibc >= 2.28.

Runtime dependency: GTK+ 3.22 or newer, e.g.
  Debian/Ubuntu: sudo apt install libgtk-3-0
  Fedora:        sudo dnf install gtk3
For CJK input also install your IME framework (ibus/fcitx5) and CJK fonts.

Run with: ./notebook
EOF

tarball="/src/notebook-linux-${arch}.tar.gz"
tar -C "$out" -czf "$tarball" notebook README.txt

# Hand the artifact to the runner user so later steps can read/replace it.
chown -R "${HOST_UID:-0}:${HOST_GID:-0}" "$tarball"

echo "=== artifact ==="
ls -l "$tarball"
