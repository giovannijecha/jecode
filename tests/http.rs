use jecode::http::{self, Decoder, Error, Event, Limits};
use std::ops::ControlFlow;

fn decode(wire: &[u8], split: usize) -> Result<(u16, Vec<u8>, usize), Error> {
    let mut decoder = Decoder::default();
    let (mut status, mut body, mut ends) = (0, Vec::new(), 0);
    let mut accept = |event: Event<'_>| {
        match event {
            Event::Head(head) => status = head.status,
            Event::Data(data) => body.extend_from_slice(data),
            Event::End => ends += 1,
        }
        ControlFlow::Continue(())
    };
    decoder.push(&wire[..split], &mut accept)?;
    decoder.push(&wire[split..], &mut accept)?;
    decoder.finish(&mut accept)?;
    assert!(decoder.is_complete());
    Ok((status, body, ends))
}

#[test]
fn all_byte_boundaries_preserve_chunked_unicode_and_trailers() {
    let wire = b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>\r\n\r\nHTTP/1.1 200 OK\r\nTransfer-Encoding: Chunked\r\nContent-Type: text/event-stream\r\n\r\n4;name=\"quoted\\\"value\"\r\n\xf0\x9f\x9a\x80\r\n2;x=y\r\nHi\r\n0\r\nX-Checksum: abc\r\n\r\n";
    for split in 0..=wire.len() {
        assert_eq!(
            decode(wire, split),
            Ok((200, "🚀Hi".as_bytes().to_vec(), 1)),
            "split {split}"
        );
    }
    let mut decoder = Decoder::default();
    let mut body = Vec::new();
    for byte in wire {
        decoder
            .push(&[*byte], |event| {
                if let Event::Data(data) = event {
                    body.extend_from_slice(data);
                }
                ControlFlow::Continue(())
            })
            .unwrap();
    }
    assert_eq!(body, "🚀Hi".as_bytes());
    assert!(decoder.is_complete());
}

#[test]
fn length_close_and_empty_responses_obey_their_boundaries() {
    for wire in [
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc".as_slice(),
        b"HTTP/1.0 200 OK\r\n\r\nabc",
    ] {
        for split in 0..=wire.len() {
            assert_eq!(decode(wire, split), Ok((200, b"abc".to_vec(), 1)));
        }
    }
    assert_eq!(
        decode(b"HTTP/1.1 204 No Content\r\n\r\n", 0),
        Ok((204, vec![], 1))
    );
    assert_eq!(
        decode(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", 0),
        Ok((200, vec![], 1))
    );
    assert_eq!(
        decode(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nab", 0),
        Err(Error::ExtraData)
    );
}

#[test]
fn incomplete_and_ambiguous_messages_are_rejected() {
    assert_eq!(
        decode(
            b"HTTP/1.0 304 Not Modified\r\nTransfer-Encoding: chunked\r\n\r\n",
            0
        ),
        Err(Error::Unsupported)
    );
    for wire in [
        "HTTP/1.1 200 OK\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\na",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nab",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n",
    ] {
        assert_eq!(decode(wire.as_bytes(), 0), Err(Error::Truncated));
    }
    for headers in [
        "Content-Length: 1\r\nContent-Length: 1",
        "Content-Length: 1\r\nTransfer-Encoding: chunked",
        "Content-Length: 1, 1",
        "Content-Length: -1",
        "Content-Length : 1",
        " folded: text",
        "Transfer-Encoding: gzip, chunked",
        "Content-Encoding: gzip",
        "Content-Type: text/plain\r\nContent-Type: text/event-stream",
    ] {
        let wire = format!("HTTP/1.1 200 OK\r\n{headers}\r\n\r\n");
        assert!(decode(wire.as_bytes(), 0).is_err(), "accepted {headers}");
    }
    assert_eq!(
        decode(b"HTTP/1.1 101 Switching Protocols\r\n\r\n", 0),
        Err(Error::Unsupported)
    );
    assert!(decode(b"HTTP/1.1 200 OK\n\n", 0).is_err());
}

#[test]
fn malformed_chunks_and_forbidden_trailers_fail_closed() {
    for tail in [
        "+1\r\na\r\n0\r\n\r\n",
        "1x\r\na\r\n0\r\n\r\n",
        "1;\r\na\r\n0\r\n\r\n",
        "1;x=\"\r\na\r\n0\r\n\r\n",
        "1\r\naX\n0\r\n\r\n",
        "0\r\nContent-Length: 3\r\n\r\n",
        "FFFFFFFFFFFFFFFFFFFFFFFF\r\n",
        "1\u{000b};x=y\r\na\r\n0\r\n\r\n",
    ] {
        let wire = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{tail}");
        assert!(decode(wire.as_bytes(), 0).is_err(), "accepted {tail:?}");
    }
}

#[test]
fn budgets_and_callback_stop_are_terminal() {
    let wire = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc";
    for limits in [
        Limits {
            header_bytes: 10,
            ..Default::default()
        },
        Limits {
            body_bytes: 2,
            ..Default::default()
        },
        Limits {
            framing_bytes: 10,
            ..Default::default()
        },
    ] {
        let mut decoder = Decoder::new(limits);
        assert_eq!(
            decoder.push(wire, |_| ControlFlow::Continue(())),
            Err(Error::Limit)
        );
        assert_eq!(
            decoder.push(b"", |_| ControlFlow::Continue(())),
            Err(Error::Closed)
        );
    }
    let mut decoder = Decoder::default();
    let mut events = 0;
    assert_eq!(
        decoder.push(wire, |_| {
            events += 1;
            ControlFlow::Break(())
        }),
        Err(Error::Stopped)
    );
    assert_eq!(events, 1);
    assert_eq!(
        decoder.finish(|_| ControlFlow::Continue(())),
        Err(Error::Closed)
    );
    let flood = "HTTP/1.1 103 Early Hints\r\n\r\n".repeat(9);
    assert_eq!(decode(flood.as_bytes(), 0), Err(Error::Limit));
}

#[test]
fn request_bytes_have_one_length_and_cannot_inject_framing() {
    let bytes = http::post_json(
        "chatgpt.com",
        "/path?x=1",
        &[("Authorization", "Bearer synthetic")],
        "\"🚀\"",
        4096,
    )
    .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("Content-Length: 6\r\n"));
    assert!(text.ends_with("\r\n\r\n\"🚀\""));
    assert!(text.contains("Accept-Encoding: identity\r\n"));
    for headers in [
        vec![("Host", "evil.test")],
        vec![("x", "a\r\nInjected: true")],
        vec![("x", "1"), ("X", "2")],
        vec![("Content-Length", "0")],
        vec![("bad name", "a")],
    ] {
        assert_eq!(
            http::post_json("chatgpt.com", "/", &headers, "{}", 4096),
            Err(Error::Invalid)
        );
    }
    for host in ["", "chatgpt.com:443", "user@host", "-bad.test", "host\r\nx"] {
        assert!(http::post_json(host, "/", &[], "{}", 4096).is_err());
    }
    for path in [
        "https://other.test/",
        "//other.test/",
        "/ bad",
        "/#fragment",
    ] {
        assert!(http::post_json("chatgpt.com", path, &[], "{}", 4096).is_err());
    }
    assert_eq!(
        http::post_json("chatgpt.com", "/", &[], "{}", 1),
        Err(Error::Limit)
    );
}
