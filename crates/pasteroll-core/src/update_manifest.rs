//! The signed update manifest: what an update is, and how it is trusted.
//!
//! Each release carries `update-manifest.txt` (the version and the exact name, size and SHA-256 of
//! the installer and of the bare program file) and `update-manifest.txt.sig` (an ECDSA P-256
//! signature of that text made with a private key that never leaves the maintainer's hands). The
//! program holds only the matching public keys. An update is run only if the signature on the
//! manifest verifies and the downloaded file matches the manifest exactly, so neither a swapped
//! download nor a compromised release page can get the program to run anything.
//!
//! No network and no Windows code here: the platform downloads the files and supplies the
//! signature check ([`SignatureVerifier`]).

use crate::update::{Version, REPO};
use sha2::{Digest, Sha256};

pub const MANIFEST_NAME: &str = "update-manifest.txt";
pub const SIGNATURE_NAME: &str = "update-manifest.txt.sig";
/// The longest manifest or signature file that is read.
pub const MAX_SMALL_FILE_BYTES: usize = 4096;
/// The largest installer or program file that is downloaded.
pub const MAX_ASSET_BYTES: u64 = 25 * 1024 * 1024;

/// ECDSA P-256 public keys (the X then Y coordinate, 64 bytes) whose signatures are accepted:
/// the release key, then a backup key kept elsewhere, so losing one does not strand installed
/// copies. An all-zero entry is a slot that is not in use.
pub const PUBLIC_KEYS: [[u8; 64]; 2] = [
    // release key (fingerprint 1)
    [
        0xdf, 0xe6, 0xc4, 0x4f, 0xc0, 0x12, 0x71, 0x75, 0x5c, 0x11, 0xf0, 0x82, 0x1c, 0x4a, 0x5a, 0x84,
        0xe7, 0xbd, 0xa9, 0x82, 0xc3, 0x28, 0x45, 0xed, 0xa7, 0x8e, 0xd6, 0x59, 0xe9, 0xd3, 0xad, 0x25,
        0x6e, 0x32, 0x18, 0x3b, 0xa5, 0x67, 0x2a, 0xcc, 0xcc, 0x47, 0x86, 0xb2, 0x20, 0x25, 0xec, 0xa0,
        0xf7, 0x40, 0xf1, 0x08, 0xf4, 0xb4, 0xc4, 0xf3, 0x61, 0x56, 0xd3, 0x38, 0x19, 0xec, 0xc1, 0xd5,
    ],
    // backup key, kept offline on separate media (fingerprint 2)
    [
        0xe9, 0x1f, 0x69, 0x14, 0xc9, 0xb5, 0xf3, 0xf4, 0xe3, 0xea, 0x1d, 0xec, 0x4a, 0x08, 0x1d, 0x14,
        0xf9, 0xe3, 0x9f, 0x5c, 0xe2, 0x36, 0x8b, 0x38, 0x33, 0x71, 0x8f, 0x69, 0x34, 0x31, 0x51, 0x8b,
        0x54, 0x2e, 0x79, 0x72, 0xdc, 0x15, 0x3e, 0x51, 0x7a, 0xec, 0x30, 0xb5, 0x4b, 0x08, 0x9a, 0x61,
        0x06, 0x82, 0x3b, 0x8f, 0x01, 0x24, 0x48, 0x45, 0xf6, 0x95, 0x34, 0xab, 0x81, 0x2d, 0x10, 0xb1,
    ],
];

/// The keys that are in use.
pub fn configured_keys() -> Vec<[u8; 64]> {
    PUBLIC_KEYS.iter().copied().filter(|k| k.iter().any(|b| *b != 0)).collect()
}

/// One downloadable file named by the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub version: Version,
    /// The Windows Setup installer.
    pub setup: Asset,
    /// The bare program file (for copies that are not installed by Setup).
    pub exe: Asset,
}

pub fn setup_name(v: &Version) -> String {
    format!("pasteroll-{v}-windows-x64-setup.exe")
}

pub fn exe_name(v: &Version) -> String {
    format!("pasteroll-{v}-windows-x64.exe")
}

/// The address of a file of release `v`. Built from a parsed version and a name this module made,
/// never from text the server sent.
pub fn asset_url(v: &Version, name: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/v{v}/{name}")
}

/// Checks a signature of `message` made with `key` (X then Y). The platform provides it.
pub trait SignatureVerifier {
    fn verify(&self, key: &[u8; 64], message: &[u8], signature: &[u8; 64]) -> bool;
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Incremental SHA-256, for hashing a download as it arrives.
pub struct Hasher(Sha256);

impl Hasher {
    pub fn new() -> Hasher {
        Hasher(Sha256::new())
    }

    pub fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    pub fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Hasher::new()
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(data);
    h.finish()
}

fn base64_value(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decodes a standard base64 signature file (whitespace around it is ignored) into the 64 bytes of
/// an ECDSA P-256 signature. Anything else is refused.
pub fn decode_signature(text: &str) -> Option<[u8; 64]> {
    let t = text.trim();
    // 64 bytes is 88 characters with two padding signs
    if t.len() != 88 || !t.ends_with("==") {
        return None;
    }
    let body = &t.as_bytes()[..86];
    let mut out = Vec::with_capacity(66);
    for chunk in body.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= u32::from(base64_value(*c)?) << (18 - 6 * i as u32);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    out.truncate(64);
    out.try_into().ok()
}

fn asset_line(line: &str, kind: &str, expected_name: &str) -> Result<Asset, String> {
    let parts: Vec<&str> = line.split(' ').collect();
    if parts.len() != 4 || parts[0] != kind {
        return Err(format!("the {kind} line is malformed"));
    }
    if parts[1] != expected_name {
        return Err(format!("the {kind} file has an unexpected name"));
    }
    let size: u64 = parts[2]
        .parse()
        .ok()
        .filter(|s| (1..=MAX_ASSET_BYTES).contains(s) && parts[2].bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| format!("the {kind} size is not acceptable"))?;
    let sha256 = unhex32(parts[3]).ok_or_else(|| format!("the {kind} hash is malformed"))?;
    Ok(Asset { name: expected_name.to_owned(), size, sha256 })
}

/// Reads a manifest. Exactly these four lines, each ending in a newline, nothing else:
///
/// ```text
/// pasteroll-update-manifest 1
/// version 0.1.0-beta.9
/// setup pasteroll-0.1.0-beta.9-windows-x64-setup.exe 4649973 <64 hex digits>
/// exe pasteroll-0.1.0-beta.9-windows-x64.exe 6105600 <64 hex digits>
/// ```
///
/// File names are not taken from the manifest: they must be exactly the names this project gives
/// release files, and sizes must be within [`MAX_ASSET_BYTES`].
pub fn parse(text: &str) -> Result<Manifest, String> {
    if text.len() > MAX_SMALL_FILE_BYTES || text.contains('\r') || !text.ends_with('\n') {
        return Err("the manifest is not in the expected form".to_owned());
    }
    let lines: Vec<&str> = text[..text.len() - 1].split('\n').collect();
    if lines.len() != 4 || lines[0] != "pasteroll-update-manifest 1" {
        return Err("the manifest is not in the expected form".to_owned());
    }
    let version = lines[1]
        .strip_prefix("version ")
        .and_then(Version::from_release_number)
        .ok_or_else(|| "the manifest version is not acceptable".to_owned())?;
    let setup = asset_line(lines[2], "setup", &setup_name(&version))?;
    let exe = asset_line(lines[3], "exe", &exe_name(&version))?;
    Ok(Manifest { version, setup, exe })
}

/// Verifies the signature on a manifest and reads it. `keys` are the accepted public keys; the
/// signature must verify under one of them. The signature is checked before the text is parsed.
pub fn verify_manifest(
    manifest: &[u8],
    signature_file: &[u8],
    keys: &[[u8; 64]],
    verifier: &dyn SignatureVerifier,
) -> Result<Manifest, String> {
    if keys.is_empty() {
        return Err("this build has no update key, so updates cannot be verified".to_owned());
    }
    if manifest.len() > MAX_SMALL_FILE_BYTES || signature_file.len() > MAX_SMALL_FILE_BYTES {
        return Err("the update information is unexpectedly large".to_owned());
    }
    let signature = std::str::from_utf8(signature_file)
        .ok()
        .and_then(decode_signature)
        .ok_or_else(|| "the update signature is not in the expected form".to_owned())?;
    if !keys.iter().any(|k| verifier.verify(k, manifest, &signature)) {
        return Err("the update is not signed by the Pasteroll release key".to_owned());
    }
    let text = std::str::from_utf8(manifest).map_err(|_| "the manifest is not text".to_owned())?;
    parse(text)
}

/// Whether an update to `m` may be installed over `current`, when the update check found
/// `announced`: the manifest must be for the very release that was announced, and newer.
pub fn check_installable(m: &Manifest, announced: &Version, current: &Version) -> Result<(), String> {
    if &m.version != announced {
        return Err("the update information is for a different version than announced".to_owned());
    }
    if &m.version <= current {
        return Err("that version is not newer than this one".to_owned());
    }
    Ok(())
}

/// Whether a downloaded file is the one the manifest names.
pub fn check_download(asset: &Asset, size: u64, digest: &[u8; 32]) -> Result<(), String> {
    if size != asset.size {
        return Err("the download is not the expected size".to_owned());
    }
    if digest != &asset.sha256 {
        return Err("the download does not match its checksum".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in check: a signature is "valid" when it is the byte-for-byte tag of key and message.
    struct Fake;

    fn tag(key: &[u8; 64], msg: &[u8]) -> [u8; 64] {
        let mut h = Hasher::new();
        h.update(key);
        h.update(msg);
        let a = h.finish();
        let mut out = [0u8; 64];
        out[..32].copy_from_slice(&a);
        out[32..].copy_from_slice(&a);
        out
    }

    impl SignatureVerifier for Fake {
        fn verify(&self, key: &[u8; 64], message: &[u8], signature: &[u8; 64]) -> bool {
            &tag(key, message) == signature
        }
    }

    fn b64(bytes: &[u8; 64]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in bytes.chunks(3) {
            let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
            s.push(T[(n >> 18) as usize & 63] as char);
            s.push(T[(n >> 12) as usize & 63] as char);
            s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
            s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
        }
        s
    }

    fn manifest_text(version: &str) -> String {
        let v = Version::parse(version).unwrap();
        format!(
            "pasteroll-update-manifest 1\nversion {v}\nsetup {} 4649973 {}\nexe {} 6105600 {}\n",
            setup_name(&v),
            hex(&sha256(b"setup")),
            exe_name(&v),
            hex(&sha256(b"exe"))
        )
    }

    const KEY: [u8; 64] = [7; 64];
    const OTHER: [u8; 64] = [9; 64];

    fn signed(text: &str, key: &[u8; 64]) -> (Vec<u8>, Vec<u8>) {
        (text.as_bytes().to_vec(), b64(&tag(key, text.as_bytes())).into_bytes())
    }

    #[test]
    fn a_well_formed_manifest_is_read() {
        let m = parse(&manifest_text("0.1.0-beta.9")).unwrap();
        assert_eq!(m.version, Version::parse("0.1.0-beta.9").unwrap());
        assert_eq!(m.setup.name, "pasteroll-0.1.0-beta.9-windows-x64-setup.exe");
        assert_eq!(m.exe.name, "pasteroll-0.1.0-beta.9-windows-x64.exe");
        assert_eq!((m.setup.size, m.exe.size), (4_649_973, 6_105_600));
        assert_eq!(m.setup.sha256, sha256(b"setup"));
    }

    #[test]
    fn anything_unexpected_in_a_manifest_is_refused() {
        let good = manifest_text("0.1.0-beta.9");
        assert!(parse(&good).is_ok());
        let h = hex(&sha256(b"setup"));
        let bad = [
            good.replace("\n", "\r\n"),
            good.trim_end().to_owned(),
            format!("{good}extra line\n"),
            good.replace("manifest 1", "manifest 2"),
            good.replace("version 0.1.0-beta.9", "version vv0.1.0-beta.9"),
            good.replace("windows-x64-setup.exe", "windows-x64-setup.exe.bak"),
            good.replace("setup pasteroll", "setup ..\\pasteroll"),
            good.replace("4649973", "0"),
            good.replace("4649973", "999999999"),
            good.replace("4649973", "4649973 "),
            good.replace("4649973", "+4649973"),
            good.replace(&h, &h.to_uppercase()),
            good.replace(&h, &h[..60]),
            good.replace("setup ", "setup  "),
            good.replace("exe pasteroll-0.1.0-beta.9-windows-x64.exe", "exe pasteroll-0.1.0-beta.8-windows-x64.exe"),
            "x".repeat(MAX_SMALL_FILE_BYTES + 1),
            String::new(),
        ];
        for (i, b) in bad.iter().enumerate() {
            assert!(parse(b).is_err(), "case {i}: {b:?}");
        }
    }

    #[test]
    fn only_a_signature_from_a_known_key_is_accepted() {
        let text = manifest_text("0.1.0-beta.9");
        let (m, s) = signed(&text, &KEY);
        let keys = [KEY, OTHER];
        assert!(verify_manifest(&m, &s, &keys, &Fake).is_ok());
        // the backup key works too
        let (m2, s2) = signed(&text, &OTHER);
        assert!(verify_manifest(&m2, &s2, &keys, &Fake).is_ok());
        // a key that is not in the list does not
        let (m3, s3) = signed(&text, &[1; 64]);
        assert!(verify_manifest(&m3, &s3, &keys, &Fake).is_err());
        // a changed manifest does not match the old signature
        let tampered = text.replace("4649973", "4649974");
        assert!(verify_manifest(tampered.as_bytes(), &s, &keys, &Fake).is_err());
        // no keys configured: nothing can be verified
        assert!(verify_manifest(&m, &s, &[], &Fake).unwrap_err().contains("no update key"));
    }

    #[test]
    fn a_damaged_signature_file_is_refused() {
        let text = manifest_text("0.1.0-beta.9");
        let (m, s) = signed(&text, &KEY);
        let keys = [KEY];
        let s = String::from_utf8(s).unwrap();
        for bad in [
            String::new(),
            s[..80].to_owned(),
            format!("{s}A"),
            s.replace('=', "A"),
            s.replacen(&s[..1], "!", 1),
            format!("{s}\0"),
        ] {
            assert!(verify_manifest(&m, bad.as_bytes(), &keys, &Fake).is_err(), "{bad:?}");
        }
        assert!(verify_manifest(&m, &[0xff, 0xfe, 0x00], &keys, &Fake).is_err(), "not text");
        // whitespace around the signature is fine
        assert!(verify_manifest(&m, format!("\r\n{s}\r\n").as_bytes(), &keys, &Fake).is_ok());
        assert!(verify_manifest(&m, &vec![b'A'; MAX_SMALL_FILE_BYTES + 1], &keys, &Fake).is_err());
    }

    #[test]
    fn base64_signatures_decode_exactly() {
        let mut sig = [0u8; 64];
        for (i, b) in sig.iter_mut().enumerate() {
            *b = (i * 7 + 3) as u8;
        }
        assert_eq!(decode_signature(&b64(&sig)), Some(sig));
        assert_eq!(decode_signature(&format!("  {}\n", b64(&sig))), Some(sig));
        assert_eq!(decode_signature(""), None);
    }

    #[test]
    fn only_the_announced_newer_release_may_be_installed() {
        let m = parse(&manifest_text("0.1.0-beta.9")).unwrap();
        let v = |s: &str| Version::parse(s).unwrap();
        assert!(check_installable(&m, &v("0.1.0-beta.9"), &v("0.1.0-beta.8")).is_ok());
        assert!(check_installable(&m, &v("0.1.0-beta.10"), &v("0.1.0-beta.8")).is_err(), "a different release was announced");
        assert!(check_installable(&m, &v("0.1.0-beta.9"), &v("0.1.0-beta.9")).is_err(), "not newer");
        assert!(check_installable(&m, &v("0.1.0-beta.9"), &v("0.1.0-beta.10")).is_err(), "a downgrade");
    }

    #[test]
    fn an_older_signed_release_that_is_still_newer_than_this_one_is_accepted() {
        // Known and documented limit (SECURITY.md): the announced version comes from the release list,
        // which is not signed, so whoever controls that list can announce an older genuine release.
        // The signature still proves the release is genuine, and it is never older than the running copy.
        let m = parse(&manifest_text("0.1.0-beta.9")).unwrap();
        let v = |s: &str| Version::parse(s).unwrap();
        assert!(check_installable(&m, &v("0.1.0-beta.9"), &v("0.1.0-beta.8")).is_ok());
        assert!(check_installable(&m, &v("0.1.0-beta.9"), &v("0.1.0-beta.9")).is_err());
    }

    #[test]
    fn a_download_must_match_the_manifest() {
        let a = Asset { name: "x".into(), size: 5, sha256: sha256(b"hello") };
        assert!(check_download(&a, 5, &sha256(b"hello")).is_ok());
        assert!(check_download(&a, 6, &sha256(b"hello")).is_err());
        assert!(check_download(&a, 5, &sha256(b"hellp")).is_err());
        let mut h = Hasher::new();
        h.update(b"hel");
        h.update(b"lo");
        assert_eq!(h.finish(), sha256(b"hello"));
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn both_release_keys_are_in_use_and_different() {
        assert_eq!(configured_keys().len(), 2, "the release key and the backup key");
        assert_ne!(PUBLIC_KEYS[0], PUBLIC_KEYS[1]);
    }

    #[test]
    fn urls_are_built_from_the_version() {
        let v = Version::parse("0.1.0-beta.9").unwrap();
        assert_eq!(
            asset_url(&v, &setup_name(&v)),
            "https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.9/pasteroll-0.1.0-beta.9-windows-x64-setup.exe"
        );
        assert!(configured_keys().len() <= PUBLIC_KEYS.len());
    }
}
