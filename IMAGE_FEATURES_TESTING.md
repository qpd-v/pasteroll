# Image and interaction feature checks

These checks open windows and use the mouse, keyboard and clipboard. Run when convenient.
The automated controller/rendering tests use fake platforms and do not need desktop control.

## Run against synthetic history

From the repository in PowerShell:

```powershell
cargo build -p pasteroll-win
.\target\debug\pasteroll.exe seed --db .checks\manual-image-features\history.db
& "$env:LOCALAPPDATA\Programs\Pasteroll\pasteroll.exe" stop
.\target\debug\pasteroll.exe run --db .checks\manual-image-features\history.db --no-capture --no-notice
```

Quit this test instance from its tray when finished, then launch your normal installed Pasteroll.
The test history is separate, and clipboard capture is disabled.

## Multiple images

1. Put the caret in the intended destination, then open Pasteroll.
2. Choose the Image filter. Enable Multi-paste and select both synthetic images.
3. Press Enter. Check that both images arrive, with the expected order, rather than just one.
4. Repeat by clicking the destination instead of pressing Enter.
5. Repeat in ChatGPT's web composer and another upload/chat app. Inspect the attached images;
   sending a message is unnecessary.
6. With images selected, Ctrl+E opens batch options. Select Inline images, then paste into a
   document editor. Verify two separate images, not a combined bitmap.
7. Ctrl+C should copy the batch without pasting. Paste manually after the carousel has closed.
8. Escape during preparation should cancel delivery. Mixed text/image selections should show
   an explicit message rather than silently omitting an image.

Windows/X11 offer file attachments and HTML together, prioritizing the selected representation.
GNOME Wayland offers the chosen representation. Native app/browser support varies.
Attachment PNGs are private transfer files in paste-cache, retained for delayed readers and manual
paste; see PRIVACY.md for retention.

## Advanced Image Paste

1. With Multi-paste off, Ctrl+click the current image to open the editor.
2. Try the zoom-out, zoom-in, Fit and Actual size icons, then Ctrl+wheel over the image.
   The percentage is a read-only display, not another button.
3. Drag the normal preview to pan. In Crop/Draw modes, Alt+drag pans instead of editing.
4. Crop, draw and place text while enlarged. Confirm that marks follow the pointer and crop handles.
5. Paste/save the result: zooming must not resize the exported image.
6. Repeat on another display scale if available.

## Double-click and selection

1. Open a short text clip over a text editor.
2. Double-click its header, then repeat with blank space beneath the content. The clip should paste.
3. Double-click a word: it should select the word without pasting.
4. Keep the second click held and drag across words and lines, in both directions.
5. Verify ordinary text dragging, buttons, scroll areas, and Multi-paste selection still work.

## Automated checks

```powershell
cargo test -p pasteroll-app -p pasteroll-render --lib
cargo test -p pasteroll-win clipboard::batch_tests
```

On Linux: `cargo test -p pasteroll-linux --bin pasteroll`.

## Selection entry behavior

- Ctrl+click an inactive clip: Multi-paste starts with the active and clicked clips selected.
- Click the Multi-paste button: selection starts empty. The active clip has no selection border or tint.
- Select and deselect clips: only selected clips have the selection highlight.

## Beta.24 batch editor and mixed Multi-paste

- Select two or more images, hold Ctrl and click a target window: the editor opens with one tab per image.
- Click tabs or use Page Up/Page Down. Edit and zoom each image independently.
- Choose Paste Paths to save every edited image into the chosen folder and paste all paths.
- Select images and text/file clips together, then Enter or Ctrl+click: the clipboard carries text, HTML and image files.
- Over the image preview, wheel up zooms in and wheel down zooms out. Drag to pan; Alt+drag pans while cropping/drawing.
