use jecode::stream::{Decoder, Error, Limits};
use std::ops::ControlFlow::{Break, Continue};

#[test]
fn every_byte_boundary_preserves_events_and_unicode() {
    let wire = "\u{feff}: heartbeat\r\nevent: response\r\ndata: hé😀\r\ndata: second\r\n\r\ndata:\r\n\r\ndata: [DONE]\n\n".as_bytes();
    for boundary in 0..=wire.len() {
        let mut decoder = Decoder::new(Limits::default());
        let mut events = Vec::new();
        for part in [&wire[..boundary], &wire[boundary..]] {
            decoder
                .push(part, |text| {
                    events.push(text.to_owned());
                    Continue(())
                })
                .unwrap();
        }
        assert_eq!(
            events,
            ["hé😀\nsecond", "", "[DONE]"],
            "split at {boundary}"
        );
    }
    let mut decoder = Decoder::new(Limits::default());
    let mut events = Vec::new();
    for byte in wire {
        decoder
            .push(&[*byte], |text| {
                events.push(text.to_owned());
                Continue(())
            })
            .unwrap();
    }
    assert_eq!(events, ["hé😀\nsecond", "", "[DONE]"]);
}

#[test]
fn cancellation_stops_the_current_chunk_without_delivering_later_events() {
    let mut decoder = Decoder::new(Limits::default());
    let mut calls = 0;
    assert_eq!(
        decoder.push(b"data: first\n\ndata: second\n\n", |_| {
            calls += 1;
            Break(())
        }),
        Err(Error::Cancelled)
    );
    assert_eq!(calls, 1);
    assert_eq!(
        decoder.push(b"data: retry\n\n", |_| panic!("closed")),
        Err(Error::Closed)
    );
}

#[test]
fn bounds_cover_unterminated_lines_and_multi_line_events() {
    let limits = Limits {
        line_bytes: 8,
        event_bytes: 3,
    };
    let mut decoder = Decoder::new(limits);
    decoder
        .push(b"data: ab\n", |_| panic!("incomplete event"))
        .unwrap();
    assert_eq!(
        decoder.push(b"data: c\n", |_| panic!("oversized event")),
        Err(Error::EventTooLarge)
    );
    let mut decoder = Decoder::new(limits);
    decoder
        .push(b"12345678", |_| panic!("incomplete line"))
        .unwrap();
    assert_eq!(
        decoder.push(b"9", |_| panic!("oversized line")),
        Err(Error::LineTooLong)
    );
    let mut decoder = Decoder::new(limits);
    let mut events = Vec::new();
    decoder
        .push(b"data:abc\n\n", |text| {
            events.push(text.to_owned());
            Continue(())
        })
        .unwrap();
    assert_eq!(events, ["abc"]);
}

#[test]
fn malformed_utf8_is_terminal_but_prior_events_remain_observable() {
    let mut decoder = Decoder::new(Limits::default());
    let mut events = Vec::new();
    assert_eq!(
        decoder.push(b"data: good\n\ndata: \xff\n\n", |text| {
            events.push(text.to_owned());
            Continue(())
        }),
        Err(Error::InvalidUtf8)
    );
    assert_eq!(events, ["good"]);
    assert_eq!(
        decoder.push(b"\n", |_| panic!("closed")),
        Err(Error::Closed)
    );
}

#[test]
fn eof_never_publishes_an_unterminated_event() {
    for bytes in [b"data: partial".as_slice(), b"data: partial\n"] {
        let mut decoder = Decoder::new(Limits::default());
        decoder
            .push(bytes, |_| panic!("no event delimiter"))
            .unwrap();
        decoder.close();
        assert_eq!(
            decoder.push(b"\n", |_| panic!("closed")),
            Err(Error::Closed)
        );
    }
}

#[test]
fn comments_unknown_fields_and_line_endings_follow_data_framing() {
    let mut decoder = Decoder::new(Limits::default());
    let mut events = Vec::new();
    decoder
        .push(
            b": ping\r\rid: x\revent: ignored\rdata\rdata:  x\r\r",
            |text| {
                events.push(text.to_owned());
                Continue(())
            },
        )
        .unwrap();
    assert_eq!(events, ["\n x"]);
}

#[test]
fn large_chunks_are_delivered_incrementally_without_an_event_queue() {
    let mut decoder = Decoder::new(Limits {
        line_bytes: 7,
        event_bytes: 1,
    });
    let wire = b"data:x\n\n".repeat(10_000);
    let mut count = 0;
    decoder
        .push(&wire, |text| {
            assert_eq!(text, "x");
            count += 1;
            Continue(())
        })
        .unwrap();
    assert_eq!(count, 10_000);
}
