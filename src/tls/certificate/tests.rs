use super::*;
fn tlv(tag: u8, bytes: &[u8]) -> Vec<u8> {
    let mut result = vec![tag];
    if bytes.len() < 128 {
        result.push(bytes.len() as u8);
    } else if bytes.len() < 256 {
        result.extend_from_slice(&[0x81, bytes.len() as u8]);
    } else {
        result.extend_from_slice(&[0x82, (bytes.len() >> 8) as u8, bytes.len() as u8]);
    }
    result.extend_from_slice(bytes);
    result
}
fn sequence(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}
fn extension(id: u8, critical: bool, value: &[u8]) -> Vec<u8> {
    let mut fields = vec![tlv(6, &[0x55, 0x1d, id])];
    if critical {
        fields.push(tlv(1, &[255]));
    }
    fields.push(tlv(4, value));
    sequence(&fields)
}
fn san(name: &str) -> Vec<u8> {
    extension(17, false, &sequence(&[tlv(0x82, name.as_bytes())]))
}
fn fixture(extensions: Vec<Vec<u8>>) -> Vec<u8> {
    // Deliberately inert keys/signatures. Metadata success must never mean trust.
    let algorithm = sequence(&[tlv(6, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 4, 3, 2])]);
    let key_algorithm = sequence(&[
        tlv(6, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 2, 1]),
        tlv(6, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 3, 1, 7]),
    ]);
    let mut fields = vec![
        tlv(0xa0, &tlv(2, &[2])),
        tlv(2, &[1]),
        algorithm.clone(),
        sequence(&[]),
        sequence(&[tlv(0x17, b"260101000000Z"), tlv(0x17, b"270101000000Z")]),
        sequence(&[]),
        sequence(&[key_algorithm, tlv(3, &[0, 4])]),
    ];
    if !extensions.is_empty() {
        fields.push(tlv(0xa3, &sequence(&extensions)));
    }
    let tbs = sequence(&fields);
    sequence(&[tbs, algorithm, tlv(3, &[0, 0xaa])])
}
fn profile() -> Vec<Vec<u8>> {
    vec![
        san("chatgpt.com"),
        extension(19, true, &sequence(&[])),
        extension(15, true, &tlv(3, &[7, 0x80])),
        extension(
            37,
            false,
            &sequence(&[tlv(6, &[0x2b, 6, 1, 5, 5, 7, 3, 1])]),
        ),
    ]
}

#[test]
fn metadata_preserves_signed_bytes_and_checks_name_dates_and_purpose() {
    let encoded = fixture(profile());
    let certificate = Certificate::parse(&encoded).unwrap();
    assert_eq!(certificate.dns_names(), ["chatgpt.com"]);
    assert_eq!(
        certificate.public_key_parameter_oid().unwrap(),
        Some([0x2a, 0x86, 0x48, 0xce, 0x3d, 3, 1, 7].as_slice())
    );
    assert_eq!(certificate.not_before, 1_767_225_600);
    assert!(!certificate.is_ca());
    for time in [certificate.not_before, certificate.not_after] {
        assert_eq!(certificate.check_leaf_metadata("CHATGPT.COM", time), Ok(()));
    }
    assert_eq!(
        certificate.check_leaf_metadata("auth.openai.com", certificate.not_before),
        Err(Error::Name)
    );
    for time in [certificate.not_before - 1, certificate.not_after + 1] {
        assert_eq!(
            certificate.check_leaf_metadata("chatgpt.com", time),
            Err(Error::Validity)
        );
    }
    let mut outer = Reader::sequence(&encoded).unwrap();
    assert_eq!(certificate.tbs, outer.expect(0x30).unwrap().encoded);
    assert_eq!(certificate.signature, [0xaa]);
    let expected = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 4, 3, 2];
    assert_eq!(certificate.signature_oid().unwrap(), expected);
}

#[test]
fn san_has_no_common_name_fallback_and_wildcards_match_one_label() {
    let encoded = fixture(vec![san("*.openai.com")]);
    let certificate = Certificate::parse(&encoded).unwrap();
    for host in ["auth.openai.com", "AUTH.OPENAI.COM"] {
        assert!(
            certificate
                .check_leaf_metadata(host, certificate.not_before)
                .is_ok()
        );
    }
    for host in [
        "openai.com",
        "a.b.openai.com",
        "openai.com.evil.test",
        "127.0.0.1",
        "auth.openai.com\0",
    ] {
        assert!(
            certificate
                .check_leaf_metadata(host, certificate.not_before)
                .is_err()
        );
    }
    let no_san = fixture(vec![]);
    let certificate = Certificate::parse(&no_san).unwrap();
    assert_eq!(
        certificate.check_leaf_metadata("chatgpt.com", certificate.not_before),
        Err(Error::Name)
    );
    for name in [
        "",
        "*.com",
        "a*.openai.com",
        "a.*.com",
        "*.*.com",
        "chatgpt.com\0.evil",
        "x..test",
        "x-.test",
        "café.test",
    ] {
        assert!(Certificate::parse(&fixture(vec![san(name)])).is_err());
    }
}

#[test]
fn duplicate_extensions_unsupported_critical_and_constraints_fail_closed() {
    assert!(Extensions::parse(&sequence(&[])).is_err());
    assert!(Certificate::parse(&fixture(vec![san("chatgpt.com"), san("evil.test")])).is_err());
    for extra in [
        extension(99, true, &[5, 0]),
        extension(30, false, &sequence(&[])),
        extension(36, false, &sequence(&[])),
    ] {
        let encoded = fixture(vec![san("chatgpt.com"), extra]);
        let certificate = Certificate::parse(&encoded).unwrap();
        assert!(certificate.has_unsupported_constraints());
        assert_eq!(
            certificate.check_leaf_metadata("chatgpt.com", certificate.not_before),
            Err(Error::Unsupported)
        );
    }
    let encoded = fixture(vec![san("chatgpt.com"), extension(99, false, &[5, 0])]);
    let certificate = Certificate::parse(&encoded).unwrap();
    assert!(!certificate.has_unsupported_constraints());
}

#[test]
fn ca_key_usage_eku_and_noncanonical_defaults_are_rejected_as_leaf() {
    for extra in [
        extension(19, true, &sequence(&[tlv(1, &[255])])),
        extension(15, true, &tlv(3, &[2, 4])),
        extension(
            37,
            false,
            &sequence(&[tlv(6, &[0x2b, 6, 1, 5, 5, 7, 3, 2])]),
        ),
    ] {
        let encoded = fixture(vec![san("chatgpt.com"), extra]);
        let certificate = Certificate::parse(&encoded).unwrap();
        assert_eq!(
            certificate.check_leaf_metadata("chatgpt.com", certificate.not_before),
            Err(Error::Purpose)
        );
    }
    for extra in [
        extension(19, true, &sequence(&[tlv(1, &[0])])),
        extension(19, true, &sequence(&[tlv(2, &[1])])),
        extension(15, true, &tlv(3, &[0, 0x80])),
        extension(15, true, &tlv(3, &[7, 0x81])),
        extension(15, true, &tlv(3, &[0, 1])),
        extension(15, true, &tlv(3, &[7, 0x80, 0x80])),
        extension(37, false, &sequence(&[])),
    ] {
        assert!(Certificate::parse(&fixture(vec![extra])).is_err());
    }
    let encoded = fixture(vec![extension(
        19,
        true,
        &sequence(&[tlv(1, &[255]), tlv(2, &[2])]),
    )]);
    let certificate = Certificate::parse(&encoded).unwrap();
    assert_eq!(certificate.path_length(), Some(2));
}

#[test]
fn der_rejects_indefinite_overlong_truncated_negative_and_duplicate_data() {
    for bytes in [
        vec![0x30, 0x80, 0, 0],
        vec![0x30, 0x81, 0],
        vec![0x30, 0x82, 0, 128],
        vec![0x30, 0xff],
        vec![0x3f, 0],
        vec![0, 0],
    ] {
        assert!(Reader(&bytes).read().is_err());
    }
    for bytes in [vec![2, 0], vec![2, 1, 128], vec![2, 2, 0, 1]] {
        assert!(Reader(&bytes).integer().is_err());
    }
    for oid in [vec![], vec![0x80, 1], vec![1, 0x80, 0], vec![1, 0x81]] {
        assert!(Reader(&tlv(6, &oid)).oid().is_err());
    }
    let encoded = fixture(profile());
    for end in 0..encoded.len() {
        assert!(Certificate::parse(&encoded[..end]).is_err());
    }
    let mut extended = encoded.clone();
    extended.push(0);
    assert!(Certificate::parse(&extended).is_err());
    assert!(matches!(
        Certificate::parse(&vec![0; 65_537]),
        Err(Error::Limit)
    ));
    // Outer/inner algorithm identifiers must agree byte-for-byte.
    let mut changed = encoded;
    let last = changed
        .windows(8)
        .rposition(|w| w == [0x2a, 0x86, 0x48, 0xce, 0x3d, 4, 3, 2])
        .unwrap();
    changed[last + 7] = 3;
    assert!(Certificate::parse(&changed).is_err());
}

#[test]
fn time_calendar_and_encoding_are_strict() {
    for (tag, text, expected) in [
        (0x17, "700101000000Z", 0),
        (0x17, "000229000000Z", 951_782_400),
        (0x17, "691231235959Z", -1),
        (0x18, "20500101000000Z", 2_524_608_000),
    ] {
        let encoded = tlv(tag, text.as_bytes());
        assert_eq!(time::parse(Reader(&encoded).read().unwrap()), Ok(expected));
    }
    for (tag, text) in [
        (0x17, "260229000000Z"),
        (0x17, "260101240000Z"),
        (0x17, "260101000060Z"),
        (0x17, "260101000000+0000"),
        (0x18, "20260101000000Z"),
        (0x18, "21000229000000Z"),
    ] {
        let encoded = tlv(tag, text.as_bytes());
        assert!(time::parse(Reader(&encoded).read().unwrap()).is_err());
    }
}

#[test]
fn public_rfc_certificate_is_parsed_without_becoming_trusted() {
    let line = include_str!("../../../tests/fixtures/rfc8448-handshake.txt")
        .lines()
        .find(|line| line.starts_with("Certificate\t"))
        .unwrap();
    let message = crate::tls::tests::hex(line.split_once('\t').unwrap().1);
    // One certificate: handshake header, context length, list length, DER length.
    let certificate = Certificate::parse(&message[11..message.len() - 2]).unwrap();
    assert_eq!(
        certificate.signature_oid().unwrap(),
        [0x2a, 0x86, 0x48, 0x86, 0xf7, 13, 1, 1, 11]
    );
    assert_eq!(
        certificate.public_key_oid().unwrap(),
        [0x2a, 0x86, 0x48, 0x86, 0xf7, 13, 1, 1, 1]
    );
    assert!(
        certificate
            .check_leaf_metadata("chatgpt.com", certificate.not_before)
            .is_err()
    );
}
