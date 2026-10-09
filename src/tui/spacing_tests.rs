use super::{
    State,
    activity::Activity,
    line::Line,
    markdown,
    state::{Item, Kind},
    theme::{CODE_BACKGROUND, USER_BACKGROUND},
    view,
};
use crate::{events::Event, json::Value};

#[derive(Clone, Copy, Debug)]
enum Block {
    User,
    Assistant,
    Tool,
    Local,
}

fn start(state: &mut State, label: &str) {
    state.event(Event::ToolStarted {
        id: label.into(),
        name: "bash".into(),
        arguments: Value::object([("command", Value::string(label))]),
    });
}

fn finish(state: &mut State, label: &str, output: &str) {
    state.event(Event::ToolFinished {
        id: label.into(),
        name: "bash".into(),
        summary: "exit 0".into(),
        result: Value::object([
            ("stdout", Value::string(output)),
            ("exit_code", Value::number(0)),
        ]),
    });
}

fn add(state: &mut State, block: Block, label: &str) {
    match block {
        Block::User => state.message(Kind::User, label),
        Block::Assistant => state.message(Kind::Assistant, label),
        Block::Tool => {
            start(state, label);
            finish(state, label, "");
            state.close_tools();
        }
        Block::Local => {
            state.close_tools();
            state.items.push(Item::Local {
                command: "/help".into(),
                result: Some(label.into()),
                kind: Kind::Notice,
                details: vec![("key".into(), "value".into())],
            });
        }
    }
}

#[test]
fn block_transitions_have_one_neutral_separator_plus_their_own_panel_padding() {
    use Block::*;
    let cases = [
        (User, Assistant, 2),
        (Assistant, User, 2),
        (User, Tool, 2),
        (Tool, User, 2),
        (Assistant, Tool, 1),
        (Tool, Assistant, 1),
        (Assistant, Assistant, 1),
        (User, User, 3),
        (Tool, Tool, 1), // Separate, closed trees.
    ];
    for width in [12, 40, 80] {
        for (before, after, expected) in cases {
            let mut state = State {
                width,
                ..State::default()
            };
            add(&mut state, before, "first");
            add(&mut state, after, "second");
            let blocks = state.rows();
            let left = blocks[0]
                .iter()
                .rposition(|row| !row.plain().trim().is_empty())
                .unwrap();
            let right = blocks[1]
                .iter()
                .position(|row| !row.plain().trim().is_empty())
                .unwrap();
            let blanks = blocks[0][left + 1..]
                .iter()
                .chain(&blocks[1][..right])
                .collect::<Vec<_>>();
            assert_eq!(blanks.len(), expected, "{before:?} -> {after:?}, {width}");
            assert_eq!(
                blanks.iter().filter(|row| row.background.is_none()).count(),
                1
            );
            assert!(blanks.iter().all(|row| row.plain().trim().is_empty()));
            assert!(blocks[0][0].background.is_some() || !blocks[0][0].plain().is_empty());
        }
    }
}

#[test]
fn panels_keep_their_padding_and_text_without_a_sent_prompt_marker() {
    let mut state = State::default();
    state.message(Kind::User, "one\ntwo\nthree");
    state.message(Kind::Assistant, "answer");
    let blocks = state.rows();
    assert_eq!(blocks[0].len(), 5);
    assert!(
        blocks[0]
            .iter()
            .all(|row| row.background == Some(USER_BACKGROUND))
    );
    assert_eq!(blocks[0][0].plain(), "");
    assert_eq!(blocks[0][4].plain(), "");
    assert_eq!(blocks[0][1].plain(), "  one");
    assert_eq!(
        blocks[1],
        [Line::default(), Line::new("answer", super::theme::BODY)]
    );
}

#[test]
fn empty_assistant_updates_leave_tools_connected_and_add_no_rows() {
    let mut state = State::default();
    start(&mut state, "first");
    finish(&mut state, "first", "one\n\nthree\n");
    let original = state.rows();
    for text in ["", " ", "\r\n\t"] {
        state.event(Event::Streaming { text: text.into() });
        state.event(Event::Message { text: text.into() });
        assert_eq!(state.rows(), original);
    }
    assert!(matches!(state.items[0], Item::Tool { last: None, .. }));
    // Even a retained invisible item must not become the separator's predecessor.
    state.items.push(Item::Text {
        kind: Kind::Assistant,
        text: " \n ".into(),
    });
    start(&mut state, "second");
    finish(&mut state, "second", "second output\n");
    state.close_tools();
    let rows = state.rows();
    assert!(rows[1].is_empty());
    let all = rows.iter().flatten().map(Line::plain).collect::<Vec<_>>();
    assert_eq!(all.len(), 3);
    assert!(all[0].starts_with("├─ bash  first"));
    assert!(all[1].starts_with("└─ bash  second"));
    assert_eq!(all[2], "     second output");
    // Earlier results remain available through inline expansion.
    state.select_tool(Some(0));
    state.toggle_tool();
    state.select_tool(None);
    let expanded = state
        .rows()
        .into_iter()
        .flatten()
        .map(|row| row.plain())
        .collect::<Vec<_>>();
    assert_eq!(&expanded[1..4], &["│    one", "│    ", "│    three"]);
    let response = "\n\nanswer\n \n";
    state.event(Event::Streaming {
        text: response.into(),
    });
    let streamed = state.rows();
    let answer = streamed.last().unwrap();
    assert_eq!(answer.len(), 4); // External separator and two initial Markdown blanks.
    assert!(answer[..3].iter().all(|row| row == &Line::default()));
    state.event(Event::Message {
        text: response.into(),
    });
    assert_eq!(state.rows(), streamed);
    assert!(matches!(state.items.last(), Some(Item::Text { text, .. }) if text == response));
}

#[test]
fn compact_tool_output_keeps_actual_trailing_empty_rows_without_adding_a_terminator_row() {
    for (output, count) in [("line\n", 1), ("line\n\n\n", 3)] {
        let mut state = State::default();
        start(&mut state, "fixture");
        finish(&mut state, "fixture", output);
        state.close_tools();
        let rows = state.rows();
        assert_eq!(rows[0].len(), count + 1);
        assert_eq!(rows[0][1].plain(), "     line");
        assert!(rows[0][2..].iter().all(|row| row.plain() == "     "));
        let Item::Tool {
            result: Some(result),
            ..
        } = &state.items[0]
        else {
            panic!("expected the retained tool result");
        };
        assert_eq!(result.get("stdout").and_then(Value::as_str), Some(output));
    }
}

#[test]
fn assistant_markdown_keeps_internal_and_leading_blanks_without_automatic_margins() {
    let source = "\n# Title\nParagraph\n\n\n- one\n- two\n> quote\n---\nEnd\n \n\t";
    let rows = markdown::render(source, 20);
    let plain = rows.iter().map(Line::plain).collect::<Vec<_>>();
    assert_eq!(
        plain,
        [
            "",
            "Title",
            "Paragraph",
            "",
            "",
            "• one",
            "• two",
            "│ quote",
            &"─".repeat(20),
            "End"
        ]
    );
    assert_eq!(markdown::render("A\n\n", 20).len(), 1);
    assert_eq!(markdown::render("A\n\nB", 20).len(), 3);
    for empty in ["", "\n\n", " \t\r\n"] {
        assert!(markdown::render(empty, 20).is_empty());
    }
}

#[test]
fn complete_code_panels_have_two_colored_frame_rows_and_preserve_blank_code_lines() {
    for language in ["", "rust", "an-unsupported-long-language-name"] {
        let source = format!("before\n```{language}\nfirst\n\nsecond\n```\n\nafter");
        let rows = markdown::render(&source, 10);
        assert_eq!(rows.len(), 8);
        assert!(
            rows[1..6]
                .iter()
                .all(|row| row.background == Some(CODE_BACKGROUND))
        );
        assert_eq!(rows[3].plain(), "");
        assert_eq!(rows[5].plain(), "");
        assert_eq!(rows[6], Line::default());
        assert_eq!(rows[7].plain(), "after");
        assert!(super::text::cells(&rows[1].plain()) <= 10);
        if language.is_empty() {
            assert_eq!(rows[1].plain(), "");
        }
    }
    let wrapped = markdown::render("```\nabcdefghijkl\n```", 4);
    assert_eq!(wrapped.len(), 5); // Three visual code rows plus two frames.
    assert!(
        wrapped
            .iter()
            .all(|row| row.background == Some(CODE_BACKGROUND))
    );
}

#[test]
fn invisible_items_and_feedback_do_not_add_transcript_rows_or_separators() {
    let mut state = State::default();
    state.items.push(Item::Text {
        kind: Kind::Assistant,
        text: " \n".into(),
    });
    state.message(Kind::Assistant, "first");
    state.items.push(Item::Text {
        kind: Kind::Notice,
        text: " \n".into(),
    });
    state.message(Kind::Assistant, "second");
    state.message(Kind::Notice, "local\n\n");
    let rows = state.rows();
    assert!(rows[0].is_empty());
    assert_eq!(rows[1].len(), 1);
    assert!(rows[2].is_empty());
    assert_eq!(rows[3].len(), 2);
    assert_eq!(rows[3][0], Line::default());
    assert_eq!(rows.len(), 4);
    assert_eq!(state.notice.as_ref().unwrap().text, "local\n\n");
    add(&mut state, Block::Local, "hidden help");
    assert!(state.rows().last().unwrap().is_empty());
}

#[test]
fn the_lower_area_stays_at_the_bottom_with_space_before_queue_notices_and_activity() {
    let mut state = State::default();
    state.message(Kind::Assistant, "answer");
    state.activity = Some(Activity::new());
    state.queue.push("next prompt".into()).unwrap();
    state.notice = Some(super::Feedback::result(Kind::Warning, "notice"));
    let frame = view::frame(&state, "model", "directory");
    assert_eq!(frame.composer, state.height - 7);
    assert_eq!(frame.live[frame.composer - 1], Line::default());
    assert!(
        frame.live[frame.composer]
            .plain()
            .starts_with("› next prompt")
    );
    assert!(frame.live[frame.composer + 1].plain().contains("notice"));
    assert!(frame.live[frame.composer + 2].plain().contains("Waiting"));
    assert!(frame.live[frame.composer + 3].plain().starts_with('─'));
    state.items.clear();
    let empty = view::frame(&state, "model", "directory");
    assert_eq!(empty.composer, frame.composer);
    assert!(
        empty.live[empty.composer]
            .plain()
            .starts_with("› next prompt")
    );
}

#[test]
fn a_short_viewport_drops_the_gap_before_displacing_input_or_all_visible_content() {
    for height in 1..=6 {
        let mut state = State {
            height,
            ..State::default()
        };
        state.message(Kind::Assistant, "answer");
        state.editor.insert("draft");
        let frame = view::frame(&state, "model", "directory");
        assert!(frame.live.len() <= height);
        assert!(
            frame.live[frame.cursor.unwrap().0]
                .plain()
                .contains("draft")
        );
        if height <= 4 {
            assert_eq!(frame.composer, 0);
        } else {
            assert_eq!(frame.composer, height - 4);
            if height == 6 {
                assert_eq!(frame.live[frame.composer - 1], Line::default());
            }
        }
    }
}
