#!/bin/sh
# Local archive installer. No downloads, services, autostart or history removal.
set -eu

system=0
uninstall=0
for arg do
    case "$arg" in
        --system) system=1 ;;
        --uninstall) uninstall=1 ;;
        --help|-h)
            printf '%s\n' 'Usage: ./install.sh [--system] [--uninstall]' \
                'Default: install for this account using ./pasteroll install.' \
                '--system: install under /usr/local (run with sudo).' \
                '--uninstall: remove the selected installation; always keep history.'
            exit 0 ;;
        *) printf 'Unknown option: %s\n' "$arg" >&2; exit 2 ;;
    esac
done

source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
binary="$source_dir/pasteroll"
if [ ! -x "$binary" ]; then
    printf '%s\n' 'The executable is missing. Extract the complete archive before installing.' >&2
    exit 1
fi
if [ -f "$source_dir/portable.txt" ]; then
    printf '%s\n' 'This is a portable copy. Remove portable.txt before installing or uninstalling.' >&2
    exit 1
fi

if [ "$system" -eq 0 ]; then
    if [ -n "${DESTDIR:-}" ]; then
        printf '%s\n' 'DESTDIR is supported only with --system.' >&2
        exit 2
    fi
    if [ "$uninstall" -eq 1 ]; then
        printf '%s\n' 'Will stop Pasteroll and remove its per-user program, menu entry, icon and autostart entry.' \
            'Your clipboard history and settings will be kept.'
        "$binary" stop
        exec "$binary" uninstall
    fi
    printf '%s\n' 'Will stop Pasteroll and install for this account in ~/.local/bin, with a menu entry and icon.' \
        'Existing history, settings and autostart preferences will be kept. No autostart is enabled.'
    "$binary" stop
    exec "$binary" install
fi

# DESTDIR is for staging/tests, never a different final installation prefix. The desktop
# entry always targets /usr/local/bin. Canonicalize it before deciding whether root is needed.
destdir=
if [ -n "${DESTDIR:-}" ]; then
    case "$DESTDIR" in /*) ;; *) printf '%s\n' 'DESTDIR must be absolute.' >&2; exit 2 ;; esac
    destdir=$(realpath -m -- "$DESTDIR")
    [ "$destdir" != / ] || destdir=
fi
if [ -z "$destdir" ] && [ "$(id -u)" -ne 0 ]; then
    printf '%s\n' 'System installation/removal needs root. Run: sudo ./install.sh --system [--uninstall]' >&2
    exit 1
fi
prefix="$destdir/usr/local"
marker="$prefix/share/pasteroll/install-source.txt"
sizes='16 24 32 48 64 128 256 512'
docs='README.txt LICENSE-MIT LICENSE-APACHE THIRD-PARTY-NOTICES.md'

if [ "$uninstall" -eq 1 ]; then
    printf 'Will remove the archive installation under %s.\n' "$prefix"
    printf '%s\n' 'All accounts retain their history and settings. Quit running copies first.'
    if [ ! -f "$marker" ] || [ -L "$marker" ]; then
        if [ -e "$prefix/bin/pasteroll" ] || [ -L "$prefix/bin/pasteroll" ]; then
            printf '%s\n' 'Archive ownership marker is missing; refusing to remove an unknown installation.' >&2
            exit 1
        fi
        printf '%s\n' 'No archive installation found; nothing to remove.'
        exit 0
    fi
    if [ "$(cat -- "$marker")" != 'Pasteroll system tar installation v1' ]; then
        printf '%s\n' 'Archive ownership marker is unrecognized; nothing was removed.' >&2
        exit 1
    fi
    rm -f -- "$prefix/bin/pasteroll" "$prefix/share/applications/pasteroll.desktop"
    for size in $sizes; do
        rm -f -- "$prefix/share/icons/hicolor/${size}x${size}/apps/pasteroll.png"
    done
    for name in $docs; do rm -f -- "$prefix/share/doc/pasteroll/$name"; done
    rm -f -- "$marker"
    rmdir -- "$prefix/share/doc/pasteroll" "$prefix/share/pasteroll" 2>/dev/null || :
    printf '%s\n' 'System archive installation removed. History and settings were kept.'
    exit 0
fi

printf 'Will install the program under %s/bin, with a system menu entry, icons and licenses.\n' "$prefix"
printf '%s\n' 'No autostart is enabled. History and settings remain per-account.' \
    'Quit and restart any existing running copies to use the new version.'

if [ -e "$marker" ] || [ -L "$marker" ]; then
    if [ ! -f "$marker" ] || [ -L "$marker" ] || \
        [ "$(cat -- "$marker")" != 'Pasteroll system tar installation v1' ]; then
        printf '%s\n' 'Archive ownership marker is unrecognized; nothing was installed.' >&2
        exit 1
    fi
else
    # Check all destinations first so a conflict cannot leave a partial installation.
    refuse_existing() {
        if [ -e "$1" ] || [ -L "$1" ]; then
            printf '%s\n' 'An unmarked system installation already exists; refusing to overwrite it.' >&2
            exit 1
        fi
    }
    refuse_existing "$prefix/bin/pasteroll"
    refuse_existing "$prefix/share/applications/pasteroll.desktop"
    for size in $sizes; do refuse_existing "$prefix/share/icons/hicolor/${size}x${size}/apps/pasteroll.png"; done
    for name in $docs; do refuse_existing "$prefix/share/doc/pasteroll/$name"; done
fi

# Replace a program atomically, including if an older Linux process is still using its inode.
# A destination symlink is replaced rather than followed. Each temporary file has its own trap.
install_file() (
    src=$1
    dst=$2
    mode=$3
    install -d -m 0755 -- "$(dirname -- "$dst")"
    tmp=$(mktemp "${dst}.XXXXXX")
    trap 'rm -f -- "$tmp"' EXIT
    trap 'exit 1' HUP INT TERM
    install -m "$mode" -- "$src" "$tmp"
    mv -f -- "$tmp" "$dst"
)
# Record ownership first, after preflight, so a retry can complete an interrupted file copy.
install_file "$source_dir/install-source.txt" "$marker" 0644
install_file "$binary" "$prefix/bin/pasteroll" 0755
install_file "$source_dir/pasteroll.desktop" "$prefix/share/applications/pasteroll.desktop" 0644
for size in $sizes; do
    install_file "$source_dir/icons/${size}x${size}/apps/pasteroll.png" \
        "$prefix/share/icons/hicolor/${size}x${size}/apps/pasteroll.png" 0644
done
for name in $docs; do install_file "$source_dir/$name" "$prefix/share/doc/pasteroll/$name" 0644; done
printf '%s\n' 'System archive installation complete. Start Pasteroll from the menu in an X11 session.'
