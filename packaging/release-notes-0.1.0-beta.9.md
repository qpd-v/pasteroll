# Pasteroll 0.1.0-beta.9 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

## New

- **Download and install updates.** When the optional update check finds a newer version, the tray
  menu offers **Download and install...** (the notification offers it too). Nothing is downloaded until
  you click it and confirm in a window that defaults to Cancel. Pasteroll then downloads the update from
  GitHub, checks that it is signed with the Pasteroll release key and matches its checksum (a file that
  fails is deleted and never run), installs it, and starts the new version. Your history, keys and
  settings are kept.
  - A copy installed with Setup runs the new Setup silently.
  - A copy installed **for all users** shows Windows' administrator prompt. Its update is downloaded,
    checked and run from a folder only administrators can open, so nothing you could change is trusted
    by the elevated part.
  - A **portable** copy replaces its own program file and keeps the old one as `pasteroll.exe.old`
    until the next start.
  - `pasteroll update` shows what is available; `pasteroll update --yes` installs it without the window.
- **Signed releases.** Each release now carries `update-manifest.txt` and `update-manifest.txt.sig`
  (an ECDSA P-256 signature made with a key kept offline), and a bare
  `pasteroll-0.1.0-beta.9-windows-x64.exe` that portable copies update from. There is no code-signing
  certificate: the key fingerprints (`4c78c3487cccaa16` and backup `cf5f92f51d2512ed`) are in
  [SECURITY.md](https://github.com/qpd-v/pasteroll/blob/main/SECURITY.md), which also lists exactly
  what this does and does not protect against.
- The Settings checkbox now reads "Check for updates once a day (asks GitHub; you choose whether to
  install)". The update check is still off until you say yes.

## Downloads

- pasteroll-0.1.0-beta.9-windows-x64-setup.exe
- pasteroll-0.1.0-beta.9-windows-x64-portable.zip
- pasteroll-0.1.0-beta.9-windows-x64.exe (the program file alone, used by portable updates)
- update-manifest.txt and update-manifest.txt.sig (used by the updater)
- SHA256SUMS covering the Setup, the ZIP and the program file

Upgrades preserve history, encryption keys and settings. **To get beta.9, install it by hand** (quit
Pasteroll from its tray icon and run the new Setup; Setup also asks a running copy to quit): beta.7 and
beta.8 cannot download updates. From beta.9 on, the tray menu can do this for you. Windows uninstall keeps
data unless deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen shows a warning for
new programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated, on this machine: the Windows workspace tests; Windows packaging; the About and upgrade
fixtures (upgrading from the beta.8 package); an end-to-end test of Download and install against a local
test server with a throwaway key (a Setup install updates itself and is started again, a portable copy
replaces its own file, and a wrong key, a changed installer and a changed manifest are all refused); and
the same for a copy installed for all users in Program Files, with real Windows administrator prompts
(install, update through the administrator helper, relaunch without administrator rights, uninstall,
nothing left behind). The release signature was checked with the keys built into the program before
publishing. An independent review of the updater and of the administrator helper was done and its
findings were fixed or are documented in SECURITY.md.

The real-window self-test (real hooks and clicks, including a real Windows Terminal window) passed: 72 checks,
0 failed. The new confirmation window was also looked at on screen.

NOT TESTED: an update downloaded from the real GitHub (the first real one will be beta.9 to a later
version; the redirect to GitHub's file host was checked against a published release, but not an
install from it), an update with a different administrator's credentials, Windows Defender or
Controlled Folder Access during an update, a clean-machine all-users (UAC) install, start-at-sign-in
across a real login, and the interactive uninstall data-deletion prompt.

Known limits of the signature scheme (all in SECURITY.md): whoever holds a signing key and its
passphrase can publish an update every copy will accept, and there is no key revocation; the list of
releases comes from GitHub and is not signed, so someone who controls it (or the network) can announce
an older genuine release or hide a newer one, but cannot make Pasteroll install anything unsigned.

No Linux code was built or tested for this release. The shared code changed, so Linux behavior
should be treated as untested until its catch-up release. The update check and Download and install are
Windows-only.
