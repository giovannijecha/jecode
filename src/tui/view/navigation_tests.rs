use super::*;
use crate::{
    effort::Effort,
    openrouter::Model,
    tui::{
        selector::Selector,
        theme::{CURSOR, MUTED},
    },
};

fn draft() -> State {
    let mut state = State::default();
    state
        .editor
        .insert("one\ntwo\nthree\nfour\nfive\nsix\nseven");
    state
}

#[test]
fn composer_position_tracks_the_caret_and_wrapping_without_changing_the_draft() {
    let mut state = draft();
    let original = state.editor.text.clone();
    for (cursor, range, first) in [
        (0, "1–5 / 7", "one"),
        ("one\ntwo\nthree\nfo".len(), "2–6 / 7", "two"),
        (original.len(), "3–7 / 7", "three"),
    ] {
        state.editor.cursor = cursor;
        let frame = composer_frame(&state, "model", "directory");
        assert!(frame.live[0].plain().contains(range));
        assert_eq!(frame.live[1].plain().trim_end(), format!("› {first}"));
        let (row, column) = frame.cursor.unwrap();
        assert!((1..=5).contains(&row));
        assert!(column < state.width);
        assert!(frame.live[row].paint(state.width).contains(CURSOR));
        assert_eq!(state.editor.text, original);
    }
    state.editor.replace("x".repeat(14));
    state.width = 10;
    assert_eq!(composer_frame(&state, "model", "dir").live.len(), 6);
    state.width = 80;
    let wide = composer_frame(&state, "model", "dir");
    assert_eq!(wide.live[0].plain(), "─".repeat(80));
    assert_eq!(state.editor.text, "x".repeat(14));
}

#[test]
fn short_composers_keep_a_continuous_window_and_an_accurate_range() {
    let mut state = draft();
    state.height = 6;
    let frame = composer_frame(&state, "model", "directory");
    assert_eq!(frame.live.len(), 6);
    assert!(frame.live[0].plain().contains("5–7 / 7"));
    assert_eq!(frame.live[1].plain(), "› five");
    assert_eq!(frame.live[2].plain(), "  six");
    assert_eq!(frame.live[3].plain(), "  seven ");
}

#[test]
fn selector_positions_follow_navigation_filtering_and_empty_results_in_one_heading() {
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
    for (selected, range) in [(0, "1–8 / 12"), (9, "3–10 / 12"), (11, "5–12 / 12")] {
        state.selector.as_mut().unwrap().selected = selected;
        let frame = composer_frame(&state, "model", "directory");
        assert!(frame.live[1].plain().contains(range));
        assert_eq!(
            frame
                .live
                .iter()
                .filter(|row| row.plain().contains("fixture/model-"))
                .count(),
            8
        );
        assert!(
            frame.live[1]
                .spans
                .iter()
                .any(|span| span.style == MUTED && span.text.contains(range))
        );
        assert!(
            !frame
                .live
                .iter()
                .any(|row| row.plain().contains("earlier") || row.plain().contains("more"))
        );
    }
    let menu = state.selector.as_mut().unwrap();
    menu.editor.insert("model-10");
    menu.refresh();
    let filtered = composer_frame(&state, "model", "directory");
    assert!(filtered.live[1].plain().contains("1 match"));
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
    assert!(!empty.live[1].plain().contains('/'));
}

#[test]
fn queue_overflow_uses_its_first_preview_without_displacing_the_next_message() {
    let mut state = State {
        height: 11,
        ..State::default()
    };
    for index in 0..8 {
        state.queue.push(format!("queued request {index}")).unwrap();
    }
    let frame = composer_frame(&state, "model", "directory");
    assert_eq!(frame.live.len(), 11);
    assert!(frame.live[0].plain().starts_with("› queued request 0"));
    assert!(frame.live[0].plain().ends_with("queued 1–7 / 8"));
    assert!(frame.live[6].plain().starts_with("› queued request 6"));
    assert!(
        !frame
            .live
            .iter()
            .any(|row| row.plain().contains("more queued"))
    );
    state.height = 5;
    let short = composer_frame(&state, "model", "directory");
    assert!(short.live[0].plain().starts_with("› queued request 0"));
    assert!(short.live[0].plain().ends_with("queued 1–1 / 8"));
    for width in [12, 20] {
        state.width = width;
        let narrow = composer_frame(&state, "model", "directory");
        assert!(narrow.live[0].plain().starts_with("› queued"));
        assert!(!narrow.live[0].plain().contains('/'));
        assert!(text::cells(&narrow.live[0].plain()) <= width);
    }
    assert_eq!(state.queue.messages.len(), 8);
}

#[test]
fn command_panels_show_suggestion_or_multiline_position_in_the_title() {
    let mut state = State::default();
    state.editor.insert("/");
    state.suggestions.refresh(&state.editor.text);
    let commands = composer_frame(&state, "model", "directory");
    assert!(commands.live[1].plain().contains("1–6 / 11"));
    state
        .editor
        .replace("/tmp clean\none\ntwo\nthree\nfour\nfive\nsix".into());
    state.suggestions.refresh(&state.editor.text);
    let multiline = composer_frame(&state, "model", "directory");
    assert!(multiline.live[1].plain().contains("3–7 / 7"));
    assert_eq!(multiline.live[2].plain(), "  › two");
    assert!(
        !multiline
            .live
            .iter()
            .any(|row| row.plain().contains("more lines"))
    );
}
