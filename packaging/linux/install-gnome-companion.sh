#!/bin/sh
set -eu
if [ "$(id -u)" -eq 0 ]; then
    printf '%s\n' 'Run the companion installer as your normal desktop account, without sudo.' >&2; exit 1
fi
source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
uuid=pasteroll@pasteroll.com
target="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/$uuid"
case "${1:-}" in
    --uninstall)
        gnome-extensions disable "$uuid" 2>/dev/null || :
        gnome-extensions uninstall "$uuid"
        exit 0 ;;
    '' ) ;;
    *) printf '%s\n' 'Usage: ./install-gnome-companion.sh [--uninstall]' >&2; exit 2 ;;
esac
[ "$#" -le 1 ] || exit 2
[ -d "$source_dir/gnome-companion" ] || { printf '%s\n' 'The complete GNOME archive is required.' >&2; exit 1; }
# A fixed file list avoids copying unrelated data and permits repeat upgrades.
install -d -m 0755 -- "$target"
for file in bridge.js dialogs.js extension.js input-helper.js panel.js settings-ui.js metadata.json stylesheet.css; do
    install -m 0644 -- "$source_dir/gnome-companion/$file" "$target/$file"
done
printf '%s\n' 'Companion installed. Log out and back in, then run:' \
    '  gnome-extensions enable pasteroll@pasteroll.com' \
    '  pasteroll run --backend gnome'
