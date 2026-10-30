use super::*;

#[test]
fn active_command_controls_describe_execution_queueing_and_closing() {
    let mut state = State {
        activity: Some(crate::tui::activity::Activity::new()),
        ..State::default()
    };
    for (command, action) in [
        ("/", "queue"),
        ("/copy", "run"),
        ("/COPY latest", "run"),
        ("/drafts", "run"),
        ("/DRAFTS", "run"),
        ("/tmp clean", "queue"),
    ] {
        state.editor.replace(command.into());
        state.suggestions.refresh(command);
        let frame = frame(&state, "fixture/model", "directory");
        assert!(
            frame
                .live
                .iter()
                .any(|line| line.plain().contains(&format!(" Enter  {action}")))
        );
        assert!(
            frame
                .live
                .iter()
                .any(|line| line.plain().contains(" Esc  close"))
        );
        assert!(
            !frame
                .live
                .iter()
                .any(|line| line.plain().contains("Esc stops"))
        );
    }
}

#[test]
fn narrow_panels_keep_the_keyboard_controls_readable_instead_of_cutting_the_footer() {
    let mut state = State {
        width: 40,
        selector: Some(Selector::sessions(&sessions(), "1234-56-0")),
        ..State::default()
    };
    let resume = frame(&state, "fixture/model", "directory");
    // Keys that do not fit wrap onto another row instead of being cut.
    let controls: String = resume
        .live
        .iter()
        .map(Line::plain)
        .filter(|line| line.contains("Ctrl+D") || line.contains(" Esc "))
        .collect();
    assert!(controls.contains(" Enter  resume"), "{controls}");
    assert!(controls.contains(" Ctrl+D  delete"), "{controls}");
    assert!(controls.contains(" Esc  close"), "{controls}");
    assert!(!controls.contains('…'));
    // The key form's keys end the dock whole, wrapping when they must.
    state.selector = Some(Selector::key());
    for width in [30, 40, 60] {
        state.width = width;
        let key = frame(&state, "fixture/model", "directory");
        let rows: Vec<_> = key.live.iter().map(Line::plain).collect();
        let title = key.cursor.unwrap().0 + 1;
        assert!(rows[title].starts_with("─ Providers"), "{rows:#?}");
        let controls = rows[title + 1..].concat();
        assert!(controls.contains(" Enter  validate and save"), "{rows:#?}");
        assert!(controls.contains(" Esc  back"), "{rows:#?}");
        assert!(!controls.contains('…'), "{rows:#?}");
    }
    // With room, the bottom rule names where the key is stored.
    assert!(
        frame(&state, "fixture/model", "directory")
            .live
            .iter()
            .any(|line| line.plain().ends_with(" ~/.jecode/config.json ─"))
    );
}

#[test]
fn narrow_key_rows_keep_enter_and_escape_before_movement() {
    let hints = [
        panel::key("Enter", "open"),
        panel::key("↑↓", "move"),
        panel::key("Esc", "close"),
    ];
    for width in [30, 36] {
        let one = panel::row(&hints, width).plain();
        let many = panel::keys(&hints, width, true)
            .into_iter()
            .map(|line| line.plain())
            .collect::<Vec<_>>()
            .join(" ");
        for rendered in [one, many] {
            assert!(
                rendered.contains("Enter") && rendered.contains("Esc"),
                "{rendered}"
            );
            assert!(!rendered.contains("move"), "{rendered}");
        }
    }

    let wide_labels = [
        panel::key("Enter", "approve and remember"),
        panel::key("↑↓", "move"),
        panel::key("Esc", "close"),
    ];
    let one = panel::row(&wide_labels, 48).plain();
    assert!(one.contains("Enter") && one.contains("Esc"), "{one}");
    assert!(!one.contains("move"), "{one}");
}

#[test]
fn narrow_key_rows_keep_their_primary_key_and_never_vanish() {
    let resume = [panel::key("/resume", "all conversations")];
    let opening = [
        panel::key("/", "commands"),
        panel::key("Alt+V", "image"),
        panel::key("Ctrl+O", "expand"),
        panel::key("Ctrl+Q", "quit"),
    ];
    for width in [8, 12, 20, 30] {
        let one = panel::row(&resume, width).plain();
        assert!(one.contains("/res"), "{width}: {one:?}");
        let first = panel::row(&opening, width).plain();
        assert!(first.starts_with(" / "), "{width}: {first:?}");
        assert!(!first.contains("Ctrl+Q"), "{width}: {first:?}");
    }
    let wide = panel::row(&opening, 40).plain();
    assert!(
        wide.contains("commands") && wide.contains("image"),
        "{wide}"
    );
}
