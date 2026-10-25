use std::io::Write;
use std::net::TcpStream;

const TEMPLATE: &str = include_str!("page.html");

pub(super) fn respond(stream: &mut TcpStream, status: &str, message: &str) {
    let response = response(status, message);
    let _ = stream.write_all(response.as_bytes());
}

fn response(status: &str, message: &str) -> String {
    let (heading, description) = match message {
        "Return to Jecode. This tab can be closed." => (
            "Return to Jecode",
            "Finish sign-in in your terminal. You can close this tab.",
        ),
        "Sign-in was not completed. Return to Jecode." => (
            "Sign-in not completed",
            "Return to your terminal to try again. You can close this tab.",
        ),
        _ => ("Sign-in interrupted", message),
    };
    let body = TEMPLATE
        .replace("{{HEADING}}", &escape_html(heading))
        .replace("{{DESCRIPTION}}", &escape_html(description));
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::response;

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
    fn success_page_requests_terminal_completion_without_claiming_connection() {
        let result = response("200 OK", "Return to Jecode. This tab can be closed.");
        let html = body(&result);
        assert!(result.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(html.contains("<main"));
        assert!(html.contains("<h1>Return to Jecode</h1>"));
        assert!(html.contains("Finish sign-in in your terminal."));
        assert!(!html.contains("account connected"));
    }

    #[test]
    fn declined_and_invalid_state_pages_are_distinct() {
        let declined = response("200 OK", "Sign-in was not completed. Return to Jecode.");
        assert!(body(&declined).contains("<h1>Sign-in not completed</h1>"));
        let invalid = response("400 Bad Request", "Sign-in state mismatch");
        assert!(invalid.starts_with("HTTP/1.1 400 Bad Request\r\n"));
        assert!(body(&invalid).contains("Sign-in state mismatch"));
    }

    #[test]
    fn dynamic_error_text_is_escaped() {
        let result = response("400 Bad Request", "Invalid <callback> & \"state\" 'value'");
        let html = body(&result);
        assert!(html.contains("Invalid &lt;callback&gt; &amp; &quot;state&quot; &#39;value&#39;"));
        assert!(!html.contains("<callback>"));
    }
}
