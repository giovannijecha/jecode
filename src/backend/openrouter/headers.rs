use crate::backend::retry::retry_delay;
use std::time::Duration;

#[derive(Default)]
pub(super) struct Headers {
    pending: Vec<u8>,
    pub status: u16,
    pub retry_after: Option<Duration>,
    body: bool,
}
impl Headers {
    pub fn feed(
        &mut self,
        bytes: &[u8],
        observer: &mut impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.body {
            return observer(bytes);
        }
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self
            .pending
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
        {
            let block: Vec<_> = self.pending.drain(..end + 4).collect();
            let block = std::str::from_utf8(&block).map_err(|_| "Invalid HTTP response headers")?;
            self.status = block
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|status| status.parse().ok())
                .ok_or("Invalid HTTP status")?;
            self.retry_after = block.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("retry-after")
                    .then(|| retry_delay(value.trim()))
                    .flatten()
            });
            if self.status >= 200 {
                self.body = true;
                return observer(&std::mem::take(&mut self.pending));
            }
        }
        Ok(())
    }
}

pub(super) fn body(mut response: &str) -> Result<&str, String> {
    while response.starts_with("HTTP/") {
        let (headers, rest) = response
            .split_once("\r\n\r\n")
            .ok_or("Incomplete HTTP headers")?;
        let status: u16 = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse().ok())
            .ok_or("Invalid HTTP status")?;
        response = rest;
        if status >= 200 {
            break;
        }
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_headers_and_server_delay_do_not_enter_sse() {
        let wire =
            b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 429 Retry\r\nRetry-After: 90\r\n\r\nbody";
        let mut headers = Headers::default();
        let mut body = Vec::new();
        for bytes in wire.chunks(3) {
            headers
                .feed(bytes, &mut |bytes| {
                    body.extend_from_slice(bytes);
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(headers.status, 429);
        assert_eq!(headers.retry_after, Some(Duration::from_secs(90)));
        assert_eq!(body, b"body");
        assert!(retry_delay("Wed, 21 Oct 2015 07:28:00 GMT").is_some());
        assert!(retry_delay("Wed, 31 Feb 2026 07:28:00 GMT").is_none());
    }
}
