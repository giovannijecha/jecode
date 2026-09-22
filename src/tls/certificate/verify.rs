//! Bounded path search to independently supplied native roots. No AIA fetches.
use super::{Certificate, Error, signature};
use crate::tls::{UnverifiedPeer, trust::TrustStore};

/// Checks the host, time, path constraints, signatures and CertificateVerify.
/// This is a diagnostic result, not an application-key capability. Only the
/// handshake's authenticated transition can release application traffic keys.
pub fn verify_peer(
    peer: &UnverifiedPeer,
    trust: &TrustStore,
    unix_seconds: i64,
) -> Result<(), Error> {
    verify_chain(peer.certificates(), peer.host(), trust, unix_seconds)?;
    let leaf = Certificate::parse(&peer.certificates()[0])?;
    signature::handshake(
        &leaf,
        peer.signature_algorithm(),
        peer.signed_message(),
        peer.signature(),
    )
}
fn verify_chain(chain: &[Vec<u8>], host: &str, trust: &TrustStore, now: i64) -> Result<(), Error> {
    if chain.is_empty() || chain.len() > 8 || chain.iter().map(Vec::len).sum::<usize>() > 262_144 {
        return Err(Error::Limit);
    }
    let mut certificates = Vec::with_capacity(chain.len());
    for encoded in chain {
        if trust.distrusted(encoded) {
            return Err(Error::Distrusted);
        }
        certificates.push(Certificate::parse(encoded)?);
    }
    certificates[0].check_leaf_metadata(host, now)?;
    let mut path = vec![0];
    let mut attempts = 0;
    if search(&certificates, trust, now, &mut path, 0, &mut attempts)? {
        Ok(())
    } else {
        Err(Error::Untrusted)
    }
}
fn search(
    chain: &[Certificate<'_>],
    trust: &TrustStore,
    now: i64,
    path: &mut Vec<usize>,
    ca_below: usize,
    attempts: &mut usize,
) -> Result<bool, Error> {
    let current = &chain[*path.last().expect("leaf path")];
    let next_below = ca_below + usize::from(current.is_ca() && current.subject != current.issuer);
    // Test trust anchors as data from a separate channel. A self-signed peer
    // certificate or a matching subject alone can never establish trust.
    for encoded in trust.roots() {
        if trust.distrusted(encoded) {
            continue;
        }
        let Ok(root) = Certificate::parse(encoded) else {
            continue;
        };
        if root.check_ca(now, next_below).is_err() {
            continue;
        }
        if current.issuer == root.subject {
            *attempts += 1;
            if *attempts > 64 {
                return Err(Error::Limit);
            }
            if signature::certificate(current, &root).is_ok() {
                return Ok(true);
            }
        }
    }
    if path.len() >= 8 {
        return Ok(false);
    }
    for (i, issuer) in chain.iter().enumerate().skip(1) {
        if path.contains(&i) || current.issuer != issuer.subject {
            continue;
        }
        // For a CA's issuer, that CA counts toward pathLen unless self-issued.
        if issuer.check_ca(now, next_below).is_err() {
            continue;
        }
        *attempts += 1;
        if *attempts > 64 {
            return Err(Error::Limit);
        }
        if signature::certificate(current, issuer).is_err() {
            continue;
        }
        path.push(i);
        let found = search(chain, trust, now, path, next_below, attempts)?;
        path.pop();
        if found {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
pub(super) mod tests;
