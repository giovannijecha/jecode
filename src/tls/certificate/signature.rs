//! Exact AlgorithmIdentifier/SPKI admission before public signature arithmetic.
use super::{
    Certificate, Error,
    der::{Reader, bits},
};
use crate::tls::crypto::{
    nist::{self, Curve},
    rsa::PublicKey,
    signature::Hash,
};

const RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 1, 1, 1];
const EC: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 2, 1];
enum Key<'a> {
    Rsa(Box<PublicKey>),
    Ec(Curve, &'a [u8]),
}
fn key(encoded: &[u8]) -> Result<Key<'_>, Error> {
    let mut spki = Reader::sequence(encoded)?;
    let mut algorithm = Reader(spki.expect(0x30)?.body);
    let oid = algorithm.oid()?;
    let (bytes, unused) = bits(spki.expect(3)?.body)?;
    spki.end()?;
    if unused != 0 {
        return Err(Error::Encoding);
    }
    let key = match oid {
        RSA => {
            if !algorithm.expect(5)?.body.is_empty() {
                return Err(Error::Encoding);
            }
            let mut components = Reader::sequence(bytes)?;
            let n = unsigned(components.integer()?);
            let e = unsigned(components.integer()?);
            components.end()?;
            Key::Rsa(Box::new(
                PublicKey::from_components(n, e).ok_or(Error::Unsupported)?,
            ))
        }
        EC => {
            let curve = match algorithm.oid()? {
                [0x2a, 0x86, 0x48, 0xce, 0x3d, 3, 1, 7] => Curve::P256,
                [0x2b, 0x81, 4, 0, 34] => Curve::P384,
                _ => return Err(Error::Unsupported),
            };
            Key::Ec(curve, bytes)
        }
        _ => return Err(Error::Unsupported),
    };
    algorithm.end()?;
    Ok(key)
}
fn unsigned(bytes: &[u8]) -> &[u8] {
    if bytes[0] == 0 { &bytes[1..] } else { bytes }
}
pub(super) fn certificate(child: &Certificate<'_>, issuer: &Certificate<'_>) -> Result<(), Error> {
    let mut algorithm = Reader::sequence(child.signature_algorithm)?;
    let (hash, rsa) = match algorithm.oid()? {
        [0x2a, 0x86, 0x48, 0xce, 0x3d, 4, 3, 2] => (Hash::Sha256, false),
        [0x2a, 0x86, 0x48, 0xce, 0x3d, 4, 3, 3] => (Hash::Sha384, false),
        [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 1, 1, 11] => (Hash::Sha256, true),
        [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 1, 1, 12] => (Hash::Sha384, true),
        _ => return Err(Error::Unsupported),
    };
    if rsa && !algorithm.expect(5)?.body.is_empty() {
        return Err(Error::Encoding);
    }
    algorithm.end()?;
    let valid = match (rsa, key(issuer.public_key_info)?) {
        (true, Key::Rsa(key)) => key.verify(hash, false, child.tbs, child.signature),
        (false, Key::Ec(curve, key)) => {
            nist::verify(curve, key, &hash.digest(&[child.tbs]), child.signature)
        }
        _ => return Err(Error::Signature),
    };
    if valid { Ok(()) } else { Err(Error::Signature) }
}
pub(super) fn handshake(
    leaf: &Certificate<'_>,
    scheme: u16,
    message: &[u8],
    signature: &[u8],
) -> Result<(), Error> {
    let valid = match (scheme, key(leaf.public_key_info)?) {
        (0x0403, Key::Ec(Curve::P256, key)) => nist::verify(
            Curve::P256,
            key,
            &Hash::Sha256.digest(&[message]),
            signature,
        ),
        (0x0804, Key::Rsa(key)) => key.verify(Hash::Sha256, true, message, signature),
        _ => return Err(Error::Unsupported),
    };
    if valid { Ok(()) } else { Err(Error::Signature) }
}
