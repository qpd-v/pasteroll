# Pasteroll 0.1.0-beta.18 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

## In short

- Rolls: separate collections of clips you roll through left and right, with LEFT and RIGHT built in on either side of Main.
- Drag the carousel by its empty margins; pin it open so it stays above other windows while you use them.
- Pins are now Favorites everywhere, and Ctrl+F makes a favorite (the star still works in the expanded viewer).
- Multi-paste and the Paste button work while the carousel is pinned open.
- Set which side of Main each roll sits on, and its order, in Settings.

## New

- **Rolls.** Separate collections of clips that you roll through left and right, with a slide between
  them. The main roll is your history; new copies always land there. Two rolls are built in, LEFT and
  RIGHT, on either side of Main. Move clips into a roll from the right-click menu (Move to roll lists
  the rolls in the order the carousel rolls in; New left roll... and New right roll... name a new one
  on that side). Settings makes, renames and removes your own rolls, places each one on the side of
  Main you want and orders them, and can add a Most used roll and an Images roll. Roll with
  Ctrl+Left / Ctrl+Right, by clicking the left or right part of the footer, or with the sideways
  clicks of a tilt wheel; Settings sets the keys and what each sideways click does. Clips in a custom
  roll are exempt from the limits, like favorites, and deleting a roll gives its clips back to Main.
  The footer shows the roll centered and flashes a black arrow while it rolls.
- **Move the carousel.** Press on an empty part of it (the margins and the gaps between things) and
  drag it anywhere on the screen. Clicking elsewhere still pastes there and closes it, as before.
- **Pin it open.** The pin button in the search bar (or Ctrl+K) keeps the carousel on top of the other
  windows while you use them. The keyboard and the wheel go back to the window you are using; the
  wheel over the carousel turns it. The bar's Paste button and the favorite tiles paste into the
  window that is in front, and the pin button closes it. Multi-paste works with the mouse alone.
  While it is pinned open there is no typing into the search box and no expanded viewer or image
  editor. The open gesture brings a fresh carousel to the pointer instead.
- **Pins are now Favorites.** The pin that keeps a clip until you remove it is called a favorite
  everywhere (the star, the Favorites filter, the tiles). It is Ctrl+F instead of Ctrl+P; inside the
  expanded viewer Ctrl+F still finds text, and the star button makes a clip a favorite there.

## Downloads

- pasteroll-0.1.0-beta.18-windows-x64-setup.exe
- pasteroll-0.1.0-beta.18-windows-x64-portable.zip
- pasteroll-0.1.0-beta.18-windows-x64.exe (the program file alone, used by portable updates)
- update-manifest.txt and update-manifest.txt.sig (used by the updater)
- SHA256SUMS covering the Setup, the ZIP and the program file

Upgrades preserve history, encryption keys and settings. The tray menu can install this for you
(**Download and install**); the older way is to quit Pasteroll from its tray icon and run the new
Setup (Setup also asks a running copy to quit). Windows uninstall keeps data unless deletion is
explicitly chosen. Windows builds are unsigned, so SmartScreen shows a warning for new programs:
check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated, on this machine: the Windows workspace tests (new tests cover rolls, side and order,
dragging, pinning open, favorites, and the centered footer indicator); Windows packaging; privacy
scans of all three downloads and the source; the About dialog check; and the upgrade fixture from the
beta.17 package, which keeps the encrypted history, key and settings. The real-window self-test passed
87 of 87 checks, run twice. Its Settings mode showed the same nine known foreground-focus failures in
the overlay checks that earlier releases show in that mode.

The self-update and all-users update fixtures were not re-run: this release does not touch the
updater, the installer or the elevated helper.

NOT TESTED: dragging between monitors with different display scaling, the tilt wheel on this machine,
pasting into elevated or terminal windows while pinned open, a clean-machine all-users (UAC) install,
start-at-sign-in across a real login, and the interactive uninstall data-deletion prompt.

No Linux code was built or tested for this release. The update check and Download and install are
Windows-only.
