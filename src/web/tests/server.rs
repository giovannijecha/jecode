use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};
pub(super) struct Server {
    pub(super) base: String,
    stop: Arc<AtomicBool>,
    pub(super) requests: Arc<Mutex<Vec<String>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub(super) fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let running = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let thread = thread::spawn(move || {
            while !running.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                recorded.lock().unwrap().push(path.to_string());
                let (status, mime, extra, body): (&str, &str, &str, &[u8]) = match path {
                    "/redirect" => ("302 Found", "text/plain", "Location: /\r\n", b"redirect"),
                    "/binary" => ("200 OK", "application/octet-stream", "", &[0, 1, 2, 255]),
                    "/a" => ("200 OK", "text/plain", "", b"page a"),
                    "/b" => ("404 Not Found", "text/plain", "", b"missing"),
                    "/slow" => { thread::sleep(Duration::from_secs(2)); ("200 OK", "text/plain", "", b"slow") },
                    _ => ("200 OK", "text/html", "", b"<title>Home</title><h1>Welcome</h1><a href='/a'>A</a><a href='/b'>B</a><a href='/a'>Again</a>"),
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        Self {
            base,
            stop,
            requests,
            thread: Some(thread),
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if bytes.len() > 64 * 1024 || !read_more(stream, &mut bytes, deadline) {
            return None;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let content_length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok());
    if let Some(length) = content_length {
        if length > 1024 * 1024 {
            return None;
        }
        while bytes.len().saturating_sub(header_end) < length {
            if !read_more(stream, &mut bytes, deadline) {
                return None;
            }
        }
    } else if headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .any(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value.trim().eq_ignore_ascii_case("chunked")
        })
    {
        while !bytes[header_end..].ends_with(b"0\r\n\r\n") {
            if bytes.len().saturating_sub(header_end) > 1024 * 1024
                || !read_more(stream, &mut bytes, deadline)
            {
                return None;
            }
        }
    }
    Some(headers)
}

fn read_more(stream: &mut TcpStream, bytes: &mut Vec<u8>, deadline: Instant) -> bool {
    let mut chunk = [0; 4096];
    while Instant::now() < deadline {
        match stream.read(&mut chunk) {
            Ok(0) => return false,
            Ok(length) => {
                bytes.extend_from_slice(&chunk[..length]);
                return true;
            }
            Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {}
            Err(_) => return false,
        }
    }
    false
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
