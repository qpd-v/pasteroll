"""The release asset allowlist and strict checksum handling. No publication actions."""
import hashlib
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parent.parent
CRATES = ("app", "core", "render", "win", "linux")


def release_version():
    versions = []
    for name in CRATES:
        manifest = ROOT / "crates" / f"pasteroll-{name}" / "Cargo.toml"
        with manifest.open("rb") as source:
            package = tomllib.load(source)
        versions.append(package["package"]["version"])
        for dependency in package.get("dependencies", {}).values():
            if isinstance(dependency, dict) and "path" in dependency:
                versions.append(dependency["version"])
    if len(set(versions)) != 1 or not re.fullmatch(r"\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?", versions[0]):
        raise ValueError("Workspace and internal dependency versions must agree.")
    return versions[0]


def artifact_names(version, *, with_gnome=False, windows_only=False):
    if windows_only and with_gnome:
        raise ValueError("Windows-only releases cannot include GNOME packages.")
    names = (f"pasteroll-{version}-windows-x64-setup.exe",
            f"pasteroll-{version}-windows-x64-portable.zip",
            f"pasteroll-{version}-linux-x86_64.tar.gz",
            f"pasteroll_{version}_amd64.deb")
    if windows_only:
        return names[:2]
    if with_gnome:
        names += (f"pasteroll-{version}-linux-x86_64-gnome-development.tar.gz",
                  f"pasteroll_{version}_amd64-gnome-development.deb")
    return names


def digest(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError("An expected release artifact is missing or is a symbolic link.")
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def checksum_text(directory, names):
    return "".join(f"{digest(directory / name)}  {name}\n" for name in names)


def write_checksums(directory, names):
    text = checksum_text(directory, names)  # never replace a good manifest with a partial list
    temporary = directory / "SHA256SUMS.tmp"
    temporary.write_text(text, encoding="ascii", newline="\n")
    temporary.replace(directory / "SHA256SUMS")
    return text


def verify_checksums(directory, names):
    actual = (directory / "SHA256SUMS").read_text(encoding="ascii")
    if actual != checksum_text(directory, names):
        raise ValueError("SHA256SUMS is stale, incomplete, or contains unexpected assets.")
