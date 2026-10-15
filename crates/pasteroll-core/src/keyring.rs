//! The set of master keys a history may currently be sealed under.
//!
//! Normally one key. During a rotation there are two for a short time (the new key first, then the
//! previous one), so a crash at any point leaves a state that still opens: whichever key matches
//! the database is used, and the other is dropped on the next start.
//!
//! The encoded form is what the platform protects (DPAPI, Keychain...) and stores. A bare 32-byte
//! value is accepted too: that is what the first version wrote.

use crate::vault::MASTER_LEN;
use zeroize::Zeroize;

const MAGIC: &[u8; 4] = b"CKR1";
const MAX_KEYS: usize = 2;

pub struct KeyRing {
    keys: Vec<[u8; MASTER_LEN]>,
}

impl KeyRing {
    pub fn new(current: [u8; MASTER_LEN]) -> KeyRing {
        KeyRing { keys: vec![current] }
    }

    /// A ring in the middle of a rotation: `current` is the new key, `previous` the one the
    /// database may still be sealed under.
    pub fn rotating(current: [u8; MASTER_LEN], previous: [u8; MASTER_LEN]) -> KeyRing {
        KeyRing { keys: vec![current, previous] }
    }

    pub fn current(&self) -> &[u8; MASTER_LEN] {
        &self.keys[0]
    }

    pub fn previous(&self) -> Option<&[u8; MASTER_LEN]> {
        self.keys.get(1)
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Every key, newest first.
    pub fn iter(&self) -> impl Iterator<Item = &[u8; MASTER_LEN]> {
        self.keys.iter()
    }

    /// A ring holding only the first key (the one a finished rotation keeps).
    pub fn without_previous(&self) -> KeyRing {
        KeyRing { keys: vec![self.keys[0]] }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(5 + self.keys.len() * MASTER_LEN);
        v.extend_from_slice(MAGIC);
        v.push(self.keys.len() as u8);
        for k in &self.keys {
            v.extend_from_slice(k);
        }
        v
    }

    pub fn decode(b: &[u8]) -> Option<KeyRing> {
        if b.len() == MASTER_LEN {
            return Some(KeyRing::new(b.try_into().ok()?));
        }
        if b.len() < 5 || &b[..4] != MAGIC {
            return None;
        }
        let n = b[4] as usize;
        if n == 0 || n > MAX_KEYS || b.len() != 5 + n * MASTER_LEN {
            return None;
        }
        let keys = b[5..].chunks_exact(MASTER_LEN).map(|c| c.try_into().unwrap()).collect();
        Some(KeyRing { keys })
    }
}

impl Drop for KeyRing {
    fn drop(&mut self) {
        for k in &mut self.keys {
            k.zeroize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_one_and_two_keys() {
        let r = KeyRing::new([1; 32]);
        let back = KeyRing::decode(&r.encode()).unwrap();
        assert_eq!((back.len(), back.current(), back.previous()), (1, &[1u8; 32], None));
        let r = KeyRing::rotating([2; 32], [1; 32]);
        let back = KeyRing::decode(&r.encode()).unwrap();
        assert_eq!((back.current(), back.previous()), (&[2u8; 32], Some(&[1u8; 32])));
    }

    #[test]
    fn a_bare_key_from_the_first_version_is_accepted() {
        let r = KeyRing::decode(&[9u8; 32]).unwrap();
        assert_eq!(r.current(), &[9u8; 32]);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(KeyRing::decode(&[]).is_none());
        assert!(KeyRing::decode(&[0; 31]).is_none());
        assert!(KeyRing::decode(b"XXXX\x01").is_none());
        let mut bad = KeyRing::new([1; 32]).encode();
        bad.push(0);
        assert!(KeyRing::decode(&bad).is_none());
        let mut three = vec![];
        three.extend_from_slice(b"CKR1");
        three.push(3);
        three.extend_from_slice(&[0u8; 96]);
        assert!(KeyRing::decode(&three).is_none());
        let mut zero = vec![];
        zero.extend_from_slice(b"CKR1");
        zero.push(0);
        assert!(KeyRing::decode(&zero).is_none());
    }

    #[test]
    fn without_previous_keeps_the_new_key_only() {
        let r = KeyRing::rotating([2; 32], [1; 32]).without_previous();
        assert_eq!((r.len(), r.current()), (1, &[2u8; 32]));
    }
}
