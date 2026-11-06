use super::*;

#[test]
fn tiny_menu_and_command_panels_fit_preserve_their_editors_and_restore_after_resize() {
    let mut key = Selector::key("openrouter");
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
            Default::default(),
            &crate::backend::openrouter::Routing::Auto,
            Default::default(),
            Default::default(),
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
                    menu.searchable || matches!(menu.purpose, crate::tui::selector::Purpose::Key(_))
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
    let mut key = Selector::key("openrouter");
    key.editor.insert("fixture-secret");
    state.width = 2;
    state.selector = Some(key);
    let frame = frame(&state, "fixture/model", "directory");
    let (row, _) = frame.cursor.unwrap();
    // The dots wrap under the field; a full last row puts the caret on the next one.
    let dots = frame
        .live
        .iter()
        .rposition(|line| line.plain().contains('•'))
        .unwrap();
    assert!(row == dots || row == dots + 1);
    assert!(
        !frame
            .live
            .iter()
            .any(|line| line.plain().contains("fixture-secret"))
    );
}

#[test]
fn a_long_value_wraps_under_itself_and_a_long_label_stacks_above_it() {
    let mut key = Selector::key("openrouter");
    key.editor.insert(&"k".repeat(200));
    let mut state = State {
        width: 80,
        height: 40,
        selector: Some(key),
        ..State::default()
    };
    let frame = frame(&state, "fixture/model", "directory");
    let rows: Vec<String> = frame.live.iter().map(Line::plain).collect();
    let first = rows
        .iter()
        .position(|row| row.starts_with("  Paste your key  •"))
        .unwrap();
    // The value ends two cells from the edge and continues under itself.
    let width = 80 - "  Paste your key  ".len() - 2;
    assert_eq!(rows[first].trim_end().len(), 18 + "•".len() * width);
    for row in &rows[first + 1..first + 3] {
        assert!(
            row.starts_with(&format!("{}•", " ".repeat(18))),
            "{rows:#?}"
        );
    }
    let (row, column) = frame.cursor.unwrap();
    assert_eq!(row, first + 3, "{rows:#?}");
    assert_eq!(column, 18 + 200 - 2 * width - width);
    assert!(frame.live[row].paint(80).contains(&CURSOR.to_string()));
    // No room beside the label: it takes its own row, and the value has the cells it needs.
    state.width = 30;
    let frame = super::frame(&state, "fixture/model", "directory");
    let rows: Vec<String> = frame.live.iter().map(Line::plain).collect();
    let label = rows
        .iter()
        .position(|row| row.trim() == "Paste your key")
        .unwrap();
    assert!(rows[label + 1].starts_with("  •"), "{rows:#?}");
}
