//! RFC 7748 Montgomery ladder; exactly 255 scalar-bit steps.
use super::field25519::Field;

pub(crate) fn multiply(scalar: &[u8; 32], coordinate: &[u8; 32], output: &mut [u8; 32]) {
    let x1 = Field::from_bytes(coordinate);
    let mut x2 = Field::small(1);
    let mut z2 = Field::small(0);
    let mut x3 = Field::from_bytes(coordinate);
    let mut z3 = Field::small(1);
    let a24 = Field::small(121665);
    let mut swap = 0;
    for bit in (0..255).rev() {
        let selected = (scalar[bit / 8] >> (bit % 8)) & 1;
        swap ^= selected;
        Field::swap(&mut x2, &mut x3, swap);
        Field::swap(&mut z2, &mut z3, swap);
        swap = selected;
        let a = x2.add(&z2);
        let aa = a.square();
        let b = x2.sub(&z2);
        let bb = b.square();
        let e = aa.sub(&bb);
        let c = x3.add(&z3);
        let d = x3.sub(&z3);
        let da = d.mul(&a);
        let cb = c.mul(&b);
        x3 = da.add(&cb).square();
        z3 = x1.mul(&da.sub(&cb).square());
        x2 = aa.mul(&bb);
        z2 = e.mul(&aa.add(&a24.mul(&e)));
    }
    Field::swap(&mut x2, &mut x3, swap);
    Field::swap(&mut z2, &mut z3, swap);
    x2.mul(&z2.inverse()).write_bytes(output);
}
