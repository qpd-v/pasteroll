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
  only text keys are consumed.
- **No network**: see [PRIVACY.md](PRIVACY.md).

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
