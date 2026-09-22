//! Jacobian arithmetic for finite uncompressed prime-order public keys.
use super::{Curve, field::Element, p256, p384};
#[derive(Clone, Copy)]
pub(super) struct Point {
    x: Element,
    y: Element,
    z: Element,
    curve: Curve,
}
impl Point {
    #[cfg(test)]
    pub(super) fn encode_fixture(self) -> Vec<u8> {
        let inverse = self.z.inverse().unwrap();
        let mut encoded = vec![4; 1 + 2 * self.curve.bytes()];
        self.x
            .mul(inverse.square())
            .write(&mut encoded[1..1 + self.curve.bytes()]);
        self.y
            .mul(inverse.square().mul(inverse))
            .write(&mut encoded[1 + self.curve.bytes()..]);
        encoded
    }
    fn identity(curve: Curve) -> Self {
        Self {
            x: Element::one(curve.field()),
            y: Element::one(curve.field()),
            z: Element::zero(curve.field()),
            curve,
        }
    }
    pub fn base(curve: Curve) -> Self {
        let (x, y): (&[u8], &[u8]) = match curve {
            Curve::P256 => (&p256::GX, &p256::GY),
            Curve::P384 => (&p384::GX, &p384::GY),
        };
        Self {
            x: Element::from_bytes(curve.field(), x).expect("base x"),
            y: Element::from_bytes(curve.field(), y).expect("base y"),
            z: Element::one(curve.field()),
            curve,
        }
    }
    pub fn parse(curve: Curve, encoded: &[u8]) -> Option<Self> {
        let width = curve.bytes();
        if encoded.len() != 1 + 2 * width || encoded[0] != 4 {
            return None;
        }
        let x = Element::from_bytes(curve.field(), &encoded[1..1 + width])?;
        let y = Element::from_bytes(curve.field(), &encoded[1 + width..])?;
        let b: &[u8] = match curve {
            Curve::P256 => &p256::B,
            Curve::P384 => &p384::B,
        };
        let b = Element::from_bytes(curve.field(), b)?;
        let three = Element::small(curve.field(), 3);
        if !y
            .square()
            .equals(x.square().mul(x).sub(x.mul(three)).add(b))
        {
            return None;
        }
        // Both curves have cofactor one; every admitted finite point has order n.
        Some(Self {
            x,
            y,
            z: Element::one(curve.field()),
            curve,
        })
    }
    pub fn multiply(self, scalar: &[u8]) -> Self {
        assert_eq!(scalar.len(), self.curve.bytes());
        let mut result = Self::identity(self.curve);
        for byte in scalar {
            for bit in (0..8).rev() {
                result = result.double();
                if byte >> bit & 1 != 0 {
                    result = result.add(self);
                }
            }
        }
        result
    }
    fn double(self) -> Self {
        if self.z.is_zero() || self.y.is_zero() {
            return Self::identity(self.curve);
        }
        let c = |n| Element::small(self.curve.field(), n);
        let delta = self.z.square();
        let gamma = self.y.square();
        let beta = self.x.mul(gamma);
        let alpha = self.x.sub(delta).mul(self.x.add(delta)).mul(c(3));
        let x = alpha.square().sub(beta.mul(c(8)));
        let y = alpha
            .mul(beta.mul(c(4)).sub(x))
            .sub(gamma.square().mul(c(8)));
        let z = self.y.add(self.z).square().sub(gamma).sub(delta);
        Self { x, y, z, ..self }
    }
    pub fn add(self, other: Self) -> Self {
        assert!(self.curve == other.curve);
        if self.z.is_zero() {
            return other;
        }
        if other.z.is_zero() {
            return self;
        }
        let c = |n| Element::small(self.curve.field(), n);
        let zz1 = self.z.square();
        let zz2 = other.z.square();
        let u1 = self.x.mul(zz2);
        let u2 = other.x.mul(zz1);
        let s1 = self.y.mul(other.z).mul(zz2);
        let s2 = other.y.mul(self.z).mul(zz1);
        if u1.equals(u2) {
            return if s1.equals(s2) {
                self.double()
            } else {
                Self::identity(self.curve)
            };
        }
        let h = u2.sub(u1);
        let i = h.mul(c(2)).square();
        let j = h.mul(i);
        let r = s2.sub(s1).mul(c(2));
        let v = u1.mul(i);
        let x = r.square().sub(j).sub(v.mul(c(2)));
        let y = r.mul(v.sub(x)).sub(s1.mul(j).mul(c(2)));
        let z = self.z.add(other.z).square().sub(zz1).sub(zz2).mul(h);
        Self { x, y, z, ..self }
    }
    pub fn x(self, output: &mut [u8]) -> Option<()> {
        self.x.mul(self.z.inverse()?.square()).write(output);
        Some(())
    }
}
