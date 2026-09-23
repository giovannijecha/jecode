use super::*;
use crate::tls::{
    ContentType, HandshakeSecrets, KeyShare, Receiver, Sender, certificate,
    crypto::{nist::fixture, sha256::Sha256},
    schedule::{hmac, label},
    tests::{fixed, hex},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};
const NOW: i64 = 1_790_035_200;
fn client() -> ServerFlight {
    ServerFlight::new(
        "chatgpt.com",
        &[0x24; 32],
        KeyShare::from_bytes(&[0x42; 32]),
    )
    .unwrap()
}
fn message(kind: u8, bytes: &[u8]) -> Vec<u8> {
    [
        vec![
            kind,
            (bytes.len() >> 16) as u8,
            (bytes.len() >> 8) as u8,
            bytes.len() as u8,
        ],
        bytes.to_vec(),
    ]
    .concat()
}
fn flight() -> (Vec<u8>, HandshakeSecrets, [u8; 32]) {
    let hello = client().client_hello().unwrap().to_vec();
    let mut server = hex(
        include_str!("../../../tests/fixtures/rfc8448-handshake.txt")
            .lines()
            .nth(1)
            .unwrap()
            .split_once('\t')
            .unwrap()
            .1,
    );
    let key = KeyShare::from_bytes(&[0x55; 32]);
    let end = server.len();
    server[end - 38..end - 6].copy_from_slice(&key.public());
    let mut hash = Sha256::new();
    hash.update(&hello[5..]);
    hash.update(&server);
    let shared = key
        .shared(&KeyShare::from_bytes(&[0x42; 32]).public())
        .unwrap();
    let secrets = HandshakeSecrets::derive(&shared, &hash.clone().finish());
    let (_, certificate) = certificate::identity();
    let mut entry = message(0, &certificate)[1..].to_vec();
    entry.extend([0, 0]);
    let list = message(0, &entry); // leading zero is the empty request_context
    let extensions = message(8, &[0, 0]);
    let cert = message(11, &list);
    hash.update(&extensions);
    hash.update(&cert);
    let signed = [
        vec![32; 64],
        b"TLS 1.3, server CertificateVerify\0".to_vec(),
        hash.clone().finish().to_vec(),
    ]
    .concat();
    let signature = fixture::sign(1, &signed);
    let proof = message(
        15,
        &[vec![4, 3, 0, signature.len() as u8], signature].concat(),
    );
    hash.update(&proof);
    let finished_key = label::<32>(secrets.server.as_bytes(), b"finished", &[]);
    let finished = message(
        20,
        hmac(finished_key.as_bytes(), &[&hash.clone().finish()]).as_bytes(),
    );
    hash.update(&finished);
    let record = [
        vec![22, 3, 3, (server.len() >> 8) as u8, server.len() as u8],
        server,
    ]
    .concat();
    let encrypted = Sender::new(&secrets.server)
        .seal(
            ContentType::Handshake,
            &[extensions, cert, proof, finished].concat(),
        )
        .unwrap();
    ([record, encrypted].concat(), secrets, hash.finish())
}
fn read_record(socket: &mut TcpStream) -> Vec<u8> {
    let mut header = [0; 5];
    socket.read_exact(&mut header).unwrap();
    let mut output = vec![0; 5 + usize::from(u16::from_be_bytes([header[3], header[4]]))];
    output[..5].copy_from_slice(&header);
    socket.read_exact(&mut output[5..]).unwrap();
    output
}
#[test]
fn authenticated_handshake_preserves_coalesced_data_and_finishes_at_the_right_epoch() {
    let (wire, secrets, hash) = flight();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        assert_eq!(read_record(&mut stream), client().client_hello().unwrap());
        let (client_secret, server_secret) = secrets.application(&hash);
        let mut sender = Sender::new(&server_secret);
        let mut receiver = Receiver::new(&client_secret);
        let data = sender
            .seal(ContentType::Application, b"early authenticated reply")
            .unwrap();
        stream.write_all(&[wire, data].concat()).unwrap();
        let finished = Receiver::new(&secrets.client)
            .open(&read_record(&mut stream))
            .unwrap();
        assert_eq!(finished.kind, ContentType::Handshake);
        assert_eq!(finished.bytes, secrets.client_finished(&hash));
        let request = receiver.open(&read_record(&mut stream)).unwrap();
        assert_eq!(request.bytes, b"application request");
        stream
            .write_all(&sender.seal(ContentType::Alert, &[1, 0]).unwrap())
            .unwrap();
    });
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &cancelled,
    };
    let trust = TrustStore::fixture(vec![certificate::identity().0], &[]);
    let mut connected = Connection::handshake(
        TcpStream::connect(address).unwrap(),
        client(),
        &trust,
        &budget,
        NOW,
    )
    .unwrap();
    connected.write(b"application request", &budget).unwrap();
    assert_eq!(
        connected.read(&budget).unwrap().unwrap().bytes,
        b"early authenticated reply"
    );
    assert!(connected.read(&budget).unwrap().is_none());
    assert_eq!(
        connected.write(b"closed", &budget),
        Err(NetworkError::Closed)
    );
    server.join().unwrap();
}
#[test]
fn untrusted_finished_never_releases_client_finished_or_application_bytes() {
    let (wire, _, _) = flight();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        read_record(&mut socket);
        socket.write_all(&wire).unwrap();
        assert_eq!(socket.read(&mut [0; 1]).unwrap(), 0);
    });
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &cancelled,
    };
    assert!(matches!(
        Connection::handshake(
            TcpStream::connect(address).unwrap(),
            client(),
            &TrustStore::fixture(vec![], &[]),
            &budget,
            NOW
        ),
        Err(NetworkError::Certificate(_))
    ));
    server.join().unwrap();
}
#[test]
fn blocked_partial_record_cancels_and_all_workers_are_joined() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let cancelled = AtomicBool::new(false);
    thread::scope(|scope| {
        let server = scope.spawn(|| {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            socket.write_all(&[23, 3]).unwrap();
            cancelled.store(true, Ordering::Release);
            // Cancellation may close before the partial record is consumed.
            // The peer can observe EOF or reset; neither permits more data.
            let closed = socket.read(&mut [0; 1]);
            assert!(
                matches!(&closed, Ok(0))
                    || matches!(&closed, Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset),
                "cancelled peer was not closed: {closed:?}"
            );
        });
        let mut socket = TcpStream::connect(address).unwrap();
        socket::configure(&socket).unwrap();
        let budget = Budget {
            deadline: Instant::now() + Duration::from_secs(10),
            cancelled: &cancelled,
        };
        assert_eq!(
            socket::record(&mut socket, &budget),
            Err(NetworkError::Cancelled)
        );
        drop(socket);
        server.join().unwrap();
    });
}
#[test]
fn tcp_eof_and_deadlines_are_not_authenticated_success() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let mut socket = TcpStream::connect(address).unwrap();
    let (peer, _) = listener.accept().unwrap();
    drop(peer);
    let cancelled = AtomicBool::new(false);
    let mut budget = Budget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &cancelled,
    };
    socket::configure(&socket).unwrap();
    assert_eq!(
        socket::record(&mut socket, &budget),
        Err(NetworkError::Eof(IoOperation::ReadRecordHeader))
    );
    budget.deadline = Instant::now();
    assert_eq!(
        socket::record(&mut socket, &budget),
        Err(NetworkError::Timeout)
    );
}

#[test]
fn partial_tls_record_eof_identifies_the_record_body() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let sender = thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        peer.write_all(&[23, 3, 3, 0, 4, 1, 2]).unwrap();
    });
    let mut socket = TcpStream::connect(address).unwrap();
    socket::configure(&socket).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        deadline: Instant::now() + Duration::from_secs(5),
        cancelled: &cancelled,
    };
    assert_eq!(
        socket::record(&mut socket, &budget),
        Err(NetworkError::Eof(IoOperation::ReadRecordBody))
    );
    sender.join().unwrap();
}
#[test]
fn rfc8448_application_secrets_and_client_finished_known_answers() {
    let messages: Vec<_> = include_str!("../../../tests/fixtures/rfc8448-handshake.txt")
        .lines()
        .map(|line| hex(line.split_once('\t').unwrap().1))
        .collect();
    let shared = super::super::Secret(fixed(
        "8bd4054fb55b9d63fdfbacf9f04b9f0d35e6d63f537563efd46272900f89492d",
    ));
    let mut hash = Sha256::new();
    hash.update(&messages[0]);
    hash.update(&messages[1]);
    let secrets = HandshakeSecrets::derive(&shared, &hash.clone().finish());
    for message in &messages[2..] {
        hash.update(message);
    }
    let hash = hash.finish();
    let (client, server) = secrets.application(&hash);
    assert_eq!(
        client.as_bytes(),
        &fixed::<32>("9e40646ce79a7f9dc05af8889bce6552875afa0b06df0087f792ebb7c17504a5")
    );
    assert_eq!(
        server.as_bytes(),
        &fixed::<32>("a11af9f05531f856ad47116b45a950328204b4f44bfb6b3a4b4f1f3fcb631643")
    );
    let finished = secrets.client_finished(&hash);
    assert_eq!(
        finished,
        hex("14000020a8ec436d677634ae525ac1fcebe11a039ec17694fac6e98527b642f2edd5ce61")
    );
    assert_eq!(
        Sender::new(&secrets.client)
            .seal(ContentType::Handshake, &finished)
            .unwrap(),
        hex(
            "170303003575ec4dc238cce60b298044a71e219c56cc77b0517fe9b93c7a4bfc44d87f38f80338ac98fc46deb384bd1caeacab6867d726c40546"
        )
    );
}
