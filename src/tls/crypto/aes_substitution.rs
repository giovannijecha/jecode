//! FIPS 197 field arithmetic and affine substitution, without secret lookups.

pub(super) fn double(value: u8) -> u8 {
    (value << 1) ^ (0_u8.wrapping_sub(value >> 7) & 0x1b)
}

fn multiply(mut left: u8, mut right: u8) -> u8 {
    let mut product = 0;
    for _ in 0..8 {
        product ^= left & 0_u8.wrapping_sub(right & 1);
        left = double(left);
        right >>= 1;
    }
    product
}

pub(super) fn substitute(value: u8) -> u8 {
    // Fixed addition chain for value^254. Zero maps to zero before the affine
    // transform; no special branch or inverse lookup is necessary.
    let x2 = multiply(value, value);
    let x3 = multiply(x2, value);
    let x6 = multiply(x3, x3);
    let x12 = multiply(x6, x6);
    let x15 = multiply(x12, x3);
    let x30 = multiply(x15, x15);
    let x60 = multiply(x30, x30);
    let x120 = multiply(x60, x60);
    let x240 = multiply(x120, x120);
    let x252 = multiply(x240, x12);
    let inverse = multiply(x252, x2);
    inverse
        ^ inverse.rotate_left(1)
        ^ inverse.rotate_left(2)
        ^ inverse.rotate_left(3)
        ^ inverse.rotate_left(4)
        ^ 0x63
}
