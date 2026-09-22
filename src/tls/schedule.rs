//! RFC 8446 section 7.1: SHA-256 schedule for a fresh, non-PSK handshake.
use super::crypto::{
    secret::{Secret, equal, erase},
    sha256::Sha256,
};

pub struct HandshakeSecrets {
    pub client: Secret,
    pub server: Secret,
    master: Secret,
}

impl HandshakeSecrets {
    /// `hello_hash` hashes the encoded ClientHello and ServerHello handshake
    /// messages, including their headers, excluding TLS record headers.
    pub fn derive(shared: &Secret, hello_hash: &[u8; 32]) -> Self {
        let early = hmac(&[0; 32], &[&[0; 32]]);
        let derived = label::<32>(early.as_bytes(), b"derived", &Sha256::digest(&[]));
        let handshake = hmac(derived.as_bytes(), &[shared.as_bytes()]);
        let derived = label::<32>(handshake.as_bytes(), b"derived", &Sha256::digest(&[]));
        Self {
            client: label::<32>(handshake.as_bytes(), b"c hs traffic", hello_hash),
            server: label::<32>(handshake.as_bytes(), b"s hs traffic", hello_hash),
            master: hmac(derived.as_bytes(), &[&[0; 32]]),
        }
    }

    pub(super) fn application(&self, server_finished_hash: &[u8; 32]) -> (Secret, Secret) {
        (
            label::<32>(
                self.master.as_bytes(),
                b"c ap traffic",
                server_finished_hash,
            ),
            label::<32>(
                self.master.as_bytes(),
                b"s ap traffic",
                server_finished_hash,
            ),
        )
    }
    pub(super) fn client_finished(&self, server_finished_hash: &[u8; 32]) -> Vec<u8> {
        let key = label::<32>(self.client.as_bytes(), b"finished", &[]);
        let mac = hmac(key.as_bytes(), &[server_finished_hash]);
        [vec![20, 0, 0, 32], mac.as_bytes().to_vec()].concat()
    }

    /// This authenticates the handshake transcript under its derived secret;
    /// it is NOT certificate or CertificateVerify validation.
    pub fn verify_server_finished(&self, transcript_hash: &[u8; 32], received: &[u8]) -> bool {
        let key = label::<32>(self.server.as_bytes(), b"finished", &[]);
        equal(
            hmac(key.as_bytes(), &[transcript_hash]).as_bytes(),
            received,
        )
    }
}

// Only a single HKDF block is needed for this suite's key, IV and secret sizes.
// Return a clearing owner even for short key/IV values; unused bytes remain zero.
pub(super) fn label<const N: usize>(secret: &[u8], label: &[u8], context: &[u8]) -> Secret {
    assert!(N <= 32 && label.len() <= 249 && context.len() <= 255);
    let mut info = Vec::with_capacity(11 + label.len() + context.len());
    info.extend_from_slice(&(N as u16).to_be_bytes());
    info.push((6 + label.len()) as u8);
    info.extend_from_slice(b"tls13 ");
    info.extend_from_slice(label);
    info.push(context.len() as u8);
    info.extend_from_slice(context);
    let mut output = hmac(secret, &[&info, &[1]]);
    erase(&mut output.0[N..]);
    output
}

pub(super) fn hmac(key: &[u8], parts: &[&[u8]]) -> Secret {
    let mut pad = Pad([0; 64]);
    if key.len() > 64 {
        let digest = Secret(Sha256::digest(key));
        pad.0[..32].copy_from_slice(digest.as_bytes());
    } else {
        pad.0[..key.len()].copy_from_slice(key);
    }
    for byte in &mut pad.0 {
        *byte ^= 0x36;
    }
    let mut inner = Sha256::new();
    inner.update(&pad.0);
    for part in parts {
        inner.update(part);
    }
    let digest = Secret(inner.finish());
    for byte in &mut pad.0 {
        *byte ^= 0x36 ^ 0x5c;
    }
    let mut outer = Sha256::new();
    outer.update(&pad.0);
    outer.update(digest.as_bytes());
    Secret(outer.finish())
}

struct Pad([u8; 64]);
impl Drop for Pad {
    fn drop(&mut self) {
        erase(&mut self.0);
    }
}
