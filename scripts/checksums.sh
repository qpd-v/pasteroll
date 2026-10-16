#!/bin/bash
# Generate the current-version manifest on Linux/WSL, or pass --verify to validate it.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
exec python3 "$root/scripts/checksums.py" "$@"
