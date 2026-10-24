use super::composer_frame as frame;
use super::*;
use crate::{
    backend::Model,
    effort::Effort,
    sessions::Summary,
    tui::{
        selector::Selector,
        state::Kind,
        theme::{CURSOR, SELECTED_SURFACE, USER_SURFACE},
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
            backend: "openrouter".into(),
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
        Selector::settings(
            "fixture/model",
            Effort::High,
            80,
            &crate::backend::openrouter::Routing::Auto,
        ),
        Selector::models(&models, false, "fixture/model-1"),
        Selector::efforts(model(1), false, Effort::High),
        Selector::sessions(&sessions(), "1234-56-0"),
        Selector::sessions(&[], ""),
        Selector::copy(&[]),
        Selector::loading("Model", "Loading model catalog…", false),
        key,
    ] {
        let mut state = State {
            selector: Some(menu),
            ..State::default()
        };
        state.editor.insert("kept private draft");
        let draft = state.editor.clone();
        let frame = frame(&state, "fixture/model", "directory");
        assert_eq!(frame.live.first().unwrap().plain().trim(), "");
        assert_eq!(frame.live.last().unwrap().plain().trim(), "");
        assert!(frame.live.iter().all(|line| {
            line.background == Some(USER_SURFACE)
                && !line.plain().starts_with('─')
                && !line.plain().contains("kept private draft")
                && !line.plain().contains("isolated-fixture-key")
        }));
        if let Some((row, column)) = frame.cursor {
            let line = &frame.live[row];
            assert!(line.paint(state.width).contains(&CURSOR.to_string()));
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
            .all(|line| line.background == Some(USER_SURFACE))
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
    assert!(title.plain().contains("12 conversations"));
    assert!(!title.plain().contains("Saved request"));
    let (row, _) = frame.cursor.unwrap();
    assert!(frame.live[row].plain().contains(" Saved request"));
    assert!(!frame.live[row].plain().contains("conversations"));
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
        Some(Selector::settings(
            "fixture/model",
            Effort::High,
            80,
            &crate::backend::openrouter::Routing::Auto,
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
                        text::cells(&line.plain()) <= width
                            && matches!(line.background, Some(USER_SURFACE | SELECTED_SURFACE))
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
fn settings_feedback_stays_visible_above_the_panel_in_a_short_window() {
    let mut state = State {
        height: 6,
        selector: Some(Selector::settings(
            "fixture/model",
            Effort::High,
            80,
            &crate::backend::openrouter::Routing::Auto,
        )),
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
            .any(|line| line.plain() == "▌ Could not save settings"
                && line.background == Some(USER_SURFACE))
    );
    assert!(frame.live.iter().any(focused));
    assert_eq!(state.selector.as_ref().unwrap().selected, original);
    state.height = 24;
    assert!(
        super::frame(&state, "fixture/model", "directory")
            .live
            .iter()
            .any(|line| line.plain().contains("API key"))
    );
}

#[test]
fn routing_detail_keeps_figures_when_the_table_columns_do_not_fit() {
    use crate::backend::{Entry, Metrics, Routing};

    let current = Routing::Endpoint("alpha".into());
    let entries = vec![Entry {
        routing: current.clone(),
        name: "alpha".into(),
        description: "$0.216/$0.720 · 48 tok/s · 0.9 s".into(),
        metrics: Some(Metrics {
            input: Some(0.216),
            output: Some(0.720),
            throughput: Some(48.0),
            latency: Some(0.9),
        }),
        facts: "fp8".into(),
        help: "Pinned endpoint".into(),
        problem: None,
    }];
    let menu = Selector::routing("provider/model", false, entries, &current, vec![]);
    let mut state = State {
        width: 30,
        height: 30,
        selector: Some(menu),
        ..State::default()
    };
    let narrow: String = frame(&state, "provider/model", "directory")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(narrow.contains("$0.216/$0.72"), "{narrow}");
    assert!(narrow.contains("48 tok/s"), "{narrow}");
    assert!(narrow.contains("0.9 s"), "{narrow}");

    state.width = 80;
    let wide: String = frame(&state, "provider/model", "directory")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(wide.matches("$0.216/$0.72").count(), 1, "{wide}");
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
    state.width = 60;
    state.selector = Some(Selector::key());
    let key = frame(&state, "fixture/model", "directory");
    assert!(
        key.live
            .iter()
            .any(|line| line.plain().contains("plain text") && !line.plain().contains('…'))
    );
}

#[test]
fn options_align_facts_in_one_column_and_mark_the_current_choice() {
    let models: Vec<_> = [
        ("fixture/a", "Short description"),
        ("fixture/a-longer-identifier", "Long description"),
        ("fixture/b", "Middle description"),
    ]
    .into_iter()
    .map(|(id, name)| Model {
        id: id.into(),
        name: name.into(),
        ..model(0)
    })
    .collect();
    let mut state = State {
        width: 80,
        height: 30,
        ..State::default()
    };
    state.selector = Some(Selector::models(&models, false, "fixture/b"));
    let rows: Vec<_> = frame(&state, "fixture/model", "directory")
        .live
        .iter()
        .map(Line::plain)
        .collect();
    let options: Vec<_> = rows.iter().filter(|row| row.contains("fixture/")).collect();
    assert_eq!(options.len(), 3, "{rows:#?}");
    // Facts share one column after the widest name.
    let starts: Vec<_> = options
        .iter()
        .zip(["Short d", "Long d", "Middle d"])
        .map(|(row, description)| text::cells(&row[..row.find(description).unwrap()]))
        .collect();
    assert!(starts.iter().all(|start| *start == starts[0]), "{rows:#?}");
    assert!(options[2].contains("fixture/b") && options[2].ends_with("● current"));
    assert!(!rows.iter().any(|row| row.contains('✓')));

    // Too narrow for both: names stay whole and descriptions give way.
    state.width = 40;
    let rows: Vec<_> = frame(&state, "fixture/model", "directory")
        .live
        .iter()
        .map(Line::plain)
        .collect();
    assert!(
        rows.iter()
            .any(|row| row.contains("fixture/a-longer-identifier")),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|row| row.contains("Short")), "{rows:#?}");
}
