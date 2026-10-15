//! Recovery that survives key rotation.
//!
//! A passphrase export of the master key goes stale the first time the key rotates. This module
//! removes that trap: once, at export time, a recovery key *pair* is made.
//!
//! - the **public** half is kept in the history; every rotation seals the new master key to it,
//!   without needing any secret (rotation runs unattended at start-up);
//! - the **private** half goes into the passphrase export, and nowhere else.
//!
//! So an old export plus a copy of the history is always enough: the private half opens the
//! history's current recovery blob, which holds whatever the master key is *now*.
//!
//! Sealing is an ECIES-style box: an ephemeral X25519 key agreement, the shared secret run through
//! HMAC-SHA256 with both public keys, then XChaCha20-Poly1305.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroize;

type HmacSha256 = Hmac<Sha256>;

const NONCE_LEN: usize = 24;
const LABEL: &[u8] = b"carousel recovery v1";

pub type Secret = [u8; 32];
pub type Public = [u8; 32];

/// A fresh recovery key pair.
pub fn generate() -> Result<(Secret, Public), getrandom::Error> {
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret)?;
    let public = PublicKey::from(&StaticSecret::from(secret));
    Ok((secret, public.to_bytes()))
}

pub fn public_of(secret: &Secret) -> Public {
    PublicKey::from(&StaticSecret::from(*secret)).to_bytes()
}

fn derive_key(shared: &[u8; 32], epk: &[u8; 32], recipient: &Public) -> [u8; 32] {
    let mut m = <HmacSha256 as KeyInit>::new_from_slice(shared).expect("any key length");
    m.update(LABEL);
    m.update(epk);
    m.update(recipient);
    m.finalize().into_bytes().into()
}

/// Seals `plain` so that only the holder of the private half of `recipient` can open it.
pub fn seal_to(recipient: &Public, plain: &[u8]) -> Result<Vec<u8>, getrandom::Error> {
    let mut e = [0u8; 32];
    getrandom::fill(&mut e)?;
    let ephemeral = StaticSecret::from(e);
    e.zeroize();
    let epk = PublicKey::from(&ephemeral).to_bytes();
    let shared = ephemeral.diffie_hellman(&PublicKey::from(*recipient));
    let mut key = derive_key(shared.as_bytes(), &epk, recipient);
    let cipher = XChaCha20Poly1305::new(&key.into());
    key.zeroize();
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce)?;
    let ct = cipher
        .encrypt(&XNonce::from(nonce), Payload { msg: plain, aad: &epk })
        .expect("encryption of an in-memory buffer cannot fail");
    let mut out = Vec::with_capacity(32 + NONCE_LEN + ct.len());
    out.extend_from_slice(&epk);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Opens a blob made by [`seal_to`] with the private half.
pub fn open_with(secret: &Secret, blob: &[u8]) -> Option<Vec<u8>> {
    if blob.len() < 32 + NONCE_LEN + 16 {
        return None;
    }
    let epk: [u8; 32] = blob[..32].try_into().ok()?;
    let nonce: [u8; NONCE_LEN] = blob[32..32 + NONCE_LEN].try_into().ok()?;
    let me = StaticSecret::from(*secret);
    let public = PublicKey::from(&me).to_bytes();
    let shared = me.diffie_hellman(&PublicKey::from(epk));
    let mut key = derive_key(shared.as_bytes(), &epk, &public);
    let cipher = XChaCha20Poly1305::new(&key.into());
    key.zeroize();
    cipher
        .decrypt(&XNonce::from(nonce), Payload { msg: &blob[32 + NONCE_LEN..], aad: &epk })
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let (secret, public) = generate().unwrap();
        let blob = seal_to(&public, b"the master key bytes").unwrap();
        assert_eq!(open_with(&secret, &blob).as_deref(), Some(&b"the master key bytes"[..]));
    }

    #[test]
    fn the_public_half_is_derivable_and_cannot_open_anything() {
        let (secret, public) = generate().unwrap();
        assert_eq!(public_of(&secret), public);
        let blob = seal_to(&public, b"x").unwrap();
        // opening needs the private half; the public key as a "secret" must not work
        assert_eq!(open_with(&public, &blob), None);
    }

    #[test]
    fn another_recipient_cannot_open_it() {
        let (_, public) = generate().unwrap();
        let (other_secret, _) = generate().unwrap();
        let blob = seal_to(&public, b"secret").unwrap();
        assert_eq!(open_with(&other_secret, &blob), None);
    }

    #[test]
    fn tampering_with_any_byte_is_detected() {
        let (secret, public) = generate().unwrap();
        let blob = seal_to(&public, b"payload payload payload").unwrap();
        for i in [0, 31, 32, 50, blob.len() - 1] {
            let mut bad = blob.clone();
            bad[i] ^= 1;
            assert_eq!(open_with(&secret, &bad), None, "byte {i}");
        }
    }

    #[test]
    fn two_seals_of_the_same_plaintext_differ() {
        let (_, public) = generate().unwrap();
        assert_ne!(seal_to(&public, b"same").unwrap(), seal_to(&public, b"same").unwrap());
    }

    #[test]
    fn short_or_empty_blobs_are_rejected() {
        let (secret, _) = generate().unwrap();
        assert_eq!(open_with(&secret, &[]), None);
        assert_eq!(open_with(&secret, &[0u8; 60]), None);
    }
}
