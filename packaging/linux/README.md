# Linux packages

Build in Linux or Ubuntu/WSL from the repository root:

```sh
bash scripts/package-linux.sh
python3 scripts/test-linux-packaging.py
```

Prerequisites are a recent Rust toolchain with `x86_64-unknown-linux-musl` already installed,
`musl-tools`, Python 3 (3.11+ for the package tests), binutils, and `dpkg-deb`. The scripts never
install tools or download dependencies. Cargo builds with `--locked --offline`. The generated files
are `dist/pasteroll-0.1.0-beta.2-linux-x86_64.tar.gz` and `dist/pasteroll_0.1.0-beta.2_amd64.deb`.

The build uses `dpkg-deb` because it is a standard Debian/Ubuntu tool and needs no added Cargo
dependency. It verifies the executable has no ELF interpreter or shared-library dependencies.
Build paths are remapped, archive owner names are omitted, and the Debian payload is root-owned.
`SOURCE_DATE_EPOCH` defaults to the latest local commit's timestamp; archives omit machine times.

## Archive

```sh
tar xzf pasteroll-0.1.0-beta.2-linux-x86_64.tar.gz
cd pasteroll-0.1.0-beta.2-linux-x86_64
./install.sh                      # per-user install using the app's existing installer
sudo ./install.sh --system        # alternative: all users, under /usr/local
```

The script announces its actions before changing files. The default install respects
`XDG_DATA_HOME` and `XDG_CONFIG_HOME` and preserves existing autostart preferences. Fresh installs
do not enable autostart. It requests a graceful stop before per-user installation/removal; an older
running version without the stop pipe must be quit from the tray before retrying.

The system install places the executable in `/usr/local/bin`, a desktop entry under
`/usr/local/share/applications`, eight PNG sizes under `/usr/local/share/icons/hicolor`, and the
README/licenses under `/usr/local/share/doc/pasteroll`. It records archive ownership under
`/usr/local/share/pasteroll/install-source.txt` and refuses to overwrite an unmarked installation.
Updates replace files atomically. Quit/restart copies in other sessions to use an updated binary.
No root-owned history is created. Every account keeps its own data and optional autostart setting.

```sh
./install.sh --uninstall
sudo ./install.sh --system --uninstall
```

Both modes support repeated installation and removal. Removal always keeps history/settings and
unrelated files. Keep the extracted installer, or extract the archive again when removing it.
The app's own `install` and `uninstall` commands only manage per-user installation;
they do not remove a system archive or Debian installation. Disable each account's optional
autostart before removing a system-wide installation to avoid a leftover per-user startup entry.

For portable use, create `portable.txt` beside the extracted executable, then run `./pasteroll run`.
All application data stays in the adjacent `data` directory. Remove that marker before using the
archive installer. To remove a portable copy, quit it and remove its folder.

## Debian / Ubuntu

```sh
sudo dpkg -i pasteroll_0.1.0-beta.2_amd64.deb
sudo dpkg -r pasteroll            # keeps all accounts' data
```

The `.deb` uses `/usr/bin/pasteroll`, `/usr/share/applications`, hicolor icons and licenses under
`/usr/share/doc/pasteroll`. It contains no maintainer scripts, services or autostart entries, and
has no `Depends` field. Both removal and purge keep all user data. `Recommends: tesseract-ocr, zenity`
describes optional local OCR and dialogs. `dpkg -i` makes no downloads; if you choose a different
package-manager frontend, its handling of recommendations follows that frontend's settings.

Debian metadata uses `0.1.0~beta.2`: Debian's `~` orders the beta before the stable `0.1.0`. The
download filename retains the requested Cargo version. The maintainer field contains the project
brand, with no private identity or invented contact address.

AppImage is omitted: the static archive already covers other x86-64 Linux distributions, and an
AppImage would add another build tool and runtime to verify.

## Verification boundaries

Package tests use scratch homes and roots, never the live clipboard or history. They exercise
per-user and staged system installs twice, autostart preservation, portable paths, refusal to
overwrite unknown system files, archive metadata and icons, and real `dpkg` install/upgrade/remove/
purge against a private package database/root. `DESTDIR=/absolute/scratch ./install.sh --system`
is available for filesystem staging; desktop entries still point to the final `/usr/local/bin`.

These checks do not replace a clean Ubuntu X11 desktop test, real system installation with sudo,
tray/menu integration, optional OCR/dialog tools, or Phase 4 personal-string scans and checksums.
