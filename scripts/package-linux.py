#!/usr/bin/env python3
"""Stage Linux assets and build a static archive and .deb using standard local tools."""
import json
import argparse
import gzip
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent.parent
SIZES = (16, 24, 32, 48, 64, 128, 256, 512)
DOCS = ("LICENSE-MIT", "LICENSE-APACHE", "THIRD-PARTY-NOTICES.md")


def run(*args, **kwargs):
    return subprocess.run(args, cwd=ROOT, check=True, **kwargs)


def copy(source, destination, mode=0o644):
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(mode)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--with-gnome',action='store_true')
    parser.add_argument('--app-bin',type=Path)
    parser.add_argument('--helper-bin',type=Path)
    parser.add_argument('--output',type=Path)
    args=parser.parse_args()
    os.umask(0o022)
    metadata = json.loads(run("cargo", "metadata", "--format-version", "1", "--no-deps", "--locked", "--offline", capture_output=True).stdout)
    package = next(p for p in metadata["packages"] if p["name"] == "pasteroll-linux")
    version = package["version"]
    deb_version = version.replace("-", "~", 1)
    # Both values come from Cargo, never the build machine's login or hostname.
    publisher, homepage = package["authors"][0], package["homepage"]
    executable = args.app_bin or ROOT / "target" / "x86_64-unknown-linux-musl" / "release" / "pasteroll"
    helper=args.helper_bin
    if args.with_gnome and (helper is None or not helper.is_file()):
        raise SystemExit('The separately built glibc host helper is required; use scripts/package-gnome.sh.')
    # Fail before packaging if musl accidentally becomes a dynamic build.
    headers = run("readelf", "-lW", str(executable), capture_output=True, text=True).stdout
    dynamic = run("readelf", "-dW", str(executable), capture_output=True, text=True).stdout
    if "INTERP" in headers or "(NEEDED)" in dynamic:
        raise SystemExit("The Linux release executable is not fully static.")
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    if epoch is None:
        epoch = run("git", "log", "-1", "--format=%ct", capture_output=True, text=True).stdout.strip()
    epoch = int(epoch)
    dist = args.output or ROOT / "dist"
    dist.mkdir(parents=True,exist_ok=True)
    flavor='-gnome-development' if args.with_gnome else ''
    with tempfile.TemporaryDirectory(prefix="pasteroll-package-") as scratch:
        temp = Path(scratch)
        archive_root = temp / f"pasteroll-{version}-linux-x86_64{flavor}"
        copy(executable, archive_root / "pasteroll", 0o755)
        copy(ROOT / "packaging/linux/install.sh", archive_root / "install.sh", 0o755)
        # Git may check this text out as CRLF on Windows; the shell ownership
        # comparison requires a platform-independent marker in Linux archives.
        (archive_root / "install-source.txt").write_text(
            (ROOT / "packaging/linux/install-source.txt").read_text(), newline="\n")
        copy(ROOT / "packaging/README-linux.txt", archive_root / "README.txt")
        desktop = (ROOT / "packaging/linux/pasteroll.desktop").read_text()
        (archive_root / "pasteroll.desktop").write_text(desktop.replace("@BINDIR@", "/usr/local/bin"))
        for name in DOCS:
            copy(ROOT / name, archive_root / name)
        for size in SIZES:
            copy(ROOT / f"crates/pasteroll-core/assets/logo-{size}.png", archive_root / f"icons/{size}x{size}/apps/pasteroll.png")
        if args.with_gnome:
            (archive_root/'README.txt').write_text((ROOT/'packaging/README-linux-gnome.txt').read_text().replace('@VERSION@',version))
            copy(helper,archive_root/'pasteroll-input',0o755)
            for name in ('install-input-helper.sh','install-gnome-companion.sh','com.pasteroll.Input.conf'):
                copy(ROOT/'packaging/linux'/name,archive_root/name,0o755 if name.endswith('.sh') else 0o644)
            unit=(ROOT/'packaging/linux/pasteroll-input.service').read_text().replace('@LIBEXECDIR@','/usr/local/libexec')
            (archive_root/'pasteroll-input.service').write_text(unit)
            for file in sorted((ROOT/'extensions/pasteroll@pasteroll.com').iterdir()):
                if file.suffix in ('.js','.json','.css'):copy(file,archive_root/'gnome-companion'/file.name)
            copy(ROOT/'extensions/INPUT_HELPER.md',archive_root/'GNOME-README.md')
            (archive_root/'pasteroll-input-modules.conf').write_text('uinput\n')
        tar_path = dist / f"pasteroll-{version}-linux-x86_64{flavor}.tar.gz"
        # The archive never carries source paths, uid/gid names or machine timestamps.
        with tar_path.open("wb") as output, gzip.GzipFile(fileobj=output, mode="wb", filename="", mtime=epoch) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
                for path in [archive_root, *sorted(archive_root.rglob("*"))]:
                    info = archive.gettarinfo(str(path), arcname=str(path.relative_to(temp)))
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mtime = epoch
                    if path.is_file():
                        with path.open("rb") as content:
                            archive.addfile(info, content)
                    else:
                        archive.addfile(info)

        deb = temp / "deb"
        copy(executable, deb / "usr/bin/pasteroll", 0o755)
        app = deb / "usr/share/applications/pasteroll.desktop"
        app.parent.mkdir(parents=True, exist_ok=True)
        app.write_text(desktop.replace("@BINDIR@", "/usr/bin"))
        for size in SIZES:
            copy(ROOT / f"crates/pasteroll-core/assets/logo-{size}.png", deb / f"usr/share/icons/hicolor/{size}x{size}/apps/pasteroll.png")
        for name in DOCS:
            copy(ROOT / name, deb / "usr/share/doc/pasteroll" / name)
        copy(ROOT / "packaging/README-linux.txt", deb / "usr/share/doc/pasteroll/README.txt")
        if args.with_gnome:
            (deb/'usr/share/doc/pasteroll/README.txt').write_text((ROOT/'packaging/README-linux-gnome.txt').read_text().replace('@VERSION@',version))
            copy(helper,deb/'usr/libexec/pasteroll-input',0o755)
            copy(ROOT/'packaging/linux/com.pasteroll.Input.conf',deb/'usr/share/dbus-1/system.d/com.pasteroll.Input.conf')
            unit_path=deb/'usr/lib/systemd/system/pasteroll-input.service'
            unit_path.parent.mkdir(parents=True)
            unit_path.write_text((ROOT/'packaging/linux/pasteroll-input.service').read_text().replace('@LIBEXECDIR@','/usr/libexec'))
            modules=deb/'usr/lib/modules-load.d/pasteroll-input.conf'
            modules.parent.mkdir(parents=True)
            modules.write_text('uinput\n')
            for file in sorted((ROOT/'extensions/pasteroll@pasteroll.com').iterdir()):
                if file.suffix in ('.js','.json','.css'):copy(file,deb/'usr/share/gnome-shell/extensions/pasteroll@pasteroll.com'/file.name)
            copy(ROOT/'extensions/INPUT_HELPER.md',deb/'usr/share/doc/pasteroll/GNOME-README.md')
        installed_kib = sum((p.stat().st_size + 1023) // 1024 for p in deb.rglob("*") if p.is_file())
        control = deb / "DEBIAN/control"
        control.parent.mkdir()
        depends='Depends: libc6 (>= 2.34), libgcc-s1, libevdev2, gnome-shell (>= 46), gnome-shell (<< 47), systemd\n' if args.with_gnome else ''
        description=("Description: Local encrypted clipboard history for X11 and GNOME 46 Wayland\n"
            " GNOME mouse activation uses an optional administrator-enabled host helper.\n"
            " Development package; physical desktop validation is still required.\n") if args.with_gnome else (
            "Description: Local encrypted clipboard history for X11\n"
            " Scroll through clipboard history, search, select and paste text, images or files.\n"
            " Requires an X11 session; native Wayland windows are not supported.\n"
            " Static executable. No telemetry, update checks or network services.\n")
        control.write_text(
            f"Package: pasteroll\nVersion: {deb_version}\nArchitecture: amd64\n"
            f"Maintainer: {publisher}\nSection: utils\nPriority: optional\n"
            f"Installed-Size: {installed_kib}\nHomepage: {homepage}\n"
            f"{depends}"
            "Recommends: tesseract-ocr, zenity\n"
            f"{description}"
        )
        if args.with_gnome:
            for name in ('postinst','prerm','postrm'):
                copy(ROOT/'packaging/linux'/name,deb/'DEBIAN'/name,0o755)
        deb_path = dist / f"pasteroll_{version}_amd64{flavor}.deb"
        env = dict(os.environ, SOURCE_DATE_EPOCH=str(epoch))
        run("dpkg-deb", "--root-owner-group", "-Zxz", "-z9", "--build", str(deb), str(deb_path), env=env, stdout=subprocess.DEVNULL)
    print(f"Built {tar_path.name}")
    print(f"Built {deb_path.name} (Debian version {deb_version})")


if __name__ == "__main__":
    main()
