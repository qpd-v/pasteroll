//! Passphrase-protected key export, so losing the Windows account (a reinstall, a password reset)
//! does not mean losing the history.
//!
//! The master key is wrapped with a key derived from a passphrase by Argon2id and sealed with
//! XChaCha20-Poly1305; the header (parameters and salt) is authenticated. The file alone is useless
//! without the passphrase, and the passphrase alone is useless without the file and the history.
//!
//! Layout: `"CKX1" | m_cost u32 | t_cost u32 | p_cost u32 | salt[16] | nonce[24] | ciphertext+tag`.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroize;

const MAGIC: &[u8; 4] = b"CKX1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = 4 + 12 + SALT_LEN;
/// Anything above these in a file is refused before any work is done: a hostile file must not be
/// able to demand gigabytes of memory.
const MAX_M_COST_KIB: u32 = 256 * 1024;
const MAX_T_COST: u32 = 12;
const MAX_P_COST: u32 = 8;

pub const MIN_PASSPHRASE_CHARS: usize = 12;

/// What an export carries: the key ring in force when it was made, and (when recovery is set up)
/// the private half of the recovery key pair, which still works after the key has rotated.
pub struct ExportContents {
    pub ring: Vec<u8>,
    pub recovery_secret: Option<[u8; 32]>,
}

impl Drop for ExportContents {
    fn drop(&mut self) {
        self.ring.zeroize();
        if let Some(s) = &mut self.recovery_secret {
            s.zeroize();
        }
    }
}

impl ExportContents {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(8 + self.ring.len() + 33);
        v.extend_from_slice(b"CKP1");
        v.extend_from_slice(&(self.ring.len() as u16).to_le_bytes());
        v.extend_from_slice(&self.ring);
        match &self.recovery_secret {
            Some(s) => {
                v.push(1);
                v.extend_from_slice(s);
            }
            None => v.push(0),
        }
        v
    }

    pub fn decode(b: &[u8]) -> Option<ExportContents> {
        if b.len() < 7 || &b[..4] != b"CKP1" {
            return None;
        }
        let n = u16::from_le_bytes([b[4], b[5]]) as usize;
        let ring = b.get(6..6 + n)?.to_vec();
        let flag = *b.get(6 + n)?;
        let recovery_secret = match flag {
            0 if b.len() == 7 + n => None,
            1 if b.len() == 7 + n + 32 => Some(b[7 + n..].try_into().ok()?),
            _ => return None,
        };
        Some(ExportContents { ring, recovery_secret })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for KdfParams {
    /// 64 MiB, 3 passes, 1 lane: well above the commonly recommended minimum for Argon2id.
    fn default() -> Self {
        KdfParams { m_cost_kib: 64 * 1024, t_cost: 3, p_cost: 1 }
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    #[error("the passphrase must be at least {MIN_PASSPHRASE_CHARS} characters")]
    WeakPassphrase,
    #[error("not a Pasteroll key export")]
    NotAnExport,
    #[error("key export uses parameters that are not accepted")]
    BadParameters,
    #[error("wrong passphrase, or the file is damaged")]
    WrongPassphraseOrCorrupt,
    #[error("key derivation failed")]
    Kdf,
    #[error("no random source")]
    Random,
}

fn derive(pass: &str, salt: &[u8], p: KdfParams) -> Result<[u8; 32], ExportError> {
    let params = Params::new(p.m_cost_kib, p.t_cost, p.p_cost, Some(32)).map_err(|_| ExportError::BadParameters)?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(pass.as_bytes(), salt, &mut out)
        .map_err(|_| ExportError::Kdf)?;
    Ok(out)
}

/// Wraps `secret` (the encoded key ring) under `passphrase`.
pub fn wrap(secret: &[u8], passphrase: &str, params: KdfParams) -> Result<Vec<u8>, ExportError> {
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(ExportError::WeakPassphrase);
    }
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut salt).map_err(|_| ExportError::Random)?;
    getrandom::fill(&mut nonce).map_err(|_| ExportError::Random)?;
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&params.m_cost_kib.to_le_bytes());
    header.extend_from_slice(&params.t_cost.to_le_bytes());
    header.extend_from_slice(&params.p_cost.to_le_bytes());
    header.extend_from_slice(&salt);
    let mut key = derive(passphrase, &salt, params)?;
    let cipher = XChaCha20Poly1305::new(&key.into());
    key.zeroize();
    let ct = cipher
        .encrypt(&XNonce::from(nonce), Payload { msg: secret, aad: &header })
        .map_err(|_| ExportError::Kdf)?;
    let mut out = header;
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Opens an export. Never reveals whether the passphrase or the file was at fault.
pub fn unwrap(blob: &[u8], passphrase: &str) -> Result<Vec<u8>, ExportError> {
    if blob.len() < HEADER_LEN + NONCE_LEN + 16 || &blob[..4] != MAGIC {
        return Err(ExportError::NotAnExport);
    }
    let rd = |at: usize| u32::from_le_bytes(blob[at..at + 4].try_into().unwrap());
    let params = KdfParams { m_cost_kib: rd(4), t_cost: rd(8), p_cost: rd(12) };
    if params.m_cost_kib > MAX_M_COST_KIB || params.t_cost > MAX_T_COST || params.p_cost > MAX_P_COST {
        return Err(ExportError::BadParameters);
    }
    let header = &blob[..HEADER_LEN];
    let salt = &blob[16..HEADER_LEN];
    let nonce: [u8; NONCE_LEN] = blob[HEADER_LEN..HEADER_LEN + NONCE_LEN].try_into().unwrap();
    let mut key = derive(passphrase, salt, params)?;
    let cipher = XChaCha20Poly1305::new(&key.into());
    key.zeroize();
    cipher
        .decrypt(&XNonce::from(nonce), Payload { msg: &blob[HEADER_LEN + NONCE_LEN..], aad: header })
        .map_err(|_| ExportError::WrongPassphraseOrCorrupt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap parameters: the tests are about the format, not the cost.
    const FAST: KdfParams = KdfParams { m_cost_kib: 64, t_cost: 1, p_cost: 1 };
    const PASS: &str = "correct horse battery";

    #[test]
    fn roundtrip() {
        let secret = [7u8; 37];
        let blob = wrap(&secret, PASS, FAST).unwrap();
        assert_eq!(unwrap(&blob, PASS).unwrap(), secret);
    }

    #[test]
    fn the_file_does_not_contain_the_secret() {
        let secret = b"THIS-IS-THE-MASTER-KEY-MATERIAL!";
        let blob = wrap(secret, PASS, FAST).unwrap();
        assert!(!blob.windows(16).any(|w| w == &secret[..16]));
    }

    #[test]
    fn a_wrong_passphrase_is_rejected() {
        let blob = wrap(b"secret", PASS, FAST).unwrap();
        assert_eq!(unwrap(&blob, "incorrect horse battery"), Err(ExportError::WrongPassphraseOrCorrupt));
    }

    #[test]
    fn tampering_with_any_part_is_detected() {
        let blob = wrap(b"secret", PASS, FAST).unwrap();
        for i in [5, 9, 14, 20, 40, blob.len() - 1] {
            let mut bad = blob.clone();
            bad[i] ^= 1;
            assert!(unwrap(&bad, PASS).is_err(), "byte {i} was not authenticated");
        }
    }

    #[test]
    fn weak_passphrases_are_refused_on_export() {
        assert_eq!(wrap(b"s", "short", FAST), Err(ExportError::WeakPassphrase));
    }

    #[test]
    fn two_exports_of_the_same_secret_differ() {
        assert_ne!(wrap(b"s", PASS, FAST).unwrap(), wrap(b"s", PASS, FAST).unwrap());
    }

    #[test]
    fn hostile_parameters_are_refused_before_any_work() {
        let mut blob = wrap(b"secret", PASS, FAST).unwrap();
        blob[4..8].copy_from_slice(&u32::MAX.to_le_bytes()); // 4 TiB of memory
        assert_eq!(unwrap(&blob, PASS), Err(ExportError::BadParameters));
        let mut blob = wrap(b"secret", PASS, FAST).unwrap();
        blob[8..12].copy_from_slice(&1_000_000u32.to_le_bytes());
        assert_eq!(unwrap(&blob, PASS), Err(ExportError::BadParameters));
    }

    #[test]
    fn other_files_are_not_exports() {
        assert_eq!(unwrap(b"hello", PASS), Err(ExportError::NotAnExport));
        assert_eq!(unwrap(&[0u8; 200], PASS), Err(ExportError::NotAnExport));
    }

    #[test]
    fn payloads_roundtrip_with_and_without_a_recovery_secret() {
        let a = ExportContents { ring: vec![1, 2, 3], recovery_secret: None };
        let back = ExportContents::decode(&a.encode()).unwrap();
        assert_eq!((back.ring.clone(), back.recovery_secret), (vec![1, 2, 3], None));
        let b = ExportContents { ring: vec![9; 70], recovery_secret: Some([5; 32]) };
        let back = ExportContents::decode(&b.encode()).unwrap();
        assert_eq!((back.ring.len(), back.recovery_secret), (70, Some([5u8; 32])));
    }

    #[test]
    fn malformed_payloads_are_rejected() {
        assert!(ExportContents::decode(&[]).is_none());
        assert!(ExportContents::decode(b"CKP1\x05\x00ab").is_none());
        let good = ExportContents { ring: vec![1], recovery_secret: Some([1; 32]) }.encode();
        assert!(ExportContents::decode(&good[..good.len() - 1]).is_none());
        let mut extra = good.clone();
        extra.push(0);
        assert!(ExportContents::decode(&extra).is_none());
        assert!(ExportContents::decode(b"XXXX\x01\x00a\x00").is_none());
    }

    #[test]
    fn default_parameters_are_strong() {
        let d = KdfParams::default();
        assert!(d.m_cost_kib >= 64 * 1024 && d.t_cost >= 3);
    }

    #[test]
    fn default_parameters_actually_work_and_take_noticeable_time() {
        let start = std::time::Instant::now();
        let blob = wrap(b"k", PASS, KdfParams::default()).unwrap();
        assert_eq!(unwrap(&blob, PASS).unwrap(), b"k");
        assert!(start.elapsed() > std::time::Duration::from_millis(30), "{:?}", start.elapsed());
    }
}
