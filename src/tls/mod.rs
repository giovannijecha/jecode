//! Experimental owned TLS 1.3 client with a deliberately narrow certificate profile.
//!
//! Record authentication proves possession of a key, not server identity.
//! Certificate/path/name verification precedes client Finished and application
//! traffic. Native APIs supply entropy and trust data, never TLS or signatures.

mod application;
pub mod certificate;
mod connection;
mod crypto;
mod socket;
pub use connection::{ApplicationWrite, Budget, Connection, IoFailure, IoOperation, NetworkError};
mod entropy;
mod handshake;
mod record;
mod schedule;
pub mod trust;

pub use crypto::{
    secret::Secret,
    x25519::{InvalidPeer, Key as KeyShare},
};
pub use handshake::{ServerFlight, UnverifiedPeer};
pub use record::{ContentType, Error, Plaintext, Receiver, Sender};
pub use schedule::HandshakeSecrets;

#[cfg(test)]
mod tests;
