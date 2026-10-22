# Security

## Reporting a vulnerability

Please report security problems privately, not in a public issue: use GitHub's
**Security → Report a vulnerability** on this repository. Include the version
(`pasteroll --version`), your OS, and steps to reproduce. This is a one-person beta, so replies
may take a few days.

## What is protected, and what is not

Pasteroll encrypts the history at rest. It is **not** a defence against malware running as you.

| Threat | Protected? |
|---|---|
| Someone copies the database file (stolen disk, backup, another user account) | **Yes**: every clip is sealed with XChaCha20-Poly1305; the key is separate |
| Another local user reads your files | **Yes** (Windows: key protected with DPAPI for your account; Linux: key file mode 0600 in a 0700 folder) |
| Malware or a process running as your own account | **No**: it can read the key the same way Pasteroll does, and the clipboard itself |
| Someone with admin/root on your machine | **No** |
| Cold-boot / full-disk theft on Linux without disk encryption | **Partly**: the key file is protected by permissions only; use full-disk encryption |
| Screen capture of the open carousel (Windows Recall, screen sharing) | **No**: the overlay is not excluded from capture |

## Design

- **Storage**: SQLite; each clip's content is sealed individually with an authenticated cipher
  (XChaCha20-Poly1305). Record identifiers are bound into the sealed data so rows cannot be
  swapped.
- **Key**: a random master key. Windows wraps it with DPAPI (your account, with extra entropy).
  Linux stores it in a key file with mode 0600 in a private folder. The key is replaced
  automatically every 30 days and clips are re-sealed.
- **Recovery export** (`key-export`): the key material is wrapped with a key derived from your
  passphrase using Argon2id (64 MiB, 3 passes) and sealed with XChaCha20-Poly1305. The file is
  useless without the passphrase. Keep it, and the passphrase, away from the computer.
- **Input handling**: global input hooks are used to open the carousel and to swallow keys while
  it is open. Input injected by other programs is ignored by design. While the carousel is open
  only text keys are consumed. The carousel lets go after at most 60 seconds; the expanded viewer,
  which is for reading, keeps the keys while it is open and closes after 10 minutes without
  input. Either way, input is released if the program stops responding (Windows: within half a
  second; Linux: the program exits after 4 seconds so the X server drops its grabs).
- **Image editor**: while Advanced Image Paste is open the input hooks also swallow clicks outside
  its window (so a stray click cannot lose the work), under the same limits as above: they stop at
  once if the program stops responding, and after ten minutes without input the session ends. The
  picture is decoded and edited in memory; the original clip is only read.
- **Recycle bin**: a deleted clip stays in the database, encrypted, for 30 days (so it can be
  restored), unless it is deleted permanently. Permanent deletion overwrites it as before.
- **Network**: none, except the optional update check on Windows (off until the person says yes) and,
  only when the person chooses "Download and install", the download described under
  "Installing updates". The check makes one HTTPS GET to `api.github.com` with the system's certificate checks and proxy
  settings (WinHTTP). TLS 1.2 or newer is required, and so is refusing redirects and Windows'
  automatic sign-in to proxies and servers: if one of those settings cannot be applied the request
  is not made. The whole check has a 45-second limit and the reply a 1 MB cap. Certificate
  revocation is not checked and nothing is pinned, so someone who controls a trusted certificate
  authority on the network could read the request or give a false answer; the worst that does is a
  wrong "update available" message. Only `"tag_name"` strings that are exactly this project's
  release tags (`v0.1.0-beta.7`) are read, and the page that is opened is built from that parsed version, never
  copied from the reply. The check downloads and runs nothing, so a hostile reply can at worst show a wrong
  "update available" message; installing is a separate, signed step (see below). See [PRIVACY.md](PRIVACY.md).

## Installing updates (Windows)

"Download and install" runs a program that came from the internet, so it is built so that the
network, GitHub, and anyone who can change a release page cannot make Pasteroll run anything that the
maintainer did not sign. The limits of that are listed below; please read them.

**How an update is trusted**

- **Signature.** Each release has `update-manifest.txt` (the version and the exact name, size and
  SHA-256 of the installer and of the program file) and `update-manifest.txt.sig`, an ECDSA P-256
  signature of it. The program holds only public keys (a release key and a backup key) and checks the
  signature with Windows' own crypto, before the manifest is read. The manifest is read strictly: four
  lines, file names that must be exactly this project's names, sizes within a limit. The private keys
  are not in this repository and not in the program; each is kept encrypted with a passphrase, the
  backup on separate offline media. Key fingerprints (the first eight bytes of the SHA-256 of the
  64-byte public key): release `4c78c3487cccaa16`, backup `cf5f92f51d2512ed`.
- **What is installed.** Only the release the update check announced, only if it is newer than the
  running version, only the file the manifest names, only if its size and SHA-256 match. A downloaded
  file that fails any check is deleted. The updater uses no file that the signed manifest does not name.
- **Download rules.** HTTPS only. Redirects are followed by hand: at most three, and only to an
  `https` address on `github.com`, `release-assets.githubusercontent.com` or
  `objects.githubusercontent.com` (the redirect address comes from the server, but is used only if it
  passes those rules). No automatic sign-in, a size cap and a time cap. The first address is built from
  the parsed version, never from what a server sends.
- **How it installs.** Copies installed for one account run the signed Setup silently. A copy installed
  for all users (in Program Files) asks Windows for administrator permission to run its own installed
  program, `pasteroll apply-update`, which does the whole job again by itself (see below). Portable and
  self-installed copies prepare the new program file beside the old one, check it, then rename the old
  one to `.old` and the new one into place; if the last step fails the old one is put back, and if even
  that fails the message names the files to rename by hand. If Setup was started but the version did
  not change, the next start says so.
- **All-users copies: no file the ordinary account can write is read or run by the elevated part.**
  The elevated helper is the program in Program Files, which only administrators can change in a
  default Windows (it refuses to run unless it is elevated and sits in a Program Files folder; it does
  not check the folder's access list). It downloads the manifest and the installer itself, checks the
  signature and the checksum, and keeps them in a new folder that only Administrators and the system can
  open (under Windows' `Temp`, random name, created with that access in one step, and refused if its
  owner is anyone else). Setup's own temporary files are pointed there too. It accepts exactly what the
  normal check accepts, so running it by hand can at most install a genuine, newer, signed release. The
  system's `cmd.exe`, started by full path with a fixed `PATH`, waits for Setup, starts the new program
  (by full path, through `explorer.exe`) and removes the folder; folders left by an update that did not
  finish are removed at the next update. The helper stops itself after ten minutes.
- **Other copies: the downloaded file is held.** It is hashed again through a handle that nobody else
  may write to, rename or delete, and Pasteroll keeps that handle until it exits. The download folder
  inside the data folder is created by Pasteroll, locked to your account, and only ever cleared if it
  carries Pasteroll's marker file.

**What this does not protect against**

- **Whoever holds a signing key and its passphrase** can publish an update that every copy with this
  feature will accept. The keys are kept offline and passphrase-protected for that reason. There is no
  key revocation: if a key leaks, a new version is released and signed with the backup key, and copies
  must be updated by hand. A compromise of the GitHub account alone cannot produce a valid update.
- **Rollback and freezing.** Which release is announced comes from GitHub's release list, which is not
  signed. Someone who controls that list or the network can announce an older genuine release that is
  still newer than the running copy, or hide a newer one. They cannot make Pasteroll install anything
  that is not genuinely signed. A signed "latest version" pointer would close this; it is not built.
- **All-users copies and who "you" are.** Windows' administrator prompt is not a security boundary
  against a program already running as you: such a program can open the same prompt, and if you accept
  it, the helper still only installs a genuine, newer, signed release. The elevated account can be a
  different one from yours (an administrator's credentials typed over your shoulder), in which case
  Setup's per-account steps (registering the install, start at sign-in) apply to that account, as they
  do for any all-users Setup. The new program is started again through `explorer.exe` so that it runs as
  you, which depends on Windows Explorer running as you in the normal way; it is not started if Setup
  fails, if the install folder's name has unusual characters, or when the sign-in is not a normal one.
  A copy installed for all users outside Program Files is not updated by Pasteroll (it only says that a
  newer version exists). Whether Setup worked is reported at the next start (an all-users Setup keeps no
  log, because a log would sit in a folder you control).
- **First install.** The keys built into a copy are only as trustworthy as the copy you first
  installed; check the published checksums and the fingerprints above.
- **Network observers.** A network attacker, or a certificate authority your computer trusts, can see
  that you are updating, read the request, stall it, or serve older genuine releases (see above). They
  cannot forge a release.
- **`pasteroll update --yes`** installs without the confirmation window, since the flag is the
  confirmation.
- **No Authenticode signature.** The installer has no code-signing certificate, and a file that
  Pasteroll downloads itself carries no "downloaded from the internet" mark, so Windows SmartScreen
  never looks at it. Some antivirus programs treat a program that downloads and runs an installer
  warily.

## Known issues and accepted risks

- A key that is held in memory is zeroized when dropped, but nothing prevents a debugger or
  another same-user process from reading it while Pasteroll runs.
- Dead keys and IME composition are not handled in the search box.
- Another same-user process can post window messages to the app (it cannot do more than a user
  could do with the keyboard).
- `cargo audit` reports no vulnerabilities in the dependencies; it flags one *unmaintained* crate
  (`ttf-parser`, pulled in by the text-rendering library), which has no known flaw.
- The beta is unsigned: verify the checksum from the release page.

Findings that you consider security-relevant but are listed here are still welcome; they help
set priorities.
