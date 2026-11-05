use super::*;
use crate::test_support::server::{self, Server};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

fn request(reader: &mut BufReader<TcpStream>) -> (String, Vec<(String, String)>, Vec<u8>) {
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.trim_end().split_once(':').unwrap();
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .map_or(0, |(_, value)| value.parse().unwrap());
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    (first, headers, body)
}

#[test]
fn body_framing_is_owned_by_the_transport_on_reused_connections() {
    let server = Server::start(|listener, stop| {
        let stream = server::accept(&listener, &stop).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut requests = Vec::new();
        for _ in 0..2 {
            requests.push(request(&mut reader));
            writer
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .unwrap();
        }
        requests
    });
    // Each caller header disagrees with the body. Neither may change where
    // that body ends or leave bytes at the start of the following request.
    let pool = pool::Pool::new();
    for headers in [
        vec![("cOnTeNt-LeNgTh", "0")],
        vec![("Transfer-Encoding", "chunked"), ("Content-Length", "999")],
    ] {
        let output = pool
            .run(
                wire_request(&Request {
                    url: &server.endpoint,
                    method: "POST",
                    headers: &headers,
                    body: Some(b"payload"),
                    timeout: Some(Duration::from_secs(5)),
                    idle: None,
                    output_limit: 4096,
                }),
                pool::Limits {
                    timeout: Some(Duration::from_secs(5)),
                    idle: None,
                    output: 4096,
                },
                &Cancellation::default(),
                &mut |_| Ok(()),
            )
            .unwrap();
        assert_eq!(
            output.exit_code,
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for (first, headers, body) in server.finish() {
        assert_eq!(first, "POST / HTTP/1.1\r\n");
        assert_eq!(body, b"payload");
        assert_eq!(
            headers
                .iter()
                .filter(|(name, _)| name == "content-length")
                .count(),
            1
        );
        assert!(!headers.iter().any(|(name, _)| name == "transfer-encoding"));
    }
}
