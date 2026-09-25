//! Selected owned primitives, adapted from Jecode's archived implementation.
//! SHA-256 is also reused by private file recovery; no public crypto API is offered.

mod aes;
pub(super) mod aes_gcm;
mod aes_substitution;
mod field25519;
mod ghash;
pub(super) mod nist;
pub(super) mod rsa;
pub(super) mod secret;
pub(crate) mod sha256;
pub(super) mod sha384;
mod sha384_compress;
pub(super) mod signature;
pub(super) mod x25519;
mod x25519_ladder;

#[cfg(test)]
mod signature_tests;
#[cfg(test)]
mod tests;
