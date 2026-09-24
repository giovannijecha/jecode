use super::*;
use crate::session::{self, Event};

fn ready() -> (model::Model, session::Session) {
    let mut model = account::model(session::Model::Luna, None);
    account::event(&mut model, Event::Ready);
    (model, session::tests::ready_fixture())
}
fn finish(model: &mut model::Model, session: &mut session::Session) {
    loop {
        let event = session::tests::next(session);
        let done = matches!(event, Event::Finished(..));
        account::event(model, event);
        if done {
            break;
        }
    }
}

#[test]
fn vt_shortcuts_keep_enter_distinct_and_accept_common_modifier_encodings() {
    let now = Instant::now();
    for (bytes, expected) in [
        (b"\x1b[1;5D".as_slice(), Key::WordLeft),
        (b"\x1b[1;5C", Key::WordRight),
        (b"\x1b[3;5~", Key::WordDelete),
        (b"\x1b[127;5u", Key::WordBackspace),
        (b"\x1b[1;5H", Key::DraftStart),
        (b"\x1b[1;5F", Key::DraftEnd),
        (b"\x1b[1;3A", Key::RetrieveQueued),
        (b"\x1b[1;3B", Key::AbandonRecovered),
        (b"\x1b[A", Key::Up),
        (b"\x1b[B", Key::Down),
        (b"\x1b[13;2u", Key::Newline),
        (b"\x17", Key::WordBackspace),
        (b"\x0f", Key::Newline),
        (b"\x0a", Key::Newline),
        (b"\x10", Key::HistoryPrevious),
        (b"\x0e", Key::HistoryNext),
        (b"\x0d", Key::Enter),
        (b"\x11", Key::Quit),
    ] {
        for split in 0..=bytes.len() {
            let mut decoder = input::Decoder::default();
            let mut keys = decoder.push(&bytes[..split], now);
            keys.extend(decoder.push(&bytes[split..], now));
            assert_eq!(
                keys.as_slice(),
                std::slice::from_ref(&expected),
                "{bytes:?}, split {split}"
            );
        }
    }
}

#[test]
fn bracketed_paste_retains_lines_tabs_unicode_and_is_literal_at_any_cursor() {
    let now = Instant::now();
    let mut decoder = input::Decoder::default();
    let keys = decoder.push(
        "\x1b[200~/help\r\n  indented\t👩‍💻\nlast\x1b[201~".as_bytes(),
        now,
    );
    assert_eq!(keys, [Key::Paste("/help\r\n  indented\t👩‍💻\nlast".into())]);
    for (initial, cursor, expected) in [
        ("tail", 0, "/help\n  indented\t👩‍💻\nlasttail"),
        ("headtail", 4, "head/help\n  indented\t👩‍💻\nlasttail"),
        ("head", 4, "head/help\n  indented\t👩‍💻\nlast"),
    ] {
        let (mut model, mut session) = ready();
        model.editor.insert(initial);
        model.editor.cursor = cursor;
        for key in &keys {
            account::input(&mut model, key.clone(), &mut session);
        }
        assert_eq!(model.editor.text, expected);
        assert!(model.blocks.is_empty());
        assert!(session.ready());
        let rows = view::chrome(&model, 40, 12);
        assert!(
            rows.iter()
                .all(|row| !row.text.contains(['\n', '\t', '\r']))
        );
    }
    let (mut model, mut session) = ready();
    account::input(&mut model, Key::Paste("/help".into()), &mut session);
    assert!(!model.menu.active(&model.editor.text));
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(model.blocks[0].speaker, "You");
    assert_eq!(model.blocks[0].text, "/help");
}

#[test]
fn word_line_and_visual_row_editing_preserve_safe_boundaries() {
    let mut model = model::Model::new(Instant::now());
    model.editor.set_columns(5);
    model.input(Key::Text("one two\nabcde\nxyz".into()), Instant::now());
    model.input(Key::Home, Instant::now());
    assert_eq!(model.editor.cursor, "one two\nabcde\n".len());
    model.input(Key::DraftStart, Instant::now());
    model.input(Key::End, Instant::now());
    assert_eq!(model.editor.cursor, "one two".len());
    model.input(Key::WordLeft, Instant::now());
    assert_eq!(model.editor.cursor, "one ".len());
    model.input(Key::WordDelete, Instant::now());
    assert_eq!(model.editor.text, "one abcde\nxyz");
    model.input(Key::DraftEnd, Instant::now());
    model.input(Key::Up, Instant::now());
    assert_eq!(&model.editor.text[model.editor.cursor..], "e\nxyz");
    model.input(Key::Down, Instant::now());
    assert_eq!(model.editor.cursor, model.editor.text.len());
    model.input(Key::WordBackspace, Instant::now());
    assert_eq!(model.editor.text, "one abcde\n");
    model.input(Key::Text("e\u{301}👩‍💻".into()), Instant::now());
    model.input(Key::Backspace, Instant::now());
    assert_eq!(model.editor.text, "one abcde\ne\u{301}");
    model.input(Key::Backspace, Instant::now());
    assert_eq!(model.editor.text, "one abcde\n");
}

#[test]
fn conjoining_hangul_is_one_editing_unit_across_cursor_word_and_visual_navigation() {
    let syllable = "\u{1100}\u{1161}\u{11a8}";
    let text = format!("a{syllable}b");
    let start = 1;
    let end = start + syllable.len();
    let mut editor = editor::Editor::default();
    editor.set_columns(2);
    assert!(editor.insert(&text));
    editor.left();
    assert_eq!(editor.cursor, end);
    editor.left();
    assert_eq!(editor.cursor, start);
    editor.right();
    assert_eq!(editor.cursor, end);
    editor.backspace();
    assert_eq!(editor.text, "ab");
    assert_eq!(editor.cursor, start);

    editor.replace(&text);
    editor.cursor = start;
    editor.delete();
    assert_eq!(editor.text, "ab");

    editor.replace(&text);
    editor.word_left();
    assert!(editor::boundaries(&editor.text).contains(&editor.cursor));
    editor.word_delete();
    assert!(editor::boundaries(&editor.text).contains(&editor.cursor));
    editor.replace(&format!("x {syllable} y"));
    editor.cursor = "x ".len() + syllable.len();
    editor.word_backspace();
    assert_eq!(editor.text, "x  y");
    let layout = editor_visual::Visual::new(&text, 2);
    assert_eq!(
        layout
            .stops
            .iter()
            .map(|stop| stop.index)
            .collect::<Vec<_>>(),
        vec![0, start, end, text.len()]
    );
    editor.replace(&text);
    assert!(editor.vertical(false));
    assert!(editor::boundaries(&editor.text).contains(&editor.cursor));

    for cluster in [
        syllable,
        "\u{1100}\u{1100}\u{1161}\u{1161}\u{11a8}\u{11a8}",
        "\u{ac00}\u{11a8}",
        "\u{ac01}\u{11a8}",
        "e\u{301}",
        "\u{1f469}\u{200d}\u{1f4bb}",
        "\u{1f1fa}\u{1f1f8}",
    ] {
        let mut editor = editor::Editor::default();
        assert!(editor.insert(cluster));
        editor.backspace();
        assert!(editor.text.is_empty(), "{cluster:?}");
    }
}

#[test]
fn history_restores_unsent_multiline_draft_and_does_not_mutate_recalled_prompt() {
    let mut model = model::Model::new(Instant::now());
    model
        .prompt_history
        .load(vec!["first".into(), "second\n  original".into()]);
    model.editor.insert("draft\n  pending");
    model.input(Key::Up, Instant::now());
    assert_eq!(model.editor.text, "draft\n  pending");
    let cursor = model.editor.cursor;
    model.input(Key::PageUp, Instant::now());
    assert_eq!(model.editor.text, "second\n  original");
    model.input(Key::Text(" edited".into()), Instant::now());
    model.input(Key::PageUp, Instant::now());
    assert_eq!(model.editor.text, "first");
    model.input(Key::PageDown, Instant::now());
    assert_eq!(model.editor.text, "second\n  original");
    model.input(Key::PageDown, Instant::now());
    assert_eq!(model.editor.text, "draft\n  pending");
    assert_eq!(model.editor.cursor, cursor);
    model.editor.take();
    model.input(Key::Up, Instant::now());
    assert_eq!(model.editor.text, "second\n  original");
}

#[test]
fn account_submission_and_local_commands_feed_only_user_prompt_history() {
    let (mut model, mut session) = ready();
    for prompt in ["first prompt", "second prompt"] {
        account::input(&mut model, Key::Text(prompt.into()), &mut session);
        account::input(&mut model, Key::Enter, &mut session);
        finish(&mut model, &mut session);
    }
    account::input(&mut model, Key::Text("/help".into()), &mut session);
    account::input(&mut model, Key::Enter, &mut session);
    account::event(&mut model, Event::LoginCode("FAKE-DEVICE".into()));
    model.editor.insert("unsent\n  draft");
    account::input(&mut model, Key::PageUp, &mut session);
    assert_eq!(model.editor.text, "second prompt");
    account::input(&mut model, Key::PageUp, &mut session);
    assert_eq!(model.editor.text, "first prompt");
    account::input(&mut model, Key::PageDown, &mut session);
    assert_eq!(model.editor.text, "second prompt");
    account::input(&mut model, Key::PageDown, &mut session);
    assert_eq!(model.editor.text, "unsent\n  draft");
    assert!(session.ready());
}

#[test]
fn new_canonical_queue_turn_enters_live_recall_without_losing_a_browsed_draft() {
    let (mut model, mut session) = ready();
    model
        .prompt_history
        .load(vec!["older prompt".into(), "recent prompt".into()]);
    model.editor.insert("unsent\n  draft");
    model.editor.home();
    let saved_cursor = model.editor.cursor;
    account::input(&mut model, Key::HistoryPrevious, &mut session);
    assert_eq!(model.editor.text, "recent prompt");
    account::event(
        &mut model,
        Event::Guidance {
            text: "queued new turn".into(),
            new_turn: true,
        },
    );
    assert_eq!(model.editor.text, "recent prompt");
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert_eq!(model.editor.text, "queued new turn");
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert_eq!(model.editor.text, "unsent\n  draft");
    assert_eq!(model.editor.cursor, saved_cursor);

    account::event(
        &mut model,
        Event::Guidance {
            text: "intraturn guidance".into(),
            new_turn: false,
        },
    );
    account::event(&mut model, Event::GuidanceReturned("returned".into()));
    account::input(&mut model, Key::HistoryPrevious, &mut session);
    assert_eq!(model.editor.text, "queued new turn");
    account::input(&mut model, Key::HistoryPrevious, &mut session);
    assert_eq!(model.editor.text, "recent prompt");
    assert!(session.ready());
}

#[test]
fn live_history_stays_bounded_when_a_new_turn_evicts_the_viewed_entry() {
    let mut history = prompt_history::PromptHistory::default();
    history.load((0..64).map(|index| format!("prompt {index}")).collect());
    let mut editor = editor::Editor::default();
    editor.insert("draft\n  with cursor");
    editor.home();
    let saved_cursor = editor.cursor;
    let mut literal = false;
    for _ in 0..64 {
        history.previous(&mut editor, &mut literal);
    }
    assert_eq!(editor.text, "prompt 0");
    history.record_new_turn("prompt 64");
    assert_eq!(editor.text, "prompt 0");
    history.next(&mut editor, &mut literal);
    assert_eq!(editor.text, "prompt 1");
    for _ in 0..63 {
        history.next(&mut editor, &mut literal);
    }
    assert_eq!(editor.text, "prompt 64");
    history.next(&mut editor, &mut literal);
    assert_eq!(editor.text, "draft\n  with cursor");
    assert_eq!(editor.cursor, saved_cursor);
}

#[test]
fn invalid_utf8_paste_rejects_all_bytes_without_changing_the_draft() {
    let now = Instant::now();
    let mut decoder = input::Decoder::default();
    let keys = decoder.push(b"\x1b[200~part\xff\x1b[201~", now);
    assert_eq!(
        keys,
        [Key::PasteRejected("Invalid paste text / draft kept")]
    );
    let (mut model, mut session) = ready();
    account::input(&mut model, Key::Text("keep".into()), &mut session);
    for key in keys {
        account::input(&mut model, key, &mut session);
    }
    assert_eq!(model.editor.text, "keep");
    assert!(
        model
            .account
            .as_ref()
            .unwrap()
            .local_notice
            .contains("Invalid paste")
    );
    assert!(session.ready());
}

#[test]
fn input_and_submission_limits_report_and_retain_draft_inside_composer() {
    let (mut model, mut session) = ready();
    account::input(&mut model, Key::Text("keep".into()), &mut session);
    account::input(&mut model, Key::Paste("x".repeat(8190)), &mut session);
    assert_eq!(model.editor.text, "keep");
    assert!(
        model
            .account
            .as_ref()
            .unwrap()
            .local_notice
            .contains("8 KiB")
    );
    assert!(
        view::chrome(&model, 80, 24)
            .iter()
            .any(|row| row.text.contains("draft kept"))
    );
    model.editor.text = "x".repeat(8193);
    model.editor.cursor = model.editor.text.len();
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(model.editor.text.len(), 8193);
    assert!(
        model
            .account
            .as_ref()
            .unwrap()
            .local_notice
            .contains("Prompt exceeds")
    );
    assert!(model.blocks.is_empty());
    assert!(session.ready());
}

#[test]
fn long_multiline_viewport_keeps_one_cursor_and_completed_rows_unchanged() {
    let mut model = model::Model::new(Instant::now());
    model.blocks.push(model::Block {
        speaker: "You",
        text: "immutable marker".into(),
    });
    model.editor.insert(&"  row\t👩‍💻\n".repeat(150));
    let mut renderer = render::Renderer::default();
    let mut layout = view::Layout::default();
    let _ = renderer.draw(layout.frame(&model, 80, 24), (80, 24), false);
    for width in [40, 25, 80, 32, 80] {
        model.editor.set_columns(width - 4);
        for cursor in [0, model.editor.text.len() / 2, model.editor.text.len()] {
            model.editor.cursor = editor_visual::Visual::new(&model.editor.text, width - 4)
                .stops
                .iter()
                .find(|stop| stop.index >= cursor)
                .unwrap()
                .index;
            let frame = layout.frame(&model, width, 12);
            let count = frame
                .iter()
                .flat_map(|row| &row.spans)
                .filter(|(_, tone)| *tone == style::Tone::Cursor)
                .count();
            assert_eq!(count, 1);
            assert!(frame.iter().filter(|row| row.transient).count() < 12);
            let changed = renderer.draw(frame, (width, 12), false);
            assert!(!changed.contains("immutable marker"));
        }
    }
}
