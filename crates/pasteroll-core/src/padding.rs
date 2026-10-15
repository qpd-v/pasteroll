//! Length hiding.
//!
//! The ciphertext of a clip is as long as the clip, so someone holding only the database file could
//! tell a one-line clip from a 4 MB screenshot, or spot two clips of identical length. Bodies are
//! therefore padded to size buckets before sealing: every size in a bucket looks the same.
//!
//! Buckets start at 256 bytes and grow by about 12.5%, so the overhead is bounded (at most ~12.5%
//! plus 64 bytes) and a 16 MB clip lands in one of a few dozen buckets.
//!
//! Layout: `u32 LE original length | data | zeros`.

const MIN_BUCKET: usize = 256;
const HEADER: usize = 4;

/// The padded size for a body of `n` bytes.
pub fn bucket(n: usize) -> usize {
    let need = n + HEADER;
    let mut b = MIN_BUCKET;
    while b < need {
        b = (b + b / 8).next_multiple_of(64);
    }
    b
}

pub fn pad(plain: &[u8]) -> Vec<u8> {
    let total = bucket(plain.len());
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(plain.len() as u32).to_le_bytes());
    out.extend_from_slice(plain);
    out.resize(total, 0);
    out
}

/// `None` if the length header is inconsistent with the buffer (corruption).
pub fn unpad(padded: &[u8]) -> Option<&[u8]> {
    let len = u32::from_le_bytes(padded.get(..HEADER)?.try_into().ok()?) as usize;
    padded.get(HEADER..HEADER.checked_add(len)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_for_many_sizes() {
        for n in [0, 1, 2, 251, 252, 253, 255, 256, 257, 1000, 4096, 100_000] {
            let data: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
            let p = pad(&data);
            assert_eq!(p.len(), bucket(n));
            assert_eq!(unpad(&p), Some(&data[..]), "size {n}");
        }
    }

    #[test]
    fn small_clips_are_indistinguishable() {
        let sizes: Vec<usize> = [0usize, 5, 40, 100, 200, 250].iter().map(|&n| pad(&vec![7; n]).len()).collect();
        assert!(sizes.iter().all(|&s| s == 256), "{sizes:?}");
    }

    #[test]
    fn buckets_are_monotonic_and_overhead_is_bounded() {
        let mut last = 0;
        for n in (0..2_000_000).step_by(997) {
            let b = bucket(n);
            assert!(b >= n + 4);
            assert!(b >= last, "bucket shrank at {n}");
            // at most one growth step (12.5%) plus alignment above what was needed
            assert!(b <= (n + 4) + (n + 4) / 8 + 64 + 256, "n {n} -> {b}");
            last = b;
        }
    }

    #[test]
    fn nearby_sizes_share_a_bucket() {
        assert_eq!(bucket(100_000), bucket(100_050));
        assert_ne!(bucket(100_000), bucket(200_000));
    }

    #[test]
    fn corrupt_headers_are_rejected() {
        assert_eq!(unpad(&[]), None);
        assert_eq!(unpad(&[1, 2]), None);
        let mut p = pad(b"hello");
        p[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(unpad(&p), None);
    }

    #[test]
    fn sixteen_megabytes_is_one_modest_bucket() {
        let b = bucket(16 * 1024 * 1024);
        assert!(b < 16 * 1024 * 1024 * 9 / 8 + 1024, "{b}");
    }
}
