use super::*;
use crate::{
    backend::Model,
    effort::Effort,
    tui::{selector::Selector, theme::CURSOR},
};

fn draft() -> State {
    let mut state = State::default();
    state
        .editor
        .insert("one\ntwo\nthree\nfour\nfive\nsix\nseven");
    state
}

/// The composer's bottom rule, below the caret's row, carries the panel's
/// title and count.
fn title(frame: &Frame) -> String {
    let (row, _) = frame.cursor.unwrap();
    let rule = frame.live[row + 1].plain();
    assert!(rule.starts_with("─ "), "{rule}");
    rule
}

#[test]
fn composer_position_tracks_the_caret_and_wrapping_without_changing_the_draft() {
    let mut state = draft();
    // Fifteen rows give the draft its smallest window, five rows.
    state.height = 15;
    let original = state.editor.text.clone();
    for (cursor, range, first) in [
        (0, "1–5 / 7", "one"),
        ("one\ntwo\nthree\nfo".len(), "2–6 / 7", "two"),
        (original.len(), "3–7 / 7", "three"),
    ] {
        state.editor.cursor = cursor;
        let frame = composer_frame(&state, "model", "directory");
        assert!(frame.live[1].plain().contains(range));
        assert_eq!(frame.live[2].plain().trim_end(), format!("› {first}"));
        let (row, column) = frame.cursor.unwrap();
        assert!((2..=6).contains(&row));
        assert!(column < state.width);
        assert!(
            frame.live[row]
                .paint(state.width)
                .contains(&CURSOR.to_string())
        );
        assert_eq!(state.editor.text, original);
    }
    state.editor.replace("x".repeat(14));
    state.width = 10;
    // Status, two rules, three wrapped rows with the caret's own, and the footer.
    assert_eq!(composer_frame(&state, "model", "dir").live.len(), 7);
    state.width = 80;
    let wide = composer_frame(&state, "model", "dir");
    assert_eq!(wide.live[1].plain(), "─".repeat(80));
    assert_eq!(state.editor.text, "x".repeat(14));
}

#[test]
fn short_composers_keep_a_continuous_window_and_an_accurate_range() {
    let mut state = draft();
    state.height = 6;
    let frame = composer_frame(&state, "model", "directory");
    // The status row keeps its place, so two draft rows remain.
    assert_eq!(frame.live.len(), 6);
    assert!(frame.live[1].plain().contains("6–7 / 7"));
    assert_eq!(frame.live[2].plain(), "› six");
    assert_eq!(frame.live[3].plain(), "  seven ");
}

#[test]
fn selector_counts_and_scrollbar_follow_navigation_filtering_and_empty_results() {
    let models: Vec<_> = (0..12)
        .map(|index| Model {
            id: format!("fixture/model-{index}"),
            name: format!("Model {index}"),
            prompt_price: None,
            completion_price: None,
            efforts: vec![Effort::Default],
        })
        .collect();
    let mut state = State {
        selector: Some(Selector::models(&models, false, "")),
        ..State::default()
    };
    for selected in [0, 9, 11] {
        state.selector.as_mut().unwrap().selected = selected;
        let frame = composer_frame(&state, "model", "directory");
        assert!(title(&frame).contains("12 models"));
        let focus = format!("› fixture/model-{selected} ");
        assert!(
            frame
                .live
                .iter()
                .any(|row| super::focused(row) && row.plain().contains(&focus))
        );
        // A scrollbar replaces the position range.
        assert!(frame.live.iter().any(|row| row.plain().ends_with('┃')));
        assert!(
            !frame
                .live
                .iter()
                .any(|row| row.plain().contains("earlier") || row.plain().contains(" / 12"))
        );
    }
    let menu = state.selector.as_mut().unwrap();
    menu.editor.insert("model-10");
    menu.refresh();
    let filtered = composer_frame(&state, "model", "directory");
    assert!(title(&filtered).contains("1 of 12 models"));
    assert!(
        filtered
            .live
            .iter()
            .any(|row| row.plain().contains("fixture/model-10"))
    );
    let menu = state.selector.as_mut().unwrap();
    menu.editor.replace("no-such-model".into());
    menu.refresh();
    let empty = composer_frame(&state, "model", "directory");
    assert!(
        empty
            .live
            .iter()
            .any(|row| row.plain().contains("No matching option"))
    );
    assert!(title(&empty).contains("0 of 12 models"));
}

#[test]
fn the_pending_draft_row_fits_short_heights_and_narrow_widths() {
    let mut state = State {
        height: 6,
        ..State::default()
    };
    state.queue.push("queued request").unwrap();
    let frame = composer_frame(&state, "model", "directory");
    assert_eq!(frame.live.len(), 6);
    assert!(frame.live[0].plain().starts_with("› queued request"));
    assert!(frame.live[0].plain().ends_with("queued"));
    for width in [12, 20] {
        state.width = width;
        let narrow = composer_frame(&state, "model", "directory");
        assert!(narrow.live[0].plain().starts_with("› queued"));
        assert!(text::cells(&narrow.live[0].plain()) <= width);
    }
    assert_eq!(state.queue.messages(), ["queued request"]);
}

#[test]
fn command_panels_group_a_bare_slash_and_keep_a_multiline_caret() {
    let mut state = State::default();
    state.editor.insert("/");
    state.suggestions.refresh(&state.editor.text);
    let commands = composer_frame(&state, "model", "directory");
    let count = format!("{} commands", crate::session::commands::COMMANDS.len());
    assert!(title(&commands).contains(&count));
    // A bare slash lists commands under their groups.
    assert!(
        commands
            .live
            .iter()
            .any(|row| row.plain().trim_start().starts_with("Conversation "))
    );
    assert!(commands.live.iter().any(|row| row.plain().ends_with('┃')));
    state.editor.replace(
        "/tmp clean
one
two
three
four
five
six"
        .into(),
    );
    state.suggestions.refresh(&state.editor.text);
    let multiline = composer_frame(&state, "model", "directory");
    let (row, _) = multiline.cursor.unwrap();
    // The caret stays on the draft's last row in the composer.
    assert_eq!(multiline.live[row].plain().trim_end(), "  six");
    assert!(
        multiline.live[row]
            .spans
            .iter()
            .any(|span| span.style == CURSOR)
    );
    assert!(multiline.live[row - 6].plain().starts_with("› /tmp clean"));
    assert!(title(&multiline).starts_with("─ Commands"));
    // The typed command keeps its description below the composer.
    let description = multiline
        .live
        .iter()
        .position(|line| line.plain().contains("temporary"))
        .unwrap();
    assert_eq!(description, row + 2);
}

#[test]
fn single_counts_read_singular_and_narrow_rows_keep_their_marker_before_a_chip() {
    let models = vec![Model {
        id: "fixture/only".into(),
        name: "Only".into(),
        prompt_price: None,
        completion_price: None,
        efforts: vec![Effort::Default],
    }];
    let mut state = State {
        selector: Some(Selector::models(&models, false, "fixture/only")),
        ..State::default()
    };
    let frame = composer_frame(&state, "model", "directory");
    let rows: Vec<_> = frame.live.iter().map(Line::plain).collect();
    assert!(
        rows.iter().any(|row| row.ends_with("1 model ─")),
        "{rows:#?}"
    );
    assert!(
        frame
            .live
            .iter()
            .any(|line| line.plain().contains(" current "))
    );
    state.width = 8;
    let narrow = composer_frame(&state, "model", "directory");
    let focused = narrow.live.iter().find(|line| focused(line)).unwrap();
    assert!(focused.plain().starts_with("› fix"), "{}", focused.plain());
    assert!(!focused.plain().contains("current"));
}

#[test]
fn the_placeholder_shortens_with_an_ellipsis_inside_the_input_width() {
    let state = State {
        width: 12,
        ..State::default()
    };
    let frame = composer_frame(&state, "model", "directory");
    let (row, _) = frame.cursor.unwrap();
    assert_eq!(frame.live[row].plain(), "› Ask anyt…");
    assert!(
        frame
            .live
            .iter()
            .all(|line| text::cells(&line.plain()) <= 12)
    );
}
