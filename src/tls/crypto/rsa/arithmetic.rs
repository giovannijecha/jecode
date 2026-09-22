//! Schoolbook product and Montgomery reduction for bounded public RSA operands.
use super::modulus::{Modulus, Words};

/// Input is below 2n, with at most one carry beyond the active word width.
fn reduce_once(value: Words, high: u64, modulus: &Modulus) -> Words {
    let mut difference = [0; 128];
    let mut borrow = 0;
    for i in 0..modulus.words {
        let next = u128::from(value[i]) + (1_u128 << 64) - u128::from(modulus.value[i]) - borrow;
        difference[i] = next as u64;
        borrow = 1 - (next >> 64);
    }
    if high != 0 || borrow == 0 {
        difference
    } else {
        value
    }
}

pub(super) fn double(value: &Words, modulus: &Modulus) -> Words {
    let mut result = [0; 128];
    let mut carry = 0;
    for i in 0..modulus.words {
        let next = u128::from(value[i]) * 2 + carry;
        result[i] = next as u64;
        carry = next >> 64;
    }
    reduce_once(result, carry as u64, modulus)
}

pub(super) fn multiply(a: &Words, b: &Words, modulus: &Modulus) -> Words {
    let n = modulus.words;
    let mut product = [0_u64; 257];
    for (i, left) in a.iter().take(n).enumerate() {
        let mut carry = 0;
        for (j, right) in b.iter().take(n).enumerate() {
            // One word product plus an existing word and carry is <= 2^128-1.
            let next = u128::from(*left) * u128::from(*right) + u128::from(product[i + j]) + carry;
            product[i + j] = next as u64;
            carry = next >> 64;
        }
        product[i + n] = carry as u64;
    }
    for i in 0..n {
        let quotient = product[i].wrapping_mul(modulus.negative_inverse);
        let mut carry = 0;
        for j in 0..n {
            let next = u128::from(quotient) * u128::from(modulus.value[j])
                + u128::from(product[i + j])
                + carry;
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
    // a,b < n imply T < n^2 < nR. Thus (T + qn)/R < 2n; one
    // subtraction suffices without assuming n > R/2 or that n is prime.
    let mut result = [0; 128];
    result[..n].copy_from_slice(&product[n..2 * n]);
    reduce_once(result, product[2 * n], modulus)
}
