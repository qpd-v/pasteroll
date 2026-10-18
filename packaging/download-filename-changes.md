# Download updates after beta.3 approval and publication

Do not change the website or the public README download links before publication.
The site is maintained in a separate repository; no website edits are part of this work.
Its approved three feature videos should be preserved.

| Existing beta.2 asset | Candidate replacement |
|---|---|
| `pasteroll-0.1.0-beta.2-windows-x64-setup.exe` | `pasteroll-0.1.0-beta.3-windows-x64-setup.exe` |
| `pasteroll-0.1.0-beta.2-windows-x64-portable.zip` | `pasteroll-0.1.0-beta.3-windows-x64-portable.zip` |
| `pasteroll-0.1.0-beta.2-linux-x86_64.tar.gz` | `pasteroll-0.1.0-beta.3-linux-x86_64.tar.gz` |
| `pasteroll_0.1.0-beta.2_amd64.deb` | `pasteroll_0.1.0-beta.3_amd64.deb` |

Change release tag/checksum URLs from `v0.1.0-beta.2` to `v0.1.0-beta.3`, visible
version labels and archive extraction folder examples. In the app README these
occur in Status, Download, release notes/SHA256SUMS, Windows/Linux install and
Verify your download. Locate current line numbers immediately before editing.
In the website, locate the equivalent download buttons/platform tabs and install
commands in index.html/script.js/README.md; prior line numbers became stale when
the feature videos were deployed. Do not infer their current locations.

If GNOME desktop acceptance passes, add clearly labelled GNOME 46 experimental
alternatives for the `-gnome-development.tar.gz` and `-gnome-development.deb` assets,
link their setup instructions, and state the administrator-enabled helper requirement
for mouse activation. Keep the regular Linux downloads labelled X11. The two Debian
packages are alternatives. Do not claim general Wayland, newer GNOME or KDE support.

No website changes, deployment, commit, push or publication has been performed here.
