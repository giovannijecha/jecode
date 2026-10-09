use super::composer_frame as frame;
use super::*;
use crate::tui::state::Kind;
use crate::{
    effort::Effort,
    events::Event,
    tui::{
        activity::Activity,
        selector::Selector,
        theme::{ACCENT, CURSOR, MUTED, PROMPT, SELECTED_BACKGROUND, USER_BACKGROUND},
    },
};

#[test]
fn idle_composer_has_four_rows_a_caret_over_placeholder_and_one_gray_footer() {
    let state = State::default();
    let frame = frame(&state, "provider/model", r"C:\project");
    assert_eq!(frame.live.len(), 4);
    assert_eq!(frame.live[0].plain(), "─".repeat(state.width));
    assert_eq!(frame.live[0].spans[0].style, ACCENT);
    assert_eq!(frame.live[2].spans[0].style, ACCENT);
    assert_eq!(
        frame.live[1]
            .spans
            .iter()
            .find(|span| span.style == CURSOR)
            .unwrap()
            .text,
        "A"
    );
    let footer = frame.live[3].plain();
    assert!(footer.starts_with(r"C:\project"));
    assert!(footer.ends_with("provider/model · default"));
    assert!(frame.live[3].spans.iter().all(|span| span.style == MUTED));
}

#[test]
fn long_drafts_show_five_contiguous_rows_and_a_position_in_the_top_border() {
    let mut state = State::default();
    state
        .editor
        .insert("line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7");
    let at_end = frame(&state, "model", "directory");
    let rows: Vec<_> = at_end.live.iter().map(Line::plain).collect();
    assert_eq!(
        &rows[1..6],
        ["› line 3", "  line 4", "  line 5", "  line 6", "  line 7 "]
    );
    assert!(rows[0].contains("3–7 / 7"));
    assert_eq!(text::cells(&rows[0]), state.width);
    assert!(
        at_end.live[0]
            .spans
            .iter()
            .any(|span| span.style == MUTED && span.text.contains("3–7 / 7"))
    );
    assert_eq!(at_end.live.len(), 8);
    state.editor.cursor = "line 1\nline 2\nline 3\nli".len();
    let middle = frame(&state, "model", "directory");
    let rows: Vec<_> = middle.live.iter().map(Line::plain).collect();
    assert!(rows[0].contains("2–6 / 7"));
    assert_eq!(
        &rows[1..6],
        ["› line 2", "  line 3", "  line 4", "  line 5", "  line 6"]
    );
    assert_eq!(middle.live.len(), at_end.live.len());
    assert_eq!(
        middle.live[middle.cursor.unwrap().0]
            .spans
            .iter()
            .find(|span| span.style == CURSOR)
            .unwrap()
            .text,
        "n"
    );
    state.editor.replace("abcdef".repeat(7));
    state.width = 10;
    let wrapped = frame(&state, "model", "dir");
    assert_eq!(wrapped.live.len(), 8);
    assert!(
        !wrapped
            .live
            .iter()
            .any(|row| row.plain().contains("more lines"))
    );
    assert_eq!(state.editor.text, "abcdef".repeat(7));
}

#[test]
fn active_borders_are_gray_and_queued_messages_precede_notices_and_activity() {
    let mut state = State {
        activity: Some(Activity::new()),
        ..State::default()
    };
    state.event(Event::Reasoning);
    state.queue.push("one\ntwo").unwrap();
    state.notice = Some(crate::tui::Feedback::result(Kind::Warning, "queue notice"));
    let frame = frame(&state, "model", "dir");
    assert!(frame.live[0].plain().starts_with("› one…"));
    assert!(frame.live[0].plain().ends_with("queued"));
    assert!(frame.live[1].plain().contains("queue notice"));
    assert!(frame.live[2].plain().contains("Thinking"));
    let borders: Vec<_> = frame
        .live
        .iter()
        .filter(|line| line.plain().starts_with('─'))
        .collect();
    assert_eq!(borders.len(), 2);
    assert!(borders.iter().all(|line| line.spans[0].style == MUTED));
    assert!(
        frame.live[frame.cursor.unwrap().0]
            .spans
            .iter()
            .any(|span| span.style == CURSOR)
    );
}

#[test]
fn slash_commands_replace_the_composer_with_one_panel_and_a_distinct_selection() {
    let mut state = State::default();
    state.editor.insert("/mo");
    state.suggestions.refresh(&state.editor.text);
    let frame = frame(&state, "model", "directory");
    assert!(frame.live[3].plain().contains("/model"));
    assert_eq!(frame.live[3].background, Some(SELECTED_BACKGROUND));
    assert!(
        frame.live[3]
            .paint(state.width)
            .contains(SELECTED_BACKGROUND)
    );
    assert!(
        frame
            .live
            .iter()
            .all(|line| matches!(line.background, Some(USER_BACKGROUND | SELECTED_BACKGROUND)))
    );
    assert!(!frame.live.iter().any(|line| line.plain().contains('─')));
    assert!(
        frame.live[2]
            .spans
            .iter()
            .any(|span| span.style == PROMPT && span.text.contains("/mo"))
    );
    assert!(
        frame.live[3]
            .spans
            .iter()
            .any(|span| span.style == ACCENT && span.text.contains("/mo"))
    );
    assert!(
        !frame
            .live
            .iter()
            .any(|line| line.plain().contains("model · default"))
    );
    state.editor.replace("/unknown".into());
    state.suggestions.refresh(&state.editor.text);
    assert_eq!(
        super::composer_frame(&state, "model", "dir").live[3]
            .plain()
            .trim(),
        "No matching command"
    );
}

#[test]
fn selectors_replace_all_composer_rows_but_keep_the_draft_and_mask_keys() {
    let mut state = State::default();
    state.editor.insert("private draft");
    state.selector = Some(Selector::settings("provider/model", Effort::High));
    let selection = frame(&state, "model", "directory");
    assert!(
        selection
            .live
            .iter()
            .any(|row| row.plain().contains("Settings"))
    );
    assert!(
        !selection
            .live
            .iter()
            .any(|row| row.plain().contains("private draft"))
    );
    state.selector = Some(Selector::key());
    state
        .selector
        .as_mut()
        .unwrap()
        .editor
        .insert("isolated-fixture-key");
    let key = frame(&state, "model", "directory");
    assert!(
        !key.live
            .iter()
            .any(|row| row.plain().contains("isolated-fixture-key"))
    );
    assert!(key.live.iter().any(|row| row.plain().contains("********")));
    assert_eq!(state.editor.text, "private draft");
}
