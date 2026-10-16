#!/bin/bash
# Private mount/network namespaces avoid WSLg's real X sockets and the live clipboard.
# Requires existing unattended sudo; no host mounts or network settings are changed.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
test_uid=$(id -u)
test_gid=$(id -g)
exec sudo -n unshare --mount --net --fork sh -eu -c '
    mount --make-rprivate /
    mount -t tmpfs -o mode=1777 tmpfs /tmp
    mkdir -m 1777 /tmp/.X11-unix
    exec setpriv --reuid "$1" --regid "$2" --clear-groups python3 "$3/scripts/test-linux-x11.py"
' pasteroll-private-x11 "$test_uid" "$test_gid" "$root"
