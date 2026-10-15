Pasteroll for Windows - 0.1.0-beta.1
====================================

Pasteroll keeps a history of what you copy and lets you paste it again: scroll through
your clips with the mouse wheel and paste the whole clip, or select part of it and paste
just that part.

Install
  Double-click pasteroll.exe and choose Yes, or run:   pasteroll.exe install
  It installs for your account only (no administrator rights), adds a Start menu
  entry, and does not start with Windows unless you turn that on in Settings.

Use
  Ctrl+Shift+middle-click      open the carousel at the pointer
  mouse wheel / Up / Down      move through clips
  type                         search (file names too)
  Tab                          choose a filter (Text, Image, Document...)
  Enter                        paste the clip; drag or double-click first to paste only that part
  Ctrl+P                       pin a clip      Ctrl+Delete   delete a clip      Esc  close
  Tray icon                    pause, settings, quit

Your data
  Stored encrypted in %LOCALAPPDATA%\Pasteroll for your Windows account only.
  Nothing is sent anywhere.

Remove
  Settings > Apps > Pasteroll, or:   pasteroll.exe uninstall
  Add --remove-data --yes to also delete your history and settings.

Command line
  pasteroll.exe status | config show | config set KEY VALUE | exclude PROGRAM.exe |
                autostart on|off | key-export --out FILE | key-import --in FILE

License
  MIT or Apache-2.0, at your option (LICENSE-MIT, LICENSE-APACHE).
  Source and issues: https://github.com/qpd-v/pasteroll
