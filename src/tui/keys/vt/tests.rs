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
fn ctrl_right_bracket_reaches_direct_terminal_mode() {
    let keys = decoded_keys(&[0x1d]);
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].code, 221);
    assert!(keys[0].ctrl());
    assert_eq!(keys[0].character, 29);
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
    assert!(matches!(events.as_slice(), [Decoded::Paste(text)] if text == "\x1b[<64;10;5M"));
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
fn the_keyboard_protocol_decodes_like_legacy_keys_whatever_locks_are_on() {
    for (sequence, code, modifiers) in [
        (&b"\x1b[27u"[..], 27, 0), // Esc
        (b"\x1b[27;65u", 27, 0),   // Esc with Caps Lock on
        (b"\x1b[99;5u", 67, 4),    // Ctrl+C
        (b"\x1b[99;133u", 67, 4),  // Ctrl+C with Num Lock on
        (b"\x1b[118;3u", 86, 1),   // Alt+V
        (b"\x1b[118;5u", 86, 4),   // Ctrl+V
        (b"\x1b[13;2u", 13, 2),    // Shift+Enter
        (b"\x1b[13;3u", 13, 1),    // Alt+Enter
        (b"\x1b[57414u", 13, 0),   // Keypad Enter
        (b"\x1b[106;5u", 74, 4),   // Ctrl+J
    ] {
        let keys = decoded_keys(sequence);
        assert_eq!(keys.len(), 1, "{sequence:?}");
        assert_eq!(
            (keys[0].code, keys[0].modifiers),
            (code, modifiers),
            "{sequence:?}"
        );
    }
    // Cmd on a Mac, reported as Super, must not become a plain letter.
    assert!(decoded_keys(b"\x1b[99;9u").is_empty());
}

#[test]
fn protocol_punctuation_keeps_its_shortcuts_and_never_reads_as_navigation() {
    for (sequence, code, modifiers, character) in [
        (&b"\x1b[93;5u"[..], 221, 4, 93), // Ctrl+] leaves direct input.
        (b"\x1b[92;5u", 220, 4, 92),      // Ctrl+\ is the terminal's quit signal.
        (b"\x1b[46;5u", 190, 4, 46),      // Ctrl+. is not Ctrl+Delete.
        (b"\x1b[39;5u", 222, 4, 39),      // Ctrl+' is not Ctrl+Right.
        (b"\x1b[38;5u", 55, 4, 38),       // Ctrl+& on AZERTY is not Ctrl+Up.
        (b"\x1b[91;5u", 27, 0, 0),        // Ctrl+[ is Escape.
        (b"\x1b[46;3u", 190, 1, 46),      // Alt+.
        (b"\x1b.", 190, 1, 46),           // Alt+. without the protocol.
        (b"\x1b[97;7u", 65, 5, 0),        // Ctrl+Alt+A is a chord, not the text `a`.
        (&[0x1c][..], 220, 4, 28),        // Ctrl+\ without the protocol.
        (&[0x00][..], 32, 4, 0),          // Ctrl+Space without the protocol.
    ] {
        let keys = decoded_keys(sequence);
        assert_eq!(keys.len(), 1, "{sequence:?}");
        assert_eq!(
            (keys[0].code, keys[0].modifiers, keys[0].character),
            (code, modifiers, character),
            "{sequence:?}"
        );
    }
}

#[test]
fn protocol_keys_follow_the_base_layout_and_the_keypad() {
    for (sequence, code, modifiers) in [
        (&b"\x1b[1089::99;5u"[..], 67, 4), // Ctrl+C on a Russian layout.
        (b"\x1b[99::99;5u", 67, 4),        // Reported alternates on a US layout.
        (b"\x1b[99:67:99;6u", 67, 6),      // A shifted alternate is not the base.
        (b"\x1b[57417u", 37, 0),           // Keypad Left.
        (b"\x1b[57419;5u", 38, 4),         // Ctrl+keypad Up.
        (b"\x1b[57426u", 46, 0),           // Keypad Delete.
    ] {
        let keys = decoded_keys(sequence);
        assert_eq!(keys.len(), 1, "{sequence:?}");
        assert_eq!(
            (keys[0].code, keys[0].modifiers),
            (code, modifiers),
            "{sequence:?}"
        );
    }
    // Without the terminal's base layout there is no key to act on.
    assert!(decoded_keys(b"\x1b[1089;5u").is_empty());
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
    assert!(matches!(&output[0], Decoded::Paste(text) if text == "first\r\n\x03\x11\x1b[A\tlast"));
    assert!(matches!(&output[1], Decoded::Key(key) if key.code == 13 && !key.ctrl()));
}

#[test]
fn escape_is_bounded_and_late_cursor_reports_do_not_enter_the_draft() {
    let mut parser = Parser::default();
    assert!(parser.wait().is_none());
    assert!(parser.push(27, false).is_empty());
    assert!(
        parser
            .wait()
            .is_some_and(|wait| wait <= super::super::ESCAPE_TIMEOUT)
    );
    parser.escaped_at = Some(Instant::now() - Duration::from_secs(1));
    assert_eq!(parser.wait(), Some(Duration::ZERO));
    assert_eq!(parser.idle().unwrap().code, 27);
    assert!(parser.wait().is_none());
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
