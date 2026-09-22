use super::{
    crypto::{secret::Secret, sha256::Sha256},
    *,
};

pub(super) fn hex(text: &str) -> Vec<u8> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(text.len().is_multiple_of(2));
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
pub(super) fn fixed<const N: usize>(text: &str) -> [u8; N] {
    hex(text).try_into().unwrap()
}

#[test]
fn rfc8448_handshake_secrets_finished_and_exact_server_record() {
    let messages: Vec<Vec<u8>> = include_str!("../../tests/fixtures/rfc8448-handshake.txt")
        .lines()
        .map(|line| hex(line.split_once('\t').unwrap().1))
        .collect();
    assert_eq!(messages.len(), 6);
    let client = KeyShare::from_bytes(&fixed(
        "49af42ba7f7994852d713ef2784bcbcaa7911de26adc5642cb634540e7ea5005",
    ));
    let server_public = fixed("c9828876112095fe66762bdbf7c672e156d6cc253b833df1dd69b1b04e751f0f");
    assert_eq!(
        client.public(),
        fixed::<32>("99381de560e4bd43d23d8e435a7dbafeb3c06e51c13cae4d5413691e529aaf2c")
    );
    let shared = client.shared(&server_public).unwrap();
    assert_eq!(
        shared.as_bytes(),
        &fixed::<32>("8bd4054fb55b9d63fdfbacf9f04b9f0d35e6d63f537563efd46272900f89492d")
    );
    let mut transcript = Sha256::new();
    transcript.update(&messages[0]);
    transcript.update(&messages[1]);
    let secrets = HandshakeSecrets::derive(&shared, &transcript.clone().finish());
    assert_eq!(
        secrets.client.as_bytes(),
        &fixed::<32>("b3eddb126e067f35a780b3abf45e2d8f3b1a950738f52e9600746a0e27a55a21")
    );
    assert_eq!(
        secrets.server.as_bytes(),
        &fixed::<32>("b67b7d690cc16c4e75e54213cb2d37b4e9c912bcded9105d42befd59d391ad38")
    );
    for message in &messages[2..5] {
        transcript.update(message);
    }
    let hash = transcript.finish();
    assert!(secrets.verify_server_finished(&hash, &messages[5][4..]));
    let mut changed = hash;
    changed[0] ^= 1;
    assert!(!secrets.verify_server_finished(&changed, &messages[5][4..]));
    assert!(!secrets.verify_server_finished(&hash, &messages[5][5..]));

    let expected = hex(include_str!(
        "../../tests/fixtures/rfc8448-server-record.txt"
    ));
    assert_eq!(expected.len(), 679);
    let content = messages[2..].concat();
    let produced = Sender::new(&secrets.server)
        .seal(ContentType::Handshake, &content)
        .unwrap();
    assert_eq!(produced, expected);
    let decoded = Receiver::new(&secrets.server).open(&expected).unwrap();
    assert_eq!(decoded.kind, ContentType::Handshake);
    assert_eq!(decoded.bytes, content);
}

#[test]
fn records_enforce_sequence_integrity_limits_and_terminal_close() {
    let secret = Secret([7; 32]);
    let mut sender = Sender::new(&secret);
    let first = sender.seal(ContentType::Application, b"hello").unwrap();
    let second = sender.seal(ContentType::Application, b"hello").unwrap();
    assert_ne!(first, second);
    let mut receiver = Receiver::new(&secret);
    assert_eq!(receiver.open(&first).unwrap().bytes, b"hello");
    assert_eq!(receiver.open(&second).unwrap().bytes, b"hello");
    assert!(matches!(receiver.open(&second), Err(Error::Authentication)));
    assert!(matches!(receiver.open(&first), Err(Error::Closed)));
    assert!(matches!(
        Receiver::new(&secret).open(&second),
        Err(Error::Authentication)
    ));
    for index in 0..first.len() {
        let mut altered = first.clone();
        altered[index] ^= 1;
        let mut receiver = Receiver::new(&secret);
        assert!(receiver.open(&altered).is_err());
        assert!(matches!(receiver.open(&first), Err(Error::Closed)));
    }
    for length in 0..first.len() {
        assert!(Receiver::new(&secret).open(&first[..length]).is_err());
    }
    let mut extended = first.clone();
    extended.push(0);
    assert!(Receiver::new(&secret).open(&extended).is_err());
    let largest = Sender::new(&secret)
        .seal(ContentType::Application, &[42; 16_384])
        .unwrap();
    assert_eq!(
        Receiver::new(&secret).open(&largest).unwrap().bytes,
        [42; 16_384]
    );
    let mut sender = Sender::new(&secret);
    assert_eq!(
        sender.seal(ContentType::Application, &[0; 16_385]),
        Err(Error::Limit)
    );
    assert_eq!(
        sender.seal(ContentType::Application, &[]),
        Err(Error::Closed)
    );
    let empty = Sender::new(&secret)
        .seal(ContentType::Application, &[])
        .unwrap();
    assert!(
        Receiver::new(&secret)
            .open(&empty)
            .unwrap()
            .bytes
            .is_empty()
    );
    let mut receiver = Receiver::new(&secret);
    receiver.close();
    assert!(matches!(receiver.open(&first), Err(Error::Closed)));
    let mut sender = Sender::new(&secret);
    sender.close();
    assert_eq!(
        sender.seal(ContentType::Application, &[]),
        Err(Error::Closed)
    );
}

#[test]
fn authenticated_inner_types_padding_and_empty_handshakes_are_checked() {
    use super::{crypto::aes_gcm::Key, schedule::label};
    let secret = Secret([3; 32]);
    let key = label::<16>(secret.as_bytes(), b"key", &[]);
    let iv = label::<12>(secret.as_bytes(), b"iv", &[]);
    for (mut inner, expected) in [
        (
            vec![1, 2, 23, 0, 0],
            Some((ContentType::Application, vec![1, 2])),
        ),
        (vec![1, 0, 21, 0], Some((ContentType::Alert, vec![1, 0]))),
        (vec![22, 0], None),
        (vec![0, 0, 0], None),
        (vec![1, 20], None),
        (vec![1, 21], None),
    ] {
        let length = (inner.len() + 16) as u16;
        let header = [23, 3, 3, (length >> 8) as u8, length as u8];
        let tag = Key::new(&key.as_bytes()[..16])
            .unwrap()
            .seal(iv.as_bytes()[..12].try_into().unwrap(), &header, &mut inner)
            .unwrap();
        let wire = [&header[..], &inner, &tag].concat();
        let result = Receiver::new(&secret).open(&wire);
        if let Some((kind, bytes)) = expected {
            let result = result.unwrap();
            assert_eq!(result.kind, kind);
            assert_eq!(result.bytes, bytes);
        } else {
            assert!(matches!(result, Err(Error::Malformed)));
        }
    }
}
