# Pasteroll 0.1.0-beta.22 (Windows and Linux)

Experimental prerelease for Windows, Linux X11 and GNOME 46 Wayland.

## In short

- Multi-paste can paste several selected images as attachments or inline images.
- Advanced Image Paste has zoom controls, Ctrl+wheel zoom and panning.
- Double-click clip headers or blank content space to paste; double-click and drag text to select words.
- Ctrl+S saves the clip shown; change or disable the shortcut in Settings.
- Existing history, encryption keys, settings and rolls are kept.

## Changes

- **Image Multi-paste.** Enable Multi-paste and select image clips, then press Enter or click a destination.
  Ctrl+C copies the batch. Ctrl+E opens Attachments / Inline images and order options.
  Images remain separate, in selection order. Batches support up to 32 images and 24 MB.
  Mixed image/text selections show an explicit message instead of silently dropping pictures.
- **Clipboard formats.** Windows and X11 offer image files and HTML together, prioritizing the selected
  representation. GNOME Wayland offers the selected representation. The receiving app decides which
  formats it accepts; multiple-image support in ChatGPT, other web composers and document editors
  remains subject to manual verification.
- **Zoom in Advanced Image Paste.** Minus, plus, Fit, 100% and a percentage display accompany
  pointer-centered Ctrl+wheel zoom. Drag the normal preview to pan; Alt+drag pans while cropping or
  drawing. The wheel pans over the preview and scrolls over settings. Zoom does not resize exported images.
- **Double-click paste and word dragging.** Double-click the current clip header or unused area beneath
  its content to paste. Double-click actual text still selects a word; hold and drag to extend the
  selection by words in either direction.
- **Save a clip as a file.** Ctrl+S works in the carousel and expanded viewer. Text saves as TXT,
  file lists as one path per line, and pictures as re-encoded PNGs. Settings can change or disable
  the shortcut. A single clip's right-click menu also offers Save as.
- **Private transfer files.** Image batches create ordinary PNG attachment files in the private
  paste-cache folder beside history. They survive session close for delayed readers and manual
  paste. Batches older than seven days are removed when another batch is prepared. These transfer
  files are not encrypted database records; see PRIVACY.md.

## Downloads

- pasteroll-0.1.0-beta.22-windows-x64-setup.exe
- pasteroll-0.1.0-beta.22-windows-x64-portable.zip
- pasteroll-0.1.0-beta.22-linux-x86_64.tar.gz
- pasteroll_0.1.0-beta.22_amd64.deb
- pasteroll-0.1.0-beta.22-linux-x86_64-gnome-development.tar.gz
- pasteroll_0.1.0-beta.22_amd64-gnome-development.deb

The Windows updater also uses the bare executable and signed update manifest.
SHA256SUMS covers all seven program assets. Windows executables are not Authenticode-signed;
the updater manifests carry the Pasteroll release-key signature.

## Testing and limitations

Automated controller, rendering, clipboard-format and Linux tests passed before packaging.
Mouse/keyboard takeover and live ChatGPT/document-editor tests were deferred at the maintainer's
request. Physical GNOME helper devices, login/lock screens, suspend and mixed-DPI desktop behavior,
and clean-machine Windows UAC/startup/uninstall acceptance remain NOT TESTED.
See IMAGE_FEATURES_TESTING.md for manual checks. Report bugs on GitHub.

Linux packages update manually. GNOME requires GNOME 46 and the matching protocol-4 companion;
reload the companion after upgrading. The X11 and GNOME Debian packages are alternatives.
