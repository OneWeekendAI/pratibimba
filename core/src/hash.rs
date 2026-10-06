//! Stable bucketing. `std`'s hasher is not stable across Rust versions, and a visitor's holdout
//! group must not move when the binary is upgraded, so this is plain FNV-1a.

const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

pub fn fnv1a64(parts: &[&[u8]]) -> u64 {
    let mut h = OFFSET;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            // A separator, so ("ab","c") and ("a","bc") land in different buckets.
            h ^= 0xff;
            h = h.wrapping_mul(PRIME);
        }
        for b in *part {
            h ^= u64::from(*b);
            h = h.wrapping_mul(PRIME);
        }
    }
    h
}

/// MurmurHash3's 64-bit finaliser. FNV-1a alone barely moves its high bits for the last few
/// input bytes, so ids like `v1`, `v2`, ... would bunch into the same buckets.
pub fn fmix64(mut k: u64) -> u64 {
    k ^= k >> 33;
    k = k.wrapping_mul(0xff51_afd7_ed55_8ccd);
    k ^= k >> 33;
    k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    k ^= k >> 33;
    k
}

/// A point in [0, 1) for the given parts.
pub fn unit(parts: &[&[u8]]) -> f64 {
    // The top 53 bits make an exactly representable fraction.
    (fmix64(fnv1a64(parts)) >> 11) as f64 / (1u64 << 53) as f64
}

/// Whether a visitor is in a site's holdout. Per visitor per site, never per decision.
pub fn in_holdout(site: &str, subject: &str, fraction: f64) -> bool {
    unit(&[b"holdout", site.as_bytes(), subject.as_bytes()]) < fraction
}

/// Whether a visitor may see a variant approved with a planned traffic share.
pub fn within_share(variant_id: i64, subject: &str, share: f64) -> bool {
    unit(&[b"share", &variant_id.to_le_bytes(), subject.as_bytes()]) < share
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_the_reference_vector() {
        // FNV-1a 64 of "a" from the reference implementation.
        assert_eq!(fnv1a64(&[b"a"]), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn separator_distinguishes_splits() {
        assert_ne!(fnv1a64(&[b"ab", b"c"]), fnv1a64(&[b"a", b"bc"]));
    }

    #[test]
    fn unit_is_roughly_uniform() {
        let below = (0..10_000).filter(|i| unit(&[i.to_string().as_bytes()]) < 0.1).count();
        assert!((900..=1100).contains(&below), "{below}");
    }

    #[test]
    fn sequential_ids_spread_over_buckets() {
        // Ids that differ only in their last characters, as real subject ids often do.
        let below = (0..2000).filter(|i| unit(&[b"site", format!("s{i}").as_bytes()]) < 0.1).count();
        assert!((150..=250).contains(&below), "{below}/2000 below 0.1");
    }
}
