//! Native HTTP transport. Request details travel on stdin, never command arguments.

use crate::cancel::Cancellation;
use crate::process::{self, Output};
use std::process::Command;
use std::time::Duration;

pub(crate) struct Request<'a> {
    pub url: &'a str,
    pub method: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: Option<&'a [u8]>,
    pub timeout: Option<Duration>,
    pub idle: Option<Duration>,
    pub output_limit: usize,
}

pub(crate) fn run(
    request: Request<'_>,
    cancellation: &Cancellation,
    observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
) -> Result<Output, String> {
    let mut command = Command::new(crate::native::helper("http")?);
    crate::auth::isolate_environment(&mut command);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::process::CREATE_NO_WINDOW);
    }
    let mut input = String::new();
    input.push_str(&encode(request.url.as_bytes()));
    input.push('\n');
    input.push_str(&encode(request.method.as_bytes()));
    input.push('\n');
    input.push_str(
        &request
            .timeout
            .unwrap_or(Duration::from_secs(600))
            .as_millis()
            .to_string(),
    );
    input.push('\n');
    input.push_str(&request.headers.len().to_string());
    input.push('\n');
    for (name, value) in request.headers {
        input.push_str(&encode(name.as_bytes()));
        input.push('\n');
        input.push_str(&encode(value.as_bytes()));
        input.push('\n');
    }
    input.push_str(&encode(request.body.unwrap_or_default()));
    input.push('\n');
    // The native helper owns no descendants; the runner enforces cancellation
    // and inactivity while forwarding each response chunk to the caller.
    process::run_leaf(
        &mut command,
        Some(input.into_bytes()),
        request.timeout,
        request.output_limit,
        cancellation,
        request.idle,
        &mut |bytes| observer(bytes),
    )
}

fn encode(bytes: &[u8]) -> String {
    crate::attachments::base64::encode(bytes)
}

#[derive(Default)]
pub(crate) struct Decoder {
    pending: Vec<u8>,
    pub status: Option<u16>,
    pub headers: Vec<(String, String)>,
    ready: bool,
}

impl Decoder {
    pub fn feed(
        &mut self,
        bytes: &[u8],
        observer: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.ready {
            return observer(bytes);
        }
        self.pending.extend_from_slice(bytes);
        let Some(end) = self.pending.windows(4).position(|part| part == b"\r\n\r\n") else {
            if self.pending.len() > 64 * 1024 {
                return Err("HTTP response headers are too large".into());
            }
            return Ok(());
        };
        let header = std::str::from_utf8(&self.pending[..end])
            .map_err(|_| "Invalid HTTP response headers")?;
        let mut lines = header.split("\r\n");
        self.status = lines
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|number| number.parse().ok());
        if self.status.is_none() {
            return Err("Invalid HTTP status".into());
        }
        for line in lines {
            let (name, value) = line.split_once(':').ok_or("Invalid HTTP header")?;
            self.headers
                .push((name.to_string(), value.trim().to_string()));
        }
        self.ready = true;
        let body = self.pending.split_off(end + 4);
        self.pending.clear();
        observer(&body)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::server::{self, Server};
    use std::io::{BufRead, BufReader, Read, Write};

    #[test]
    fn child_fixture() {
        let Ok(url) = std::env::var("JECODE_HTTP_CHILD_FIXTURE") else {
            return;
        };
        let mut decoder = Decoder::default();
        let mut body = Vec::new();
        let output = run(
            Request {
                url: &url,
                method: "POST",
                headers: &[("X-Fixture-Request", "seen")],
                body: Some(b"payload"),
                timeout: Some(Duration::from_secs(5)),
                idle: None,
                output_limit: 0,
            },
            &Cancellation::default(),
            &mut |bytes| {
                decoder.feed(bytes, &mut |part| {
                    body.extend_from_slice(part);
                    Ok(())
                })
            },
        )
        .unwrap();
        assert_eq!(output.exit_code, Some(0));
        assert_eq!(decoder.status, Some(200));
        assert_eq!(decoder.header("X-Fixture-Response"), Some("passed"));
        assert_eq!(body, b"ok");
    }

    #[test]
    fn native_transport_ignores_curl_override_and_path() {
        let server = Server::start(|listener, stop| {
            let mut stream = server::accept(&listener, &stop).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut length = 0;
            let mut fixture = false;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                    if name.eq_ignore_ascii_case("x-fixture-request") {
                        fixture = value.trim() == "seen";
                    }
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nX-Fixture-Response: passed\r\nContent-Length: 2\r\n\r\nok",
                )
                .unwrap();
            (first, fixture, body)
        });
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "http::tests::child_fixture"])
            .env("JECODE_HTTP_CHILD_FIXTURE", &server.endpoint)
            .env("JECODE_CURL", "C:/missing/curl")
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        let (first, fixture, body) = server.finish();
        assert_eq!(first, "POST / HTTP/1.1\r\n");
        assert!(fixture);
        assert_eq!(body, b"payload");
    }

    #[cfg(windows)]
    #[test]
    fn ipv6_loopback_connects_without_proxy() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::thread;
        use std::time::Instant;
        let Ok(listener) = TcpListener::bind("[::1]:0") else {
            return;
        };
        let endpoint = format!("http://[::1]:{}/", listener.local_addr().unwrap().port());
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let started = Instant::now();
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        loop {
                            let mut line = String::new();
                            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                            if line == "\r\n" {
                                break;
                            }
                        }
                        stream
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                            .unwrap();
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(started.elapsed() < Duration::from_secs(5));
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("IPv6 loopback accept failed: {error}"),
                }
            }
        });
        let mut decoder = Decoder::default();
        let mut body = Vec::new();
        let output = run(
            Request {
                url: &endpoint,
                method: "GET",
                headers: &[],
                body: None,
                timeout: Some(Duration::from_secs(5)),
                idle: None,
                output_limit: 4096,
            },
            &Cancellation::default(),
            &mut |bytes| {
                decoder.feed(bytes, &mut |part| {
                    body.extend_from_slice(part);
                    Ok(())
                })
            },
        )
        .unwrap();
        assert_eq!(
            output.exit_code,
            Some(0),
            "stdout={:?} stderr={:?} input={:?}",
            output.stdout,
            output.stderr,
            output.input_error
        );
        server.join().unwrap();
        assert_eq!(decoder.status, Some(200));
        assert_eq!(body, b"ok");
    }

    #[cfg(windows)]
    #[test]
    fn native_http_rejects_truncated_and_malformed_responses() {
        for reply in [
            &b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhi"[..],
            &b"invalid status\r\n\r\n"[..],
        ] {
            let reply = reply.to_vec();
            let server = Server::start(move |listener, stop| {
                let mut stream = server::accept(&listener, &stop).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                    if line == "\r\n" {
                        break;
                    }
                }
                stream.write_all(&reply).unwrap();
            });
            let output = run(
                Request {
                    url: &server.endpoint,
                    method: "GET",
                    headers: &[],
                    body: None,
                    timeout: Some(Duration::from_secs(5)),
                    idle: None,
                    output_limit: 4096,
                },
                &Cancellation::default(),
                &mut |_| Ok(()),
            )
            .unwrap();
            server.finish();
            assert_ne!(output.exit_code, Some(0));
        }
    }

    #[test]
    fn responses_without_bodies_keep_representation_lengths() {
        for (method, status, length) in [("GET", 304, 321), ("GET", 204, 0), ("HEAD", 200, 321)] {
            let reply = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
            );
            let server = Server::start(move |listener, stop| {
                let mut stream = server::accept(&listener, &stop).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                    if line == "\r\n" {
                        break;
                    }
                }
                stream.write_all(reply.as_bytes()).unwrap();
            });
            let mut decoder = Decoder::default();
            let mut body = Vec::new();
            let output = run(
                Request {
                    url: &server.endpoint,
                    method,
                    headers: &[],
                    body: None,
                    timeout: Some(Duration::from_secs(5)),
                    idle: None,
                    output_limit: 4096,
                },
                &Cancellation::default(),
                &mut |bytes| {
                    decoder.feed(bytes, &mut |part| {
                        body.extend_from_slice(part);
                        Ok(())
                    })
                },
            )
            .unwrap();
            server.finish();
            assert_eq!(
                output.exit_code,
                Some(0),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(decoder.status, Some(status));
            assert!(body.is_empty());
        }
    }

    #[test]
    fn native_transport_delivers_body_before_response_completes() {
        use std::sync::mpsc;
        use std::thread;

        let (release, resume) = mpsc::channel();
        let server = Server::start(move |listener, stop| {
            let mut stream = server::accept(&listener, &stop).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nfi")
                .unwrap();
            resume.recv_timeout(Duration::from_secs(5)).unwrap();
            stream.write_all(b"ni").unwrap();
        });

        let (first_body, observed) = mpsc::channel();
        let endpoint = server.endpoint.clone();
        let worker = thread::spawn(move || {
            let mut decoder = Decoder::default();
            let mut body = Vec::new();
            let output = run(
                Request {
                    url: &endpoint,
                    method: "GET",
                    headers: &[],
                    body: None,
                    timeout: Some(Duration::from_secs(5)),
                    idle: None,
                    output_limit: 4096,
                },
                &Cancellation::default(),
                &mut |bytes| {
                    decoder.feed(bytes, &mut |part| {
                        if !part.is_empty() {
                            let _ = first_body.send(());
                            body.extend_from_slice(part);
                        }
                        Ok(())
                    })
                },
            )
            .unwrap();
            (output, decoder.status, body)
        });

        let first_arrived = observed.recv_timeout(Duration::from_secs(3)).is_ok();
        release.send(()).unwrap();
        let (output, status, body) = worker.join().unwrap();
        server.finish();
        assert!(first_arrived, "response body was buffered until completion");
        assert_eq!(output.exit_code, Some(0), "{:?}", output.stderr);
        assert_eq!(status, Some(200));
        assert_eq!(body, b"fini");
    }

    #[test]
    #[ignore = "requires external HTTPS connectivity; no account credentials"]
    fn trusted_https_uses_os_tls_policy() {
        let mut decoder = Decoder::default();
        let mut body = Vec::new();
        let output = run(
            Request {
                url: "https://example.com/",
                method: "GET",
                headers: &[],
                body: None,
                timeout: Some(Duration::from_secs(20)),
                idle: None,
                output_limit: 0,
            },
            &Cancellation::default(),
            &mut |bytes| {
                decoder.feed(bytes, &mut |part| {
                    if body.len().saturating_add(part.len()) > 1024 * 1024 {
                        return Err("HTTPS fixture exceeded one megabyte".into());
                    }
                    body.extend_from_slice(part);
                    Ok(())
                })
            },
        )
        .unwrap();
        assert_eq!(output.exit_code, Some(0), "{:?}", output.stderr);
        assert_eq!(decoder.status, Some(200));
        assert!(body.windows(14).any(|part| part == b"Example Domain"));
    }
}
