//! Untrusted peer evidence for the future certificate verifier, never authorization.
use super::wire::Cursor;
use crate::tls::Error;

pub struct UnverifiedPeer {
    pub(super) host: String,
    pub(super) certificates: Vec<Vec<u8>>,
    pub(super) algorithm: u16,
    pub(super) signature: Vec<u8>,
    pub(super) signed_message: Vec<u8>,
}
impl UnverifiedPeer {
    pub fn host(&self) -> &str {
        &self.host
    }
    /// DER bytes are bounded, but not parsed or trusted by this layer.
    pub fn certificates(&self) -> &[Vec<u8>] {
        &self.certificates
    }
    pub fn signature_algorithm(&self) -> u16 {
        self.algorithm
    }
    pub fn signature(&self) -> &[u8] {
        &self.signature
    }
    pub fn signed_message(&self) -> &[u8] {
        &self.signed_message
    }
}

pub(super) fn certificates(body: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    let mut input = Cursor(body);
    if !input.vector8()?.is_empty() {
        return Err(Error::Malformed);
    }
    let mut list = Cursor(input.vector24()?);
    input.end()?;
    let mut certificates = Vec::new();
    while !list.0.is_empty() {
        if certificates.len() >= 8 {
            return Err(Error::Limit);
        }
        let certificate = list.vector24()?;
        if certificate.is_empty() {
            return Err(Error::Malformed);
        }
        // No status/SCT/other CertificateEntry extension was solicited.
        if !list.vector16()?.is_empty() {
            return Err(Error::Unsupported);
        }
        certificates.push(certificate.to_vec());
    }
    if certificates.is_empty() {
        return Err(Error::Malformed);
    }
    Ok(certificates)
}

pub(super) fn signature(
    body: &[u8],
    hash: &[u8; 32],
    peer: &mut UnverifiedPeer,
) -> Result<(), Error> {
    let mut input = Cursor(body);
    let algorithm = input.word()?;
    if !matches!(algorithm, 0x0804 | 0x0403) {
        return Err(Error::Unsupported);
    }
    let signature = input.vector16()?;
    input.end()?;
    if signature.is_empty() || signature.len() > 1024 {
        return Err(Error::Limit);
    }
    peer.algorithm = algorithm;
    peer.signature = signature.to_vec();
    peer.signed_message = vec![0x20; 64];
    peer.signed_message
        .extend_from_slice(b"TLS 1.3, server CertificateVerify\0");
    peer.signed_message.extend_from_slice(hash);
    Ok(())
}
