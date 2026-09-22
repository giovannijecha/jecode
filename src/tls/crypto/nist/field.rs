//! Bounded Montgomery arithmetic for public P-256/P-384 verification operands.
//! Adapted from Jecode's owned field implementation. Not a signing API.
use super::{Curve, p256, p384};
pub(super) type Words = [u64; 6];
pub(super) struct Parameters {
    pub words: usize,
    pub value: Words,
    pub negative_inverse: u64,
    pub r_squared: Words,
    pub one: Words,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Modulus {
    pub curve: Curve,
    pub order: bool,
}
impl Modulus {
    fn get(self) -> &'static Parameters {
        match (self.curve, self.order) {
            (Curve::P256, false) => &p256::FIELD,
            (Curve::P256, true) => &p256::ORDER,
            (Curve::P384, false) => &p384::FIELD,
            (Curve::P384, true) => &p384::ORDER,
        }
    }
    pub fn bytes(self) -> usize {
        self.curve.bytes()
    }
}
#[derive(Clone, Copy)]
pub(super) struct Element {
    words: Words,
    modulus: Modulus,
}
impl Element {
    pub fn zero(modulus: Modulus) -> Self {
        Self {
            words: [0; 6],
            modulus,
        }
    }
    pub fn one(modulus: Modulus) -> Self {
        Self {
            words: modulus.get().one,
            modulus,
        }
    }
    pub fn small(modulus: Modulus, value: u64) -> Self {
        let mut words = [0; 6];
        words[0] = value;
        Self::plain(modulus, &words)
    }
    fn plain(modulus: Modulus, value: &Words) -> Self {
        Self {
            words: multiply(value, &modulus.get().r_squared, modulus.get()),
            modulus,
        }
    }
    pub fn from_bytes(modulus: Modulus, bytes: &[u8]) -> Option<Self> {
        if bytes.len() != modulus.bytes() {
            return None;
        }
        let words = load(bytes);
        if difference(&words, &modulus.get().value, modulus.get().words).1 == 0 {
            return None;
        }
        Some(Self::plain(modulus, &words))
    }
    pub fn reduced(modulus: Modulus, bytes: &[u8]) -> Self {
        assert!(bytes.len() <= modulus.bytes());
        // Both supported order moduli exceed half of their byte-aligned width.
        Self::plain(modulus, &reduce(load(bytes), 0, modulus.get()))
    }
    pub fn write(self, output: &mut [u8]) {
        assert_eq!(output.len(), self.modulus.bytes());
        let mut one = [0; 6];
        one[0] = 1;
        let words = multiply(&self.words, &one, self.modulus.get());
        for (i, byte) in output.iter_mut().rev().enumerate() {
            *byte = (words[i / 8] >> (8 * (i % 8))) as u8;
        }
    }
    pub fn is_zero(self) -> bool {
        self.words == [0; 6]
    }
    pub fn equals(self, other: Self) -> bool {
        self.modulus == other.modulus && self.words == other.words
    }
    pub fn add(self, other: Self) -> Self {
        assert!(self.modulus == other.modulus);
        let p = self.modulus.get();
        let mut words = [0; 6];
        let mut carry = 0_u128;
        for (i, word) in words.iter_mut().enumerate().take(p.words) {
            let next = u128::from(self.words[i]) + u128::from(other.words[i]) + carry;
            *word = next as u64;
            carry = next >> 64;
        }
        Self {
            words: reduce(words, carry as u64, p),
            ..self
        }
    }
    pub fn sub(self, other: Self) -> Self {
        assert!(self.modulus == other.modulus);
        let p = self.modulus.get();
        let (mut words, borrow) = difference(&self.words, &other.words, p.words);
        if borrow != 0 {
            let mut carry = 0_u128;
            for (i, word) in words.iter_mut().enumerate().take(p.words) {
                let next = u128::from(*word) + u128::from(p.value[i]) + carry;
                *word = next as u64;
                carry = next >> 64;
            }
        }
        Self { words, ..self }
    }
    pub fn mul(self, other: Self) -> Self {
        assert!(self.modulus == other.modulus);
        Self {
            words: multiply(&self.words, &other.words, self.modulus.get()),
            ..self
        }
    }
    pub fn square(self) -> Self {
        self.mul(self)
    }
    pub fn inverse(self) -> Option<Self> {
        if self.is_zero() {
            return None;
        }
        let mut exponent = self.modulus.get().value;
        // Each supported prime is odd and its low word exceeds two.
        exponent[0] -= 2;
        let mut result = Self::one(self.modulus);
        for bit in (0..self.modulus.bytes() * 8).rev() {
            result = result.square();
            if exponent[bit / 64] >> (bit % 64) & 1 != 0 {
                result = result.mul(self);
            }
        }
        Some(result)
    }
}
fn load(bytes: &[u8]) -> Words {
    let mut words = [0; 6];
    for (i, byte) in bytes.iter().rev().enumerate() {
        words[i / 8] |= u64::from(*byte) << (8 * (i % 8));
    }
    words
}
fn difference(a: &Words, b: &Words, n: usize) -> (Words, u64) {
    let mut output = [0; 6];
    let mut borrow = 0_u128;
    for i in 0..n {
        let next = u128::from(a[i]) + (1_u128 << 64) - u128::from(b[i]) - borrow;
        output[i] = next as u64;
        borrow = 1 - (next >> 64);
    }
    (output, borrow as u64)
}
fn reduce(value: Words, high: u64, p: &Parameters) -> Words {
    let (candidate, borrow) = difference(&value, &p.value, p.words);
    if high != 0 || borrow == 0 {
        candidate
    } else {
        value
    }
}
fn multiply(a: &Words, b: &Words, p: &Parameters) -> Words {
    let n = p.words;
    let mut product = [0_u64; 13];
    for (i, a) in a.iter().enumerate().take(n) {
        let mut carry = 0_u128;
        for (j, b) in b.iter().enumerate().take(n) {
            let next = u128::from(*a) * u128::from(*b) + u128::from(product[i + j]) + carry;
            product[i + j] = next as u64;
            carry = next >> 64;
        }
        product[i + n] = carry as u64;
    }
    for i in 0..n {
        let quotient = product[i].wrapping_mul(p.negative_inverse);
        let mut carry = 0_u128;
        for j in 0..n {
            let next =
                u128::from(quotient) * u128::from(p.value[j]) + u128::from(product[i + j]) + carry;
            product[i + j] = next as u64;
            carry = next >> 64;
        }
        for word in &mut product[i + n..=2 * n] {
            let next = u128::from(*word) + carry;
            *word = next as u64;
            carry = next >> 64;
        }
        debug_assert_eq!(product[i], 0);
        debug_assert_eq!(carry, 0);
    }
    let mut output = [0; 6];
    output[..n].copy_from_slice(&product[n..2 * n]);
    reduce(output, product[2 * n], p)
}
