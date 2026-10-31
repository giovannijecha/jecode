use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(super) use crate::attachments::base64::encode as base64;

#[cfg(test)]
mod tests;

/// Whether a `Cdp::call` error came from its deadline rather than the page.
pub(super) fn timed_out(error: &str) -> bool {
    error.starts_with("DevTools ") && error.ends_with(" timed out")
}

pub(super) fn http_get(port: u16, path: &str) -> Result<Value, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).map_err(|e| e.to_string())?;
        bytes.push(byte[0]);
        if bytes.len() > 8192 {
            return Err("Oversized DevTools HTTP headers".into());
        }
    }
    let headers = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    if !headers.starts_with("HTTP/1.1 200") && !headers.starts_with("HTTP/1.0 200") {
        return Err(format!(
            "DevTools HTTP error: {}",
            headers.lines().next().unwrap_or("")
        ));
    }
    let length = headers
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(key, _)| key.eq_ignore_ascii_case("Content-Length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        })
        .ok_or("DevTools HTTP response has no length")?;
    if length > 4_000_000 {
        return Err("Oversized DevTools HTTP response".into());
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body).map_err(|e| e.to_string())?;
    json::parse(&String::from_utf8(body).map_err(|e| e.to_string())?)
}

pub(super) struct Cdp {
    stream: TcpStream,
    next_id: u64,
    usable: bool,
    pub events: super::logs::EventLog,
}

impl Cdp {
    pub fn connect(url: &str) -> Result<Self, String> {
        let endpoint = url
            .strip_prefix("ws://")
            .ok_or("Only local ws:// DevTools endpoints are supported")?;
        let (authority, path) = endpoint
            .split_once('/')
            .ok_or("Invalid DevTools endpoint")?;
        let (host, port) = authority
            .rsplit_once(':')
            .ok_or("DevTools endpoint has no port")?;
        if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
            return Err("DevTools endpoint must be loopback".into());
        }
        let port: u16 = port.parse().map_err(|_| "Invalid DevTools port")?;
        let addr = ("127.0.0.1", port)
            .to_socket_addrs()
            .map_err(|e| e.to_string())?
            .next()
            .ok_or("No loopback address")?;
        let mut stream =
            TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
            .to_be_bytes();
        let key = base64(&nonce);
        write!(stream, "GET /{path} HTTP/1.1\r\nHost: {authority}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n").map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut headers = Vec::new();
        let mut byte = [0u8];
        while !headers.ends_with(b"\r\n\r\n") {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("DevTools WebSocket handshake timed out".into());
            }
            stream
                .set_read_timeout(Some(remaining.max(Duration::from_millis(1))))
                .map_err(|e| e.to_string())?;
            match stream.read(&mut byte) {
                Ok(0) => return Err("DevTools connection closed".into()),
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error.to_string()),
            }
            if Instant::now() >= deadline {
                return Err("DevTools WebSocket handshake timed out".into());
            }
            headers.push(byte[0]);
            if headers.len() > 8192 {
                return Err("Oversized DevTools handshake".into());
            }
        }
        let response = String::from_utf8(headers).map_err(|e| e.to_string())?;
        if !response.starts_with("HTTP/1.1 101") && !response.starts_with("HTTP/1.0 101") {
            return Err("DevTools WebSocket handshake failed".into());
        }
        let accept = base64(&sha1(
            format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
        ));
        if !response.lines().any(|line| {
            line.trim()
                .eq_ignore_ascii_case(&format!("Sec-WebSocket-Accept: {accept}"))
        }) {
            return Err("DevTools WebSocket handshake key mismatch".into());
        }
        Ok(Self {
            stream,
            next_id: 0,
            usable: true,
            events: super::logs::EventLog::default(),
        })
    }

    pub fn usable(&self) -> bool {
        self.usable
    }

    pub fn invalidate(&mut self) {
        self.usable = false;
    }

    pub fn call(
        &mut self,
        method: &str,
        params: Value,
        cancellation: &Cancellation,
        timeout: Duration,
    ) -> Result<Value, String> {
        if !self.usable {
            return Err("DevTools connection is no longer usable".into());
        }
        self.next_id += 1;
        let id = self.next_id;
        let request = Value::object([
            ("id", Value::number(id)),
            ("method", Value::string(method)),
            ("params", params),
        ]);
        self.frame(1, request.encode().as_bytes())
            .inspect_err(|_| {
                self.usable = false;
            })?;
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            self.usable = false;
            "DevTools timeout is too large".to_string()
        })?;
        loop {
            if cancellation.requested() {
                self.usable = false;
                return Err("Operation cancelled".into());
            }
            if Instant::now() >= deadline {
                self.usable = false;
                return Err(format!("DevTools {method} timed out"));
            }
            let message = match self.message(deadline, cancellation).inspect_err(|_| {
                self.usable = false;
            })? {
                Some(message) => message,
                None => continue,
            };
            if cancellation.requested() {
                self.usable = false;
                return Err("Operation cancelled".into());
            }
            if Instant::now() >= deadline {
                self.usable = false;
                return Err(format!("DevTools {method} timed out"));
            }
            let text = String::from_utf8(message).map_err(|_| {
                self.usable = false;
                "Invalid DevTools UTF-8".to_string()
            })?;
            let value = json::parse(&text).inspect_err(|_| {
                self.usable = false;
            })?;
            if value.get("id").and_then(Value::as_usize) == Some(id as usize) {
                if let Some(error) = value.get("error") {
                    return Err(format!("DevTools {method}: {}", error.encode()));
                }
                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
            }
            if value.get("method").is_some() {
                self.events.push(value);
            }
        }
    }

    fn frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        let mut header = vec![0x80 | opcode];
        match payload.len() {
            0..=125 => header.push(0x80 | payload.len() as u8),
            126..=65535 => {
                header.push(0x80 | 126);
                header.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            }
            _ => {
                header.push(0x80 | 127);
                header.extend_from_slice(&(payload.len() as u64).to_be_bytes());
            }
        }
        let mask = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .subsec_nanos()
            .to_be_bytes();
        header.extend_from_slice(&mask);
        self.stream.write_all(&header).map_err(|e| e.to_string())?;
        let masked: Vec<u8> = payload
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ mask[i % 4])
            .collect();
        self.stream.write_all(&masked).map_err(|e| e.to_string())
    }

    fn message(
        &mut self,
        deadline: Instant,
        cancellation: &Cancellation,
    ) -> Result<Option<Vec<u8>>, String> {
        let mut collected = Vec::new();
        let mut fragment = false;
        loop {
            let mut head = [0u8; 2];
            if !self.read_exact(&mut head, deadline, cancellation)? {
                return Ok(None);
            }
            let length = match head[1] & 0x7f {
                n @ 0..=125 => n as u64,
                126 => {
                    let mut b = [0; 2];
                    self.read_exact(&mut b, deadline, cancellation)?;
                    u16::from_be_bytes(b) as u64
                }
                _ => {
                    let mut b = [0; 8];
                    self.read_exact(&mut b, deadline, cancellation)?;
                    u64::from_be_bytes(b)
                }
            };
            if length > 16_000_000 {
                return Err("Oversized DevTools frame".into());
            }
            let mut mask = [0u8; 4];
            let masked = head[1] & 0x80 != 0;
            if masked {
                self.read_exact(&mut mask, deadline, cancellation)?;
            }
            let mut body = vec![0; length as usize];
            self.read_exact(&mut body, deadline, cancellation)?;
            if masked {
                for (i, b) in body.iter_mut().enumerate() {
                    *b ^= mask[i % 4];
                }
            }
            match head[0] & 0x0f {
                8 => return Err("DevTools connection closed".into()),
                9 => {
                    self.frame(10, &body)?;
                }
                10 => {}
                1 => {
                    collected = body;
                    fragment = true;
                }
                0 if fragment => collected.extend(body),
                _ => return Err("Unexpected DevTools frame".into()),
            }
            if collected.len() > 16_000_000 {
                return Err("Oversized DevTools message".into());
            }
            if fragment && head[0] & 0x80 != 0 {
                return Ok(Some(collected));
            }
        }
    }

    fn read_exact(
        &mut self,
        buf: &mut [u8],
        deadline: Instant,
        cancellation: &Cancellation,
    ) -> Result<bool, String> {
        let mut done = 0;
        while done < buf.len() {
            if cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("DevTools read timed out".into());
            }
            self.stream
                .set_read_timeout(Some(remaining.min(Duration::from_millis(200))))
                .map_err(|error| error.to_string())?;
            match self.stream.read(&mut buf[done..]) {
                Ok(0) => return Err("DevTools connection closed".into()),
                Ok(n) => done += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(true)
    }
}

pub(super) fn sha1(bytes: &[u8]) -> [u8; 20] {
    let mut data = bytes.to_vec();
    let bits = (data.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bits.to_be_bytes());
    let mut h = [
        0x67452301u32,
        0xefcdab89,
        0x98badcfe,
        0x10325476,
        0xc3d2e1f0,
    ];
    for block in data.chunks(64) {
        let mut w = [0u32; 80];
        for (i, c) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes(c.try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (slot, n) in h.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(n);
        }
    }
    let mut out = [0; 20];
    for (i, n) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&n.to_be_bytes());
    }
    out
}
