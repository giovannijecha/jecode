//! Owned non-cryptographic fingerprints for frame checksums and content lookup.
//! A matching digest only nominates a reuse candidate; callers compare bytes.

const LANE: [u64; 2] = [0x9e37_79b9_7f4a_7c15, 0xc2b2_ae3d_27d4_eb4f];

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Digest(pub [u64; 2]);

pub fn digest(bytes: &[u8]) -> Digest {
    let length = bytes.len() as u64;
    let mut a = 0x243f_6a88_85a3_08d3 ^ length;
    let mut b = 0x1319_8a2e_0370_7344 ^ length.rotate_left(32);
    let mut blocks = bytes.chunks_exact(16);
    for block in &mut blocks {
        let (x, y) = block.split_at(8);
        a = round(a, word(x), LANE[0]);
        b = round(b, word(y), LANE[1]);
        a = a.wrapping_add(b.rotate_left(17));
    }
    let tail = blocks.remainder();
    if !tail.is_empty() {
        let mut padded = [0u8; 16];
        padded[..tail.len()].copy_from_slice(tail);
        let (x, y) = padded.split_at(8);
        a = round(a, word(x) ^ tail.len() as u64, LANE[0]);
        b = round(b, word(y), LANE[1]);
    }
    Digest([finish(a ^ b.rotate_left(29)), finish(b ^ a.rotate_left(41))])
}

/// A 64-bit checksum for framed records.
pub fn checksum(parts: &[&[u8]]) -> u64 {
    let mut value = 0x6a09_e667_f3bc_c908u64;
    for part in parts {
        let lane = digest(part);
        value = finish(value ^ lane.0[0]).wrapping_add(lane.0[1]);
    }
    finish(value)
}

fn word(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().unwrap_or_default())
}

fn round(state: u64, input: u64, multiplier: u64) -> u64 {
    (state ^ input).wrapping_mul(multiplier).rotate_left(31)
}

fn finish(mut value: u64) -> u64 {
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51_afd7_ed55_8ccd);
    value ^= value >> 33;
    value = value.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    value ^ (value >> 33)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_depend_on_every_byte_and_the_length() {
        let base = vec![7u8; 4096];
        let mut seen = std::collections::HashSet::new();
        assert!(seen.insert(digest(&base)));
        for index in [0, 15, 16, 4095] {
            let mut changed = base.clone();
            changed[index] ^= 1;
            assert!(seen.insert(digest(&changed)), "byte {index}");
        }
        assert!(seen.insert(digest(&base[..4095])));
        assert!(seen.insert(digest(&[])));
        assert!(seen.insert(digest(&[0])));
        assert_ne!(checksum(&[b"ab", b"c"]), checksum(&[b"a", b"bc"]));
    }
}
