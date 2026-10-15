#!/bin/bash
# Writes dist/SHA256SUMS for the release archives (run from the repository root after both
# packages are built).
set -euo pipefail
cd dist
sha256sum pasteroll-windows-x64.zip pasteroll-linux-x86_64.tar.gz > SHA256SUMS
cat SHA256SUMS
