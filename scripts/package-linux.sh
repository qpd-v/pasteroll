#!/bin/bash
# Build both versioned Linux packages locally; never install software or download dependencies.
# Run on Linux/WSL: bash scripts/package-linux.sh
# Prerequisites: Rust with the musl target, musl-tools, Python 3, binutils, dpkg-deb.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
cd "$root"
for tool in cargo rustc musl-gcc python3 readelf dpkg-deb; do
    command -v "$tool" >/dev/null || { printf 'Required local tool missing: %s\n' "$tool" >&2; exit 1; }
done
target=x86_64-unknown-linux-musl
sysroot="$(rustc --print sysroot)"
# Encoded arguments also work when the checkout or home folder contains spaces. Use a generic
# home prefix without /home/ so the artifact scanner can flag any unremapped Linux home paths.
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=$HOME=/user"$'\x1f'"--remap-path-prefix=$sysroot=/rust"$'\x1f'"--remap-path-prefix=$root=/src"
cargo build --release --locked --offline --target "$target" --target-dir "$root/target" -p pasteroll-linux
python3 scripts/package-linux.py
