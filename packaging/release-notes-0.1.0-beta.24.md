# Pasteroll 0.1.0-beta.24 (Windows and Linux)

Experimental prerelease for Windows, Linux X11 and GNOME 46 Wayland.

## In short

- The mouse wheel zooms the image preview.
- Selected images open in a tabbed Advanced Image Paste editor.
- Batch Paste Paths saves every edited image and pastes the paths.
- Text, files and images can be pasted together.
- Existing history, keys, settings and rolls are kept.

## Changes

- **Wheel zoom.** Over the image preview, wheel up zooms in and wheel down zooms out,
  centred under the pointer. Dragging still pans; Alt+drag pans while cropping or drawing.
- **Tabbed image batches.** Select several images, hold Ctrl and click a destination to open
  one tab per image. Click a tab or use Page Up/Down. Each tab keeps its own crop, resize,
  marks, format, name and undo history. The footer reports the tab and image size.
- **Batch output.** Paste Image saves the edited tabs and offers them as attachments with
  inline HTML. Paste Paths saves every edited image to the chosen folder and pastes the
  ordered paths; Paste Names pastes their file names. Saving on the desktop saves every tab.
- **Mixed Multi-paste.** Text, file-list and image clips can be selected together. The
  clipboard carries plain text, HTML and image files so chat composers, document editors
  and file-oriented targets can choose what they accept. Mixed selections are no longer
  silently reduced to text.
- **Compatibility note.** Receiving apps decide which clipboard format they accept. Live
  ChatGPT and document-editor acceptance remains a manual check, especially for mixed
  attachments and rich content.

## Downloads

- pasteroll-0.1.0-beta.24-windows-x64-setup.exe
- pasteroll-0.1.0-beta.24-windows-x64-portable.zip
- pasteroll-0.1.0-beta.24-linux-x86_64.tar.gz
- pasteroll_0.1.0-beta.24_amd64.deb
- pasteroll-0.1.0-beta.24-linux-x86_64-gnome-development.tar.gz
- pasteroll_0.1.0-beta.24_amd64-gnome-development.deb

The Windows updater also uses the bare executable and signed update manifest. Windows
builds are unsigned; update manifests carry the Pasteroll release-key signature.

## Verification and limitations

No automated or interactive test suites were run for this release, at the maintainer's
request. Compilation, package creation, checksums, signature verification and delivery
verification are performed. Live app compatibility and visual acceptance are left for
manual testing. Physical GNOME helper devices, login/lock/suspend/mixed-DPI behavior
remain unverified. Linux updates manually; GNOME requires GNOME 46 and protocol-4 companion.
