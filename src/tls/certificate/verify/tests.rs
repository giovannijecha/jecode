use super::*;
use crate::tls::crypto::nist::fixture;
const NOW: i64 = 1_790_035_200; // 2026-09-22 UTC, inside the synthetic validity.
pub(in crate::tls) fn identity() -> (Vec<u8>, Vec<u8>) {
    (certificate(3, 3, 3, ca(0)), certificate(1, 3, 3, leaf()))
}
fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![tag];
    if body.len() < 128 {
        bytes.push(body.len() as u8);
    } else if body.len() < 256 {
        bytes.extend([0x81, body.len() as u8]);
    } else {
        bytes.extend([0x82, (body.len() >> 8) as u8, body.len() as u8]);
    }
    bytes.extend(body);
    bytes
}
fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}
fn name(id: u8) -> Vec<u8> {
    seq(&[tlv(
        0x31,
        &seq(&[tlv(6, &[0x55, 4, 3]), tlv(12, &[b'J', b' ', b'A' + id])]),
    )])
}
fn extension(id: u8, body: Vec<u8>) -> Vec<u8> {
    seq(&[tlv(6, &[0x55, 0x1d, id]), tlv(1, &[255]), tlv(4, &body)])
}
fn certificate(subject: u8, issuer: u8, signer: u8, extra: Vec<Vec<u8>>) -> Vec<u8> {
    let algorithm = seq(&[tlv(6, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 4, 3, 2])]);
    let mut key = vec![0];
    key.extend(fixture::key(subject));
    let tbs = seq(&[
        tlv(0xa0, &tlv(2, &[2])),
        tlv(2, &[subject]),
        algorithm.clone(),
        name(issuer),
        seq(&[tlv(0x17, b"260101000000Z"), tlv(0x17, b"270101000000Z")]),
        name(subject),
        seq(&[
            seq(&[
                tlv(6, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 2, 1]),
                tlv(6, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 3, 1, 7]),
            ]),
            tlv(3, &key),
        ]),
        tlv(0xa3, &seq(&extra)),
    ]);
    let mut signature = vec![0];
    signature.extend(fixture::sign(signer, &tbs));
    seq(&[tbs, algorithm, tlv(3, &signature)])
}
fn ca(path: u8) -> Vec<Vec<u8>> {
    vec![
        extension(19, seq(&[tlv(1, &[255]), tlv(2, &[path])])),
        extension(15, tlv(3, &[2, 4])),
    ]
}
fn leaf() -> Vec<Vec<u8>> {
    vec![
        extension(19, seq(&[])),
        extension(15, tlv(3, &[7, 128])),
        extension(17, seq(&[tlv(0x82, b"chatgpt.com")])),
    ]
}
fn run(chain: &[Vec<u8>], roots: Vec<Vec<u8>>) -> Result<(), Error> {
    verify_chain(chain, "chatgpt.com", &TrustStore::fixture(roots, &[]), NOW)
}
#[test]
fn only_an_independent_root_can_authorize_a_complete_path() {
    let root = certificate(3, 3, 3, ca(1));
    let issuer = certificate(2, 3, 3, ca(0));
    let leaf = certificate(1, 2, 2, leaf());
    assert_eq!(
        run(&[leaf.clone(), issuer.clone()], vec![root.clone()]),
        Ok(())
    );
    assert_eq!(
        run(
            &[leaf.clone(), root.clone(), issuer.clone()],
            vec![root.clone()]
        ),
        Ok(())
    );
    assert!(run(&[leaf.clone(), issuer.clone(), root.clone()], vec![]).is_err());
    assert!(run(std::slice::from_ref(&leaf), vec![root.clone()]).is_err());
    let mut changed = issuer.clone();
    *changed.last_mut().unwrap() ^= 1;
    assert!(run(&[leaf.clone(), changed], vec![root.clone()]).is_err());
    // Same subject metadata, signature by a different key: cannot become an issuer.
    let impostor = certificate(2, 3, 4, ca(0));
    assert!(run(&[leaf.clone(), impostor], vec![root.clone()]).is_err());
    for rejected in [leaf.clone(), issuer.clone(), root.clone()] {
        let trust = TrustStore::fixture(vec![root.clone()], &[rejected]);
        assert!(verify_chain(&[leaf.clone(), issuer.clone()], "chatgpt.com", &trust, NOW).is_err());
    }
}
#[test]
fn names_validity_constraints_and_usage_are_enforced_on_the_path() {
    let root = certificate(3, 3, 3, ca(1));
    let issuer = certificate(2, 3, 3, ca(0));
    let end = certificate(1, 2, 2, leaf());
    let trust = TrustStore::fixture(vec![root.clone()], &[]);
    let chain = vec![end.clone(), issuer.clone()];
    assert_eq!(
        verify_chain(&chain, "auth.openai.com", &trust, NOW),
        Err(Error::Name)
    );
    for time in [0, 2_000_000_000] {
        assert_eq!(
            verify_chain(&chain, "chatgpt.com", &trust, time),
            Err(Error::Validity)
        );
    }
    assert!(run(&chain, vec![certificate(3, 3, 3, ca(0))]).is_err());
    for extensions in [
        leaf(),
        vec![
            extension(19, seq(&[tlv(1, &[255])])),
            extension(15, tlv(3, &[7, 128])),
        ],
        [ca(0), vec![extension(30, seq(&[]))]].concat(),
        [
            ca(0),
            vec![extension(37, seq(&[tlv(6, &[0x2b, 6, 1, 5, 5, 7, 3, 2])]))],
        ]
        .concat(),
        [ca(0), vec![extension(99, seq(&[]))]].concat(),
    ] {
        assert!(
            run(
                &[end.clone(), certificate(2, 3, 3, extensions)],
                vec![root.clone()]
            )
            .is_err()
        );
    }
}
#[test]
fn certificate_verify_binds_the_handshake_and_permitted_scheme() {
    let leaf_bytes = certificate(1, 3, 3, leaf());
    let leaf = Certificate::parse(&leaf_bytes).unwrap();
    let message = b"TLS synthetic context and transcript";
    let signature = fixture::sign(1, message);
    assert_eq!(
        signature::handshake(&leaf, 0x0403, message, &signature),
        Ok(())
    );
    assert!(signature::handshake(&leaf, 0x0804, message, &signature).is_err());
    assert!(signature::handshake(&leaf, 0x0403, b"different transcript", &signature).is_err());
    assert!(signature::handshake(&leaf, 0x0403, message, &fixture::sign(2, message)).is_err());
}
