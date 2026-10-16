Pasteroll for Linux (X11) - 0.1.0-beta.2
========================================

Pasteroll keeps a history of what you copy and lets you paste it again: scroll through
your clips with the mouse wheel and paste the whole clip, or select part of it and paste
just that part.

Requirements
  An X11 session (or XWayland for X11 programs). Wayland-only programs are not covered.
  A system tray icon (StatusNotifier, e.g. KDE or GNOME with the AppIndicator extension, or the
  older XEmbed tray); the program works without one.

Install (for your account, no root)
  Extract pasteroll-0.1.0-beta.2-linux-x86_64.tar.gz, enter its folder, then run ./install.sh.
  This delegates to ./pasteroll install: ~/.local/bin, application menu entry and icon.
  Make sure ~/.local/bin is in your PATH. Start it from the menu or with:  pasteroll run

Install (all users, root required)
  sudo ./install.sh --system
  Copies to /usr/local/bin, with a system menu entry, icons and license files.
  Each account keeps its own history and settings. No autostart is enabled.
  Quit and restart running copies after an upgrade. Existing marked installs can be upgraded.

Debian / Ubuntu package
  sudo dpkg -i pasteroll_0.1.0-beta.2_amd64.deb
  Installs /usr/bin/pasteroll, a menu entry, icons and licenses. No scripts run at install time.
  The binary is static and has no package dependencies. Optional recommendations are
  tesseract-ocr (local text extraction) and zenity (save dialogs); install them separately if wanted.
  The internal Debian version is 0.1.0~beta.2 so the beta sorts before the stable 0.1.0.

Use
  Ctrl+Shift+middle-click      open the carousel at the pointer
  mouse wheel / Up / Down      move through clips
  type                         search (file names too)
  Tab                          choose a filter (Text, Image, Document...)
  Enter                        paste the clip; double-click or drag first to paste only that part
  Ctrl+P                       pin a clip      Delete   recycle a clip      Esc  close
  Tray icon                    pause, settings, quit

Terminals
  Most terminals paste with Ctrl+Shift+V, xterm-family terminals with Shift+Insert; both
  are handled. Other programs can be added in ~/.local/share/pasteroll/paste_keys.txt:
      myterminal = ctrl+shift+v

Your data
  Stored encrypted in ~/.local/share/pasteroll (folder mode 0700). The key file is
  protected by file permissions only, so full-disk encryption is recommended.
  Nothing is sent anywhere.

Portable mode
  Create portable.txt beside pasteroll: all app data stays in the adjacent data folder.
  Installation, autostart changes and --db are disabled. Quit, then delete the folder to remove it.
  pasteroll stop asks the instance using this data folder to quit gracefully; if none is running
  it succeeds without creating data. Older versions must be quit from the tray.

Remove
  Per-user archive:   ./install.sh --uninstall
  System archive:     sudo ./install.sh --system --uninstall
  Debian package:     sudo dpkg -r pasteroll
  All of these keep history and settings. Purging the Debian package also keeps data.
  Each archive installer can be run twice. System removal requires its ownership marker.
  The app's own install/uninstall commands manage per-user installs only; use the matching
  installer or dpkg for a system installation. The archive installer never runs network commands.

Command line
  pasteroll status | config show | config set KEY VALUE | exclude PROGRAM | stop | autostart on|off

License
  MIT or Apache-2.0, at your option (LICENSE-MIT, LICENSE-APACHE).
  Source and issues: https://github.com/qpd-v/pasteroll
