#!/bin/bash
# Record only a private X server and synthetic profile, never the desktop clipboard.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
demo_uid=$(id -u)
demo_gid=$(id -g)
exec sudo -n unshare --mount --net --fork sh -eu -c '
    mount --make-rprivate /
    mount -t tmpfs -o mode=1777 tmpfs /tmp
    mkdir -m 1777 /tmp/.X11-unix
    exec setpriv --reuid "$1" --regid "$2" --clear-groups python3 "$3/scripts/record-linux-demos.py"
' pasteroll-demo-x11 "$demo_uid" "$demo_gid" "$root"
