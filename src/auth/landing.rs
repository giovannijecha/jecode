//! The page a browser lands on when a sign-in ends. It waits for the outcome, so what it
//! says reflects the provider authorization outcome. Account flows also save their
//! tokens before replying; MCP connections are checked and saved afterwards.

use std::io::Write;
use std::net::TcpStream;

const PAGE: &str = include_str!("landing.html");

/// A browser waiting for the outcome. Dropping it, as every error path does, tells the
/// browser the sign-in failed.
pub(crate) struct Landing(Option<TcpStream>);

impl Landing {
    pub(crate) fn new(stream: TcpStream) -> Self {
        // An accepted socket inherits the listener's non-blocking mode on Windows.
        let _ = stream.set_nonblocking(false);
        Self(Some(stream))
    }

    pub(crate) fn authenticated(mut self) {
        if let Some(mut stream) = self.0.take() {
            answer(&mut stream, "200 OK", true);
        }
    }
}

impl Drop for Landing {
    fn drop(&mut self) {
        if let Some(stream) = &mut self.0 {
            answer(stream, "200 OK", false);
        }
    }
}

/// A browser that closed first changes nothing about the sign-in.
pub(crate) fn answer(stream: &mut TcpStream, status: &str, done: bool) {
    let _ = stream.write_all(response(status, done).as_bytes());
}

fn response(status: &str, done: bool) -> String {
    let (heading, next) = if done {
        ("jecode authenticated", "close this tab")
    } else {
        ("jecode not authenticated", "close this tab and try again")
    };
    let body = PAGE
        .replace("{{HEADING}}", heading)
        .replace("{{NEXT}}", next);
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    fn body(response: &str) -> &str {
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(length, body.len());
        assert!(headers.contains("Cache-Control: no-store"));
        assert!(headers.contains("Referrer-Policy: no-referrer"));
        assert!(headers.contains("Content-Security-Policy: default-src 'none'"));
        body
    }

    #[test]
    fn both_outcomes_keep_the_status_and_say_what_happened() {
        let done = response("200 OK", true);
        assert!(done.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(body(&done).contains("<h1>jecode authenticated</h1>"));
        assert!(body(&done).contains("<p>close this tab</p>"));
        let failed = response("400 Bad Request", false);
        assert!(failed.starts_with("HTTP/1.1 400 Bad Request\r\n"));
        assert!(body(&failed).contains("<h1>jecode not authenticated</h1>"));
        assert!(body(&failed).contains("<p>close this tab and try again</p>"));
    }

    fn page(finish: impl FnOnce(Landing)) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut browser = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (stream, _) = listener.accept().unwrap();
        finish(Landing::new(stream));
        let mut reply = String::new();
        browser.read_to_string(&mut reply).unwrap();
        reply
    }

    #[test]
    fn the_browser_waits_for_the_outcome_and_an_error_path_reports_failure() {
        assert!(page(Landing::authenticated).contains("<h1>jecode authenticated</h1>"));
        assert!(page(drop).contains("<h1>jecode not authenticated</h1>"));
    }

    #[test]
    fn a_browser_that_closed_first_does_not_fail_the_sign_in() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(TcpStream::connect(("127.0.0.1", port)).unwrap());
        let (stream, _) = listener.accept().unwrap();
        Landing::new(stream).authenticated();
    }
}
