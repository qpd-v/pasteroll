# Pasteroll 0.1.0-beta.10 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

## In short

- Settings has a Check for updates now button and shows when Pasteroll last checked.
- The install window shows what is new in the version, and has a Skip this version button.
- If an update's installer fails, your previous Pasteroll is started again and tells you so.
- Which version is the newest is now signed with the Pasteroll release key, not read from GitHub's list.
- After an update, Pasteroll says which version you now have.

## New

- **Which version is the newest is now signed.** The update check no longer trusts GitHub's list of
  releases. It reads a short note from this project's repository (`updates/latest.txt`) that is signed
  with the same release key as the update itself, and ignores a genuine but older note served again.
  The note also carries a few plain lines about the new version, which the install window shows.
  Copies older than beta.10 keep using the old check, so the first copy that uses the note is this
  one, when it looks for the version after it.
- **Check for updates now** in Settings, with text saying how long ago the last check was.
- **Skip this version** in the install window: that version is not mentioned again. A newer version,
  or a check you ask for yourself, brings it back.
- **A failed installer no longer leaves nothing running.** If Setup fails or crashes during an update,
  the previous Pasteroll is started again and says that the update did not finish. After a successful
  update Pasteroll says which version you now have.

## Fixed

- More tests of the downloader (a server that trickles bytes, an endless chunked answer, a partial
  answer, a sign-in request, a compressed answer) and unused code removed. The website also gained a
  sitemap and search-engine details; that is not part of the program.

## Downloads

- pasteroll-0.1.0-beta.10-windows-x64-setup.exe
- pasteroll-0.1.0-beta.10-windows-x64-portable.zip
- pasteroll-0.1.0-beta.10-windows-x64.exe (the program file alone, used by portable updates)
- update-manifest.txt and update-manifest.txt.sig (used by the updater)
- SHA256SUMS covering the Setup, the ZIP and the program file

Upgrades preserve history, encryption keys and settings. From beta.9 the tray menu can install this
for you (**Download and install**); beta.7 and beta.8 must install it by hand (quit Pasteroll from its
tray icon and run the new Setup; Setup also asks a running copy to quit). Windows uninstall keeps data
unless deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen shows a warning for
new programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated, on this machine: the Windows workspace tests; Windows packaging; the About and upgrade
fixtures (upgrading from the beta.9 package); an end-to-end test of Download and install against a
local test server with a throwaway key (43 checks: a Setup install updates itself and is started again,
a portable copy replaces its own file, a wrong key, a changed installer, a changed manifest or note and
an older genuine note are all refused, and a Setup that fails or crashes leaves the previous Pasteroll
running); and the same for a copy installed for all users in Program Files with real Windows
administrator prompts. The release signatures were checked with the keys built into the program before
publishing. An independent review of the new code was done and its findings were fixed or are
documented in SECURITY.md.

The real-window self-test (real hooks and clicks, including a real Windows Terminal window) passed:
72 checks, 0 failed. Run again with the Settings window checks on, the four Settings checks passed and
the 9 overlay focus checks that are already known to fail in that mode failed again (the other
checks passed).

After publishing, the maintainer reported updating a real beta.9 copy to beta.10 from the real GitHub:
the update check found it, and Download and install downloaded and installed it. beta.9 uses the old
check, so this did not use the signed note.

NOT TESTED: an update that uses the signed note (the first will be beta.10 looking for a later
version), an update with a different
administrator's credentials, Windows Defender or Controlled Folder Access during an update, a
clean-machine all-users (UAC) install, start-at-sign-in across a real login, the interactive uninstall
data-deletion prompt, and a restart after a failed update for a Windows user name with unusual
characters (that case does not restart the old program).

Known limits of the signature scheme (all in SECURITY.md): whoever holds a signing key and its
passphrase can publish an update every copy will accept, and there is no key revocation. The note has
no expiry, so someone who controls the network can keep serving an old genuine note and a copy is then
never told about a newer version; they cannot make Pasteroll install anything unsigned. A copy built
from a stable release only hears about a stable release that is also the newest.

No Linux code was built or tested for this release. The shared code changed, so Linux behavior
should be treated as untested until its catch-up release. The update check and Download and install are
Windows-only.
