use crate::json::{self, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};
mod protocol;
#[cfg(windows)]
pub mod sandbox;
pub mod server;
pub use protocol::{completion, tool_call};

/// History assertions exclude the separate runtime suffix; wire tests inspect it directly.
pub fn durable_messages(messages: &[Value]) -> &[Value] {
    let state = messages.last().is_some_and(|message| {
        let content = message.get("content");
        let text = content
            .and_then(Value::as_str)
            .or_else(|| content?.as_array()?.first()?.get("text")?.as_str());
        message.get("jecode_request_state") == Some(&Value::Bool(true))
            || text
                .is_some_and(|text| text.starts_with("Jecode session state for the next request."))
    });
    &messages[..messages.len() - usize::from(state)]
}

pub fn request_messages(body: &Value) -> &[Value] {
    durable_messages(
        body.get("messages")
            .or_else(|| body.get("input"))
            .and_then(Value::as_array)
            .expect("Fixture request has messages"),
    )
}

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

fn fixture_base() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-fixtures")
        .join(std::env::consts::OS)
}

pub struct Directory {
    path: PathBuf,
}

impl Directory {
    pub fn new() -> Self {
        let base = fixture_base();
        fs::create_dir_all(&base).unwrap();
        loop {
            let path = base.join(format!(
                "{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                // A prior crashed test can leave this name behind, and the
                // operating system can later reuse its process identifier.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("Could not create isolated fixture: {error}"),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let base = fixture_base();
        assert!(self.path.starts_with(base));
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A directory link: a symbolic link on Unix, a junction on Windows.
pub fn link_directory(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let created = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(target.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(created.status.success(), "{created:?}");
    }
}

/// A prior exchange large enough to make a synthetic provider rejection
/// recoverable without shrinking the original user request under test.
pub fn prior_compactable_exchange(bytes: usize) -> [Value; 2] {
    const PHRASE: &str = "Earlier assistant observation. ";
    // About three fifths of the fixed request leaves summary room after the
    // learned 75% ceiling without creating extra summary portions.
    [
        Value::object([
            ("role", Value::string("user")),
            ("content", Value::string("Review earlier observations.")),
        ]),
        Value::object([
            ("role", Value::string("assistant")),
            (
                "content",
                Value::string(PHRASE.repeat(bytes.saturating_mul(3).div_ceil(5 * PHRASE.len()))),
            ),
        ]),
    ]
}

pub struct Request {
    pub headers: Vec<String>,
    pub body: Value,
}

pub struct HttpFixture {
    pub endpoint: String,
    server: server::Server<Vec<Request>>,
    expected: usize,
}

pub enum Response {
    Json(u16, Value),
    Http(u16, Vec<(String, String)>, Value),
    Stream(Vec<(Duration, String)>),
    GatedStream {
        head: String,
        tail: String,
        release: std::sync::mpsc::Receiver<()>,
    },
}

impl HttpFixture {
    pub fn new(responses: Vec<(u16, Value)>) -> Self {
        Self::with_wait(responses, Duration::from_secs(10))
    }

    pub fn with_wait(responses: Vec<(u16, Value)>, wait: Duration) -> Self {
        Self::responses(
            responses
                .into_iter()
                .map(|(status, value)| Response::Json(status, value))
                .collect(),
            wait,
        )
    }

    pub fn streaming(responses: Vec<Response>) -> Self {
        Self::responses(responses, Duration::from_secs(10))
    }
    pub fn streaming_with_wait(responses: Vec<Response>, wait: Duration) -> Self {
        Self::responses(responses, wait)
    }

    fn responses(responses: Vec<Response>, wait: Duration) -> Self {
        let expected = responses.len();
        let server = server::Server::start(move |listener, stop| {
            let mut requests = Vec::new();
            for response in responses {
                // Request preparation belongs to the client. Keep listening
                // until the owning fixture finishes or is dropped.
                let Ok(mut stream) = server::accept(&listener, &stop) else {
                    break;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut headers = Vec::new();
                let mut length = None;
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = Some(value.trim().parse::<usize>().unwrap());
                    }
                    headers.push(line.trim_end().into());
                }
                let length = length.unwrap_or(0);
                assert!(length <= 8 * 1024 * 1024);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                requests.push(Request {
                    headers,
                    body: if body.is_empty() {
                        Value::Null
                    } else {
                        json::parse(std::str::from_utf8(&body).unwrap()).unwrap()
                    },
                });
                match response {
                    Response::Json(status, response) => {
                        let body = response.encode();
                        write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    }
                    Response::Http(status, headers, response) => {
                        let body = response.encode();
                        write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", body.len()).unwrap();
                        for (name, value) in headers {
                            write!(stream, "{name}: {value}\r\n").unwrap();
                        }
                        write!(stream, "\r\n{body}").unwrap();
                    }
                    Response::Stream(chunks) => {
                        let length: usize = chunks.iter().map(|(_, chunk)| chunk.len()).sum();
                        write!(stream, "HTTP/1.1 200 Fixture\r\nContent-Type: text/event-stream\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").unwrap();
                        stream.flush().unwrap();
                        for (delay, chunk) in chunks {
                            if stop.wait(delay).is_err() {
                                break;
                            }
                            if stream
                                .write_all(chunk.as_bytes())
                                .and_then(|_| stream.flush())
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                    Response::GatedStream {
                        head,
                        tail,
                        release,
                    } => {
                        write!(stream, "HTTP/1.1 200 Fixture\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{head}", head.len() + tail.len()).unwrap();
                        stream.flush().unwrap();
                        let started = Instant::now();
                        loop {
                            if stop.requested() {
                                return requests;
                            }
                            let remaining = wait.saturating_sub(started.elapsed());
                            match release.recv_timeout(remaining.min(Duration::from_millis(20))) {
                                Ok(()) => break,
                                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                                    if !remaining.is_zero() => {}
                                Err(_) => panic!("stream was not observed before completion"),
                            }
                        }
                        // A cancelled client may already have closed the socket.
                        let _ = stream
                            .write_all(tail.as_bytes())
                            .and_then(|_| stream.flush());
                    }
                }
            }
            requests
        });
        let endpoint = format!("{}/chat/completions", server.endpoint);
        Self {
            endpoint,
            server,
            expected,
        }
    }

    pub fn finish(self) -> Vec<Request> {
        let requests = self.server.finish();
        assert_eq!(
            requests.len(),
            self.expected,
            "fixture did not receive every expected request"
        );
        requests
    }
}

#[test]
fn http_fixture_survives_slow_client_preparation() {
    let fixture = HttpFixture::with_wait(vec![(200, Value::Null)], Duration::from_millis(10));
    thread::sleep(Duration::from_millis(100));
    let address = fixture
        .endpoint
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let mut client =
        std::net::TcpStream::connect(address).expect("fixture must live until its client finishes");
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    client
        .write_all(
            b"POST /chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}",
        )
        .unwrap();
    // Waiting for the close would race a concurrent test's child process
    // that briefly inherits the accepted socket; the status line suffices.
    let mut status = [0; 12];
    client.read_exact(&mut status).unwrap();
    assert_eq!(&status, b"HTTP/1.1 200");
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn dropping_http_fixture_cancels_an_unreleased_stream() {
    let (_release, released) = std::sync::mpsc::channel();
    let fixture = HttpFixture::streaming_with_wait(
        vec![Response::GatedStream {
            head: "head".into(),
            tail: "tail".into(),
            release: released,
        }],
        Duration::from_secs(180),
    );
    let address = fixture
        .endpoint
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let mut client = std::net::TcpStream::connect(address).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    client
        .write_all(
            b"POST /chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}",
        )
        .unwrap();
    let mut reader = BufReader::new(client);
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    assert!(status.starts_with("HTTP/1.1 200"));
    let started = Instant::now();
    drop(fixture);
    assert!(started.elapsed() < Duration::from_secs(2));
}
