use super::composer_frame as frame;
use super::*;
use crate::tui::state::Kind;
use crate::{
    effort::Effort,
    events::Event,
    tui::{
        activity::Activity,
        selector::Selector,
        theme::{
            ACCENT, CURSOR, DIM, FAINT, MUTED, PILL_SURFACE, SELECTED_SURFACE, STRONG,
            USER_SURFACE, WARNING,
        },
    },
};

#[test]
fn turn_phases_and_waiting_for_an_edit_label_the_status_row() {
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
                arguments: crate::json::Value::object([(
                    "path",
                    crate::json::Value::string("src/auth.rs"),
                )]),
            },
            "Reading auth.rs",
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
        let status = frame(&state, "model", "dir").live[0].plain();
        assert!(status.starts_with(&format!(" >_ {label} ")), "{status}");
        assert!(status.contains("Esc interrupt · 0."), "{status}");
    }
    state.queue.push("a draft").unwrap();
    let slot = state.queue.slot(0).unwrap();
    state.queue.begin_edit(slot, &mut state.editor).unwrap();
    state.event(Event::InputHeld);
    let live = frame(&state, "model", "dir").live;
    let status = live[0].plain();
    assert!(status.contains("Waiting for your edit"), "{status}");
    // The draft edit holds Esc, so the row does not offer it.
    assert!(!status.contains("Esc"), "{status}");
    assert!(live[1].plain().starts_with("─ Editing draft 1/1 ─"));
    state.activity.as_mut().unwrap().stopping = true;
    state.event(Event::Recovering {
        attempt: 1,
        delay: std::time::Duration::from_secs(1),
        error: "fixture".into(),
        reason: None,
    });
    let status = frame(&state, "model", "dir").live[0].plain();
    assert!(
        status.contains("Stopping") && !status.contains("Retrying"),
        "{status}"
    );
}

#[test]
fn the_idle_dock_has_an_empty_status_row_accent_rules_and_a_dim_path() {
    let state = State::default();
    let frame = frame(&state, "provider/model", r"C:\project");
    assert_eq!(frame.live.len(), 5);
    assert_eq!(frame.live[0].plain(), "");
    assert_eq!(frame.live[1].plain(), "─".repeat(state.width));
    assert_eq!(frame.live[1].spans[0].style, ACCENT);
    assert_eq!(frame.live[3].spans[0].style, ACCENT);
    let input = &frame.live[2];
    assert_eq!(input.spans[0].text, "› ");
    assert_eq!(input.spans[0].style, ACCENT.bold());
    assert_eq!(
        input
            .spans
            .iter()
            .find(|span| span.style == CURSOR)
            .unwrap()
            .text,
        "A"
    );
    assert_eq!(frame.cursor, Some((2, 2)));
    let footer = &frame.live[4];
    assert!(footer.plain().starts_with(r"C:\project"));
    // The footer alone names the model, with its provider while it fits.
    assert!(footer.plain().ends_with(" provider/model · default "));
    assert_eq!(footer.spans[0].style, DIM);
    let chip = footer.spans.last().unwrap();
    assert_eq!((chip.style, chip.background), (MUTED, Some(PILL_SURFACE)));
}

#[test]
fn long_drafts_show_a_third_of_the_screen_centred_on_the_caret() {
    let mut state = State::default();
    let lines: Vec<_> = (1..=12).map(|line| format!("line {line}")).collect();
    state.editor.insert(&lines.join("\n"));
    let at_end = frame(&state, "model", "directory");
    let rows: Vec<_> = at_end.live.iter().map(Line::plain).collect();
    // Twenty-four rows give a window of eight.
    assert_eq!(rows[2], "› line 5");
    assert_eq!(rows[9], "  line 12 ");
    assert!(rows[1].ends_with(" 5–12 / 12 ─"), "{}", rows[1]);
    assert_eq!(text::cells(&rows[1]), state.width);
    assert!(
        at_end.live[1]
            .spans
            .iter()
            .any(|span| span.style == MUTED && span.text == "5–12 / 12")
    );
    assert_eq!(at_end.live.len(), 12);
    state.editor.cursor = lines[..5].join("\n").len() + 3;
    let middle = frame(&state, "model", "directory");
    let rows: Vec<_> = middle.live.iter().map(Line::plain).collect();
    assert!(rows[1].contains(" 2–9 / 12 "), "{}", rows[1]);
    assert_eq!(rows[2], "› line 2");
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
    // Short screens keep five rows; tall ones stop at ten.
    for (height, window) in [(12, 5), (60, 10)] {
        state.height = height;
        let rows = frame(&state, "model", "directory").live.len();
        assert_eq!(rows, window + 4, "{height}");
    }
    state.height = 24;
    state.editor.replace("abcdef".repeat(7));
    state.width = 10;
    let wrapped = frame(&state, "model", "dir");
    assert!(wrapped.live.len() <= 12);
    assert_eq!(state.editor.text, "abcdef".repeat(7));
}

#[test]
fn working_rules_turn_faint_and_queued_drafts_precede_the_status_row() {
    let mut state = State {
        activity: Some(Activity::new()),
        ..State::default()
    };
    state.event(Event::Reasoning);
    state.queue.push("one\ntwo").unwrap();
    state.queue.push("paused").unwrap();
    state.queue.pause();
    state.notice = Some(crate::tui::Feedback::result(Kind::Warning, "queue notice"));
    let frame = frame(&state, "model", "dir");
    let queued = &frame.live[0];
    assert!(queued.plain().starts_with("› one…"));
    assert!(queued.plain().ends_with("paused"));
    // Labels end two cells from the edge.
    assert_eq!(text::cells(&queued.plain()), state.width - 2);
    assert_eq!(queued.spans.last().unwrap().style, WARNING);
    assert!(frame.live[1].plain().ends_with("paused"));
    assert!(frame.live[2].plain().starts_with(" ▲  queue notice "));
    assert!(frame.live[3].plain().starts_with("─ Drafts 2 · Alt+↑ ─"));
    let borders: Vec<_> = frame
        .live
        .iter()
        .filter(|line| line.plain().starts_with('─'))
        .collect();
    assert_eq!(borders.len(), 2);
    assert!(borders.iter().all(|line| line.spans[0].style == FAINT));
    assert!(
        frame.live[frame.cursor.unwrap().0]
            .spans
            .iter()
            .any(|span| span.style == CURSOR)
    );
}

#[test]
fn queued_drafts_name_their_state_in_dim_and_their_range_when_some_are_hidden() {
    let mut state = State {
        height: 8,
        ..State::default()
    };
    for index in 0..3 {
        state.queue.push(format!("draft {index}")).unwrap();
    }
    let rows: Vec<_> = frame(&state, "model", "dir").live;
    // Eight rows leave three for drafts beside the dock's five.
    assert_eq!(rows.len(), 8);
    assert!(rows[0].plain().ends_with(" queued"));
    assert_eq!(rows[0].spans.last().unwrap().style, DIM);
    state.height = 7;
    let rows = frame(&state, "model", "dir").live;
    assert!(
        rows[0].plain().ends_with("queued 1–2 / 3"),
        "{}",
        rows[0].plain()
    );
}

#[test]
fn slash_commands_open_one_panel_below_the_draft_with_a_distinct_selection() {
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
    assert!(row("─ Commands").plain().ends_with(" 2 matches ─"));
    // The draft stays in the composer as the search field and keeps the caret.
    let (caret, column) = frame.cursor.unwrap();
    let field = &frame.live[caret];
    assert_eq!(field.plain().trim_end(), "› /mo");
    assert_eq!(column, 5);
    assert!(
        field
            .spans
            .iter()
            .any(|span| span.style == STRONG && span.text.contains("/mo"))
    );
    assert!(field.spans.iter().any(|span| span.style == CURSOR));
    assert!(frame.live[caret - 1].plain().chars().all(|c| c == '─'));
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
        .filter(|span| span.style == ACCENT.bold())
        .map(|span| span.text.as_str())
        .collect();
    assert!(matched.contains("/mo"), "{matched}");
    // The status row stays above the composer; the panel sits on the
    // terminal surface below the composer's bottom rule.
    assert_eq!(frame.live[0].plain(), "");
    assert!(
        frame
            .live
            .iter()
            .all(|line| line.background != Some(USER_SURFACE))
    );
    let title = frame
        .live
        .iter()
        .position(|line| line.plain().starts_with("─ Commands"))
        .unwrap();
    assert_eq!(title, caret + 1);
    // Each command row carries its description in the fact column.
    assert!(
        selected
            .plain()
            .contains("Choose the model for this conversation")
    );
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
fn selectors_keep_the_draft_without_its_caret_and_mask_keys() {
    let mut state = State::default();
    state.editor.insert("private draft");
    state.selector = Some(Selector::settings(
        "provider/model",
        Effort::High,
        80,
        &crate::backend::openrouter::Routing::Auto,
        Default::default(),
        std::path::Path::new("fixture/config.json"),
    ));
    let selection = frame(&state, "model", "directory");
    assert!(
        selection
            .live
            .iter()
            .any(|row| row.plain().starts_with("─ Settings · saved defaults"))
    );
    // The composer shows the draft, without a caret, above the panel.
    let draft: Vec<_> = selection
        .live
        .iter()
        .filter(|row| row.plain().contains("private draft"))
        .collect();
    assert_eq!(draft.len(), 1);
    assert_eq!(draft[0].plain().trim_end(), "› private draft");
    assert!(!draft[0].spans.iter().any(|span| span.style == CURSOR));
    assert_eq!(selection.cursor, None);
    state.selector = Some(Selector::key());
    // The rule names the empty field once; the field holds only the caret.
    let empty = frame(&state, "model", "directory");
    let named = empty
        .live
        .iter()
        .filter(|row| row.plain().contains("Paste your key"))
        .count();
    assert_eq!(named, 1);
    let (row, _) = empty.cursor.unwrap();
    assert_eq!(empty.live[row].plain().trim_end(), "›");
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
    // The key replaces the draft in the composer, one bullet per character,
    // with the caret after it.
    let (row, column) = key.cursor.unwrap();
    assert_eq!(
        key.live[row].plain().trim_end(),
        format!("› {}", "•".repeat(20))
    );
    assert_eq!(column, 22);
    assert!(key.live[row - 1].plain().starts_with("─ Paste your key ─"));
    assert!(
        !key.live
            .iter()
            .any(|row| row.plain().contains("private draft"))
    );
    assert_eq!(state.editor.text, "private draft");
}

#[test]
fn draft_contexts_replace_the_footer_with_key_hints() {
    let mut state = State {
        activity: Some(Activity::new()),
        ..State::default()
    };
    state.queue.push("queued fix").unwrap();
    state.queue.push("later fix").unwrap();
    let slot = state.queue.slot(0).unwrap();
    state.queue.begin_edit(slot, &mut state.editor).unwrap();
    let frame_rows = frame(&state, "model", "dir").live;
    let rows: Vec<_> = frame_rows.iter().map(Line::plain).collect();
    // The later message, then the dock: the edited message leaves the queue
    // previews while it is open, and its keys take the footer's row.
    assert_eq!(rows.len(), 6, "{rows:#?}");
    assert!(rows[0].starts_with("› later fix") && rows[0].ends_with("queued"));
    assert!(rows[1].starts_with(" >_ Working ") && !rows[1].contains("Esc"));
    assert!(rows[2].starts_with("─ Editing draft 1/2 ─"));
    assert!(rows[3].starts_with("› queued fix"));
    assert_eq!(rows[5], "   Enter  save    Esc  cancel");
    let keycap = frame_rows[5]
        .spans
        .iter()
        .find(|span| span.text == " Enter ")
        .unwrap();
    assert_eq!(
        (keycap.style, keycap.background),
        (STRONG, Some(SELECTED_SURFACE))
    );

    let mut state = State::default();
    state
        .history
        .record(crate::attachments::Prompt::plain("earlier"));
    state.history.navigate(&mut state.editor, true);
    let rows: Vec<_> = frame(&state, "model", "dir")
        .live
        .iter()
        .map(Line::plain)
        .collect();
    assert!(rows[1].starts_with("─ Prompt history ─"));
    assert!(rows[4].contains(" Enter  send") && rows[4].contains(" Esc  back"));
}

#[test]
fn a_retry_counts_down_in_warning_with_its_attempt_and_cause() {
    let mut state = State {
        activity: Some(Activity::new()),
        width: 100,
        ..State::default()
    };
    state.event(Event::Recovering {
        attempt: 2,
        delay: std::time::Duration::from_millis(3500),
        error: "HTTP 429".into(),
        reason: Some(crate::backend::FailureReason::RateLimited),
    });
    let frame = frame(&state, "model", "dir");
    assert_eq!(frame.live.len(), 5);
    let status = &frame.live[0];
    assert!(
        status
            .plain()
            .starts_with(" >_ Retrying in 4s   Esc interrupt · 0."),
        "{}",
        status.plain()
    );
    assert!(
        status
            .plain()
            .ends_with(" · attempt 2 · Rate limited · HTTP 429"),
        "{}",
        status.plain()
    );
    assert_eq!(status::style_of(status, "Retrying"), Some(WARNING));
    assert_eq!(status::style_of(status, ">"), Some(WARNING.bold()));
    assert_eq!(frame.live[1].spans[0].style, FAINT);
}

#[test]
fn the_status_row_drops_facts_from_the_right_before_the_pill() {
    let mut activity = Activity::with_elapsed(std::time::Duration::from_secs(70 * 60 + 52));
    activity.generated = 1234;
    let mut state = State {
        activity: Some(activity),
        ..State::default()
    };
    state.event(Event::Reasoning);
    let status = |state: &mut State, columns| {
        state.width = columns;
        let row = frame(state, "model", "dir").live[0].plain();
        assert!(text::cells(&row) <= columns);
        row
    };
    assert_eq!(
        status(&mut state, 80),
        " >_ Thinking   Esc interrupt · 1h 10m · ↓ 1.2k"
    );
    assert_eq!(
        status(&mut state, 40),
        " >_ Thinking   Esc interrupt · 1h 10m"
    );
    assert_eq!(status(&mut state, 30), " >_ Thinking   Esc interrupt");
    assert_eq!(status(&mut state, 16), " >_ Thinking ");
    state.activity.as_mut().unwrap().stopping = true;
    assert_eq!(status(&mut state, 80), " >_ Stopping   1h 10m");
}
