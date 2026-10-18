#!/bin/sh
# Optional host helper only. Never run the user application as root.
set -eu
action=install
enable=0
for arg in "$@"; do
    case "$arg" in
        --uninstall) action=uninstall ;;
        --enable) enable=1 ;;
        --help|-h) printf '%s\n' 'sudo ./install-input-helper.sh [--enable | --uninstall]' \
            'Installs the optional input service. --enable starts it; uninstall keeps all user data.'; exit 0 ;;
        *) printf '%s\n' 'Unknown helper installer option.' >&2; exit 2 ;;
    esac
done
root=
if [ -n "${DESTDIR:-}" ]; then
    case "$DESTDIR" in /*) ;; *) printf '%s\n' 'DESTDIR must be absolute.' >&2; exit 2 ;; esac
    root=$(realpath -m -- "$DESTDIR")
    [ "$root" != / ] || root=
fi
if [ -z "$root" ] && [ "$(id -u)" -ne 0 ]; then
    printf '%s\n' 'The host input helper needs administrator installation; run this script with sudo.' >&2
    exit 1
fi
source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
binary="$root/usr/local/libexec/pasteroll-input"
unit="$root/etc/systemd/system/pasteroll-input.service"
policy="$root/etc/dbus-1/system.d/com.pasteroll.Input.conf"
modules="$root/etc/modules-load.d/pasteroll-input.conf"
marker="$root/usr/local/share/pasteroll/input-helper-source.txt"
expected='Pasteroll optional archive input helper v1'
if [ -e "$marker" ] || [ -L "$marker" ]; then
    if [ ! -f "$marker" ] || [ -L "$marker" ] || [ "$(cat -- "$marker")" != "$expected" ]; then
        printf '%s\n' 'Unknown helper ownership marker; nothing was changed.' >&2; exit 1
    fi
else
    for path in "$binary" "$unit" "$policy" "$modules" "$root/usr/libexec/pasteroll-input"; do
        if [ -e "$path" ] || [ -L "$path" ]; then
            printf '%s\n' 'Another helper installation already exists; use its package manager.' >&2; exit 1
        fi
    done
    if [ "$action" = uninstall ]; then
        printf '%s\n' 'No archive helper installation found.'; exit 0
    fi
fi
if [ "$action" = install ]; then
    for path in "$source_dir/pasteroll-input" "$source_dir/pasteroll-input.service" "$source_dir/com.pasteroll.Input.conf" "$source_dir/pasteroll-input-modules.conf"; do
        if [ ! -f "$path" ] || [ -L "$path" ]; then
            printf '%s\n' 'Extract the complete GNOME archive before installing the helper.' >&2; exit 1
        fi
    done
fi
was_active=0
if [ -z "$root" ]; then
    command -v systemctl >/dev/null || { printf '%s\n' 'A systemd host is required.' >&2; exit 1; }
    if [ "$action" = install ]; then
        command -v modprobe >/dev/null || { printf '%s\n' 'The uinput kernel module is required.' >&2; exit 1; }
        modprobe uinput
        [ -c /dev/uinput ] || { printf '%s\n' 'The uinput kernel device is unavailable.' >&2; exit 1; }
        # Fail before installing files if the dynamic runtime library is missing.
        "$source_dir/pasteroll-input" --help >/dev/null
    fi
    if systemctl is-active --quiet pasteroll-input.service; then was_active=1; fi
    systemctl stop pasteroll-input.service 2>/dev/null || :
fi
if [ "$action" = uninstall ]; then
    if [ -z "$root" ]; then systemctl disable pasteroll-input.service 2>/dev/null || :; fi
    rm -f -- "$binary" "$unit" "$policy" "$modules" "$marker"
    if [ -z "$root" ]; then systemctl daemon-reload; fi
    printf '%s\n' 'Input helper removed. Clipboard history and settings were kept.'
    exit 0
fi
put() (
    src=$1 dst=$2 mode=$3
    install -d -m 0755 -- "$(dirname -- "$dst")"
    tmp=$(mktemp "${dst}.XXXXXX")
    trap 'rm -f -- "$tmp"' EXIT
    trap 'exit 1' HUP INT TERM
    install -m "$mode" -- "$src" "$tmp"
    mv -f -- "$tmp" "$dst"
)
install -d -m 0755 -- "$(dirname -- "$marker")"
printf '%s\n' "$expected" > "$marker"
put "$source_dir/pasteroll-input" "$binary" 0755
put "$source_dir/pasteroll-input.service" "$unit" 0644
put "$source_dir/com.pasteroll.Input.conf" "$policy" 0644
put "$source_dir/pasteroll-input-modules.conf" "$modules" 0644
if [ -z "$root" ]; then
    systemctl daemon-reload
    if [ "$enable" -eq 1 ]; then systemctl enable --now pasteroll-input.service; fi
    if [ "$enable" -eq 0 ] && [ "$was_active" -eq 1 ]; then systemctl start pasteroll-input.service; fi
fi
printf '%s\n' 'Host helper installed. The Pasteroll app runs as your ordinary account.'
