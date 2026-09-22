//! Native trust configuration as data. No OS TLS or signature verification.
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "linux", test))]
mod pem;
#[cfg(target_os = "windows")]
mod windows;
use super::crypto::sha256::Sha256;
use std::{collections::HashSet, io};

pub struct TrustStore {
    roots: Vec<Vec<u8>>,
    denied: HashSet<[u8; 32]>,
}
impl TrustStore {
    /// Snapshot the configured system roots. Unsupported platforms fail closed.
    /// No roots from the server or environment-selected files are admitted here.
    pub fn native() -> io::Result<Self> {
        let Self { mut roots, denied } = platform()?;
        roots.retain(|der| !denied.contains(&Sha256::digest(der)));
        if roots.is_empty() {
            return Err(io::Error::other(
                "native trust store has no permitted roots",
            ));
        }
        Ok(Self { roots, denied })
    }
    pub fn root_count(&self) -> usize {
        self.roots.len()
    }
    pub(super) fn roots(&self) -> &[Vec<u8>] {
        &self.roots
    }
    pub(super) fn distrusted(&self, certificate: &[u8]) -> bool {
        self.denied.contains(&Sha256::digest(certificate))
    }
    #[cfg(test)]
    pub(in crate::tls) fn fixture(roots: Vec<Vec<u8>>, denied: &[Vec<u8>]) -> Self {
        Self {
            roots,
            denied: denied.iter().map(|d| Sha256::digest(d)).collect(),
        }
    }
}

#[cfg(target_os = "windows")]
fn platform() -> io::Result<TrustStore> {
    windows::load()
}
#[cfg(target_os = "linux")]
fn platform() -> io::Result<TrustStore> {
    Ok(TrustStore {
        roots: linux::load()?,
        denied: HashSet::new(),
    })
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn platform() -> io::Result<TrustStore> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native trust store unavailable",
    ))
}
