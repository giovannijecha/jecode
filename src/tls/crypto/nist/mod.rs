//! Owned ECDSA verification only. No signing, key generation or secret scalars.
mod field;
#[cfg(test)]
pub(in crate::tls) mod fixture;
mod p256;
mod p384;
mod point;
use field::{Element, Modulus};
use point::Point;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::tls) enum Curve {
    P256,
    P384,
}
impl Curve {
    pub fn bytes(self) -> usize {
        match self {
            Self::P256 => 32,
            Self::P384 => 48,
        }
    }
    fn field(self) -> Modulus {
        Modulus {
            curve: self,
            order: false,
        }
    }
    fn order(self) -> Modulus {
        Modulus {
            curve: self,
            order: true,
        }
    }
}

pub(in crate::tls) fn verify(curve: Curve, key: &[u8], digest: &[u8], signature: &[u8]) -> bool {
    verify_inner(curve, key, digest, signature).is_some()
}
fn verify_inner(curve: Curve, key: &[u8], digest: &[u8], signature: &[u8]) -> Option<()> {
    if !matches!(digest.len(), 32 | 48) {
        return None;
    }
    let key = Point::parse(curve, key)?;
    // A P-384 signature sequence is at most 104 bytes. Long-form lengths are
    // noncanonical here; the fixed grammar also rejects trailing data.
    if signature.len() < 6
        || signature[0] != 0x30
        || usize::from(signature[1]) != signature.len() - 2
    {
        return None;
    }
    let mut body = &signature[2..];
    let r = scalar(curve, &mut body)?;
    let s = scalar(curve, &mut body)?;
    if !body.is_empty() {
        return None;
    }
    let inverse = s.inverse()?;
    let e = Element::reduced(curve.order(), &digest[..digest.len().min(curve.bytes())]);
    let mut u = [0; 48];
    let u = &mut u[..curve.bytes()];
    e.mul(inverse).write(u);
    let first = Point::base(curve).multiply(u);
    r.mul(inverse).write(u);
    let sum = first.add(key.multiply(u));
    sum.x(u)?;
    Element::reduced(curve.order(), u).equals(r).then_some(())
}
fn scalar(curve: Curve, input: &mut &[u8]) -> Option<Element> {
    if input.len() < 3 || input[0] != 2 {
        return None;
    }
    let length = usize::from(input[1]);
    let mut value = input.get(2..2 + length)?;
    if value.is_empty() || value[0] & 128 != 0 {
        return None;
    }
    if value[0] == 0 {
        if value.len() == 1 || value[1] & 128 == 0 {
            return None;
        }
        value = &value[1..];
    }
    if value.len() > curve.bytes() {
        return None;
    }
    let mut padded = [0; 48];
    let padded = &mut padded[..curve.bytes()];
    padded[curve.bytes() - value.len()..].copy_from_slice(value);
    let result = Element::from_bytes(curve.order(), padded)?;
    if result.is_zero() {
        return None;
    }
    *input = &input[2 + length..];
    Some(result)
}
