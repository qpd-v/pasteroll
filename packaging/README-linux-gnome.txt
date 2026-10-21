Pasteroll for X11 and GNOME 46 Wayland - @VERSION@ development build
================================================================

This is an experimental GNOME 46 package. Read
GNOME-README.md for permissions, supported devices and validation limits.
The app is static. The optional host helper uses glibc and system libevdev2.

Archive installation
  As your ordinary desktop account:
    ./install.sh
    ./install-gnome-companion.sh

  To enable the original Ctrl+Shift+middle-click gesture on GNOME Wayland:
    sudo ./install-input-helper.sh --enable

  Log out and back in, then as your ordinary account:
    gnome-extensions enable pasteroll@pasteroll.com
    pasteroll run --backend gnome

  The helper requires systemd, logind and uinput. It supports relative mice
  with a middle button. It must not share the same device with another remapper.
  It does not patch GNOME, read clipboard data or run the app as root.

Debian package
  Install the development .deb using your package manager to resolve dependencies.
  It includes a system GNOME extension. Remove an older per-user extension copy
  if it would override that system copy. The helper stays disabled on a new install:
    sudo systemctl enable --now pasteroll-input.service
  Then log out/back in, enable the extension and start the app as above.

Keyboard alternative (no host helper required)
    pasteroll run --backend gnome --wayland-keyboard
  Uses Ctrl+Shift+Space. There is no automatic keyboard substitution.

X11
    pasteroll run
  Uses the existing X11 backend when run in an X11 session.

Remove
  Quit with: pasteroll stop
  Archive helper: sudo ./install-input-helper.sh --uninstall
  Per-user companion: ./install-gnome-companion.sh --uninstall
  Per-user app: ./install.sh --uninstall
  System archive app: sudo ./install.sh --system --uninstall
  Debian package: use the package manager to remove pasteroll.
  These paths keep clipboard history, encryption keys and settings.

Testing status and limitations
  Mixed-scale display/viewer/editor checks passed on one physical Pop!_OS setup.
  Further manual acceptance, including helper input, lock/login and suspend, is
  deferred. GNOME remains experimental and supports version 46 only.
  App and companion must both use protocol 3; reload the companion after upgrade.
  OCR accuracy improvements and Linux tray/panel/About parity are deferred.

License: MIT or Apache-2.0, at your option. See the included licenses/notices.
Source: https://github.com/qpd-v/pasteroll
