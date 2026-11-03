//! The two hashes spaCy's tagger depends on.

/// MurmurHash64A, as `spacy.strings.hash_string` uses it (seed 1, UTF-8 bytes).
pub fn murmur64a(key: &[u8], seed: u64) -> u64 {
    const M: u64 = 0xc6a4_a793_5bd1_e995;
    const R: u32 = 47;
    let mut h = seed ^ (key.len() as u64).wrapping_mul(M);
    let mut blocks = key.chunks_exact(8);
    for block in &mut blocks {
        let mut k = u64::from_le_bytes(block.try_into().unwrap());
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);
        h ^= k;
        h = h.wrapping_mul(M);
    }
    let tail = blocks.remainder();
    if !tail.is_empty() {
        for (i, &b) in tail.iter().enumerate() {
            h ^= u64::from(b) << (8 * i);
        }
        h = h.wrapping_mul(M);
    }
    h ^= h >> R;
    h = h.wrapping_mul(M);
    h ^= h >> R;
    h
}

/// thinc's `NumpyOps.hash`: MurmurHash3_x86_128 specialised to one 64-bit key, as four 32-bit words.
pub fn murmur3_uint64(val: u64, seed: u32) -> [u32; 4] {
    fn fmix(mut k: u64) -> u64 {
        k ^= k >> 33;
        k = k.wrapping_mul(0xff51_afd7_ed55_8ccd);
        k ^= k >> 33;
        k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        k ^ (k >> 33)
    }
    let seed = u64::from(seed);
    let mut h1 = val.wrapping_mul(0x87c3_7b91_1142_53d5);
    h1 = h1.rotate_left(31);
    h1 = h1.wrapping_mul(0x4cf5_ad43_2745_937f);
    h1 ^= seed ^ 8;
    let mut h2 = seed ^ 8;
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    h1 = fmix(h1);
    h2 = fmix(h2);
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    [h1 as u32, (h1 >> 32) as u32, h2 as u32, (h2 >> 32) as u32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_spacy_hash_string() {
        for (s, h) in [
            ("apple", 8566208034543834098),
            ("a", 11901859001352538922),
            ("Xxxxx", 16072095006890171862),
            ("héllo wörld!", 15695761827416483309),
            ("abcdefgh", 17751780907874141760),
            ("abcdefghi", 13885881106499164936),
        ] {
            assert_eq!(murmur64a(s.as_bytes(), 1), h, "{s}");
        }
    }

    #[test]
    fn matches_thinc_ops_hash() {
        assert_eq!(murmur3_uint64(0, 8), [0, 0, 0, 0]);
        assert_eq!(murmur3_uint64(1, 8), [1161550108, 2247783286, 1742325162, 1224191281]);
        assert_eq!(murmur3_uint64(12345678901234567890, 8), [825681028, 3746740173, 3386005190, 3472626611]);
        assert_eq!(murmur3_uint64(8566208034543834098, 8), [2653853934, 3922826080, 3980780901, 3736755472]);
    }
}
