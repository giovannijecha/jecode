//! X25519 keys and TLS-compatible all-zero shared-secret rejection.
//! The caller supplies fresh native entropy and owns handshake/key lifetimes.
use super::{
    secret::{Secret, erase},
    x25519_ladder,
};

pub struct Key([u8; 32]);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidPeer;

impl Key {
    pub fn from_bytes(entropy: &[u8; 32]) -> Self {
        let mut key = Self(*entropy);
        key.0[0] &= 248;
        key.0[31] = (key.0[31] & 127) | 64;
        key
    }

    pub fn public(&self) -> [u8; 32] {
        let mut base = [0; 32];
        base[0] = 9;
        let mut output = [0; 32];
        x25519_ladder::multiply(&self.0, &base, &mut output);
        output
    }

    /// Noncanonical points and the ignored top bit follow RFC 7748. Reject
    /// all-zero output before publishing a secret, as TLS requires.
    pub fn shared(&self, peer: &[u8; 32]) -> Result<Secret, InvalidPeer> {
        let mut secret = Secret::zero();
        x25519_ladder::multiply(&self.0, peer, secret.as_mut_bytes());
        if super::secret::equal(secret.as_bytes(), &[0; 32]) {
            return Err(InvalidPeer);
        }
        Ok(secret)
    }
}
impl Drop for Key {
    fn drop(&mut self) {
        erase(&mut self.0);
    }
}
