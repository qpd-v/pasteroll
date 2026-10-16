# Download text to update for 0.1.0-beta.2

Line numbers below are the original locations inspected during release preparation. The README
replacements have now been applied under the user's instruction to finish the work unattended.
The website remains read only under the earlier explicit restriction; apply its listed changes
separately before announcing the release.

## README.md in this repository - applied

| Existing line | Change |
|---|---|
| 12 | Windows: `pasteroll-0.1.0-beta.2-windows-x64-setup.exe` and `pasteroll-0.1.0-beta.2-windows-x64-portable.zip`. Linux: `pasteroll-0.1.0-beta.2-linux-x86_64.tar.gz` and `pasteroll_0.1.0-beta.2_amd64.deb`. |
| 47 | Replace `pasteroll-windows-x64.zip` with the setup EXE filename. Keep `SHA256SUMS`. |
| 48 | Link to `/releases/tag/v0.1.0-beta.2` after publication, or retain the Releases list rather than relying on a latest-release redirect. |
| 50-51 | Run the setup wizard. Only for me is the default; For all users needs UAC. Describe the portable ZIP as a separate choice. |
| 61 | `tar xzf pasteroll-0.1.0-beta.2-linux-x86_64.tar.gz` |
| 62 | `cd pasteroll-0.1.0-beta.2-linux-x86_64` |
| 63 | `./install.sh`; optionally `sudo ./install.sh --system`. Add the `.deb` command from the package README. |
| 179 | `Get-FileHash .\pasteroll-0.1.0-beta.2-windows-x64-setup.exe -Algorithm SHA256` |
| 190 | Building the app needs Rust and the platform toolchain. Windows packaging additionally needs Inno Setup 6 and Python 3.11+ for the release tools. |
| 197 | Use the scoped Windows debug clean plus native test command in `RELEASING.md`; the unqualified workspace command has a same-name platform binary collision. |
| 201-202 | Windows: `scripts/package-windows-installer.ps1` builds setup and portable. Linux: `scripts/package-linux.sh` builds the archive and `.deb`; mention Python 3, binutils and `dpkg-deb`. Link to `RELEASING.md`. |

`packaging/README-windows.txt` and `packaging/README-linux.txt` already describe these package names.

## Website: pasteroll-site/index.html (separate repository)

| Existing line | Change |
|---|---|
| 189 | Change displayed beta version from `0.1.0-beta.1` to `0.1.0-beta.2`. |
| 193 | Show the Windows setup filename; describe per-user default and optional all-users installation. Add the portable ZIP filename/choice. |
| 194 | Point to `https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.2`, or to the setup asset under `/releases/download/v0.1.0-beta.2/`. |
| 197 | Show the versioned Linux archive filename and add the `.deb` choice. |
| 198 | Use the tagged release page or the versioned archive asset, matching the Windows choice. |
| 210 | Replace the Windows filename in the copy button's `data-copy` command with the setup EXE filename. |
| 211 | Make the identical filename change in the visible PowerShell command. |
| 217 | Update both the copy button and visible `tar` command to the versioned archive. Change the copied `cd` directory and `./pasteroll install` to `./install.sh`. |
| 218 | Set visible `cd` to `pasteroll-0.1.0-beta.2-linux-x86_64`. |
| 219 | Set visible installation command to `./install.sh`. Add system-install and `.deb` alternatives if desired. |

Keep the Linux `sha256sum -c SHA256SUMS --ignore-missing` command at 212-213. Use the exact tagged
release URL for this prerelease rather than assuming `/releases/latest` selects it. GitHub documents
[release links](https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases)
and [the latest release endpoint](https://docs.github.com/en/rest/releases/releases#get-the-latest-release).
