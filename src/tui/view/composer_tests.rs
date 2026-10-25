use super::composer_frame as frame;
use super::*;
use crate::tui::state::Kind;
use crate::{
    effort::Effort,
    events::Event,
    tui::{
        activity::Activity,
        selector::Selector,
        theme::{ACCENT, CODE_SURFACE, CURSOR, MUTED, SELECTED_SURFACE, STRONG, USER_SURFACE},
    },
};

#[test]
fn turn_phases_and_waiting_for_an_edit_are_visible_in_the_rule() {
    let mut state = State {
        activity: Some(Activity::new()),
        ..State::default()
    };
    for (event, label) in [
        (Event::Preparing, "Preparing request"),
        (
            Event::Waiting {
                model: "fixture".into(),
            },
            "Waiting for model",
        ),
        (Event::Reasoning, "Thinking"),
        (Event::Working, "Working"),
        (
            Event::ToolStarted {
                id: "phase".into(),
                name: "read".into(),
                arguments: crate::json::Value::object([]),
            },
            "Working",
        ),
        (
            Event::ToolFinished {
                id: "phase".into(),
                name: "read".into(),
                summary: "read".into(),
                result: crate::json::Value::object([]),
            },
            "Working",
        ),
        (
            Event::Streaming {
                text: "response".into(),
            },
            "Writing the response",
        ),
    ] {
        state.event(event);
        assert!(
            frame(&state, "model", "dir").live[0]
                .plain()
                .contains(label)
        );
    }
    state.queue.push("a draft").unwrap();
    let slot = state.queue.slot(0).unwrap();
    state.queue.begin_edit(slot, &mut state.editor).unwrap();
    state.event(Event::InputHeld);
    let rule = frame(&state, "model", "dir").live[0].plain();
    assert!(
        rule.contains("Waiting for your edit") && rule.contains("Editing draft 1/1"),
        "{rule}"
    );
    state.activity.as_mut().unwrap().stopping = true;
    state.event(Event::Recovering {
        attempt: 1,
        delay: std::time::Duration::from_secs(1),
        error: "fixture".into(),
    });
    let rule = frame(&state, "model", "dir").live[0].plain();
    assert!(
        rule.contains("Stopping") && !rule.contains("Reconnecting"),
        "{rule}"
    );
    assert!(!rule.contains("esc"));
}

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
fn active_borders_are_gray_and_queued_messages_precede_notices_and_the_working_rule() {
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
    assert!(frame.live[2].plain().starts_with("─ ▰▰▱▱▱▱ "));
    assert!(frame.live[2].plain().contains(" · Drafts 1 · Alt+↑ "));
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
    let row = |text: &str| {
        frame
            .live
            .iter()
            .find(|line| line.plain().contains(text))
            .unwrap()
    };
    assert!(row("Commands").plain().contains("1 match"));
    // The draft is the search field, inset on the code surface.
    let field = row(" /mo");
    assert!(
        field
            .spans
            .iter()
            .any(|span| span.style == STRONG && span.text.contains("/mo"))
    );
    assert!(
        field
            .spans
            .iter()
            .any(|span| span.background == Some(CODE_SURFACE))
    );
    let selected = row("› /model");
    assert!(focused(selected));
    assert!(
        selected
            .paint(state.width)
            .contains(&SELECTED_SURFACE.to_string())
    );
    let matched: String = selected
        .spans
        .iter()
        .filter(|span| span.style == ACCENT)
        .map(|span| span.text.as_str())
        .collect();
    assert!(matched.contains("/mo"), "{matched}");
    assert!(
        frame
            .live
            .iter()
            .all(|line| line.background == Some(USER_SURFACE))
    );
    assert!(!frame.live.iter().any(|line| line.plain().starts_with('─')));
    // The detail strip describes the focused command.
    assert!(frame.live.iter().any(|line| {
        line.plain()
            .contains("Choose the model for this conversation")
    }));
    assert!(
        !frame
            .live
            .iter()
            .any(|line| line.plain().contains("model · default"))
    );
    state.editor.replace("/attach".into());
    state.suggestions.refresh(&state.editor.text);
    assert!(
        super::composer_frame(&state, "model", "dir")
            .live
            .iter()
            .any(|line| line.plain().trim() == "No matching command")
    );
}

#[test]
fn selectors_replace_all_composer_rows_but_keep_the_draft_and_mask_keys() {
    let mut state = State::default();
    state.editor.insert("private draft");
    state.selector = Some(Selector::settings(
        "provider/model",
        Effort::High,
        80,
        &crate::backend::openrouter::Routing::Auto,
        std::path::Path::new("fixture/config.json"),
    ));
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

#[test]
fn draft_contexts_add_a_hint_row_and_retain_the_directory_footer() {
    let mut state = State {
        activity: Some(Activity::new()),
        ..State::default()
    };
    state.queue.push("queued fix").unwrap();
    state.queue.push("later fix").unwrap();
    let slot = state.queue.slot(0).unwrap();
    state.queue.begin_edit(slot, &mut state.editor).unwrap();
    let rows: Vec<_> = frame(&state, "model", "dir")
        .live
        .iter()
        .map(Line::plain)
        .collect();
    // The later message, then the composer: the edited message leaves the
    // queue previews while it is open, and its own hint row names Esc.
    assert_eq!(rows.len(), 6, "{rows:#?}");
    assert!(rows[0].starts_with("› later fix") && rows[0].ends_with("queued"));
    assert!(rows[1].starts_with("─ ▰▰▱▱▱▱ 0.") && rows[1].contains(" · Editing draft 1/2 "));
    assert!(!rows[1].contains("esc"));
    assert!(rows[2].starts_with("› queued fix"));
    assert_eq!(rows[4], "Enter save · Esc cancel");
    assert!(rows[5].starts_with("dir"));
    assert!(rows[5].ends_with("model · default"));

    let mut state = State {
        activity: Some(Activity::new()),
        ..State::default()
    };
    state.editor.insert("draft");
    let rows: Vec<_> = frame(&state, "model", "dir")
        .live
        .iter()
        .map(Line::plain)
        .collect();
    assert_eq!(rows.len(), 4, "{rows:#?}");
    assert!(rows[0].ends_with(" esc ─"));
    assert!(rows[1].contains("draft"));
    assert!(rows[3].ends_with("model · default"));
}

#[test]
fn a_reconnect_rides_the_working_rule_and_counts_down_without_a_row_of_its_own() {
    let mut state = State {
        activity: Some(Activity::new()),
        ..State::default()
    };
    state.progress = Some(
        crate::tui::Feedback::progress(Kind::Warning, "Rate limited · HTTP 429 · attempt 2")
            .until(std::time::Instant::now() + std::time::Duration::from_millis(3500)),
    );
    let frame = frame(&state, "model", "dir");
    assert_eq!(frame.live.len(), 4);
    let rule = frame.live[0].plain();
    assert!(rule.starts_with("─ ▰▰▱▱▱▱ 0."), "{rule}");
    assert!(
        rule.contains("Rate limited · HTTP 429 · attempt 2 in 4s"),
        "{rule}"
    );
    assert!(rule.ends_with(" esc ─"));
}

#[test]
fn the_working_rule_drops_detail_before_the_scanner() {
    let mut activity = Activity::new();
    activity.started -= std::time::Duration::from_secs(70 * 60 + 52);
    let mut state = State {
        activity: Some(activity),
        ..State::default()
    };
    state.queue.push("dammi il sito").unwrap();
    let top = |state: &mut State, columns| {
        state.width = columns;
        let frame = frame(state, "model", "dir");
        let rule = frame
            .live
            .iter()
            .map(Line::plain)
            .find(|row| row.starts_with("─ "))
            .unwrap();
        assert_eq!(text::cells(&rule), columns);
        rule
    };
    let wide = top(&mut state, 80);
    assert!(wide.starts_with("─ ") && wide.contains(" 1h 10m · Drafts 1 · Alt+↑ ─"));
    assert!(wide.ends_with("─ esc ─"));
    let narrow = top(&mut state, 36);
    assert!(narrow.contains(" 1h 10m · Drafts 1 ─"), "{narrow}");
    assert!(narrow.ends_with(" esc ─"));
    let tiny = top(&mut state, 16);
    assert!(!tiny.contains("Drafts") && !tiny.contains("esc"), "{tiny}");
    assert!(tiny.contains("▰▰"));
    state.activity.as_mut().unwrap().stopping = true;
    assert!(!top(&mut state, 80).contains("esc"));
}
