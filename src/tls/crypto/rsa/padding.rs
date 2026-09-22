//! Exact RFC 8017 encodings, with SHA-256/384 and same-hash, hash-width PSS salt.
use super::Hash;
pub(super) fn pkcs1(hash: Hash, digest: &[u8], encoded: &[u8]) -> bool {
    let width = hash.bytes();
    let id = match hash {
        Hash::Sha256 => 1,
        Hash::Sha384 => 2,
    };
    let prefix = [
        0x30,
        17 + width as u8,
        0x30,
        0x0d,
        0x06,
        0x09,
        0x60,
        0x86,
        0x48,
        0x01,
        0x65,
        0x03,
        0x04,
        0x02,
        id,
        0x05,
        0x00,
        0x04,
        width as u8,
    ];
    let Some(separator) = encoded.len().checked_sub(prefix.len() + width + 1) else {
        return false;
    };
    digest.len() == width
        && separator >= 10
        && encoded[..2] == [0, 1]
        && encoded[2..separator].iter().all(|b| *b == 255)
        && encoded[separator] == 0
        && encoded[separator + 1..separator + 1 + prefix.len()] == prefix
        && encoded[separator + 1 + prefix.len()..] == *digest
}
pub(super) fn pss(hash: Hash, digest: &[u8], recovered: &[u8], bits: usize) -> bool {
    let width = hash.bytes();
    if !(2048..=4096).contains(&bits)
        || recovered.len() != bits.div_ceil(8)
        || digest.len() != width
    {
        return false;
    }
    let encoded_bits = bits - 1;
    let length = encoded_bits.div_ceil(8);
    let prefix = recovered.len() - length;
    if recovered[..prefix].iter().any(|b| *b != 0) {
        return false;
    }
    let encoded = &recovered[prefix..];
    if length < 2 * width + 2 || encoded.last() != Some(&0xbc) {
        return false;
    }
    let db_len = length - width - 1;
    let mask = 255_u8 >> (8 * length - encoded_bits);
    if encoded[0] & !mask != 0 {
        return false;
    }
    let h = &encoded[db_len..db_len + width];
    let mut db = encoded[..db_len].to_vec();
    for (counter, chunk) in db.chunks_mut(width).enumerate() {
        let mgf = hash.digest(&[h, &(counter as u32).to_be_bytes()]);
        for (b, m) in chunk.iter_mut().zip(mgf) {
            *b ^= m;
        }
    }
    db[0] &= mask;
    let separator = db_len - width - 1;
    if db[..separator].iter().any(|b| *b != 0) || db[separator] != 1 {
        return false;
    }
    h == hash.digest(&[&[0; 8], digest, &db[separator + 1..]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::crypto::signature_tests::decode;
    #[test]
    fn every_padding_byte_is_authenticated() {
        for row in include_str!("../../../../tests/fixtures/rsa-signatures.txt")
            .lines()
            .filter(|s| !s.starts_with('#'))
        {
            let f: Vec<_> = row.split_whitespace().collect();
            let bits: usize = f[0].parse().unwrap();
            let hash = if f[1] == "sha256" {
                Hash::Sha256
            } else {
                Hash::Sha384
            };
            let mut encoded = decode(f[7]);
            while encoded.len() < bits.div_ceil(8) {
                encoded.insert(0, 0);
            }
            let digest = hash.digest(&[&decode(f[3])]);
            let check = |bytes: &[u8]| {
                if f[2] == "pss" {
                    pss(hash, &digest, bytes, bits)
                } else {
                    pkcs1(hash, &digest, bytes)
                }
            };
            assert!(check(&encoded));
            for i in 0..encoded.len() {
                encoded[i] ^= 1;
                assert!(!check(&encoded), "{i}");
                encoded[i] ^= 1;
            }
        }
    }
}
