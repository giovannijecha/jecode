use super::composer_frame as frame;
use super::*;
use crate::{
    effort::Effort,
    openrouter::Model,
    sessions::Summary,
    tui::{
        selector::Selector,
        state::Kind,
        theme::{CURSOR, SELECTED_BACKGROUND, USER_BACKGROUND},
    },
};

fn model(index: usize) -> Model {
    Model {
        id: format!("fixture/model-{index}"),
        name: format!("Fixture model {index}"),
        prompt_price: None,
        completion_price: None,
        efforts: vec![Effort::Default, Effort::High],
    }
}

fn sessions() -> Vec<Summary> {
    (0..12)
        .map(|index| Summary {
            id: format!("1234-56-{index}"),
            title: format!("Saved request {index}"),
            updated: 0,
            model: "fixture/model".into(),
        })
        .collect()
}

#[test]
fn every_menu_has_the_same_padded_surface_without_composer_borders_or_draft_text() {
    let models: Vec<_> = (0..12).map(model).collect();
    let mut key = Selector::key();
    key.editor.insert("isolated-fixture-key");
    for menu in [
        Selector::settings("fixture/model", Effort::High),
        Selector::models(&models, false, "fixture/model-1"),
        Selector::efforts(model(1), false, Effort::High),
        Selector::sessions(&sessions(), "1234-56-0"),
        Selector::sessions(&[], ""),
        Selector::copy(&[]),
        Selector::loading(),
        key,
    ] {
        let mut state = State {
            selector: Some(menu),
            ..State::default()
        };
        state.editor.insert("kept private draft");
        let draft = state.editor.clone();
        let frame = frame(&state, "fixture/model", "directory");
        assert_eq!(frame.live.first().unwrap().plain(), "");
        assert_eq!(frame.live.last().unwrap().plain(), "");
        assert!(frame.live.iter().all(|line| {
            matches!(line.background, Some(USER_BACKGROUND | SELECTED_BACKGROUND))
                && !line.plain().contains('─')
                && !line.plain().contains("kept private draft")
                && !line.plain().contains("isolated-fixture-key")
        }));
        if let Some((row, column)) = frame.cursor {
            let line = &frame.live[row];
            assert!(line.paint(state.width).contains(CURSOR));
            assert!(column < state.width);
        }
        assert_eq!(state.editor, draft);
    }
}

#[test]
fn slash_arguments_keep_the_panel_and_escape_restores_the_draft_and_caret() {
    let mut state = State::default();
    state.editor.insert("/tmp clean");
    state.suggestions.refresh(&state.editor.text);
    assert!(state.suggestions.panel);
    assert!(!state.suggestions.visible);
    let draft = state.editor.clone();
    let commands = frame(&state, "fixture/model", "directory");
    assert!(
        commands
            .live
            .iter()
            .all(|line| line.background == Some(USER_BACKGROUND))
    );
    assert!(
        commands
            .live
            .iter()
            .any(|line| line.plain().contains("/tmp clean"))
    );
    assert!(
        !commands
            .live
            .iter()
            .any(|line| line.plain().contains("No matching command"))
    );
    state.suggestions.dismiss();
    let normal = frame(&state, "fixture/model", "directory");
    assert!(normal.live.iter().any(|line| line.plain().starts_with('─')));
    assert_eq!(state.editor, draft);
    assert_eq!(normal.cursor, Some((1, text::cells(&draft.text) + 2)));
    state.suggestions.refresh(&state.editor.text);
    assert!(state.suggestions.panel);
}

#[test]
fn searchable_menus_keep_the_title_and_count_separate_from_the_query() {
    let mut menu = Selector::sessions(&sessions(), "1234-56-0");
    menu.editor.insert("Saved request");
    menu.refresh();
    let state = State {
        selector: Some(menu),
        ..State::default()
    };
    let frame = frame(&state, "fixture/model", "directory");
    let title = frame
        .live
        .iter()
        .find(|line| line.plain().contains("Resume · current folder"))
        .unwrap();
    assert!(title.plain().contains("1–8 / 12"));
    assert!(!title.plain().contains("Saved request"));
    let (row, _) = frame.cursor.unwrap();
    assert!(frame.live[row].plain().contains("› Saved request"));
    assert!(!frame.live[row].plain().contains("1–8 / 12"));
}

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
        Some(Selector::settings("fixture/model", Effort::High)),
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
                        text::cells(&line.plain()) <= width
                            && matches!(
                                line.background,
                                Some(USER_BACKGROUND | SELECTED_BACKGROUND)
                            )
                    }),
                    "{width}x{height}"
                );
                if state.selector.as_ref().is_none_or(|menu| {
                    menu.searchable || matches!(menu.purpose, crate::tui::selector::Purpose::Key)
                }) {
                    let (row, column) = frame.cursor.unwrap();
                    assert!(row < frame.live.len());
                    assert!(column < width, "caret must fit {width}x{height}");
                    assert!(frame.live[row].paint(width).contains(CURSOR));
                }
                assert_eq!(state.editor, draft);
            }
        }
    }
}

#[test]
fn settings_feedback_stays_visible_above_the_panel_in_a_short_window() {
    let mut state = State {
        height: 6,
        selector: Some(Selector::settings("fixture/model", Effort::High)),
        notice: Some(crate::tui::Feedback::result(
            Kind::Error,
            "Could not save settings",
        )),
        ..State::default()
    };
    let original = state.selector.as_ref().unwrap().selected;
    let frame = frame(&state, "fixture/model", "directory");
    assert!(
        frame
            .live
            .iter()
            .any(|line| line.plain() == "Could not save settings" && line.background.is_none())
    );
    assert!(
        frame
            .live
            .iter()
            .any(|line| line.background == Some(SELECTED_BACKGROUND))
    );
    assert_eq!(state.selector.as_ref().unwrap().selected, original);
    state.height = 24;
    assert!(
        super::frame(&state, "fixture/model", "directory")
            .live
            .iter()
            .any(|line| line.plain().contains("OpenRouter key"))
    );
}

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
                .any(|line| line.plain().contains(&format!("Enter {action}")))
        );
        assert!(
            frame
                .live
                .iter()
                .any(|line| line.plain().contains("Esc close"))
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
    let controls = resume
        .live
        .iter()
        .find(|line| line.plain().contains("Ctrl+D"))
        .unwrap()
        .plain();
    assert!(controls.contains("Enter"));
    assert!(controls.contains("delete"));
    assert!(controls.contains("Esc"));
    assert!(!controls.contains('…'));
    state.width = 60;
    state.selector = Some(Selector::key());
    let key = frame(&state, "fixture/model", "directory");
    assert!(
        key.live
            .iter()
            .any(|line| line.plain().contains("plain text key") && !line.plain().contains('…'))
    );
}
