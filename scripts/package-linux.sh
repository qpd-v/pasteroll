#!/bin/bash
# Builds a release program for other people's machines and packs it with its README into dist/.
#
# - static (musl) build: runs on any x86_64 Linux, whatever its libc version
# - build paths are remapped, so the binary does not carry your user name or folder layout
#
# Run on Linux from the repository root:  bash scripts/package-linux.sh
# One-time setup:  sudo apt install musl-tools && rustup target add x86_64-unknown-linux-musl
set -euo pipefail
target=x86_64-unknown-linux-musl
sysroot="$(rustc --print sysroot)"
export RUSTFLAGS="--remap-path-prefix=$HOME=/home/user --remap-path-prefix=$sysroot=/rust --remap-path-prefix=$PWD=/src"
cargo build --release --target "$target" -p pasteroll-linux
out=dist/pasteroll-linux-x86_64
rm -rf "$out"
mkdir -p "$out"
cp "target/$target/release/pasteroll" "$out/pasteroll"
cp packaging/README-linux.txt "$out/README.txt"
cp LICENSE-MIT LICENSE-APACHE "$out/"
tar -C dist -czf dist/pasteroll-linux-x86_64.tar.gz pasteroll-linux-x86_64
sha256sum dist/pasteroll-linux-x86_64.tar.gz
