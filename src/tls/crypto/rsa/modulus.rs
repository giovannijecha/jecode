//! Odd-modulus setup without a prime or full-top-word assumption.
use super::arithmetic;

pub(super) type Words = [u64; 128];

pub(super) struct Modulus {
    pub value: Words,
    pub words: usize,
    pub bytes: usize,
    pub bits: usize,
    pub negative_inverse: u64,
    r_squared: Words,
}

pub(super) fn load(input: &[u8]) -> Words {
    assert!(input.len() <= 1024);
    let mut result = [0; 128];
    for (i, byte) in input.iter().rev().enumerate() {
        result[i / 8] |= u64::from(*byte) << (8 * (i % 8));
    }
    result
}

impl Modulus {
    pub fn new(input: &[u8]) -> Option<Self> {
        if input.is_empty() || input.len() > 1024 || input[0] == 0 || input.last()? & 1 == 0 {
            return None;
        }
        if input == [1] {
            return None;
        }
        let value = load(input);
        // Newton doubling: an odd word is its inverse modulo 2; six steps
        // establish 64 correct bits. All operations intentionally wrap.
        let mut inverse = 1_u64;
        for _ in 0..6 {
            inverse = inverse.wrapping_mul(2_u64.wrapping_sub(value[0].wrapping_mul(inverse)));
        }
        let mut modulus = Self {
            value,
            words: input.len().div_ceil(8),
            bytes: input.len(),
            bits: input.len() * 8 - input[0].leading_zeros() as usize,
            negative_inverse: inverse.wrapping_neg(),
            r_squared: [0; 128],
        };
        // Build R^2 mod n from 1 by modular doubling. Each intermediate is
        // below 2n, even when n has only one bit in its highest word.
        let mut power = [0; 128];
        power[0] = 1;
        for _ in 0..128 * modulus.words {
            power = arithmetic::double(&power, &modulus);
        }
        modulus.r_squared = power;
        Some(modulus)
    }

    pub fn contains(&self, value: &Words) -> bool {
        for i in (0..128).rev() {
            if value[i] != self.value[i] {
                return value[i] < self.value[i];
            }
        }
        false
    }

    pub fn power(&self, value: &Words, exponent: u64) -> Words {
        assert!(exponent != 0);
        let base = arithmetic::multiply(value, &self.r_squared, self);
        let mut result = base;
        // Skip the known leading one. Exponent bits and all operands are public.
        for bit in (0..63 - exponent.leading_zeros()).rev() {
            result = arithmetic::multiply(&result, &result, self);
            if (exponent >> bit) & 1 != 0 {
                result = arithmetic::multiply(&result, &base, self);
            }
        }
        let mut one = [0; 128];
        one[0] = 1;
        arithmetic::multiply(&result, &one, self)
    }
}
