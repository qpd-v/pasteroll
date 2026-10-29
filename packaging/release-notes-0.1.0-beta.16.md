# Pasteroll 0.1.0-beta.16 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

## In short

- Pinned clips now show as small square tiles above the list: click one to paste, hover for a popup, right-click to colour it.
- The image editor can draw on a picture: pen, brush, circle and arrow in any colour.
- The image editor can write text on a picture, with a font picker and a colour for each part of the text.

## New

- **Pinned clips as tiles.** Pinned clips also show as small square tiles between the search bar and the
  list. A picture clip's tile is its picture, cropped to a square from the middle (not stretched);
  text, files, video and audio get a symbol. Click a tile to paste the clip. Rest the pointer on a tile
  for one second and a popup shows the whole clip (the text, cut at 1,500 characters, or the picture
  larger); it follows the pointer while it stays on the tile. Right-click a tile to paste, unpin, or
  colour the tile's edge and symbol. The colours are kept in a small plain-text file, `pin-colors.txt`,
  in the data folder (clip numbers and colour codes only); it is removed with your other data when you
  choose to delete it on uninstall.
- **Draw and write on pictures.** The image editor has a new **Draw and write** section: a pen, a
  soft brush, an empty circle and an arrow, a colour picker (a palette or a hex code), a size slider,
  Remove last and Clear all. Undo and redo cover every mark. The marks are saved into the pasted or
  saved picture and follow it through crop, rotate and resize; presets never store them.
- **Text on pictures.** The same section writes text on the picture: choose the Text tool, click where
  it goes and type (Enter starts a new line, Esc finishes). The font picker lists the common fonts that
  are installed. Click text to change it; select part of it with the mouse (or double-click a word),
  then pick a colour to recolour just that part.

## Downloads

- pasteroll-0.1.0-beta.16-windows-x64-setup.exe
- pasteroll-0.1.0-beta.16-windows-x64-portable.zip
- pasteroll-0.1.0-beta.16-windows-x64.exe (the program file alone, used by portable updates)
- update-manifest.txt and update-manifest.txt.sig (used by the updater)
- SHA256SUMS covering the Setup, the ZIP and the program file

Upgrades preserve history, encryption keys and settings. From beta.9 the tray menu can install this
for you (**Download and install**); beta.7 and beta.8 must install it by hand (quit Pasteroll from its
tray icon and run the new Setup; Setup also asks a running copy to quit). Windows uninstall keeps data
unless deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen shows a warning for
new programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated, on this machine: the Windows workspace tests; Windows packaging; privacy scans of all three
downloads; the real-window self-test (real hooks, clicks and clipboard), which was run because the
program's interface to Windows gained two calls for the pin colours file; and the upgrade fixture
(upgrading from the beta.15 package keeps the encrypted history, key, settings and presets). The release
signatures were checked with the keys built into the program before publishing. The pinned tiles, the
drawing tools and the text tool were also tried by hand on this machine in a build of this code.

The About check and the end-to-end test of Download and install were not run: neither the About window
nor the updater was touched.

Not tried: how the tiles and the popup look on other display scales, a very long text drawn on a very
large picture (it may be slow), and a very large number of pinned clips (about ten tiles fit; the rest
are not shown as tiles). Pinned tiles cannot be chosen for a multi-paste: a click pastes just that clip.

NOT TESTED: an update with a different administrator's credentials, Windows Defender or Controlled
Folder Access during an update, a clean-machine all-users (UAC) install, start-at-sign-in across a real
login, and the interactive uninstall data-deletion prompt.

No Linux package was built or released, and nothing was tried on a real Linux desktop. The shared code
changed (the Linux crates were not compiled for this release); treat Linux as untested until its
catch-up release. The update check and Download and install are Windows-only.
