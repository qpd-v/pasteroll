//! Encryption at rest.
//!
//! One random 256-bit master key per installation. The app stores it protected by the OS (DPAPI on
//! Windows, bound to the signed-in user); this module only does the cryptography, so it is
//! platform-independent and unit-tested.
//!
//! - Clips are sealed with XChaCha20-Poly1305 (random 192-bit nonce per clip, so no nonce-reuse
//!   bookkeeping). The associated data names the clip kind, so a sealed text body cannot be swapped
//!   in as an image body.
//! - Duplicate detection uses HMAC-SHA256 under a key derived from the master key, so a database
//!   thief cannot test guessed secrets (a PIN, a known password) against the stored fingerprints.
//! - The encryption key and the MAC key are derived separately from the master key.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const VERSION: u8 = 1;
const NONCE_LEN: usize = 24;

pub const MASTER_LEN: usize = 32;

#[derive(Clone)]
pub struct Vault {
    cipher: XChaCha20Poly1305,
    mac_key: [u8; 32],
}

/// The format version is authenticated too, so a body cannot be replayed under another version.
fn bound_aad(aad: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(1 + aad.len());
    v.push(VERSION);
    v.extend_from_slice(aad);
    v
}

fn derive(master: &[u8; MASTER_LEN], label: &[u8]) -> [u8; 32] {
    let mut m = <HmacSha256 as KeyInit>::new_from_slice(master).expect("HMAC accepts any key length");
    m.update(label);
    m.finalize().into_bytes().into()
}

/// A fresh random master key from the OS CSPRNG.
pub fn generate_master() -> Result<[u8; MASTER_LEN], getrandom::Error> {
    let mut k = [0u8; MASTER_LEN];
    getrandom::fill(&mut k)?;
    Ok(k)
}

impl Drop for Vault {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.mac_key);
    }
}

impl Vault {
    pub fn new(master: &[u8; MASTER_LEN]) -> Vault {
        let enc = derive(master, b"carousel/v1/encrypt");
        let mac_key = derive(master, b"carousel/v1/fingerprint");
        Vault { cipher: XChaCha20Poly1305::new(&enc.into()), mac_key }
    }

    /// `VERSION || nonce || ciphertext+tag`.
    pub fn seal(&self, aad: &[u8], plain: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).expect("OS random source unavailable");
        let ct = self
            .cipher
            .encrypt(&XNonce::from(nonce), Payload { msg: plain, aad: &bound_aad(aad) })
            .expect("encryption cannot fail for in-memory buffers");
        let mut out = Vec::with_capacity(1 + NONCE_LEN + ct.len());
        out.push(VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        out
    }

    /// `None` if the data is truncated, was sealed under another key or different associated data,
    /// or has been modified.
    pub fn open(&self, aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
        if sealed.len() < 1 + NONCE_LEN + 16 || sealed[0] != VERSION {
            return None;
        }
        let nonce: [u8; NONCE_LEN] = sealed[1..1 + NONCE_LEN].try_into().ok()?;
        self.cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload { msg: &sealed[1 + NONCE_LEN..], aad: &bound_aad(aad) },
            )
            .ok()
    }

    /// Keyed fingerprint for duplicate detection.
    pub fn fingerprint(&self, kind: u8, data: &[u8]) -> [u8; 32] {
        let mut m =
            <HmacSha256 as KeyInit>::new_from_slice(&self.mac_key).expect("HMAC accepts any key length");
        m.update(&[kind]);
        m.update(data);
        m.finalize().into_bytes().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(b: u8) -> Vault {
        Vault::new(&[b; MASTER_LEN])
    }

    #[test]
    fn roundtrip() {
        let v = vault(1);
        let sealed = v.seal(b"text", b"hello world");
        assert_eq!(v.open(b"text", &sealed).as_deref(), Some(&b"hello world"[..]));
    }

    #[test]
    fn empty_and_large_payloads_roundtrip() {
        let v = vault(1);
        assert_eq!(v.open(b"x", &v.seal(b"x", b"")).as_deref(), Some(&b""[..]));
        let big = vec![0xAB; 3_000_000];
        assert_eq!(v.open(b"x", &v.seal(b"x", &big)).unwrap(), big);
    }

    #[test]
    fn ciphertext_does_not_contain_the_plaintext() {
        let v = vault(1);
        let sealed = v.seal(b"text", b"correct horse battery staple");
        assert!(!sealed.windows(7).any(|w| w == b"battery"));
    }

    #[test]
    fn same_plaintext_seals_differently_each_time() {
        let v = vault(1);
        assert_ne!(v.seal(b"a", b"same"), v.seal(b"a", b"same"));
    }

    #[test]
    fn tampering_is_detected() {
        let v = vault(1);
        let mut sealed = v.seal(b"text", b"important");
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert_eq!(v.open(b"text", &sealed), None);
        let mut sealed = v.seal(b"text", b"important");
        sealed[1 + NONCE_LEN + 2] ^= 0x80;
        assert_eq!(v.open(b"text", &sealed), None);
    }

    #[test]
    fn wrong_key_cannot_open() {
        let sealed = vault(1).seal(b"text", b"secret");
        assert_eq!(vault(2).open(b"text", &sealed), None);
    }

    #[test]
    fn associated_data_binds_the_kind() {
        let v = vault(1);
        let sealed = v.seal(b"text", b"secret");
        assert_eq!(v.open(b"image", &sealed), None);
    }

    #[test]
    fn truncated_or_garbage_input_is_rejected() {
        let v = vault(1);
        assert_eq!(v.open(b"t", &[]), None);
        assert_eq!(v.open(b"t", &[1, 2, 3]), None);
        assert_eq!(v.open(b"t", &vec![9; 100]), None); // wrong version byte
    }

    #[test]
    fn fingerprints_depend_on_key_kind_and_content() {
        let (a, b) = (vault(1), vault(2));
        assert_eq!(a.fingerprint(0, b"pin 1234"), a.fingerprint(0, b"pin 1234"));
        assert_ne!(a.fingerprint(0, b"pin 1234"), b.fingerprint(0, b"pin 1234"));
        assert_ne!(a.fingerprint(0, b"x"), a.fingerprint(1, b"x"));
        assert_ne!(a.fingerprint(0, b"x"), a.fingerprint(0, b"y"));
    }

    #[test]
    fn generated_keys_differ() {
        assert_ne!(generate_master().unwrap(), generate_master().unwrap());
    }
}
