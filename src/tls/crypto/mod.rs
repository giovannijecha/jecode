//! Selected owned primitives, adapted from Jecode's archived implementation.
//! These are private to the TLS boundary; no general crypto API is offered.

mod aes;
pub(super) mod aes_gcm;
mod aes_substitution;
mod field25519;
mod ghash;
pub(super) mod nist;
pub(super) mod rsa;
pub(super) mod secret;
pub(super) mod sha256;
pub(super) mod sha384;
mod sha384_compress;
pub(super) mod signature;
pub(super) mod x25519;
mod x25519_ladder;

#[cfg(test)]
mod signature_tests;
#[cfg(test)]
mod tests;
