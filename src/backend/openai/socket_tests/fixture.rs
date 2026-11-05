use crate::json::{self, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

/// SHA-1 exists here only because RFC 6455 requires it in the handshake.
fn sha1(message: &[u8]) -> [u8; 20] {
    let mut state: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut padded = message.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&((message.len() as u64) * 8).to_be_bytes());
    for block in padded.chunks(64) {
        let mut words = [0u32; 80];
        for (index, word) in block.chunks(4).enumerate() {
            words[index] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in words.iter().enumerate() {
            let (mix, constant) = match index {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(mix)
                .wrapping_add(e)
                .wrapping_add(constant)
                .wrapping_add(*word);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, next);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut digest = [0; 20];
    for (slot, word) in digest.chunks_mut(4).zip(state) {
        slot.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

#[test]
fn the_fixture_handshake_digest_matches_the_rfc_example() {
    let digest = sha1(b"dGhlIHNhbXBsZSBub25jZQ==258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    assert_eq!(
        crate::attachments::base64::encode(&digest),
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    );
}

/// The server side of one fixture connection.
pub(super) struct Peer {
    pub(super) reader: BufReader<TcpStream>,
    pub(super) writer: TcpStream,
}

impl Peer {
    pub(super) fn new(stream: TcpStream) -> Self {
        Self {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
        }
    }

    /// The request line and lowercase headers.
    pub(super) fn head(&mut self) -> (String, Vec<(String, String)>) {
        let mut first = String::new();
        self.reader.read_line(&mut first).unwrap();
        let mut headers = Vec::new();
        loop {
            let mut line = String::new();
            self.reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
        }
        (first, headers)
    }

    pub(super) fn upgrade(&mut self) -> Vec<(String, String)> {
        let (first, headers) = self.head();
        assert!(first.starts_with("GET /responses HTTP/1.1"), "{first}");
        let key = header(&headers, "sec-websocket-key").unwrap();
        let accept = crate::attachments::base64::encode(&sha1(
            format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
        ));
        write!(
            self.writer,
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
        )
        .unwrap();
        headers
    }

    /// The next whole message; `None` once the client is gone or closing.
    pub(super) fn message(&mut self) -> Option<Value> {
        let mut message = Vec::new();
        loop {
            let mut head = [0; 2];
            self.reader.read_exact(&mut head).ok()?;
            let length = match head[1] & 0x7f {
                126 => {
                    let mut bytes = [0; 2];
                    self.reader.read_exact(&mut bytes).ok()?;
                    u16::from_be_bytes(bytes) as usize
                }
                127 => {
                    let mut bytes = [0; 8];
                    self.reader.read_exact(&mut bytes).ok()?;
                    u64::from_be_bytes(bytes) as usize
                }
                short => short as usize,
            };
            let mut mask = [0; 4];
            self.reader.read_exact(&mut mask).ok()?;
            let mut payload = vec![0; length];
            self.reader.read_exact(&mut payload).ok()?;
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index & 3];
            }
            match head[0] & 0x0f {
                8 => return None,
                9 | 10 => continue,
                _ => message.extend_from_slice(&payload),
            }
            if head[0] & 0x80 != 0 {
                return Some(json::parse(std::str::from_utf8(&message).unwrap()).unwrap());
            }
        }
    }

    pub(super) fn send(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let mut frame = vec![0x81];
        if bytes.len() < 126 {
            frame.push(bytes.len() as u8);
        } else {
            frame.push(126);
            frame.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        }
        frame.extend_from_slice(bytes);
        self.writer.write_all(&frame).unwrap();
    }

    pub(super) fn answer(&mut self, text: &str) {
        for event in reply(text) {
            self.send(&event);
        }
        self.send(r#"{"type":"responsesapi.websocket_timing","timing_metrics":{}}"#);
    }
}

pub(super) fn reply(text: &str) -> Vec<String> {
    let item = Value::object([
        ("type", Value::string("message")),
        ("role", Value::string("assistant")),
        (
            "content",
            Value::Array(vec![Value::object([
                ("type", Value::string("output_text")),
                ("text", Value::string(text)),
            ])]),
        ),
    ]);
    vec![
        Value::object([
            ("type", Value::string("response.output_text.delta")),
            ("output_index", Value::number(0)),
            ("delta", Value::string(text)),
        ])
        .encode(),
        Value::object([
            ("type", Value::string("response.output_item.done")),
            ("output_index", Value::number(0)),
            ("item", item),
        ])
        .encode(),
        Value::object([
            ("type", Value::string("response.completed")),
            (
                "response",
                Value::object([
                    ("status", Value::string("completed")),
                    ("output", Value::Array(vec![])),
                ]),
            ),
        ])
        .encode(),
    ]
}

pub(super) fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}
