use super::*;

#[test]
fn tiny_menu_and_command_panels_fit_preserve_their_editors_and_restore_after_resize() {
    let mut key = Selector::key();
    key.editor.insert("fixture🙂key");
    let mut search = Selector::sessions(&sessions(), "1234-56-0");
    search.editor.insert("request");
    search.refresh();
    for menu in [
        None,
        Some(key),
        Some(search),
        Some(Selector::settings(
            "fixture/model",
            Effort::High,
            80,
            &crate::backend::openrouter::Routing::Auto,
            Default::default(),
            std::path::Path::new("fixture/config.json"),
        )),
    ] {
        let mut state = State {
            selector: menu,
            ..State::default()
        };
        state
            .editor
            .insert("/resume long🙂identifier\ncontinued arguments");
        state.suggestions.refresh(&state.editor.text);
        let draft = state.editor.clone();
        for width in [2, 3, 6, 8, 12, 18, 40, 80] {
            for height in [1, 2, 3, 4, 6, 12, 24] {
                state.width = width;
                state.height = height;
                let frame = frame(&state, "fixture/model", "directory");
                assert!(frame.live.len() <= height);
                assert!(
                    frame.live.iter().all(|line| {
                        text::cells(&line.plain()) <= width && line.background.is_none()
                    }),
                    "{width}x{height}"
                );
                if state.selector.as_ref().is_none_or(|menu| {
                    menu.searchable || matches!(menu.purpose, crate::tui::selector::Purpose::Key)
                }) {
                    let (row, column) = frame.cursor.unwrap();
                    assert!(row < frame.live.len());
                    assert!(column < width, "caret must fit {width}x{height}");
                    assert!(frame.live[row].paint(width).contains(&CURSOR.to_string()));
                }
                assert_eq!(state.editor, draft);
            }
        }
    }
}

#[test]
fn narrow_selector_fields_show_typed_text_when_a_cell_can_hold_it() {
    let mut search = Selector::sessions(&sessions(), "1234-56-0");
    search.editor.insert("x");
    search.refresh();
    let mut state = State {
        selector: Some(search),
        ..State::default()
    };
    for (width, value) in [(1, "x"), (2, "x"), (3, "界")] {
        state.width = width;
        state.selector.as_mut().unwrap().editor.set(value.into());
        let frame = frame(&state, "fixture/model", "directory");
        let (row, column) = frame.cursor.unwrap();
        assert!(column < width);
        assert!(frame.live[row].paint(width).contains(&CURSOR.to_string()));
        if width > 1 {
            assert!(
                frame.live[row].plain().contains(value),
                "{width}: {:?}",
                frame.live[row].plain()
            );
        }
    }
    let mut key = Selector::key();
    key.editor.insert("fixture-secret");
    state.width = 2;
    state.selector = Some(key);
    let frame = frame(&state, "fixture/model", "directory");
    let (row, _) = frame.cursor.unwrap();
    assert!(frame.live[row].plain().contains('•'));
    assert!(!frame.live[row].plain().contains("fixture-secret"));
}
