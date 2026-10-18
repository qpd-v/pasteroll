#!/bin/bash
# Build development GNOME archive + Debian package. Never publish or install them.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
cd "$root"
for tool in cargo rustc musl-gcc python3 readelf dpkg-deb; do
    command -v "$tool" >/dev/null || { printf 'Missing prerequisite: %s\n' "$tool" >&2; exit 1; }
done
build_root=${PASTEROLL_BUILD_ROOT:-$root/target-dev/gnome-package}
output=${PASTEROLL_PACKAGE_OUTPUT:-$root/target-dev/wayland-mouse/packages}
mkdir -p "$build_root" "$output"
sysroot=$(rustc --print sysroot)
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
unset RUSTFLAGS
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=$HOME=/user"$'\x1f'"--remap-path-prefix=$sysroot=/rust"$'\x1f'"--remap-path-prefix=$root=/src"$'\x1f'"--remap-path-prefix=$build_root=/build"
export CFLAGS="-ffile-prefix-map=$root=/src -ffile-prefix-map=$HOME=/user -ffile-prefix-map=$build_root=/build"
export CXXFLAGS="$CFLAGS"
cargo build --release --locked --offline --target x86_64-unknown-linux-musl --target-dir "$build_root" -p pasteroll-linux --bin pasteroll
cargo build --release --locked --offline --target x86_64-unknown-linux-gnu --target-dir "$build_root" -p pasteroll-linux --bin pasteroll-input
python3 scripts/package-linux.py --with-gnome \
    --app-bin "$build_root/x86_64-unknown-linux-musl/release/pasteroll" \
    --helper-bin "$build_root/x86_64-unknown-linux-gnu/release/pasteroll-input" --output "$output"
printf '%s\n' 'Development artifacts built; run the privacy scanner before sharing. No release was published.'
