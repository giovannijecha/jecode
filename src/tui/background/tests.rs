use super::*;

#[test]
fn accepts_bel_and_st_with_variable_component_widths() {
    assert!(matches!(
        candidate(b"\x1b]11;rgb:ffff/ffff/ffff\x07"),
        Match::Reply(true)
    ));
    assert!(matches!(
        candidate(b"\x1b]11;rgb:0/0/0\x1b\\"),
        Match::Reply(false)
    ));
    assert!(matches!(
        candidate(b"\x1b]11;rgb:eeee/eeee/eeee\x07"),
        Match::Reply(true)
    ));
}

#[test]
fn malformed_and_unrelated_sequences_are_not_consumed() {
    assert!(matches!(candidate(b"\x1b]11;rgb:zz/0/0\x07"), Match::Other));
    assert!(matches!(
        candidate(b"\x1b]10;rgb:fff/fff/fff\x07"),
        Match::Other
    ));
    assert!(matches!(candidate(b"\x1b[A"), Match::Other));
}

#[test]
fn split_reply_preserves_following_paste_and_unrelated_input() {
    let mut probe = Probe::for_test();
    let (events, light) = probe.feed(Input::Paste("\x1b]11;rgb:ffff/".encode_utf16().collect()));
    assert!(events.is_empty() && light.is_none());
    let (events, light) = probe.feed(Input::Paste("ffff/ffff\x07draft".encode_utf16().collect()));
    assert_eq!(light, Some(true));
    assert!(
        matches!(&events[..], [Input::Paste(units)] if String::from_utf16(units).unwrap() == "draft")
    );
    let mut probe = Probe::for_test();
    let (events, light) = probe.feed(Input::Paste("\x1b[A1".encode_utf16().collect()));
    assert!(light.is_none());
    assert!(
        matches!(&events[..], [Input::Paste(units)] if String::from_utf16(units).unwrap() == "\x1b[A1")
    );
}

#[test]
fn a_stalled_reply_is_replayed_without_a_background_decision() {
    let mut probe = Probe::for_test();
    let (events, light) = probe.feed(Input::Paste("\x1b]11;rgb:ffff".encode_utf16().collect()));
    assert!(events.is_empty() && light.is_none());
    // The window closing does not cut a reply in progress.
    probe.started -= TIMEOUT;
    let (events, fallback) = probe.expire();
    assert!(events.is_empty() && fallback.is_none());
    probe.last_at = Some(Instant::now() - REPLY_GAP);
    let (events, fallback) = probe.expire();
    assert_eq!(fallback, None);
    assert!(
        matches!(&events[..], [Input::Paste(units)] if String::from_utf16(units).unwrap() == "\x1b]11;rgb:ffff")
    );
}

fn paste(text: &str) -> Input {
    Input::Paste(text.encode_utf16().collect())
}

#[test]
fn an_escape_just_before_the_reply_stays_a_key_and_the_reply_stays_out() {
    let mut probe = Probe::for_test();
    assert!(probe.feed(paste("\x1b")).0.is_empty());
    let (events, light) = probe.feed(paste("\x1b]11;rgb:ffff/ffff/ffff\x07"));
    assert_eq!(light, Some(true));
    assert!(matches!(&events[..], [Input::Key(key)] if key.code == 27));
}

#[test]
fn escape_before_osc_reply_and_ctrl_g_after_it_remain_separate_commands() {
    use crate::tui::keys::{Decoded, Decoder};
    use crate::tui::terminal::Key;

    fn wire(bytes: &[u8]) -> Input {
        #[cfg(unix)]
        {
            Input::Bytes(bytes.to_vec())
        }
        #[cfg(not(unix))]
        {
            Input::Paste(bytes.iter().copied().map(u16::from).collect())
        }
    }

    fn decode(
        decoder: &mut Decoder,
        events: Vec<Input>,
        output: &mut Vec<Decoded>,
        raw_bytes: bool,
    ) {
        for event in events {
            match event {
                #[cfg(unix)]
                Input::Bytes(bytes) => output.extend(decoder.bytes(&bytes)),
                Input::Paste(units) if raw_bytes => {
                    // Exercise the shared VT byte decoder on Windows too.
                    let text = String::from_utf16(&units).unwrap();
                    output.extend(decoder.bytes(text.as_bytes()));
                }
                Input::Paste(units) => {
                    let text = decoder.paste(&units).unwrap();
                    if !text.is_empty() {
                        output.push(Decoded::Paste(text));
                    }
                }
                Input::Key(key) => output.extend(decoder.key(key)),
                _ => panic!("unexpected terminal event"),
            }
        }
    }

    for (raw_bytes, chunks) in [false, true].into_iter().flat_map(|raw_bytes| {
        [
            vec![b"\x1b]11;rgb:ffff/ffff/ffff\x07".as_slice()],
            vec![b"\x1b]11;rgb:ffff/".as_slice(), b"ffff/ffff\x07".as_slice()],
        ]
        .map(|chunks| (raw_bytes, chunks))
    }) {
        let mut probe = Probe::for_test();
        let mut decoder = Decoder::default();
        let mut output = Vec::new();
        let (events, light) = probe.feed(wire(b"\x1b"));
        assert!(events.is_empty() && light.is_none());
        for chunk in chunks {
            let (events, light) = probe.feed(wire(chunk));
            decode(&mut decoder, events, &mut output, raw_bytes);
            if chunk.last() == Some(&7) {
                assert_eq!(light, Some(true));
            } else {
                assert!(light.is_none());
            }
        }
        let ctrl_g = if raw_bytes {
            wire(b"\x07")
        } else {
            Input::Key(Key {
                code: 71,
                modifiers: 4,
                character: 7,
            })
        };
        let (events, light) = probe.feed(ctrl_g);
        assert!(light.is_none());
        decode(&mut decoder, events, &mut output, raw_bytes);
        assert!(
            matches!(output.as_slice(), [Decoded::Key(esc), Decoded::Key(commands)]
                if esc.code == 27 && commands.code == 71 && commands.ctrl() && !commands.alt())
        );
    }
}

#[test]
fn a_reply_slower_than_a_lone_escape_still_decides_the_background() {
    let mut probe = Probe::for_test();
    assert!(probe.feed(paste("\x1b]11;rgb:ff")).0.is_empty());
    // Longer than the lone-Escape wait, shorter than the reply gap.
    probe.last_at = Some(Instant::now() - PREFIX_TIMEOUT * 2);
    assert!(probe.expire().0.is_empty());
    let (events, light) = probe.feed(paste("ff/ffff/ffff\x07"));
    assert!(events.is_empty());
    assert_eq!(light, Some(true));
}

#[test]
fn one_late_reply_is_consumed_after_the_window_and_later_input_passes() {
    let mut probe = Probe::for_test();
    probe.native_hint = Some(false);
    probe.started -= TIMEOUT;
    let (events, fallback) = probe.expire();
    assert!(events.is_empty());
    assert_eq!(fallback, Some(false));
    assert_eq!(probe.wait(), None, "no timer while nothing is held");
    // The late reply arrives split, then typing follows in the same event.
    assert!(probe.feed(paste("\x1b]11;rgb:0000/")).0.is_empty());
    let (events, light) = probe.feed(paste("0000/0000\x1b\\draft"));
    assert_eq!(light, None, "the settled palette stays");
    assert!(
        matches!(&events[..], [Input::Paste(units)] if String::from_utf16(units).unwrap() == "draft")
    );
    // A second sequence is ordinary input again.
    let (events, _) = probe.feed(paste("\x1b]11;rgb:0/0/0\x07"));
    assert!(
        matches!(&events[..], [Input::Paste(units)] if String::from_utf16(units).unwrap() == "\x1b]11;rgb:0/0/0\x07")
    );
}

#[test]
fn a_lone_escape_is_released_promptly_and_unicode_paste_is_unchanged() {
    let inputs = [
        Input::Paste(vec![27]),
        #[cfg(unix)]
        Input::Bytes(vec![27]),
    ];
    for input in inputs {
        let mut probe = Probe::for_test();
        let (events, light) = probe.feed(input);
        assert!(events.is_empty() && light.is_none());
        probe.last_at = Some(Instant::now() - PREFIX_TIMEOUT);
        let (events, fallback) = probe.expire();
        let [Input::Key(key)] = &events[..] else {
            panic!("expired Escape was not a key: {events:?}");
        };
        let mut decoder = crate::tui::keys::Decoder::default();
        assert!(matches!(
            decoder.key(*key).as_slice(),
            [crate::tui::keys::Decoded::Key(key)] if key.code == 27
        ));
        assert!(decoder.wait().is_none(), "Escape must not wait twice");
        assert!(fallback.is_none() && probe.active);
        let unicode: Vec<u16> = "β🙂".encode_utf16().collect();
        let (events, light) = probe.feed(Input::Paste(unicode.clone()));
        assert!(light.is_none());
        assert!(matches!(&events[..], [Input::Paste(units)] if units == &unicode));
    }
}

#[test]
fn no_osc_reply_uses_only_the_native_background_hint() {
    let mut probe = Probe::for_test();
    probe.native_hint = Some(true);
    probe.started -= TIMEOUT;
    let (events, light) = probe.expire();
    assert!(events.is_empty());
    assert_eq!(light, Some(true));
    assert!(!probe.active);
}
