use super::{aes_gcm::Key, secret::Secret, sha256::Sha256, x25519};
use crate::tls::{
    schedule::hmac,
    tests::{fixed, hex},
};

#[test]
fn sha256_known_answers_and_every_fragment_boundary() {
    for (input, expected) in [
        (
            "",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
    ] {
        assert_eq!(Sha256::digest(input.as_bytes()), fixed::<32>(expected));
        for split in 0..=input.len() {
            let mut hash = Sha256::new();
            hash.update(&input.as_bytes()[..split]);
            hash.update(&[]);
            hash.update(&input.as_bytes()[split..]);
            assert_eq!(hash.finish(), fixed::<32>(expected));
        }
    }
    let mut hash = Sha256::new();
    for _ in 0..1_000 {
        hash.update(&[b'a'; 1_000]);
    }
    assert_eq!(
        hash.finish(),
        fixed::<32>("cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0")
    );
}

#[test]
fn rfc4231_hmac_short_and_long_keys() {
    assert_eq!(
        hmac(&[0x0b; 20], &[b"Hi ", b"There"]).as_bytes(),
        &fixed::<32>("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7")
    );
    assert_eq!(
        hmac(
            &[0xaa; 131],
            &[b"Test Using Larger Than Block-Size Key - Hash Key First"]
        )
        .as_bytes(),
        &fixed::<32>("60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54")
    );
    assert_eq!(format!("{:?}", Secret([42; 32])), "Secret([redacted])");
}

#[test]
fn nist_aes128_gcm_answers_and_authentication_before_decryption() {
    let key = Key::new(&[0; 16]).unwrap();
    let mut empty = [];
    assert_eq!(
        key.seal(&[0; 12], &[], &mut empty).unwrap(),
        fixed::<16>("58e2fccefa7e3061367f1d57a4e7455a")
    );
    let mut block = [0; 16];
    let tag = key.seal(&[0; 12], &[], &mut block).unwrap();
    assert_eq!(block, fixed::<16>("0388dace60b6a392f328c2b971b2fe78"));
    assert_eq!(tag, fixed::<16>("ab6e47d42cec13bdf53a67b21257bddf"));
    key.open(&[0; 12], &[], &mut block, &tag).unwrap();
    assert_eq!(block, [0; 16]);

    // NIST's partial-block case includes AAD, unlike the empty/single-block cases.
    let key = Key::new(&hex("feffe9928665731c6d6a8f9467308308")).unwrap();
    let nonce = fixed::<12>("cafebabefacedbaddecaf888");
    let aad = hex("feedfacedeadbeeffeedfacedeadbeefabaddad2");
    let original = hex(
        "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
    );
    let mut ciphertext = original.clone();
    let tag = key.seal(&nonce, &aad, &mut ciphertext).unwrap();
    assert_eq!(
        ciphertext,
        hex(
            "42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091"
        )
    );
    assert_eq!(tag, fixed::<16>("5bc94fbc3221a5db94fae95ae7121a47"));
    for index in 0..tag.len() {
        let mut bad = tag;
        bad[index] ^= 1;
        let mut preserved = ciphertext.clone();
        assert!(key.open(&nonce, &aad, &mut preserved, &bad).is_err());
        assert_eq!(preserved, ciphertext);
    }
    for index in 0..ciphertext.len() {
        let mut bad = ciphertext.clone();
        bad[index] ^= 1;
        let expected = bad.clone();
        assert!(key.open(&nonce, &aad, &mut bad, &tag).is_err());
        assert_eq!(bad, expected);
    }
    let mut bad_nonce = nonce;
    bad_nonce[0] ^= 1;
    assert!(key.open(&bad_nonce, &aad, &mut ciphertext, &tag).is_err());
    assert!(key.open(&nonce, b"wrong", &mut ciphertext, &tag).is_err());
    assert!(key.open(&nonce, &aad, &mut ciphertext, &tag[..15]).is_err());
    key.open(&nonce, &aad, &mut ciphertext, &tag).unwrap();
    assert_eq!(ciphertext, original);
}

#[test]
fn rfc7748_agreement_and_low_order_rejection() {
    let alice = x25519::Key::from_bytes(&fixed(
        "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
    ));
    let bob = x25519::Key::from_bytes(&fixed(
        "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb",
    ));
    assert_eq!(
        alice.public(),
        fixed::<32>("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
    );
    assert_eq!(
        bob.public(),
        fixed::<32>("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
    );
    let expected = fixed::<32>("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
    assert_eq!(alice.shared(&bob.public()).unwrap().as_bytes(), &expected);
    assert_eq!(bob.shared(&alice.public()).unwrap().as_bytes(), &expected);
    for value in 0..2 {
        let mut point = [0; 32];
        point[0] = value;
        assert!(alice.shared(&point).is_err());
        point[31] = 0x80;
        assert!(alice.shared(&point).is_err());
    }
    let mut alias = bob.public();
    alias[31] |= 0x80;
    assert_eq!(alice.shared(&alias).unwrap().as_bytes(), &expected);
    // Encodings of p and p+1 are also rejected, not confused with valid peers.
    for low in [0xed, 0xee] {
        let mut point = [0xff; 32];
        point[0] = low;
        point[31] = 0x7f;
        assert!(alice.shared(&point).is_err());
    }
}
