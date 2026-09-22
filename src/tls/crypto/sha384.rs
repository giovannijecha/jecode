//! SHA-384 for public certificate signatures; FIPS 180-4.
use super::{secret::erase, sha384_compress::block};

pub(in crate::tls) struct Sha384 {
    state: [u64; 8],
    buffer: [u8; 128],
    used: usize,
    bytes: u128,
}
impl Sha384 {
    pub fn new() -> Self {
        Self {
            state: [
                0xcbbb9d5dc1059ed8,
                0x629a292a367cd507,
                0x9159015a3070dd17,
                0x152fecd8f70e5939,
                0x67332667ffc00b31,
                0x8eb44a8768581511,
                0xdb0c2e0d64f98fa7,
                0x47b5481dbefa4fa4,
            ],
            buffer: [0; 128],
            used: 0,
            bytes: 0,
        }
    }
    #[cfg(test)]
    pub fn digest(input: &[u8]) -> [u8; 48] {
        let mut hash = Self::new();
        hash.update(input);
        hash.finish()
    }
    pub fn update(&mut self, mut input: &[u8]) {
        self.bytes = self
            .bytes
            .checked_add(input.len() as u128)
            .filter(|n| *n <= u128::MAX / 8)
            .expect("SHA-384 length");
        if self.used != 0 {
            let take = input.len().min(128 - self.used);
            self.buffer[self.used..self.used + take].copy_from_slice(&input[..take]);
            self.used += take;
            input = &input[take..];
            if self.used != 128 {
                return;
            }
            block(&mut self.state, &self.buffer);
            self.used = 0;
        }
        let mut chunks = input.chunks_exact(128);
        for chunk in &mut chunks {
            block(&mut self.state, chunk.try_into().expect("block"));
        }
        let tail = chunks.remainder();
        self.buffer[..tail.len()].copy_from_slice(tail);
        self.used = tail.len();
    }
    pub fn finish(mut self) -> [u8; 48] {
        self.buffer[self.used] = 0x80;
        self.buffer[self.used + 1..].fill(0);
        if self.used >= 112 {
            block(&mut self.state, &self.buffer);
            self.buffer.fill(0);
        }
        self.buffer[112..].copy_from_slice(&(self.bytes * 8).to_be_bytes());
        block(&mut self.state, &self.buffer);
        let mut output = [0; 48];
        for (word, bytes) in self.state.iter().zip(output.chunks_exact_mut(8)) {
            bytes.copy_from_slice(&word.to_be_bytes());
        }
        output
    }
}
impl Drop for Sha384 {
    fn drop(&mut self) {
        erase(&mut self.state);
        erase(&mut self.buffer);
    }
}
