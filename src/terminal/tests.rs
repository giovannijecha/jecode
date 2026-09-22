use super::*;
use std::time::Duration;

#[test]
fn editor_preserves_unicode_sequences_and_limits_input() {
    let mut editor = text::Editor::default();
    for sample in ["e\u{301}", "👩‍💻", "🇮🇹", "क्‍ष", "中文", "\u{600}a"] {
        editor.insert(sample);
        assert_eq!(editor.text, sample);
        editor.left();
        assert_eq!(editor.cursor, 0);
        editor.delete();
        assert_eq!(editor.text, "");
    }
    editor.insert("one two");
    editor.left();
    editor.backspace();
    assert_eq!(editor.text, "one to");
    editor.insert("x");
    assert_eq!(editor.text, "one txo");
    editor.insert(&"a".repeat(8192));
    assert_eq!(editor.text, "one txo");
    editor.take();
    editor.insert("a\x1b[2J\r\n\u{202e}b");
    assert!(!editor.text.contains(['\x1b', '\r', '\n', '\u{202e}']));
}

#[test]
fn stream_keeps_draft_cancels_without_replay_and_can_restart() {
    let start = Instant::now();
    let mut model = model::Model::new(start);
    model.input(Key::Text("/long".into()), start);
    model.input(Key::Enter, start);
    assert!(model.streaming());
    assert!(!model.tick(start));
    assert!(model.tick(start + Duration::from_secs(1)));
    model.input(Key::Text("next draft".into()), start);
    model.input(Key::Enter, start);
    assert_eq!(model.editor.text, "next draft");
    assert_eq!(model.blocks.len(), 3);
    model.input(Key::Escape, start);
    let partial = model.blocks.last().unwrap().text.clone();
    assert!(!partial.is_empty());
    assert!(!model.tick(start + Duration::from_secs(10)));
    assert_eq!(model.blocks.last().unwrap().text, partial);
    model.input(Key::Enter, start);
    assert!(model.streaming());
    assert_eq!(model.blocks.len(), 5);
    model.input(Key::Quit, start);
    assert!(model.quit);
}

#[test]
fn demonstration_completion_and_failure_are_distinct() {
    for (prompt, status) in [
        ("hello", "Complete - local demo"),
        (
            "/error",
            "Simulated stream failure - partial output retained",
        ),
    ] {
        let start = Instant::now();
        let mut model = model::Model::new(start);
        model.input(Key::Text(prompt.into()), start);
        model.input(Key::Enter, start);
        for n in 1..200 {
            model.tick(start + Duration::from_millis(n * 40));
        }
        assert!(!model.streaming());
        assert_eq!(model.status, status);
        assert!(!model.blocks.last().unwrap().text.is_empty());
    }
}

#[test]
fn layouts_stay_bounded_and_render_only_changed_rows() {
    let mut model = model::Model::new(Instant::now());
    model.editor.insert(&"long draft café 👩‍💻 ".repeat(40));
    model.blocks.push(model::Block {
        speaker: "Untrusted",
        text: "\x1b[2J\u{009b}secret\u{202e}\n".repeat(100),
    });
    for width in [1, 10, 25, 40, 80, 140, 600] {
        for height in [1, 5, 9, 24, 60, 250] {
            let frame = view::frame(&model, width, height);
            for row in &frame {
                assert!(text::width(&row.text) <= width.saturating_sub(1).max(1));
                assert!(!row.text.contains(['\x1b', '\n', '\u{009b}', '\u{202e}']));
            }
            let mut renderer = render::Renderer::default();
            let first = renderer.draw(frame.clone(), (width, height), false);
            assert!(!first.contains("\x1b[0m"));
            assert!(renderer.draw(frame, (width, height), false).is_empty());
        }
    }
}

#[test]
fn vt_input_survives_utf8_escape_and_paste_fragmentation() {
    let start = Instant::now();
    let bytes = "aé\x1b[D\x1b[200~paste\n\x03\x11👩‍💻\x1b[201~\r".as_bytes();
    let decode = |chunks: Vec<&[u8]>| {
        let mut decoder = input::Decoder::default();
        let mut keys = Vec::new();
        for chunk in chunks {
            keys.extend(decoder.push(chunk, start));
        }
        keys
    };
    let expected = decode(vec![bytes]);
    assert_eq!(
        expected,
        vec![
            Key::Text("a".into()),
            Key::Text("é".into()),
            Key::Left,
            Key::Text("paste\n\x03\x11👩‍💻".into()),
            Key::Enter
        ]
    );
    for split in 0..bytes.len() {
        assert_eq!(decode(vec![&bytes[..split], &bytes[split..]]), expected);
    }
    let mut decoder = input::Decoder::default();
    assert!(decoder.push(b"\x1b", start).is_empty());
    assert!(decoder.idle(start + Duration::from_millis(20)).is_empty());
    assert_eq!(
        decoder.idle(start + Duration::from_millis(80)),
        [Key::Escape]
    );
}

#[test]
fn paste_budget_and_control_bytes_cannot_submit_or_exit() {
    let mut decoder = input::Decoder::default();
    let now = Instant::now();
    assert!(decoder.push(b"\x1b[200~", now).is_empty());
    assert!(decoder.push(&vec![b'x'; 9000], now).is_empty());
    assert!(decoder.push(b"\r\n\x03\x11", now).is_empty());
    let keys = decoder.push(b"\x1b[201~", now);
    assert_eq!(keys, vec![Key::Text("x".repeat(8192))]);
    let mut model = model::Model::new(now);
    for key in keys {
        model.input(key, now);
    }
    assert!(!model.quit);
    assert!(!model.streaming());
    assert_eq!(model.editor.text.len(), 8192);
}

#[test]
fn wrapping_keeps_words_and_never_splits_unknown_unicode_runs() {
    assert_eq!(
        text::wrap("a useful terminal", 10),
        ["a useful ", "terminal"]
    );
    assert_eq!(text::wrap("中文中文", 4), ["..."]);
    assert_eq!(text::wrap("    abcdef", 6), ["    ab", "cdef"]);
}

#[test]
fn native_scrollback_keeps_completed_prefix_out_of_tail_updates() {
    let mut model = model::Model::new(Instant::now());
    model.blocks.push(model::Block {
        speaker: "Demo",
        text: "old row\n".repeat(30),
    });
    let mut renderer = render::Renderer::default();
    let mut layout = view::Layout::default();
    let first = renderer.draw(layout.frame(&model, 80, 24), (80, 24), false);
    assert!(first.contains("jecode"));
    assert!(!first.contains("1049"));
    model
        .blocks
        .last_mut()
        .unwrap()
        .text
        .push_str(&"new row\n".repeat(20));
    let changed = renderer.draw(layout.frame(&model, 80, 24), (80, 24), false);
    assert!(changed.contains("new row"));
    assert!(!changed.contains("old row"));
    assert!(!changed.contains("jecode"));
    assert!(!changed.contains("\x1b[2J"));
    let resized = renderer.draw(layout.frame(&model, 40, 12), (40, 12), false);
    assert!(
        !resized.contains("new row"),
        "resize must leave emitted transcript to native reflow"
    );
    assert!(resized.contains("\r\x1b[J"));
    assert!(!resized.contains("\x1b[2J"));
    assert!(!resized.contains("old row"));
    assert!(!resized.contains("\x1b[3J"));
    assert!(resized.contains("Local demo"));
}

#[test]
fn markdown_is_presented_without_fences_or_control_injection() {
    let input = "# Heading\nA **bold** word and `code`.\n```rust\nfn main() {\n  let x = \"hello\"; // note\n}\n```\n\x1b[2J";
    let rows = markdown::render(input, 80);
    let plain: String = rows.iter().map(|row| row.paint(false)).collect();
    assert!(plain.contains("A bold word and code."));
    assert!(plain.contains("fn main()"));
    assert!(!plain.contains(['`', '\x1b']));
    assert!(rows.iter().any(|row| row.tone == style::Tone::Code));
    let elided = markdown::render(&format!("**{}** plain", "中".repeat(30)), 24);
    assert!(elided.iter().all(|row| row.spans.is_empty()));
    assert!(rows.iter().any(|row| {
        row.spans
            .iter()
            .any(|(_, tone)| *tone == style::Tone::Keyword)
    }));
    let mixed = "**café 中文 👩‍💻** `é`\n```rust\nlet x = \"\\é👩‍💻\";\n```";
    for (end, _) in mixed.char_indices().chain([(mixed.len(), '\0')]) {
        for row in markdown::render(&mixed[..end], 24) {
            row.paint(true);
            row.paint(false);
        }
    }
}

#[test]
fn composer_keeps_one_cursor_visible_at_every_editing_boundary() {
    let mut model = model::Model::new(Instant::now());
    for sample in ["", "a long draft ", "café 👩‍💻 中文 ", "\u{e000} "] {
        model.editor.text = sample.repeat(30);
        for cursor in text::boundaries(&model.editor.text) {
            model.editor.cursor = cursor;
            let rows = view::frame(&model, 40, 12);
            let count = rows
                .iter()
                .flat_map(|row| &row.spans)
                .filter(|(_, tone)| *tone == style::Tone::Cursor)
                .count();
            assert_eq!(count, 1, "{sample:?}, cursor {cursor}");
            for row in rows {
                row.paint(true);
            }
        }
    }
}
