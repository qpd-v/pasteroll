# Pasteroll 0.1.0-beta.8 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

This is a small fix for [0.1.0-beta.7](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.7),
which added the optional update check (off until you say yes). Nothing else changed.

## Fixed

- **Settings now shows your answer to the update question.** If the one-time question ("Let
  Pasteroll check for updates?") appeared while the Settings window was open and you chose
  **Check for updates**, the "Check for updates once a day" box in Settings stayed empty even though
  your choice was saved. The open Settings window now follows a choice made elsewhere within a
  second, and Saving no longer works against it. The question also now waits until Settings is
  closed instead of appearing on top of it.

## Downloads

- pasteroll-0.1.0-beta.8-windows-x64-setup.exe
- pasteroll-0.1.0-beta.8-windows-x64-portable.zip
- SHA256SUMS covering these two packages

Upgrades preserve history, encryption keys and settings. To update from an earlier version, quit
Pasteroll from its tray icon and run the new Setup (Setup also asks a running copy to quit). Windows
uninstall keeps data unless deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen
shows a warning for new programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated: the Windows workspace tests, Windows packaging, About and upgrade fixtures (upgrading from
the beta.7 package), and a real-window self-test (real hooks and clicks, including a real Windows
Terminal window) all passed. The self-test also includes a check that the Settings box follows a
choice written to `config.toml` while the window is open.

NOT TESTED: a notification for a newer version coming from the real GitHub (there is none to find
until a newer release exists; the logic is covered by tests with a fake server), a clean-machine
all-users (UAC) install, start-at-sign-in across a real login, and the interactive uninstall
data-deletion prompt. Automated fixtures do not establish those results.

The update check's known limits are as in beta.7: Windows does not check certificate revocation here
and nothing is pinned, so the worst a hostile network can do is show a wrong "update available"
message, because nothing is downloaded or run. The check is Windows-only.

No Linux code was built or tested for this release. The shared code changed, so Linux behavior
should be treated as untested until its catch-up release.
