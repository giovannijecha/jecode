use super::composer_frame as frame;
use super::*;
use crate::{
    backend::Model,
    effort::Effort,
    sessions::Summary,
    tui::{
        selector::{Purpose, Selector},
        state::Kind,
        theme::{BADGE_IDLE, CURSOR, LIT, USER_SURFACE},
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

mod controls_tests;
mod field_tests;

#[test]
fn every_menu_docks_below_the_composer_rules_with_its_title_and_keys() {
    let models: Vec<_> = (0..12).map(model).collect();
    let mut key = Selector::key();
    key.editor.insert("isolated-fixture-key");
    for menu in [
        Selector::settings(
            "fixture/model",
            Effort::High,
            80,
            &crate::backend::openrouter::Routing::Auto,
            Default::default(),
            std::path::Path::new("fixture/config.json"),
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
        let menu = state.selector.as_ref().unwrap();
        let form = matches!(menu.purpose, Purpose::Key);
        let typed = menu.searchable || form;
        let title = menu.title.clone();
        let frame = frame(&state, "fixture/model", "directory");
        let rows: Vec<_> = frame.live.iter().map(Line::plain).collect();
        // Status row, top rule, one input row, then the title's bottom rule.
        assert_eq!(rows[0].trim(), "", "{rows:#?}");
        assert!(rows[1].starts_with('─'), "{rows:#?}");
        assert!(
            rows[3].starts_with("─ ") && rows[3].contains(&title),
            "{title}: {rows:#?}"
        );
        if typed {
            // A typed field takes the input row and the caret.
            assert_eq!(frame.cursor.map(|(row, _)| row), Some(2), "{rows:#?}");
            assert!(!rows.iter().any(|row| row.contains("kept private draft")));
        } else {
            // Otherwise the draft stays visible, without a caret.
            assert_eq!(frame.cursor, None);
            assert_eq!(rows[2].trim_end(), "› kept private draft");
        }
        // The keys end the panel after one blank row; a form without body
        // rows puts them right below its title.
        let (keys, body) = rows.split_last().unwrap();
        assert!(keys.starts_with("   ") && keys.contains("Esc"), "{rows:#?}");
        if form {
            assert_eq!(body.len(), 4, "{rows:#?}");
        } else {
            assert!(
                body.len() > 5 && body.last().unwrap().is_empty(),
                "{rows:#?}"
            );
        }
        assert!(frame.live.iter().all(|line| {
            line.background.is_none() && !line.plain().contains("isolated-fixture-key")
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
fn sign_in_link_wraps_without_losing_the_url_or_escape_control() {
    let url = "https://auth.openai.com/oauth/authorize?client_id=fixture&state=long-fixture-state";
    let mut menu = Selector::loading(
        "Continue with ChatGPT",
        &format!("Waiting… Open in browser: {url}"),
        false,
    );
    menu.parent = Some("OpenAI");
    for width in [44, 80] {
        let state = State {
            width,
            height: 30,
            selector: Some(Selector::loading(
                "Continue with ChatGPT",
                &menu.message,
                false,
            )),
            ..State::default()
        };
        let rows = frame(&state, "fixture/model", "directory").live;
        let text: String = rows
            .iter()
            .map(|line| line.plain())
            .collect::<Vec<_>>()
            .join("");
        let compact: String = text
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        assert!(text.contains("auth.openai.com"));
        assert!(compact.contains("long-fixture-state"));
        assert!(text.contains("Esc"));
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
    let rows: Vec<_> = commands.live.iter().map(Line::plain).collect();
    assert!(
        commands
            .live
            .iter()
            .all(|line| line.background != Some(USER_SURFACE))
    );
    // The draft keeps its caret in the composer, the panel's title below it
    // and the command's description as a muted row.
    let caret = Some((2, text::cells(&draft.text) + 2));
    assert_eq!(commands.cursor, caret, "{rows:#?}");
    assert_eq!(rows[2].trim_end(), "› /tmp clean");
    assert!(rows[3].starts_with("─ Commands"), "{rows:#?}");
    assert!(
        rows[4].starts_with("  ") && rows[4].contains("temporary"),
        "{rows:#?}"
    );
    assert!(!rows.iter().any(|row| row.contains("No matching command")));
    state.suggestions.dismiss();
    let normal = frame(&state, "fixture/model", "directory");
    let rows: Vec<_> = normal.live.iter().map(Line::plain).collect();
    assert!(!rows.iter().any(|row| row.starts_with("─ Commands")));
    assert!(rows.iter().any(|row| row.starts_with('─')));
    assert_eq!(state.editor, draft);
    assert_eq!(normal.cursor, caret);
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
fn settings_feedback_stays_visible_above_the_panel_in_a_short_window() {
    let mut state = State {
        height: 6,
        selector: Some(Selector::settings(
            "fixture/model",
            Effort::High,
            80,
            &crate::backend::openrouter::Routing::Auto,
            Default::default(),
            std::path::Path::new("fixture/config.json"),
        )),
        notice: Some(crate::tui::Feedback::result(
            Kind::Error,
            "Could not save settings",
        )),
        ..State::default()
    };
    let original = state.selector.as_ref().unwrap().selected;
    let frame = frame(&state, "fixture/model", "directory");
    let rows: Vec<_> = frame.live.iter().map(Line::plain).collect();
    // The notice takes the status row, above the panel's title.
    assert_eq!(rows.len(), 6, "{rows:#?}");
    assert_eq!(
        rows[0].trim_end(),
        " ✕  Could not save settings",
        "{rows:#?}"
    );
    assert!(
        rows[1].starts_with("─ Settings · saved defaults"),
        "{rows:#?}"
    );
    assert!(frame.live.iter().any(focused));
    assert_eq!(state.selector.as_ref().unwrap().selected, original);
    state.height = 24;
    assert!(
        super::frame(&state, "fixture/model", "directory")
            .live
            .iter()
            .any(|line| line.plain().contains("Providers"))
    );
}

#[test]
fn narrow_routing_tables_drop_latency_then_speed_then_price_within_the_width() {
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
        height: 30,
        selector: Some(menu),
        ..State::default()
    };
    // Narrower tables drop latency, then speed, then price; the endpoint
    // and its current chip stay.
    for (width, price, speed, latency) in [
        (80, true, true, true),
        (52, true, true, false),
        (44, true, false, false),
        (30, false, false, false),
    ] {
        state.width = width;
        let rows: Vec<_> = frame(&state, "provider/model", "directory")
            .live
            .iter()
            .map(|line| line.plain())
            .collect();
        assert!(
            rows.iter().all(|row| text::cells(row) <= width),
            "{rows:#?}"
        );
        let alpha = rows.iter().find(|row| row.starts_with("› alpha")).unwrap();
        assert!(alpha.trim_end().ends_with(" current"), "{alpha}");
        let header = rows.iter().find(|row| row.starts_with("  Endpoint"));
        let heading = |name: &str| header.is_some_and(|header| header.contains(name));
        assert_eq!(heading("$/M in/out"), price, "{rows:#?}");
        assert_eq!(alpha.contains("$0.216/$0.72"), price, "{alpha}");
        assert_eq!(heading("tok/s"), speed, "{rows:#?}");
        assert_eq!(alpha.contains(" 48 "), speed, "{alpha}");
        assert_eq!(heading("latency"), latency, "{rows:#?}");
        assert_eq!(alpha.contains("0.9 s"), latency, "{alpha}");
        // The figures never repeat outside the table.
        assert!(rows.iter().filter(|row| row.contains("$0.216")).count() <= 1);
    }
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
    let live = frame(&state, "fixture/model", "directory").live;
    let rows: Vec<_> = live.iter().map(Line::plain).collect();
    let options: Vec<_> = rows.iter().filter(|row| row.contains("fixture/")).collect();
    assert_eq!(options.len(), 3, "{rows:#?}");
    // Facts share one column after the widest name.
    let starts: Vec<_> = options
        .iter()
        .zip(["Short d", "Long d", "Middle d"])
        .map(|(row, description)| text::cells(&row[..row.find(description).unwrap()]))
        .collect();
    assert!(starts.iter().all(|start| *start == starts[0]), "{rows:#?}");
    // Names start at column 2; the current choice ends with its chip two
    // cells before the edge.
    assert!(options.iter().all(|row| row.chars().nth(2) == Some('f')));
    assert!(options[2].contains("fixture/b") && options[2].ends_with(" current   "));
    let current = live
        .iter()
        .find(|line| line.plain().contains("fixture/b"))
        .unwrap();
    assert!(current.spans.iter().any(|span| {
        span.text == " current " && span.style == LIT && span.background == Some(BADGE_IDLE)
    }));
    assert!(
        !rows
            .iter()
            .any(|row| row.contains('✓') || row.contains('●'))
    );

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
