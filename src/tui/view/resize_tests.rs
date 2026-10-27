use super::*;
use crate::tui::{
    state::Kind,
    theme::{CURSOR, USER_SURFACE},
};

#[test]
fn tiny_widths_keep_the_software_caret_visible_and_restore_the_multiline_draft() {
    let mut state = State::default();
    state.editor.insert("first\nsecond🙂");
    state.editor.cursor = "first\nse".len();
    let draft = state.editor.clone();
    for width in [2, 3, 4, 6, 8, 24, 80] {
        for height in [1, 2, 4, 12] {
            state.width = width;
            state.height = height;
            let frame = frame(&state, "fixture/model", "fixture directory");
            let (row, column) = frame.cursor.unwrap();
            assert!(column < width, "caret must fit width {width}");
            assert!(frame.live[row].paint(width).contains(&CURSOR.to_string()));
            assert!(frame.live.len() <= height);
            assert_eq!(state.editor, draft);
        }
    }
}

#[test]
fn short_command_suggestions_keep_the_selected_item_and_caret_in_view() {
    let mut state = State {
        width: 50,
        ..State::default()
    };
    state.message(Kind::Assistant, "Previous answer.");
    state.editor.insert("/");
    state.suggestions.refresh("/");
    state.suggestions.selected = state.suggestions.matches.len() - 1;
    let selected = state.suggestions.chosen().unwrap();
    for height in [2, 3, 4, 6, 12] {
        state.height = height;
        let frame = frame(&state, "fixture/model", "fixture directory");
        let rows: Vec<_> = frame.live.iter().map(Line::plain).collect();
        assert!(frame.live.len() <= height);
        let (row, _) = frame.cursor.unwrap();
        assert!(
            frame.live[row]
                .paint(state.width)
                .contains(&CURSOR.to_string())
        );
        // The caret's row and the panel's title come first, then the
        // selected command.
        assert!(rows[row + 1].starts_with("─ Commands"), "{rows:#?}");
        let shown = frame
            .live
            .iter()
            .any(|line| focused(line) && line.plain().starts_with(&format!("› {selected}")));
        assert_eq!(shown, height >= 3, "{height}: {rows:#?}");
        assert_eq!(state.editor.text, "/");
    }
}

#[test]
fn narrow_command_field_keeps_its_first_span_and_caret_at_start() {
    let mut state = State::default();
    state.editor.insert("/");
    state.editor.cursor = 0;
    state.suggestions.refresh(&state.editor.text);
    for width in 1..=8 {
        state.width = width;
        state.height = 24;
        let frame = frame(&state, "fixture/model", "fixture directory");
        let (row, column) = frame.cursor.unwrap();
        assert!(column < width, "caret must fit width {width}");
        assert!(
            frame.live[row].plain().contains('/'),
            "input lost at width {width}"
        );
        assert!(frame.live[row].paint(width).contains(&CURSOR.to_string()));
        assert_eq!(state.editor.text, "/");
        assert_eq!(state.editor.cursor, 0);
    }
}

#[test]
fn single_line_previews_keep_styles_and_mark_hidden_columns_without_emitting_newlines() {
    let mut line = Line::new("● ", super::super::theme::GOOD);
    line.push("long command\ncontinued🙂", super::super::theme::ACCENT);
    line.background = Some(USER_SURFACE);
    let short = line.shortened(12);
    assert!(short.plain().ends_with('…'));
    assert!(!short.plain().contains('\n'));
    assert_eq!(short.background, Some(USER_SURFACE));
    assert_eq!(short.spans[0].style, super::super::theme::GOOD);
    assert!(line.plain().contains('\n'));
    assert!(!line.shortened(80).plain().contains('\n'));
}
