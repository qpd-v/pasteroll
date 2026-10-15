Pasteroll for Linux (X11) - 0.1.0-beta.1
========================================

Pasteroll keeps a history of what you copy and lets you paste it again: scroll through
your clips with the mouse wheel and paste the whole clip, or select part of it and paste
just that part.

Requirements
  An X11 session (or XWayland for X11 programs). Wayland-only programs are not covered.
  A system tray icon (StatusNotifier, e.g. KDE or GNOME with the AppIndicator extension, or the
  older XEmbed tray); the program works without one.

Install (for your account, no root)
  ./pasteroll install
  This copies the program to ~/.local/bin, adds an application menu entry and an icon.
  Make sure ~/.local/bin is in your PATH. Start it from the menu or with:  pasteroll run

Use
  Ctrl+Shift+middle-click      open the carousel at the pointer
  mouse wheel / Up / Down      move through clips
  type                         search (file names too)
  Tab                          choose a filter (Text, Image, Document...)
  Enter                        paste the clip; double-click or drag first to paste only that part
  Ctrl+P                       pin a clip      Ctrl+Delete   delete a clip      Esc  close
  Tray icon                    pause, settings, quit

Terminals
  Most terminals paste with Ctrl+Shift+V, xterm-family terminals with Shift+Insert; both
  are handled. Other programs can be added in ~/.local/share/pasteroll/paste_keys.txt:
      myterminal = ctrl+shift+v

Your data
  Stored encrypted in ~/.local/share/pasteroll (folder mode 0700). The key file is
  protected by file permissions only, so full-disk encryption is recommended.
  Nothing is sent anywhere.

Remove
  pasteroll uninstall            (keeps your history)
  pasteroll uninstall --remove-data --yes     (also deletes it)

Command line
  pasteroll status | config show | config set KEY VALUE | exclude PROGRAM | autostart on|off

License
  MIT or Apache-2.0, at your option (LICENSE-MIT, LICENSE-APACHE).
  Source and issues: https://github.com/qpd-v/pasteroll
