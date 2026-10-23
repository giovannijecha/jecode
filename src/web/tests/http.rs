use crate::test_support::server::{self, Server};
use std::io::{BufRead, BufReader, Read, Write};
use std::time::Duration;

#[derive(Debug)]
pub(super) struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

pub(super) struct Reply {
    pub status: u16,
    pub location: Option<String>,
    pub body: String,
}

impl Reply {
    pub fn page(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            location: None,
            body: body.into(),
        }
    }

    pub fn redirect(status: u16, location: impl Into<String>) -> Self {
        Self {
            status,
            location: Some(location.into()),
            body: String::new(),
        }
    }
}

pub(super) fn site(handler: impl Fn(&Request) -> Reply + Send + 'static) -> Server<Vec<Request>> {
    Server::start(move |listener, stop| {
        let mut requests = Vec::new();
        while let Ok(mut stream) = server::accept(&listener, &stop) {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap().to_string();
            let path = parts.next().unwrap().to_string();
            let mut headers = Vec::new();
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                let (name, value) = line.split_once(':').unwrap();
                headers.push((name.to_string(), value.trim().to_string()));
            }
            let length = headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.parse().unwrap())
                .unwrap_or(0);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let request = Request {
                method,
                path,
                headers,
                body,
            };
            let reply = handler(&request);
            let location = reply
                .location
                .map(|location| format!("Location: {location}\r\n"))
                .unwrap_or_default();
            write!(stream, "HTTP/1.1 {} Reply\r\nContent-Type: text/html\r\nContent-Length: {}\r\n{location}Connection: close\r\n\r\n", reply.status, reply.body.len()).unwrap();
            if request.method != "HEAD" {
                let _ = stream.write_all(reply.body.as_bytes());
            }
            requests.push(request);
        }
        requests
    })
}
