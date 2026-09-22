//! AES-GCM with the TLS key sizes, 96-bit nonce and full 128-bit tag.
//! The caller owns nonce uniqueness, key rotation, record limits and cancellation.
use super::{aes::Aes, ghash::Ghash};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    KeyLength,
    Length,
    Authentication,
}

pub struct Key(Aes);

pub fn admits(aad_bytes: u64, payload_bytes: u64) -> bool {
    aad_bytes <= u64::MAX / 8 && payload_bytes <= (1_u64 << 36) - 32
}

fn lengths(aad: &[u8], payload: &[u8]) -> Result<(u64, u64), Error> {
    let a = u64::try_from(aad.len()).map_err(|_| Error::Length)?;
    let p = u64::try_from(payload.len()).map_err(|_| Error::Length)?;
    if !admits(a, p) {
        return Err(Error::Length);
    }
    Ok((a * 8, p * 8))
}

impl Key {
    pub fn new(key: &[u8]) -> Result<Self, Error> {
        Aes::new(key).map(Self).ok_or(Error::KeyLength)
    }

    /// Admission failure retains plaintext. No allocation occurs in this call.
    pub fn seal(
        &self,
        nonce: &[u8; 12],
        aad: &[u8],
        payload: &mut [u8],
    ) -> Result<[u8; 16], Error> {
        let bits = lengths(aad, payload)?;
        self.transform(nonce, payload);
        Ok(self.authenticate(nonce, aad, payload, bits))
    }

    /// Authenticate the complete ciphertext before releasing any plaintext.
    /// All failures retain input, including truncated and extended tags.
    pub fn open(
        &self,
        nonce: &[u8; 12],
        aad: &[u8],
        payload: &mut [u8],
        tag: &[u8],
    ) -> Result<(), Error> {
        let bits = lengths(aad, payload)?;
        let expected = self.authenticate(nonce, aad, payload, bits);
        if !super::secret::equal(&expected, tag) {
            return Err(Error::Authentication);
        }
        self.transform(nonce, payload);
        Ok(())
    }

    fn transform(&self, nonce: &[u8; 12], payload: &mut [u8]) {
        for (index, chunk) in payload.chunks_mut(16).enumerate() {
            // Admission permits at most 2^32-2 blocks, starting at counter two.
            let block = counter_block(nonce, index as u32 + 2);
            let mask = self.0.encrypt(&block);
            for (byte, key) in chunk.iter_mut().zip(&mask.0) {
                *byte ^= key;
            }
        }
    }

    fn authenticate(
        &self,
        nonce: &[u8; 12],
        aad: &[u8],
        ciphertext: &[u8],
        bits: (u64, u64),
    ) -> [u8; 16] {
        let hash_key = self.0.encrypt(&[0; 16]);
        let mut hash = Ghash::new(&hash_key.0);
        drop(hash_key);
        hash.field(aad);
        hash.field(ciphertext);
        let mut lengths = [0; 16];
        lengths[..8].copy_from_slice(&bits.0.to_be_bytes());
        lengths[8..].copy_from_slice(&bits.1.to_be_bytes());
        hash.block(&lengths);
        let mut result = hash.finish();
        let mask = self.0.encrypt(&counter_block(nonce, 1));
        for (byte, key) in result.iter_mut().zip(&mask.0) {
            *byte ^= key;
        }
        result
    }
}

fn counter_block(nonce: &[u8; 12], counter: u32) -> [u8; 16] {
    let mut result = [0; 16];
    result[..12].copy_from_slice(nonce);
    result[12..].copy_from_slice(&counter.to_be_bytes());
    result
}
