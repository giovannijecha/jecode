use super::*;
use crate::tui::keys::{Decoded, Decoder};

fn decoded_keys(bytes: &[u8]) -> Vec<Key> {
    Decoder::default()
        .bytes(bytes)
        .into_iter()
        .map(|value| match value {
            Decoded::Key(key) => key,
            _ => panic!("Expected a keyboard command"),
        })
        .collect()
}

#[test]
fn mouse_wheel_is_scroll_only_and_clicks_releases_and_invalid_reports_are_ignored() {
    let mut decoder = Decoder::default();
    let events = decoder.bytes(
        b"\x1b[<64;10;5M\x1b[<65;10;5M\x1b[<0;10;5M\x1b[<0;10;5m\x1b[<64;0;5M\x1b[<66;10;5M",
    );
    assert!(matches!(
        events.as_slice(),
        [Decoded::Scroll(-3), Decoded::Scroll(3)]
    ));
    let events = decoder.bytes(b"\x1b[200~\x1b[<64;10;5M\x1b[201~");
    assert!(matches!(events.as_slice(), [Decoded::Text(text)] if text == "\x1b[<64;10;5M"));
    assert!(decoder.bytes(b"\x1b[<64;10;").is_empty());
    assert!(matches!(
        decoder.bytes(b"5M").as_slice(),
        [Decoded::Scroll(-3)]
    ));
}

#[test]
fn unix_controls_distinguish_exit_stop_send_and_newline() {
    let keys = decoded_keys(b"\x03\x04\x11\r\n\x7f\t");
    assert_eq!(
        keys.iter()
            .map(|key| (key.code, key.modifiers))
            .collect::<Vec<_>>(),
        [(67, 4), (68, 4), (81, 4), (13, 0), (74, 4), (8, 0), (9, 0)]
    );
}

#[test]
fn incomplete_terminal_sequences_do_not_swallow_stop_or_quit() {
    let keys = decoded_keys(b"\x1b[\x03\x1b[1;5\x11");
    assert_eq!(
        keys.iter()
            .map(|key| (key.code, key.modifiers))
            .collect::<Vec<_>>(),
        [(67, 4), (81, 4)]
    );
}

#[test]
fn csi_ss3_and_modified_navigation_use_the_shared_editor_keys() {
    let keys = decoded_keys(b"\x1b[A\x1bOB\x1b[1;5D\x1b[1;3A\x1b[1;6B\x1b[3~\x1b[5~\x1b[6~\x1b[H\x1bOF\x1bOP\x1b[Z\x1b\r\x1b[13;2u");
    assert_eq!(
        keys.iter()
            .map(|key| (key.code, key.modifiers))
            .collect::<Vec<_>>(),
        [
            (38, 0),
            (40, 0),
            (37, 4),
            (38, 1),
            (40, 6),
            (46, 0),
            (33, 0),
            (34, 0),
            (36, 0),
            (35, 0),
            (112, 0),
            (9, 2),
            (13, 1),
            (13, 2)
        ]
    );
}

#[test]
fn utf8_survives_arbitrary_read_boundaries_and_invalid_input() {
    let mut decoder = Decoder::default();
    let mut text = String::new();
    for &byte in "hé🙂漢字"
        .as_bytes()
        .iter()
        .chain([0xff, 0xe2, b'x'].iter())
    {
        for value in decoder.bytes(&[byte]) {
            match value {
                Decoded::Text(value) => text.push_str(&value),
                _ => panic!("Expected text"),
            }
        }
    }
    assert_eq!(text, "hé🙂漢字\u{fffd}\u{fffd}x");
}

#[test]
fn bracketed_paste_keeps_control_bytes_and_arrows_as_text() {
    let mut decoder = Decoder::default();
    let mut output = vec![];
    for chunk in b"\x1b[200~first\r\n\x03\x11\x1b[A\tlast\x1b[201~\r".chunks(2) {
        output.extend(decoder.bytes(chunk));
    }
    assert_eq!(output.len(), 2);
    assert!(matches!(&output[0], Decoded::Text(text) if text == "first\r\n\x03\x11\x1b[A\tlast"));
    assert!(matches!(&output[1], Decoded::Key(key) if key.code == 13 && !key.ctrl()));
}

#[test]
fn escape_is_bounded_and_late_cursor_reports_do_not_enter_the_draft() {
    let mut parser = Parser::default();
    assert!(parser.push(27, false).is_empty());
    parser.escaped_at = Some(Instant::now() - Duration::from_secs(1));
    assert_eq!(parser.idle().unwrap().code, 27);
    let mut decoder = Decoder::default();
    assert!(decoder.bytes(b"\x1b[12;4R\x1b[?1;2c").is_empty());
    assert!(matches!(&decoder.bytes(b"kept")[0], Decoded::Text(text) if text == "k"));
    for &byte in b"\x1b[".iter().chain(std::iter::repeat_n(&b'1', 60)) {
        parser.push(byte, false);
    }
    assert!(parser.sequence.len() <= 64);
    parser.escaped_at = Some(Instant::now() - Duration::from_secs(1));
    assert!(parser.idle().is_none());
    assert!(parser.sequence.is_empty());
}

#[test]
fn pasted_input_over_limit_is_rejected_as_one_insertion() {
    let mut decoder = Decoder::default();
    assert!(decoder.bytes(b"\x1b[200~").is_empty());
    assert!(decoder.bytes(&vec![b'x'; 1024 * 1024 + 1]).is_empty());
    assert!(matches!(
        decoder.bytes(b"\x1b[201~").as_slice(),
        [Decoded::Error]
    ));
}
