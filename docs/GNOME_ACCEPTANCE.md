# GNOME 46 candidate acceptance

Use a spare Linux desktop with synthetic clipboard data. The experimental beta.4
prerelease has automated headless and synthetic input coverage. Mixed-scale
viewer/editor checks passed on one Pop!_OS setup; further manual acceptance is
deferred for beta.4 and the unchecked items below remain outstanding. Keep a terminal available to stop the optional input helper.

## Identify the desktop

Run `gnome-shell --version` and `echo "$XDG_SESSION_TYPE"` in a desktop terminal.
This candidate requires **GNOME 46 and Wayland**. Record distribution, version,
mouse model, scaling and monitor layout. For other GNOME versions, stop rather
than editing extension metadata to bypass the compatibility check.

## Install and enable

Verify the package against SHA256SUMS. On Debian/Ubuntu:

```sh
sudo apt install ./pasteroll_0.1.0-beta.4_amd64-gnome-development.deb
systemctl is-enabled pasteroll-input.service
```

Fresh installation should leave the helper disabled. The regular X11 and GNOME
Debian packages are alternatives, not coinstallable. Other distributions should
follow the GNOME archive's included README.

Log out/in, then:

```sh
gnome-extensions enable pasteroll@pasteroll.com
sudo systemctl enable --now pasteroll-input.service
pasteroll run --backend gnome
```

Never run the app as root. Do not run another remapper on the same mouse.
For a keyboard comparison, stop the app and start
`pasteroll run --backend gnome --wayland-keyboard` (Ctrl+Shift+Space).

## Physical checks

- Copy synthetic text, images and files from native Wayland applications. Open
  with Ctrl+Shift+middle-click; scroll, search, select and paste to the original
  window. Test GTK, Qt, browser and terminal targets, including multiline terminal
  confirmation, viewer search, image editing and save.
- Check normal middle-click, buttons, motion, wheel and dragging with the app
  idle, showing, closed and stopped. Release modifiers before middle-click;
  test left/right modifier keys. No stuck buttons or unexpected clicks.
- Unplug/replug the mouse, restart the helper and disable/re-enable the companion.
  Confirm cancellation, normal input and recovery.
- Lock/unlock, suspend/resume and logout/login. Test autostart enabled/disabled.
  No activation or clipboard capture should occur on the lock screen.
- Check every monitor/scaling configuration intended for support. A pass on one
  monitor at scale 1 does not cover mixed scaling or other layouts.
- Test normal dependency resolution, upgrade and removal using a disposable
  profile. Confirm history/settings survive and the helper does not remain active.

## Rollback

```sh
pasteroll stop
sudo systemctl disable --now pasteroll-input.service
gnome-extensions disable pasteroll@pasteroll.com
sudo apt remove pasteroll
```

Confirm ordinary mouse behavior immediately after stopping the helper. Archive
installs use their included uninstall scripts. Do not delete existing user data.

Record PASS, FAIL or NOT TESTED for each item, candidate hashes and reproducible
failure steps. Beta.4 was approved as experimental without completing physical acceptance.
Complete these checks before broadening support claims.
