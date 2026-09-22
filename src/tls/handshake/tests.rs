use super::*;
use crate::tls::{
    Sender,
    schedule::{hmac, label},
    tests::fixed,
};

const CLIENT_SEED: [u8; 32] = [0x42; 32];
const CLIENT_RANDOM: [u8; 32] = [0x24; 32];
fn client() -> ServerFlight {
    ServerFlight::new(
        "chatgpt.com",
        &CLIENT_RANDOM,
        KeyShare::from_bytes(&CLIENT_SEED),
    )
    .unwrap()
}
fn message(kind: u8, body: &[u8]) -> Vec<u8> {
    let length = body.len();
    [
        vec![
            kind,
            (length >> 16) as u8,
            (length >> 8) as u8,
            length as u8,
        ],
        body.to_vec(),
    ]
    .concat()
}
fn plain(bytes: &[u8]) -> Vec<u8> {
    let mut record = vec![22, 3, 3];
    wire::vector16(&mut record, bytes);
    record
}
fn fixture_messages() -> Vec<Vec<u8>> {
    include_str!("../../../tests/fixtures/rfc8448-handshake.txt")
        .lines()
        .map(|line| crate::tls::tests::hex(line.split_once('\t').unwrap().1))
        .collect()
}
// The server reuses only public RFC key/certificate fixtures. Its transcript
// binds the newly generated narrow ClientHello; it cannot reuse the RFC Finished.
fn server_messages() -> (Vec<u8>, HandshakeSecrets, Vec<Vec<u8>>) {
    let hello = client().client_hello().unwrap().to_vec();
    let fixture = fixture_messages();
    let server = KeyShare::from_bytes(&fixed(
        "b1580eeadf6dd589b8ef4f2d5652578cc810e9980191ec8d058308cea216a21e",
    ));
    let shared = server
        .shared(&KeyShare::from_bytes(&CLIENT_SEED).public())
        .unwrap();
    let mut hash = Sha256::new();
    hash.update(&hello[5..]);
    hash.update(&fixture[1]);
    let secrets = HandshakeSecrets::derive(&shared, &hash.clone().finish());
    let mut messages = vec![
        message(8, b"\x00\x00"),
        fixture[3].clone(),
        fixture[4].clone(),
    ];
    for message in &messages {
        hash.update(message);
    }
    let key = label::<32>(secrets.server.as_bytes(), b"finished", &[]);
    let finished = hmac(key.as_bytes(), &[&hash.finish()]);
    messages.push(message(20, finished.as_bytes()));
    (plain(&fixture[1]), secrets, messages)
}
fn protected(secrets: &HandshakeSecrets, messages: &[Vec<u8>]) -> Vec<u8> {
    Sender::new(&secrets.server)
        .seal(ContentType::Handshake, &messages.concat())
        .unwrap()
}
fn rejects_once(mut flight: ServerFlight, bytes: &[u8]) {
    assert!(flight.push(bytes).is_err());
    assert!(!flight.is_ready_for_verification());
    assert_eq!(flight.push(&[]), Err(Error::Closed));
    assert!(matches!(flight.finish(), Err(Error::Closed)));
}

#[test]
fn client_hello_pins_protocol_suite_group_signatures_and_http11() {
    let flight = client();
    let wire = flight.client_hello().unwrap();
    assert_eq!(&wire[..3], &[22, 3, 1]);
    assert_eq!(
        usize::from(u16::from_be_bytes([wire[3], wire[4]])),
        wire.len() - 5
    );
    assert_eq!(wire[5], 1);
    assert_eq!(wire::length24(&wire[6..9]), wire.len() - 9);
    let mut body = wire::Cursor(&wire[9..]);
    assert_eq!(body.word().unwrap(), 0x0303);
    assert_eq!(body.take(32).unwrap(), CLIENT_RANDOM);
    assert_eq!(body.vector8().unwrap(), b"");
    assert_eq!(body.vector16().unwrap(), b"\x13\x01");
    assert_eq!(body.vector8().unwrap(), b"\0");
    let extensions = wire::extensions(body.vector16().unwrap()).unwrap();
    body.end().unwrap();
    assert_eq!(extensions.len(), 6);
    for (id, value) in extensions {
        match id {
            0 => assert_eq!(value, b"\0\x0e\0\0\x0bchatgpt.com"),
            43 => assert_eq!(value, &[2, 3, 4]),
            10 => assert_eq!(value, &[0, 2, 0, 29]),
            13 => assert_eq!(value, &[0, 4, 8, 4, 4, 3]),
            16 => assert_eq!(value, b"\0\x09\x08http/1.1"),
            51 => {
                assert_eq!(&value[..6], &[0, 36, 0, 29, 0, 32]);
                assert_eq!(&value[6..], KeyShare::from_bytes(&CLIENT_SEED).public());
            }
            _ => panic!("unexpected offer"),
        }
    }
    for host in [
        "",
        "a..b",
        "-x.test",
        "x-.test",
        "https://chatgpt.com",
        "a\r\nx",
        "127.0.0.1",
        "::1",
        "café.test",
    ] {
        assert!(
            ServerFlight::new(host, &CLIENT_RANDOM, KeyShare::from_bytes(&CLIENT_SEED)).is_err()
        );
    }
}

#[test]
fn fragmented_flight_returns_only_unverified_peer_evidence() {
    let (hello, secrets, messages) = server_messages();
    let wire = [hello, protected(&secrets, &messages)].concat();
    // Header, body and UTF-agnostic DER bytes may all cross transport chunks.
    for chunk_size in [1, 2, 3, 4, 5, 7, 31, 64, wire.len()] {
        let mut flight = client();
        for chunk in wire.chunks(chunk_size) {
            flight.push(chunk).unwrap();
        }
        assert!(flight.is_ready_for_verification());
        let peer = flight.finish().unwrap();
        assert_eq!(peer.host(), "chatgpt.com");
        assert_eq!(peer.certificates().len(), 1);
        assert_eq!(peer.certificates()[0].len(), 432);
        assert_eq!(peer.signature_algorithm(), 0x0804);
        assert_eq!(peer.signature().len(), 128);
        assert_eq!(&peer.signed_message()[..64], &[0x20; 64]);
        assert_eq!(
            &peer.signed_message()[64..98],
            b"TLS 1.3, server CertificateVerify\0"
        );
        assert_eq!(peer.signed_message().len(), 130);
        let mut hash = Sha256::new();
        hash.update(&client().client_hello().unwrap()[5..]);
        hash.update(&fixture_messages()[1]);
        hash.update(&messages[0]);
        hash.update(&messages[1]);
        assert_eq!(&peer.signed_message()[98..], hash.finish());
    }
}

#[test]
fn handshake_messages_can_span_records_but_never_cross_key_changes() {
    let (hello, secrets, messages) = server_messages();
    let mut flight = client();
    let server_hello = &hello[5..];
    flight.push(&plain(&server_hello[..3])).unwrap();
    flight.push(&plain(&server_hello[3..])).unwrap();
    let mut sender = Sender::new(&secrets.server);
    let all = messages.concat();
    for chunk in all.chunks(11) {
        flight
            .push(&sender.seal(ContentType::Handshake, chunk).unwrap())
            .unwrap();
    }
    assert!(flight.finish().is_ok());
    // ServerHello must finish its record before encrypted handshake traffic.
    rejects_once(client(), &plain(&[server_hello, &[8]].concat()));
    let mut messages = messages;
    messages.push(message(4, b"ticket"));
    let mut flight = client();
    flight.push(&hello).unwrap();
    rejects_once(flight, &protected(&secrets, &messages));
}

#[test]
fn duplicates_order_invalid_finished_and_early_application_data_fail_closed() {
    let (hello, secrets, original) = server_messages();
    for messages in [
        vec![original[1].clone()],
        vec![original[0].clone(), original[0].clone()],
        vec![
            original[0].clone(),
            original[1].clone(),
            original[3].clone(),
        ],
        vec![message(13, b"client certificate requested")],
    ] {
        let mut flight = client();
        flight.push(&hello).unwrap();
        rejects_once(flight, &protected(&secrets, &messages));
    }
    let mut messages = original;
    messages[3][4] ^= 1;
    let mut flight = client();
    flight.push(&hello).unwrap();
    rejects_once(flight, &protected(&secrets, &messages));
    let mut flight = client();
    flight.push(&hello).unwrap();
    rejects_once(
        flight,
        &Sender::new(&secrets.server)
            .seal(ContentType::Application, b"HTTP")
            .unwrap(),
    );
    let mut flight = client();
    flight.push(&hello).unwrap();
    rejects_once(flight, &plain(&message(8, b"\0\0")));
}

#[test]
fn negotiated_parameters_and_extensions_cannot_be_substituted() {
    let mut body = fixture_messages()[1][4..].to_vec();
    for (offset, value) in [(0, 2), (35, 0), (36, 2), (37, 1)] {
        let mut changed = body.clone();
        changed[offset] = value;
        rejects_once(client(), &plain(&message(2, &changed)));
    }
    // HelloRetryRequest has a distinct transcript protocol: explicitly unsupported.
    body[2..34].copy_from_slice(&fixed::<32>(
        "cf21ad74e59a6111be1d8c021e65b891c2a211167abb8c5e079e09e2c8a8339c",
    ));
    rejects_once(client(), &plain(&message(2, &body)));
    let (hello, secrets, _) = server_messages();
    for extension_body in [
        b"\0\x04\0\x2b\0\0".to_vec(), // supported_versions in wrong message
        b"\0\x09\0\x10\0\x05\0\x03\x02h2".to_vec(),
        b"\0\x08\0\0\0\0\0\0\0\0".to_vec(), // duplicate SNI
        b"\0\x04\xff\xff\0\0".to_vec(),
    ] {
        let mut flight = client();
        flight.push(&hello).unwrap();
        rejects_once(flight, &protected(&secrets, &[message(8, &extension_body)]));
    }
}

#[test]
fn cancellation_eof_alerts_and_resource_limits_do_not_complete_a_flight() {
    let (hello, secrets, messages) = server_messages();
    let wire = [hello.clone(), protected(&secrets, &messages)].concat();
    for length in [0, 1, 4, 5, hello.len() - 1, hello.len(), wire.len() - 1] {
        let mut flight = client();
        flight.push(&wire[..length]).unwrap();
        assert!(matches!(flight.finish(), Err(Error::Truncated)));
    }
    let mut flight = client();
    flight.push(&hello).unwrap();
    flight.cancel();
    assert_eq!(flight.push(&wire), Err(Error::Closed));
    assert!(matches!(flight.finish(), Err(Error::Closed)));
    rejects_once(client(), &[22, 3, 3, 0xff, 0xff]); // reject length before body allocation
    rejects_once(client(), &plain(&[2, 0xff, 0xff, 0xff]));
    rejects_once(client(), &[0; MAX_WIRE + 1]);
    let mut flight = client();
    flight.0.as_mut().unwrap().records = MAX_RECORDS;
    rejects_once(flight, &hello);
    rejects_once(client(), &[21, 3, 3, 0, 2, 2, 40]);
    let mut flight = client();
    flight.push(&hello).unwrap();
    rejects_once(
        flight,
        &Sender::new(&secrets.server)
            .seal(ContentType::Alert, &[2, 40])
            .unwrap(),
    );
    // Byte following the final record cannot be silently accepted/dropped.
    let mut extra = wire.clone();
    extra.push(0);
    rejects_once(client(), &extra);
    let mut flight = client();
    flight.push(&wire).unwrap();
    rejects_once(flight, &[]);
}

#[test]
fn compatibility_ccs_is_bounded_and_cannot_interrupt_fragmented_handshake() {
    let (hello, secrets, messages) = server_messages();
    let ccs = [20, 3, 3, 0, 1, 1];
    let mut flight = client();
    flight.push(&ccs).unwrap();
    flight.push(&hello).unwrap();
    flight.push(&ccs).unwrap();
    flight.push(&protected(&secrets, &messages)).unwrap();
    assert!(flight.finish().is_ok());
    let mut flight = client();
    flight.push(&ccs).unwrap();
    flight.push(&ccs).unwrap();
    rejects_once(flight, &ccs);
    let mut flight = client();
    flight.push(&plain(&hello[5..8])).unwrap();
    rejects_once(flight, &ccs);
    rejects_once(client(), &[20, 3, 3, 0, 1, 2]);
}

#[test]
fn certificate_lists_and_signature_envelopes_are_bounded_untrusted_data() {
    for body in [
        b"\0\0\0\0".as_slice(),
        b"\x01x\0\0\0",
        b"\0\0\0\x05\0\0\0\0\0",
    ] {
        assert!(peer::certificates(body).is_err());
    }
    let entry = [0, 0, 1, 42, 0, 0];
    let many = entry.repeat(9);
    let body = [vec![0, 0, 0, many.len() as u8], many].concat();
    assert!(matches!(peer::certificates(&body), Err(Error::Limit)));
    let (hello, secrets, messages) = server_messages();
    for signature in [vec![4, 1, 0, 1, 0], vec![8, 4, 0, 0], vec![8, 4, 0, 2, 1]] {
        let mut flight = client();
        flight.push(&hello).unwrap();
        rejects_once(
            flight,
            &protected(
                &secrets,
                &[
                    messages[0].clone(),
                    messages[1].clone(),
                    message(15, &signature),
                ],
            ),
        );
    }
}
