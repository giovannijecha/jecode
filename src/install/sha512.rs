//! SHA-512 (NIST FIPS 180-4, sections 5.1.2, 5.3.5 and 6.4) for npm's
//! `sha512-` integrity strings. The initial values and round constants are
//! derived as the standard defines them, from the first 8 and 80 primes,
//! and checked against the published test vectors.

use std::sync::OnceLock;

struct Constants {
    initial: [u64; 8],
    rounds: [u64; 80],
}

fn primes(count: usize) -> Vec<u64> {
    let mut primes = Vec::with_capacity(count);
    let mut candidate = 2u64;
    while primes.len() < count {
        if primes
            .iter()
            .take_while(|prime| *prime * *prime <= candidate)
            .all(|prime| !candidate.is_multiple_of(*prime))
        {
            primes.push(candidate);
        }
        candidate += 1;
    }
    primes
}

/// Little-endian 64-bit limbs.
fn multiply(a: &[u64], b: &[u64]) -> Vec<u64> {
    let mut product = vec![0u64; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        let mut carry = 0u128;
        for (j, &y) in b.iter().enumerate() {
            let sum = u128::from(product[i + j]) + u128::from(x) * u128::from(y) + carry;
            product[i + j] = sum as u64;
            carry = sum >> 64;
        }
        product[i + b.len()] = carry as u64;
    }
    product
}

fn not_above(value: &[u64], limit: &[u64]) -> bool {
    let length = value.len().max(limit.len());
    for index in (0..length).rev() {
        let left = value.get(index).copied().unwrap_or(0);
        let right = limit.get(index).copied().unwrap_or(0);
        if left != right {
            return left < right;
        }
    }
    true
}

/// The largest x with x^power <= prime * 2^(64 * power): the root's first
/// 64 fractional bits are its low limb.
fn root_bits(prime: u64, power: usize) -> u64 {
    let mut limit = vec![0u64; power + 1];
    limit[power] = prime;
    let (mut low, mut high) = (0u128, 1u128 << 72);
    while low + 1 < high {
        let middle = (low + high) / 2;
        let limbs = [middle as u64, (middle >> 64) as u64];
        let mut value = limbs.to_vec();
        for _ in 1..power {
            value = multiply(&value, &limbs);
        }
        if not_above(&value, &limit) {
            low = middle;
        } else {
            high = middle;
        }
    }
    low as u64
}

fn constants() -> &'static Constants {
    static CONSTANTS: OnceLock<Constants> = OnceLock::new();
    CONSTANTS.get_or_init(|| {
        let primes = primes(80);
        let mut initial = [0u64; 8];
        let mut rounds = [0u64; 80];
        for (index, prime) in primes.iter().enumerate() {
            if index < 8 {
                initial[index] = root_bits(*prime, 2);
            }
            rounds[index] = root_bits(*prime, 3);
        }
        Constants { initial, rounds }
    })
}

pub struct Sha512 {
    state: [u64; 8],
    buffer: Vec<u8>,
    length: u128,
}

impl Default for Sha512 {
    fn default() -> Self {
        Self {
            state: constants().initial,
            buffer: Vec::with_capacity(128),
            length: 0,
        }
    }
}

impl Sha512 {
    pub fn update(&mut self, mut data: &[u8]) {
        self.length += data.len() as u128;
        if !self.buffer.is_empty() {
            let take = (128 - self.buffer.len()).min(data.len());
            self.buffer.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buffer.len() == 128 {
                let block = std::mem::take(&mut self.buffer);
                self.compress(&block);
            }
        }
        let mut blocks = data.chunks_exact(128);
        for block in &mut blocks {
            self.compress(block);
        }
        self.buffer.extend_from_slice(blocks.remainder());
    }

    pub fn finish(mut self) -> [u8; 64] {
        let bits = self.length * 8;
        let mut tail = std::mem::take(&mut self.buffer);
        tail.push(0x80);
        while tail.len() % 128 != 112 {
            tail.push(0);
        }
        tail.extend_from_slice(&bits.to_be_bytes());
        for block in tail.chunks_exact(128) {
            self.compress(block);
        }
        let mut digest = [0u8; 64];
        for (chunk, word) in digest.chunks_exact_mut(8).zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    fn compress(&mut self, block: &[u8]) {
        let rounds = &constants().rounds;
        let mut words = [0u64; 80];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            *word = u64::from_be_bytes(block[index * 8..index * 8 + 8].try_into().unwrap());
        }
        for index in 16..80 {
            let a = words[index - 15];
            let b = words[index - 2];
            let small_a = a.rotate_right(1) ^ a.rotate_right(8) ^ (a >> 7);
            let small_b = b.rotate_right(19) ^ b.rotate_right(61) ^ (b >> 6);
            words[index] = words[index - 16]
                .wrapping_add(small_a)
                .wrapping_add(words[index - 7])
                .wrapping_add(small_b);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for (round, word) in words.into_iter().enumerate() {
            let upper_e = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let choose = (e & f) ^ (!e & g);
            let first = h
                .wrapping_add(upper_e)
                .wrapping_add(choose)
                .wrapping_add(rounds[round])
                .wrapping_add(word);
            let upper_a = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let second = upper_a.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(first);
            d = c;
            c = b;
            b = a;
            a = first.wrapping_add(second);
        }
        for (saved, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *saved = saved.wrapping_add(value);
        }
    }
}

pub fn digest(data: &[u8]) -> [u8; 64] {
    let mut hash = Sha512::default();
    hash.update(data);
    hash.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn derived_constants_match_the_standard() {
        let constants = constants();
        assert_eq!(constants.initial[0], 0x6a09e667f3bcc908);
        assert_eq!(constants.initial[7], 0x5be0cd19137e2179);
        assert_eq!(constants.rounds[0], 0x428a2f98d728ae22);
        assert_eq!(constants.rounds[79], 0x6c44198c4a475817);
    }

    #[test]
    fn nist_vectors_cover_empty_one_block_two_blocks_and_streaming() {
        assert_eq!(
            hex(&digest(b"")),
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
        assert_eq!(
            hex(&digest(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        let two = b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu";
        let expected = "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909";
        assert_eq!(hex(&digest(two)), expected);
        let mut streamed = Sha512::default();
        for chunk in two.chunks(7) {
            streamed.update(chunk);
        }
        assert_eq!(hex(&streamed.finish()), expected);
    }
}
