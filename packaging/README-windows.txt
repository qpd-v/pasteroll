Pasteroll for Windows - 0.1.0-beta.2
====================================

Pasteroll keeps a history of what you copy and lets you paste it again: scroll through
your clips with the mouse wheel and paste the whole clip, or select part of it and paste
just that part.

Install
  Run pasteroll-0.1.0-beta.2-windows-x64-setup.exe.
  Choose Only for me (default, no administrator rights), or For all users (UAC).
  The Start menu shortcut is on by default; desktop shortcut and start at sign-in are off.
  Each account always has its own history and settings, even with an all-users install.
  Silent: /VERYSILENT /CURRENTUSER or /VERYSILENT /ALLUSERS (the latter needs elevation).
  No components are downloaded. The beta is unsigned, so Windows SmartScreen may warn.

Use
  Ctrl+Shift+middle-click      open the carousel at the pointer
  mouse wheel / Up / Down      move through clips
  type                         search (file names too)
  Tab                          choose a filter (Text, Image, Document...)
  Enter                        paste the clip; drag or double-click first to paste only that part
  Ctrl+P                       pin a clip      Delete   recycle a clip      Esc  close
  Tray icon                    pause, settings, quit

Your data
  Stored encrypted in %LOCALAPPDATA%\Pasteroll for your Windows account only.
  Nothing is sent anywhere.

Portable mode
  Extract pasteroll-0.1.0-beta.2-windows-x64-portable.zip and run pasteroll.exe.
  The included portable.txt keeps all app data in the adjacent data folder.
  Installation, autostart changes and --db are disabled. Quit, then delete the folder to remove it.
  DPAPI still binds the key to your Windows account: copying the folder to another PC/account
  cannot open its history there. Use key-export / key-import to transfer it securely.
  pasteroll.exe stop asks the current session's running copy to quit gracefully.
  A Setup-owned copy is upgraded using Setup and removed through Settings > Apps.

Remove
  Setup installation: Settings > Apps > Pasteroll. History and settings are kept by default.
  Interactive uninstall asks before deleting data for the account running the uninstaller.
  Silent uninstall always keeps data. Other accounts' data is never deleted.
  If UAC uses another administrator account, cleanup applies to that administrator's data.
  Before uninstalling, you can remove your own data from your normal account with:
    pasteroll.exe stop
    pasteroll.exe uninstall --data-only --remove-data --yes
  Older self-installs still use: pasteroll.exe uninstall [--remove-data --yes].

Command line
  pasteroll.exe status | config show | config set KEY VALUE | exclude PROGRAM.exe |
                stop | autostart on|off | key-export --out FILE | key-import --in FILE

License
  MIT or Apache-2.0, at your option (LICENSE-MIT, LICENSE-APACHE).
  Source and issues: https://github.com/qpd-v/pasteroll
