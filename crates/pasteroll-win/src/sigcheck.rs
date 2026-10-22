//! Checks the release signature on an update manifest with Windows' own crypto (CNG): ECDSA over
//! P-256 and SHA-256. Only public keys are ever involved here.

use pasteroll_core::update_manifest::{sha256, SignatureVerifier};
use windows::core::PCWSTR;
use windows::Win32::Security::Cryptography::*;

/// `BCRYPT_ECDSA_PUBLIC_P256_MAGIC`: "ECS1".
const P256_PUBLIC_MAGIC: u32 = 0x3153_4345;

pub struct CngVerifier;

/// Whether `key` is a point that Windows accepts as a P-256 public key.
pub fn is_valid_public_key(key: &[u8; 64]) -> bool {
    let mut blob = Vec::with_capacity(72);
    blob.extend_from_slice(&P256_PUBLIC_MAGIC.to_le_bytes());
    blob.extend_from_slice(&32u32.to_le_bytes());
    blob.extend_from_slice(key);
    unsafe {
        let mut provider = BCRYPT_ALG_HANDLE::default();
        if BCryptOpenAlgorithmProvider(&mut provider, BCRYPT_ECDSA_P256_ALGORITHM, PCWSTR::null(), BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0)).is_err() {
            return false;
        }
        let mut handle = BCRYPT_KEY_HANDLE::default();
        let ok = BCryptImportKeyPair(provider, None, BCRYPT_ECCPUBLIC_BLOB, &mut handle, &mut blob, 0).is_ok();
        if ok {
            let _ = BCryptDestroyKey(handle);
        }
        let _ = BCryptCloseAlgorithmProvider(provider, 0);
        ok
    }
}

impl SignatureVerifier for CngVerifier {
    fn verify(&self, key: &[u8; 64], message: &[u8], signature: &[u8; 64]) -> bool {
        // the blob CNG wants: magic, the length of one coordinate, then X and Y
        let mut blob = Vec::with_capacity(72);
        blob.extend_from_slice(&P256_PUBLIC_MAGIC.to_le_bytes());
        blob.extend_from_slice(&32u32.to_le_bytes());
        blob.extend_from_slice(key);
        let hash = sha256(message);
        unsafe {
            let mut provider = BCRYPT_ALG_HANDLE::default();
            if BCryptOpenAlgorithmProvider(&mut provider, BCRYPT_ECDSA_P256_ALGORITHM, PCWSTR::null(), BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0))
                .is_err()
            {
                return false;
            }
            let mut handle = BCRYPT_KEY_HANDLE::default();
            // refused for a point that is not on the curve
            let imported = BCryptImportKeyPair(provider, None, BCRYPT_ECCPUBLIC_BLOB, &mut handle, &mut blob, 0).is_ok();
            let valid = imported && BCryptVerifySignature(handle, None, &hash, signature, BCRYPT_FLAGS(0)).is_ok();
            if imported {
                let _ = BCryptDestroyKey(handle);
            }
            let _ = BCryptCloseAlgorithmProvider(provider, 0);
            valid
        }
    }
}

/// A throwaway signing key made by Windows for tests: nothing is stored, the key lives and dies
/// with the value. (The real signing key is never in code: see `scripts/release-keys.ps1`.)
#[cfg(test)]
pub struct TestSigner {
    provider: BCRYPT_ALG_HANDLE,
    key: BCRYPT_KEY_HANDLE,
    public: [u8; 64],
}

#[cfg(test)]
impl TestSigner {
    pub fn new() -> TestSigner {
        unsafe {
            let mut provider = BCRYPT_ALG_HANDLE::default();
            BCryptOpenAlgorithmProvider(&mut provider, BCRYPT_ECDSA_P256_ALGORITHM, PCWSTR::null(), BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0)).ok().unwrap();
            let mut key = BCRYPT_KEY_HANDLE::default();
            BCryptGenerateKeyPair(provider, &mut key, 256, 0).ok().unwrap();
            BCryptFinalizeKeyPair(key, 0).ok().unwrap();
            let mut blob = [0u8; 72];
            let mut written = 0u32;
            BCryptExportKey(key, None, BCRYPT_ECCPUBLIC_BLOB, Some(&mut blob), &mut written, 0).ok().unwrap();
            let mut public = [0u8; 64];
            public.copy_from_slice(&blob[8..72]);
            TestSigner { provider, key, public }
        }
    }

    pub fn public(&self) -> [u8; 64] {
        self.public
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        let hash = sha256(message);
        let mut sig = [0u8; 64];
        let mut written = 0u32;
        unsafe {
            BCryptSignHash(self.key, None, &hash, Some(&mut sig), &mut written, BCRYPT_FLAGS(0)).ok().unwrap();
        }
        assert_eq!(written, 64);
        sig
    }
}

#[cfg(test)]
impl Drop for TestSigner {
    fn drop(&mut self) {
        unsafe {
            let _ = BCryptDestroyKey(self.key);
            let _ = BCryptCloseAlgorithmProvider(self.provider, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pasteroll_core::update_manifest::decode_signature;

    // Made with scripts/release-keys.ps1 using a throwaway key (its private half was deleted):
    // the signature of the 15 bytes "manifest body\r\n".
    const TEST_PUBLIC: &str = "iLKUiPbKKwUkiK5J8gUEuuokXKGLmH7/h66pipvB8Yi/CCueH0sVi8s44hDqvvEYGCYmEzehFu+M0R8tcWyaEQ==";
    const TEST_SIGNATURE: &str = "ezKlv+4/nKtX/5MthGSVculm33uzFdZi8jMXrUYNECWW4r2NTjF0z0mp3IiXlHKyMFGwxthYV40MG+wa+w7lZw==";
    const MESSAGE: &[u8] = b"manifest body\r\n";

    fn key(text: &str) -> [u8; 64] {
        decode_signature(text).expect("64 bytes")
    }

    #[test]
    fn a_key_made_for_tests_signs_what_the_verifier_accepts() {
        let signer = TestSigner::new();
        let sig = signer.sign(MESSAGE);
        assert!(CngVerifier.verify(&signer.public(), MESSAGE, &sig));
        assert!(!CngVerifier.verify(&signer.public(), b"other", &sig));
        assert!(!CngVerifier.verify(&TestSigner::new().public(), MESSAGE, &sig));
    }

    #[test]
    fn out_of_range_and_zero_signature_parts_are_refused() {
        // the order of the P-256 group
        let n: [u8; 32] = [
            0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xbc, 0xe6, 0xfa, 0xad, 0xa7, 0x17, 0x9e, 0x84,
            0xf3, 0xb9, 0xca, 0xc2, 0xfc, 0x63, 0x25, 0x51,
        ];
        let (k, good) = (key(TEST_PUBLIC), key(TEST_SIGNATURE));
        let with = |r: Option<[u8; 32]>, s: Option<[u8; 32]>| {
            let mut sig = good;
            if let Some(r) = r {
                sig[..32].copy_from_slice(&r);
            }
            if let Some(s) = s {
                sig[32..].copy_from_slice(&s);
            }
            sig
        };
        assert!(!CngVerifier.verify(&k, MESSAGE, &with(Some(n), None)), "r = n");
        assert!(!CngVerifier.verify(&k, MESSAGE, &with(None, Some(n))), "s = n");
        assert!(!CngVerifier.verify(&k, MESSAGE, &with(Some([0; 32]), None)), "r = 0");
        assert!(!CngVerifier.verify(&k, MESSAGE, &with(None, Some([0; 32]))), "s = 0");
        assert!(!CngVerifier.verify(&k, MESSAGE, &with(Some([0xff; 32]), Some([0xff; 32]))), "both above n");
        // the signature itself is still fine, so the cases above fail because of what was changed
        assert!(CngVerifier.verify(&k, MESSAGE, &good));
    }

    #[test]
    fn the_built_in_release_keys_are_real_curve_points() {
        for (i, k) in pasteroll_core::update_manifest::PUBLIC_KEYS.iter().enumerate() {
            assert!(is_valid_public_key(k), "key {i}");
        }
        assert!(is_valid_public_key(&key(TEST_PUBLIC)));
        assert!(!is_valid_public_key(&[7; 64]), "not on the curve");
    }

    #[test]
    fn a_signature_from_the_matching_key_verifies() {
        assert!(CngVerifier.verify(&key(TEST_PUBLIC), MESSAGE, &key(TEST_SIGNATURE)));
    }

    #[test]
    fn nothing_else_verifies() {
        let (k, s) = (key(TEST_PUBLIC), key(TEST_SIGNATURE));
        // another message
        assert!(!CngVerifier.verify(&k, b"manifest body\n", &s));
        assert!(!CngVerifier.verify(&k, b"", &s));
        // a flipped bit anywhere in the signature
        for i in [0usize, 31, 32, 63] {
            let mut bad = s;
            bad[i] ^= 1;
            assert!(!CngVerifier.verify(&k, MESSAGE, &bad), "bit in byte {i}");
        }
        // a different key, an all-zero key and a point that is not on the curve
        let mut other = k;
        other[5] ^= 1;
        assert!(!CngVerifier.verify(&other, MESSAGE, &s));
        assert!(!CngVerifier.verify(&[0; 64], MESSAGE, &s));
        assert!(!CngVerifier.verify(&[7; 64], MESSAGE, &s));
        // an all-zero signature
        assert!(!CngVerifier.verify(&k, MESSAGE, &[0; 64]));
    }
}
