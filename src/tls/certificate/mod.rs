//! Bounded X.509 metadata and the deliberately restricted server identity verifier.
mod der;
mod extensions;
mod names;
mod signature;
mod time;
mod verify;
use der::{Reader, bits};
use extensions::Extensions;
#[cfg(test)]
pub(super) use verify::tests::identity;
pub use verify::verify_peer;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Encoding,
    Limit,
    Unsupported,
    Name,
    Validity,
    Purpose,
    Signature,
    Untrusted,
    Distrusted,
    Path,
}

pub struct Certificate<'a> {
    pub tbs: &'a [u8],
    pub signature_algorithm: &'a [u8],
    pub signature: &'a [u8],
    pub issuer: &'a [u8],
    pub subject: &'a [u8],
    pub public_key_info: &'a [u8],
    pub not_before: i64,
    pub not_after: i64,
    extensions: Extensions<'a>,
}
impl<'a> Certificate<'a> {
    pub fn parse(input: &'a [u8]) -> Result<Self, Error> {
        if input.len() > 65_536 {
            return Err(Error::Limit);
        }
        let mut outer = Reader::sequence(input)?;
        let tbs = outer.expect(0x30)?;
        let algorithm = outer.expect(0x30)?;
        let (signature, unused) = bits(outer.expect(3)?.body)?;
        outer.end()?;
        if unused != 0 || signature.is_empty() || signature.len() > 1024 {
            return Err(Error::Encoding);
        }
        let mut body = Reader(tbs.body);
        let mut version = Reader(body.expect(0xa0)?.body);
        if version.integer()? != [2] {
            return Err(Error::Unsupported);
        }
        version.end()?;
        let serial = body.integer()?;
        if serial.len() > 20 || serial.iter().all(|b| *b == 0) {
            return Err(Error::Encoding);
        }
        if body.expect(0x30)?.encoded != algorithm.encoded {
            return Err(Error::Encoding);
        }
        // Retain the original encoded signature input. Never normalize/re-encode it.
        let issuer = body.expect(0x30)?.encoded;
        let mut validity = Reader(body.expect(0x30)?.body);
        let not_before = time::parse(validity.read()?)?;
        let not_after = time::parse(validity.read()?)?;
        validity.end()?;
        if not_before > not_after {
            return Err(Error::Validity);
        }
        let subject = body.expect(0x30)?.encoded;
        let public_key_info = body.expect(0x30)?.encoded;
        let extensions = if body.0.first() == Some(&0xa3) {
            Extensions::parse(body.expect(0xa3)?.body)?
        } else {
            Extensions::default()
        };
        body.end()?;
        // Preserve exact signed bytes. Algorithm-specific admission happens when
        // the verifier uses a key or verifies a certificate signature.
        algorithm_oid(algorithm.encoded)?;
        let mut key = Reader::sequence(public_key_info)?;
        algorithm_oid(key.expect(0x30)?.encoded)?;
        let (key_bits, unused) = bits(key.expect(3)?.body)?;
        key.end()?;
        if unused != 0 || key_bits.is_empty() {
            return Err(Error::Encoding);
        }
        Ok(Self {
            tbs: tbs.encoded,
            signature_algorithm: algorithm.encoded,
            signature,
            issuer,
            subject,
            public_key_info,
            not_before,
            not_after,
            extensions,
        })
    }
    pub fn dns_names(&self) -> &[&'a str] {
        &self.extensions.dns
    }
    pub fn is_ca(&self) -> bool {
        self.extensions.ca
    }
    pub fn path_length(&self) -> Option<u32> {
        self.extensions.path_length
    }
    pub fn key_cert_sign(&self) -> Option<bool> {
        self.extensions.key_cert_sign
    }
    pub fn has_unsupported_constraints(&self) -> bool {
        self.extensions.unsupported_constraints || self.extensions.unsupported_critical
    }
    /// Metadata-only leaf checks. Success does NOT verify a signature, chain,
    /// trust anchor, revocation status or the server's CertificateVerify.
    pub fn check_leaf_metadata(&self, host: &str, unix_seconds: i64) -> Result<(), Error> {
        names::dns(host.as_bytes(), false)?;
        if self.has_unsupported_constraints() {
            return Err(Error::Unsupported);
        }
        if unix_seconds < self.not_before || unix_seconds > self.not_after {
            return Err(Error::Validity);
        }
        if self.extensions.ca
            || self.extensions.digital_signature == Some(false)
            || self.extensions.server_auth == Some(false)
        {
            return Err(Error::Purpose);
        }
        if !self
            .extensions
            .dns
            .iter()
            .any(|name| names::matches(name, host))
        {
            return Err(Error::Name);
        }
        Ok(())
    }
    fn check_ca(&self, unix_seconds: i64, ca_below: usize) -> Result<(), Error> {
        if self.has_unsupported_constraints() {
            return Err(Error::Unsupported);
        }
        if unix_seconds < self.not_before || unix_seconds > self.not_after {
            return Err(Error::Validity);
        }
        if !self.is_ca()
            || self.key_cert_sign() == Some(false)
            || self.extensions.server_auth == Some(false)
        {
            return Err(Error::Purpose);
        }
        if self
            .path_length()
            .is_some_and(|limit| ca_below > limit as usize)
        {
            return Err(Error::Path);
        }
        Ok(())
    }
    pub fn signature_oid(&self) -> Result<&'a [u8], Error> {
        algorithm_oid(self.signature_algorithm)
    }
    /// A named-curve OID when encoded as an OID parameter. This does not validate
    /// the curve, key point or any other kind of algorithm parameter.
    pub fn public_key_parameter_oid(&self) -> Result<Option<&'a [u8]>, Error> {
        let mut spki = Reader::sequence(self.public_key_info)?;
        let mut algorithm = Reader(spki.expect(0x30)?.body);
        algorithm.oid()?;
        if algorithm.0.first() == Some(&6) {
            let oid = algorithm.oid()?;
            algorithm.end()?;
            Ok(Some(oid))
        } else {
            Ok(None)
        }
    }
    pub fn public_key_oid(&self) -> Result<&'a [u8], Error> {
        algorithm_oid(
            Reader::sequence(self.public_key_info)?
                .expect(0x30)?
                .encoded,
        )
    }
}
fn algorithm_oid(encoded: &[u8]) -> Result<&[u8], Error> {
    let mut input = Reader::sequence(encoded)?;
    let oid = input.oid()?;
    if !input.0.is_empty() {
        input.read()?;
    }
    input.end()?;
    Ok(oid)
}

#[cfg(test)]
mod tests;
