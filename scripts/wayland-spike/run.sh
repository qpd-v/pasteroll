#!/bin/bash
# Phase-one research only. No live display, bus, clipboard, profile or network.
set -euo pipefail
spike_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
spike_uid=$(id -u)
spike_gid=$(id -g)
exec sudo -n unshare --mount --net --pid --fork --mount-proc sh -eu -c '
    mount --make-rprivate /
    mount -t tmpfs -o mode=1777 tmpfs /tmp
    mount -t tmpfs -o mode=755 tmpfs /run
    printf '%s\n' isolated-wayland-spike > /run/pasteroll-wayland-spike
    chmod 444 /run/pasteroll-wayland-spike
    mkdir -m 1777 /tmp/.X11-unix
    spike_uid="$1"
    spike_gid="$2"
    spike_script="$3/scripts/wayland-spike/run.py"
    shift 3
    exec setpriv --reuid "$spike_uid" --regid "$spike_gid" --clear-groups \
        env -i PATH=/usr/bin:/bin LANG=C.UTF-8 \
        PASTEROLL_WAYLAND_SPIKE=1 python3 "$spike_script" "$@"
' pasteroll-wayland "$spike_uid" "$spike_gid" "$spike_root" "$@"
