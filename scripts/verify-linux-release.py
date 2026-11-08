#!/usr/bin/env python3
"""Checksum and scan both Linux package variants without publishing anything."""
import argparse
import os
from pathlib import Path
import subprocess
import sys
from release_artifacts import ROOT, release_version, write_checksums, verify_checksums

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, default=ROOT / 'dist/linux')
    parser.add_argument('--generic-only', action='store_true')
    args = parser.parse_args()
    version = release_version()
    names = [f'pasteroll-{version}-linux-x86_64.tar.gz', f'pasteroll_{version}_amd64.deb',
             f'pasteroll-{version}-linux-x86_64-gnome-development.tar.gz',
             f'pasteroll_{version}_amd64-gnome-development.deb']
    env = dict(os.environ)
    if not args.generic_only and not env.get('PASTEROLL_SCAN_WORDS'):
        # Build-machine identity stays in the subprocess environment and is never printed.
        import getpass
        # Short account names collide with random bytes in compressed archives; the
        # home and workspace paths cover the account name in its meaningful context.
        terms = [str(Path.home()), str(ROOT)]
        user = getpass.getuser()
        if len(user) >= 8:
            terms.insert(0, user)
        env['PASTEROLL_SCAN_WORDS'] = ','.join(terms)
    for name in names:
        command = [sys.executable, str(ROOT / 'scripts/scan_binary.py'), str(args.directory / name)]
        if not args.generic_only:
            command.append('--require-personal-words')
        subprocess.run(command, env=env, check=True)
    write_checksums(args.directory, names)
    verify_checksums(args.directory, names)
    print('PASS: four Linux artifacts scanned and SHA256SUMS verified.')

if __name__ == '__main__':
    main()
