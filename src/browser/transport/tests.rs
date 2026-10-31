use super::{Cdp, base64, sha1};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn endpoint(listener: &TcpListener) -> String {
    format!(
        "ws://127.0.0.1:{}/devtools/page/handshake",
        listener.local_addr().unwrap().port()
    )
}

fn accept_key(socket: &mut TcpStream) -> String {
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut request = Vec::new();
    let mut byte = [0];
    while !request.ends_with(b"\r\n\r\n") {
        socket.read_exact(&mut byte).unwrap();
        request.push(byte[0]);
    }
    let request = String::from_utf8(request).unwrap();
    let key = request
        .lines()
        .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
        .unwrap()
        .trim();
    base64(&sha1(
        format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
    ))
}

#[test]
fn delayed_fragmented_handshake_succeeds_within_two_seconds() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = endpoint(&listener);
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let accept = accept_key(&mut socket);
        thread::sleep(Duration::from_millis(350));
        let _ = socket.write_all(b"HTTP/1.1 101 Switching Protocols\r\n");
        thread::sleep(Duration::from_millis(150));
        let _ = write!(socket, "Sec-WebSocket-Accept: {accept}\r\n\r\n");
    });
    let result = Cdp::connect(&url);
    server.join().unwrap();
    let connection = result.unwrap_or_else(|error| panic!("delayed handshake failed: {error}"));
    assert!(connection.usable());
}

#[test]
fn incomplete_handshake_expires_despite_fragment_progress() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = endpoint(&listener);
    let (done, stopped) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let _ = accept_key(&mut socket);
        if socket
            .write_all(b"HTTP/1.1 101 Switching Protocols\r\n")
            .is_err()
        {
            return;
        }
        for _ in 0..60 {
            if stopped.try_recv().is_ok() || socket.write_all(b"x").is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    });
    let started = Instant::now();
    let error = Cdp::connect(&url)
        .err()
        .expect("incomplete handshake must fail");
    let elapsed = started.elapsed();
    let _ = done.send(());
    server.join().unwrap();
    assert!(error.contains("handshake timed out"), "{error}");
    assert!(
        elapsed >= Duration::from_millis(1700) && elapsed < Duration::from_secs(4),
        "handshake expired after {elapsed:?}"
    );
}
