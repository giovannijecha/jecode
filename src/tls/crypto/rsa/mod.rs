//! Bounded RSA public verification; adapted from owned archive arithmetic.
//! Variable-time public operands only. Never use this for private operations.
mod arithmetic;
mod modulus;
mod padding;
use super::signature::Hash;
use modulus::Modulus;

pub(in crate::tls) struct PublicKey {
    modulus: Modulus,
    exponent: u64,
}
impl PublicKey {
    pub fn from_components(modulus: &[u8], exponent: &[u8]) -> Option<Self> {
        if !(256..=512).contains(&modulus.len())
            || (modulus.len() == 256 && modulus[0] < 128)
            || exponent.is_empty()
            || exponent.len() > 5
            || exponent[0] == 0
        {
            return None;
        }
        let exponent = exponent.iter().fold(0_u64, |n, b| (n << 8) | u64::from(*b));
        if !(3..(1_u64 << 33)).contains(&exponent) || exponent & 1 == 0 {
            return None;
        }
        Some(Self {
            modulus: Modulus::new(modulus)?,
            exponent,
        })
    }
    fn recover(&self, signature: &[u8]) -> Option<Vec<u8>> {
        if signature.len() != self.modulus.bytes {
            return None;
        }
        let value = modulus::load(signature);
        if value.iter().all(|w| *w == 0) || !self.modulus.contains(&value) {
            return None;
        }
        let plain = self.modulus.power(&value, self.exponent);
        let mut result = vec![0; self.modulus.bytes];
        for (i, byte) in result.iter_mut().rev().enumerate() {
            *byte = (plain[i / 8] >> (8 * (i % 8))) as u8;
        }
        Some(result)
    }
    pub fn verify(&self, hash: Hash, pss: bool, message: &[u8], signature: &[u8]) -> bool {
        let Some(encoded) = self.recover(signature) else {
            return false;
        };
        let digest = hash.digest(&[message]);
        if pss {
            padding::pss(hash, &digest, &encoded, self.modulus.bits)
        } else {
            padding::pkcs1(hash, &digest, &encoded)
        }
    }
}
