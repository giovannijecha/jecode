//! SP 800-38D GHASH, using fixed bit steps and no secret-indexed table.

pub(crate) struct Ghash {
    key: u128,
    state: u128,
}

impl Ghash {
    pub(crate) fn new(key: &[u8; 16]) -> Self {
        Self {
            key: u128::from_be_bytes(*key),
            state: 0,
        }
    }

    /// Authenticate one logical field, zero-padding its last block. Separate
    /// calls denote separate padded fields, not arbitrary message fragments.
    pub(crate) fn field(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(16);
        for chunk in &mut chunks {
            self.block(chunk.try_into().expect("GHASH block"));
        }
        let tail = chunks.remainder();
        if !tail.is_empty() {
            let mut padded = [0; 16];
            padded[..tail.len()].copy_from_slice(tail);
            self.block(&padded);
        }
    }

    pub(crate) fn block(&mut self, block: &[u8; 16]) {
        self.state = multiply(self.state ^ u128::from_be_bytes(*block), self.key);
    }

    pub(crate) fn finish(self) -> [u8; 16] {
        self.state.to_be_bytes()
    }
}

// Keep one instruction-review boundary for every authentication and fixture caller.
#[inline(never)]
pub(crate) fn multiply(mut left: u128, mut right: u128) -> u128 {
    let mut product = 0;
    // The most significant wire bit is the constant polynomial coefficient.
    for _ in 0..128 {
        product ^= right & 0_u128.wrapping_sub(left >> 127);
        let reduce = 0_u128.wrapping_sub(right & 1);
        right = (right >> 1) ^ (reduce & (0xe1_u128 << 120));
        left <<= 1;
    }
    product
}

impl Drop for Ghash {
    fn drop(&mut self) {
        super::secret::erase(std::slice::from_mut(&mut self.key));
        super::secret::erase(std::slice::from_mut(&mut self.state));
    }
}
