use super::{
    nist::{self, Curve},
    rsa::PublicKey,
    sha384::Sha384,
    signature::Hash,
};
pub(super) fn decode(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0);
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn sha384_known_answers_and_fragmentation() {
    assert_eq!(
        Sha384::digest(b"abc").as_slice(),
        decode(
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        )
    );
    let input = vec![b'a'; 1_000_000];
    let expected = decode(
        "9d0e1809716474cb086e834e310a4a1ced149e9c00f248527972cec5704c2a5b07b8b3dc38ecc4ebae97ddd87f3d8985",
    );
    assert_eq!(Sha384::digest(&input).as_slice(), expected);
    for width in [1, 111, 112, 127, 128, 129, 4096] {
        let mut hash = Sha384::new();
        for chunk in input.chunks(width) {
            hash.update(chunk);
            hash.update(&[]);
        }
        assert_eq!(hash.finish().as_slice(), expected);
    }
}
#[test]
fn ecdsa_rfc6979_rejects_changed_messages_keys_and_encodings() {
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/ecdsa-signatures.txt")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        count += 1;
        let f: Vec<_> = row.split_whitespace().collect();
        let curve = if f[0] == "256" {
            Curve::P256
        } else {
            Curve::P384
        };
        let hash = if f[1] == "sha256" {
            Hash::Sha256
        } else {
            Hash::Sha384
        };
        let digest = hash.digest(&[&decode(f[2])]);
        let key = decode(f[3]);
        let signature = decode(f[4]);
        assert!(nist::verify(curve, &key, &digest, &signature));
        let mut changed = digest.clone();
        changed[0] ^= 1;
        assert!(!nist::verify(curve, &key, &changed, &signature));
        let mut changed = key.clone();
        changed[1] ^= 1;
        assert!(!nist::verify(curve, &changed, &digest, &signature));
        for invalid in [
            vec![],
            vec![0x30, 6, 2, 1, 0, 2, 1, 1],
            [&signature[..], &[0]].concat(),
            signature[..signature.len() - 1].to_vec(),
        ] {
            assert!(!nist::verify(curve, &key, &digest, &invalid));
        }
        for i in [3, signature.len() - 1] {
            let mut altered = signature.clone();
            altered[i] ^= 1;
            assert!(!nist::verify(curve, &key, &digest, &altered));
        }
    }
    assert_eq!(count, 16);
}
#[test]
fn rsa_independent_public_equations_and_wrong_inputs() {
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/rsa-signatures.txt")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        count += 1;
        let f: Vec<_> = row.split_whitespace().collect();
        let hash = if f[1] == "sha256" {
            Hash::Sha256
        } else {
            Hash::Sha384
        };
        let message = decode(f[3]);
        let n = decode(f[4]);
        let e = decode(f[5]);
        let signature = decode(f[6]);
        let key = PublicKey::from_components(&n, &e).unwrap();
        let pss = f[2] == "pss";
        assert!(key.verify(hash, pss, &message, &signature));
        assert!(!key.verify(hash, !pss, &message, &signature));
        let mut wrong = message.clone();
        wrong[0] ^= 1;
        assert!(!key.verify(hash, pss, &wrong, &signature));
        let mut wrong = signature.clone();
        *wrong.last_mut().unwrap() ^= 1;
        assert!(!key.verify(hash, pss, &message, &wrong));
        for signature in [
            vec![],
            vec![0; n.len()],
            n.clone(),
            vec![255; n.len()],
            signature[1..].to_vec(),
        ] {
            assert!(!key.verify(hash, pss, &message, &signature));
        }
        for bad_exponent in [&[][..], &[0, 3], &[1], &[2], &[2, 0, 0, 0, 1]] {
            assert!(PublicKey::from_components(&n, bad_exponent).is_none());
        }
    }
    assert_eq!(count, 8);
}
