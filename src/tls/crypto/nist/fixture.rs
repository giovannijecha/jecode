//! Public tiny test keys only. Never compiled into an application or dev binary.
//! Integration fixtures exercise the trust boundary; RFC vectors independently
//! check production arithmetic. These generated signatures are not that oracle.
use super::{Curve, Element, Point};
use crate::tls::crypto::sha256::Sha256;
pub(in crate::tls) fn key(identity: u8) -> Vec<u8> {
    assert!(identity > 0);
    let mut scalar = [0; 32];
    scalar[31] = identity;
    Point::base(Curve::P256).multiply(&scalar).encode_fixture()
}
pub(in crate::tls) fn sign(identity: u8, message: &[u8]) -> Vec<u8> {
    let curve = Curve::P256;
    let mut nonce = [0; 32];
    nonce[31] = 17;
    let mut x = [0; 32];
    Point::base(curve).multiply(&nonce).x(&mut x).unwrap();
    let r = Element::reduced(curve.order(), &x);
    let k = Element::from_bytes(curve.order(), &nonce).unwrap();
    let e = Element::reduced(curve.order(), &Sha256::digest(message));
    let s = k
        .inverse()
        .unwrap()
        .mul(e.add(r.mul(Element::small(curve.order(), identity.into()))));
    let encode = |n: Element| {
        let mut bytes = [0; 32];
        n.write(&mut bytes);
        let start = bytes.iter().position(|b| *b != 0).unwrap();
        let bytes = &bytes[start..];
        let padding = usize::from(bytes[0] & 128 != 0);
        let mut der = vec![2, (bytes.len() + padding) as u8];
        if padding != 0 {
            der.push(0);
        }
        der.extend_from_slice(bytes);
        der
    };
    let body = [encode(r), encode(s)].concat();
    [vec![0x30, body.len() as u8], body].concat()
}
