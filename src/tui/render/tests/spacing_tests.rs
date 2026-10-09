use super::*;

#[test]
fn stream_updates_and_finalization_keep_one_block_and_one_gap_without_replaying_history() {
    let mut state = State::default();
    state.message(Kind::User, "question");
    let mut screen = Screen::new(state.width, state.height);
    let mut renderer = Renderer::new();
    update(&mut renderer, &mut screen, &mut state);
    state.event(Event::Streaming {
        text: "A\n\n".into(),
    });
    update(&mut renderer, &mut screen, &mut state);
    let short = screen.visible();
    let rows = short.lines().collect::<Vec<_>>();
    let answer = rows.iter().position(|row| row.trim_end() == "A").unwrap();
    assert!(rows[answer + 1].trim().is_empty());
    let border = rows.iter().position(|row| row.starts_with('─')).unwrap();
    assert!(
        rows[answer + 1..border]
            .iter()
            .all(|row| row.trim().is_empty())
    );
    let text = "A\n\nB\n \n";
    state.event(Event::Streaming { text: text.into() });
    update(&mut renderer, &mut screen, &mut state);
    let before = screen.visible();
    state.event(Event::Message { text: text.into() });
    update(&mut renderer, &mut screen, &mut state);
    assert_eq!(screen.visible(), before);
    let rows = before.lines().collect::<Vec<_>>();
    assert_eq!(rows[answer + 1].trim(), "");
    assert_eq!(rows[answer + 2].trim_end(), "B");
    assert_eq!(rows[answer + 3].trim(), "");
    let border = rows.iter().position(|row| row.starts_with('─')).unwrap();
    assert!(
        rows[answer + 3..border]
            .iter()
            .all(|row| row.trim().is_empty())
    );
    state.editor.insert("next");
    let edited = update(&mut renderer, &mut screen, &mut state);
    assert!(!edited.contains("question"));
    assert!(!edited.contains("\x1b[3J"));
    assert!(screen.history.is_empty());
}

#[test]
fn a_blank_model_request_does_not_close_the_native_tool_tree_or_insert_a_separator() {
    let mut state = State::default();
    let mut screen = Screen::new(state.width, state.height);
    let mut renderer = Renderer::new();
    start(&mut state, "first");
    finish(&mut state, "first", "first output\n");
    update(&mut renderer, &mut screen, &mut state);
    state.event(Event::Streaming {
        text: " \n\t".into(),
    });
    assert!(update(&mut renderer, &mut screen, &mut state).is_empty());
    state.event(Event::Message {
        text: " \n\t".into(),
    });
    assert!(update(&mut renderer, &mut screen, &mut state).is_empty());
    start(&mut state, "second");
    finish(&mut state, "second", "second output\n");
    state.close_tools();
    state.select_tool(Some(0));
    state.toggle_tool();
    state.select_tool(None);
    update(&mut renderer, &mut screen, &mut state);
    let visible = screen.visible();
    let rows = visible.lines().collect::<Vec<_>>();
    let first = rows
        .iter()
        .position(|row| row.contains("first output"))
        .unwrap();
    assert!(rows[first + 1].starts_with("└─ bash  second"));
    assert_eq!(rows[first + 3].trim(), "");
    let border = rows.iter().position(|row| row.starts_with('─')).unwrap();
    assert!(
        rows[first + 3..border]
            .iter()
            .all(|row| row.trim().is_empty())
    );
    assert_eq!(visible.matches("first output").count(), 1);
}
