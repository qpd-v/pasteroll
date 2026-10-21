#!/bin/bash
# Headless keyboard tests only. All input, profiles, buses and displays are private.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
for arg in "$@"; do
    case "$arg" in --mouse|--release) printf '%s\n' 'Mouse/helper tests require run.sh and its device isolation.' >&2; exit 1;; esac
done
layout=${PASTEROLL_GEOMETRY_LAYOUT:-}
case "$layout" in ''|physical|logical) ;; *) printf '%s\n' 'Invalid geometry test layout' >&2; exit 1;; esac
command -v bwrap >/dev/null
# UID 0 is mapped to the invoking unprivileged user ONLY inside this namespace;
# it supplies the harness marker, not host administrator access.
exec bwrap --unshare-all --uid 0 --gid 0 --die-with-parent --new-session \
    --ro-bind / / --tmpfs /home --tmpfs /root --tmpfs /media --tmpfs /run --tmpfs /tmp \
    --proc /proc --dev /dev --bind "$root" "$root" --chdir "$root" \
    -- /usr/bin/env -i PATH=/usr/bin:/bin LANG=C.UTF-8 \
    PASTEROLL_WAYLAND_SPIKE=1 PASTEROLL_GEOMETRY_LAYOUT="$layout" \
    PASTEROLL_TEST_APP_BINARY="${PASTEROLL_TEST_APP_BINARY:-}" \
    /bin/sh -ec 'printf isolated-wayland-spike > /run/pasteroll-wayland-spike; chmod 444 /run/pasteroll-wayland-spike; exec python3 scripts/wayland-spike/run.py "$@"' sh "$@"
