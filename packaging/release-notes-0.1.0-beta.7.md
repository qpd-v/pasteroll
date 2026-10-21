# Pasteroll 0.1.0-beta.7 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

## New

- **An optional update check, off until you say yes.** Until now you had to notice a new version
  yourself. The first time you run this version, Pasteroll asks once whether it may check for
  updates (the default button is **Never check**; closing the window decides nothing and it asks
  again next time). If you say yes, or tick **Check for updates once a day** in Settings, then about
  once a day it makes one HTTPS request to GitHub for the list of releases and tells you if a newer
  version exists, with a notification and a tray-menu entry that open its release page. **It never
  downloads or installs anything**: you run the new Setup yourself, as before.
  - GitHub sees your IP address and the program name and version (plus the usual request headers),
    nothing else: no clipboard content, no history, no settings, no identifier.
  - The tray menu has **Check for updates now** and the command line has `pasteroll update-check`;
    those check only when you ask, whatever the setting. With the check off, Pasteroll makes no
    network request at all.
  - `config.toml` gains `[updates] check_for_updates = true | false`. `PRIVACY.md` and `SECURITY.md`
    describe exactly what is sent and how the reply is handled.

## Changed

- The first-run welcome text and the README no longer say "nothing is sent anywhere", because that
  is no longer literally true when you turn the update check on. Your clipboard is still never sent
  anywhere.
- Uninstalling with **delete data** now also removes the update check's small state file.

## Downloads

- pasteroll-0.1.0-beta.7-windows-x64-setup.exe
- pasteroll-0.1.0-beta.7-windows-x64-portable.zip
- SHA256SUMS covering these two packages

Upgrades preserve history, encryption keys and settings. To update from an earlier version, quit
Pasteroll from its tray icon and run the new Setup (Setup also asks a running copy to quit). Windows
uninstall keeps data unless deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen
shows a warning for new programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated: the Windows workspace tests (including tests that run the real Windows HTTPS code against
a local fake server: a newer release, up to date, errors, a redirect, an oversized reply, a proxy
asking for sign-in, nothing listening), Windows packaging, About and upgrade fixtures (upgrading from
the beta.6 package), and a real-window self-test (real hooks and clicks, including a real Windows
Terminal window) all passed. One real request to GitHub from `pasteroll update-check` was made. The
update check's code had an independent security review and its findings were fixed. The one-time
question and the Settings checkbox were looked at on a real screen.

NOT TESTED: a notification for a newer version coming from the real GitHub (there is none to find
until a newer release exists; the logic is covered by tests with a fake server), a clean-machine
all-users (UAC) install, start-at-sign-in across a real login, and the interactive uninstall
data-deletion prompt. Automated fixtures do not establish those results.

Known limits of the update check: Windows does not check certificate revocation here and nothing is
pinned, so someone who controls a trusted certificate authority on your network could read the
request or give a false answer; the worst that can do is a wrong "update available" message, because
nothing is downloaded or run. The check is Windows-only.

No Linux code was built or tested for this release. The shared code changed, so Linux behavior
should be treated as untested until its catch-up release.
