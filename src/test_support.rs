use crate::json::{self, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
mod protocol;
pub use protocol::{completion, tool_call};

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
        let path = base.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
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

pub struct Request {
    pub headers: Vec<String>,
    pub body: Value,
}

pub struct HttpFixture {
    pub endpoint: String,
    server: JoinHandle<Vec<Request>>,
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
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/chat/completions", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let started = Instant::now();
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(started.elapsed() < wait, "fixture received no request");
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("fixture accept: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
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
                            thread::sleep(delay);
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
                        release
                            .recv_timeout(wait)
                            .expect("stream was not observed before completion");
                        stream.write_all(tail.as_bytes()).unwrap();
                        stream.flush().unwrap();
                    }
                }
            }
            requests
        });
        Self { endpoint, server }
    }

    pub fn finish(self) -> Vec<Request> {
        self.server.join().unwrap()
    }
}
