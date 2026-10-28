use super::*;
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    config::{Settings, Store},
    session::SessionConfig,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
    tui::{
        keys::Decoded,
        state::{Item, State},
        view,
    },
};
use std::{
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

fn record(number: usize, state: &str) -> Value {
    Value::object([
        ("id", Value::string(format!("a{number}"))),
        ("number", Value::number(number)),
        ("title", Value::string(format!("Search part {number}"))),
        ("state", Value::string(state)),
        ("workspace", Value::string("isolated")),
        ("directory", Value::string("fixture")),
        ("sent_version", Value::number(2)),
        ("received_version", Value::number(1)),
        ("instructions", Value::string("Implement search")),
        ("pending_instruction", Value::string("Preserve order")),
        ("activity", Value::string("Checking search")),
        (
            "tools",
            Value::Array(vec![Value::object([
                ("name", Value::string("read")),
                ("state", Value::string("complete")),
                ("text", Value::string("src/search.rs")),
            ])]),
        ),
        ("files", Value::Array(vec![Value::string("src/search.rs")])),
        (
            "checks",
            Value::Array(vec![Value::string("search check passed")]),
        ),
        (
            "report",
            Value::object([
                ("version", Value::number(1)),
                ("text", Value::string("Search implemented")),
            ]),
        ),
        ("reviewed", Value::Bool(false)),
        ("error", Value::Null),
    ])
}

fn receipt(records: Vec<Value>) -> Value {
    Value::object([
        ("type", Value::string("collaboration")),
        ("sequence", Value::number(1)),
        ("kind", Value::string("created")),
        ("title", Value::string("Create agents")),
        ("summary", Value::string("2 assignments v1")),
        ("details", Value::Array(vec![])),
        ("records", Value::Array(records)),
    ])
}

#[test]
fn creation_receipt_expands_snapshots_without_changing_prior_content() {
    let original = receipt(vec![record(1, "running"), record(2, "running")]);
    let mut state = State {
        width: 40,
        ..State::default()
    };
    state.collaboration_event(original.clone());
    let compact = state
        .rows()
        .pop()
        .unwrap()
        .iter()
        .map(crate::tui::line::Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(compact.contains("AGENTS"));
    assert!(!compact.contains("AGENT 1"));
    assert!(state.toggle_collaboration());
    let expanded = state
        .rows()
        .pop()
        .unwrap()
        .iter()
        .map(crate::tui::line::Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(expanded.contains("AGENT 1") && expanded.contains("AGENT 2"));
    assert!(expanded.contains("Implement search"));
    assert!(matches!(&state.items[0], Item::Collaboration { event, .. } if *event == original));
}

#[test]
fn expanded_receipt_wraps_a_long_assignment_name_at_narrow_width() {
    let mut agent = record(1, "running");
    if let Value::Object(fields) = &mut agent {
        fields.insert(
            "title".into(),
            Value::string(
                "A long assignment name that has to remain readable through the terminal-marker",
            ),
        );
    }
    let event = receipt(vec![agent]);
    let rows = crate::tui::view::agents::receipt(&event, true, 40);
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("terminal-marker"))
    );
    assert!(
        rows.iter()
            .all(|row| crate::tui::text::cells(&row.plain()) <= 40)
    );
}

#[test]
fn panel_filter_tabs_and_nested_escape_keep_the_exact_main_draft() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let mut app = App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        SessionConfig {
            store: Store::new(directory.path().join("config")),
            settings,
            bash: crate::tools::find_bash().unwrap(),
        },
        None,
    );
    app.state.editor.insert("unsent main instruction");
    app.state.editor.cursor = 6;
    let draft = app.state.editor.clone();
    app.state.activity = Some(crate::tui::activity::Activity::new());
    app.open_agents();
    app.state.collaboration = vec![record(1, "running"), record(2, "returned")];
    app.input(Decoded::Text("agent 2".into())).unwrap();
    assert_eq!(
        app.state
            .agents_panel
            .as_ref()
            .unwrap()
            .filtered(&app.state.collaboration),
        vec![1]
    );
    let key = |app: &mut App, code| {
        app.input(Decoded::Key(Key {
            code,
            modifiers: 0,
            character: 0,
        }))
        .unwrap();
    };
    key(&mut app, 13);
    assert!(app.state.agents_panel.as_ref().unwrap().detail);
    key(&mut app, 38);
    assert!(!app.state.agents_panel.as_ref().unwrap().follow);
    key(&mut app, 35);
    assert!(app.state.agents_panel.as_ref().unwrap().follow);
    key(&mut app, 39);
    key(&mut app, 39);
    assert_eq!(app.state.agents_panel.as_ref().unwrap().tab, 2);
    let shown = view::composer_frame(&app.state, "fixture/model", "fixture").live;
    assert!(
        shown
            .iter()
            .any(|line| line.plain().contains("Search implemented"))
    );
    let detail = shown
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(detail.matches("Esc").count(), 1, "{detail}");
    assert!(detail.contains("Esc  back to agents"), "{detail}");
    assert!(!detail.contains("Esc interrupt"), "{detail}");
    key(&mut app, 27);
    assert!(!app.state.agents_panel.as_ref().unwrap().detail);
    assert!(!app.state.activity.as_ref().unwrap().stopping);
    let list = view::composer_frame(&app.state, "fixture/model", "fixture")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(list.matches("Esc").count(), 1, "{list}");
    assert!(list.contains("Esc  close"), "{list}");
    key(&mut app, 27);
    assert!(app.state.agents_panel.is_none());
    assert!(!app.state.activity.as_ref().unwrap().stopping);
    let main = view::composer_frame(&app.state, "fixture/model", "fixture")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(main.contains("Esc interrupt"), "{main}");
    assert_eq!(app.state.editor, draft);
    let created = receipt(vec![record(1, "running")]);
    assert!(app.show_collaboration(created.clone()));
    assert!(!app.show_collaboration(created));
    assert_eq!(
        app.state
            .items
            .iter()
            .filter(|item| matches!(item, Item::Collaboration { .. }))
            .count(),
        1
    );
}

#[test]
fn returned_report_does_not_become_reviewed_when_opened() {
    let mut state = State {
        width: 80,
        height: 24,
        ..State::default()
    };
    state.collaboration = vec![record(1, "returned")];
    state.agents_panel = Some(Panel {
        detail: true,
        detail_id: Some("a1".into()),
        tab: 2,
        ..Panel::default()
    });
    let rows = view::composer_frame(&state, "fixture/model", "fixture").live;
    assert!(
        rows.iter()
            .any(|line| line.plain().contains("Pending review by main agent"))
    );
    assert_eq!(
        state.collaboration[0].get("reviewed"),
        Some(&Value::Bool(false))
    );
}

#[test]
fn footer_keeps_context_and_critical_counts_in_its_existing_row() {
    let mut state = State {
        width: 100,
        height: 24,
        ..State::default()
    };
    state.context = Some(crate::context::Status {
        tokens: 34_000,
        capacity: Some(100_000),
        auto_compact: 80,
    });
    state.collaboration = vec![
        record(1, "running"),
        record(2, "returned"),
        record(3, "unknown"),
    ];
    let wide = view::composer_frame(&state, "provider/model", "fixture");
    let row = wide.live.last().unwrap().plain();
    assert!(
        row.contains("AGENTS")
            && row.contains("1 running")
            && row.contains("1 for main agent review")
            && row.contains("1 unknown")
    );
    assert!(row.contains("34%") && row.contains("/agents"), "{row}");
    state.width = 40;
    let narrow = view::composer_frame(&state, "provider/model", "fixture");
    let row = narrow.live.last().unwrap().plain();
    assert!(row.contains("AGENTS") && row.contains("34%"));
    assert!(!row.contains("provider/model"));
    assert_eq!(wide.live.len(), narrow.live.len());
}

#[test]
fn receipt_during_stream_splits_text_at_its_arrival_position() {
    let mut state = State::default();
    state.event(crate::events::Event::Streaming {
        text: "Before".into(),
    });
    state.collaboration_event(receipt(vec![record(1, "running")]));
    state.event(crate::events::Event::Streaming {
        text: "Before after".into(),
    });
    state.event(crate::events::Event::Message {
        text: "Before after".into(),
    });
    assert!(matches!(&state.items[0], Item::Text { text, .. } if text == "Before"));
    assert!(matches!(&state.items[1], Item::Collaboration { .. }));
    assert!(matches!(&state.items[2], Item::Text { text, .. } if text == " after"));
}

#[test]
fn inspected_identity_stays_fixed_when_live_state_changes_the_filter() {
    let mut records = vec![record(1, "running"), record(2, "running")];
    let mut panel = Panel::default();
    panel.filter.insert("running");
    panel.detail = true;
    panel.detail_id = Some("a1".into());
    assert_eq!(
        value(panel.current(&records).unwrap(), "title"),
        "Search part 1"
    );
    if let Value::Object(fields) = &mut records[0] {
        fields.insert("state".into(), Value::string("returned"));
    }
    assert_eq!(panel.filtered(&records), vec![1]);
    assert_eq!(
        value(panel.current(&records).unwrap(), "title"),
        "Search part 1"
    );
}

#[test]
fn closed_selected_assignment_returns_to_the_open_list_without_selecting_another_detail() {
    let mut records = vec![record(1, "returned"), record(2, "running")];
    let mut panel = Panel {
        detail: true,
        detail_id: Some("a1".into()),
        tab: 2,
        selected: 1,
        ..Panel::default()
    };
    records.remove(0);
    panel.reconcile(&records);
    assert!(!panel.detail);
    assert!(panel.detail_id.is_none());
    assert_eq!(panel.tab, 0);
    assert_eq!(panel.selected, 0);
    assert_eq!(value(panel.current(&records).unwrap(), "id"), "a2");
    let mut state = State {
        width: 80,
        height: 24,
        collaboration: records,
        agents_panel: Some(panel),
        ..State::default()
    };
    state.editor.insert("unsent main instruction");
    let shown = view::composer_frame(&state, "fixture/model", "fixture")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(shown.contains("Search part 2"), "{shown}");
    assert!(!shown.contains("Search part 1"), "{shown}");
    assert_eq!(state.editor.text, "unsent main instruction");
}

#[test]
fn activity_follows_new_rows_until_the_reader_pauses() {
    let mut agent = record(1, "running");
    let tools = (0..30)
        .map(|index| {
            Value::object([
                ("name", Value::string(format!("tool-{index:02}"))),
                ("state", Value::string("done")),
                ("text", Value::string("checked")),
            ])
        })
        .collect();
    if let Value::Object(fields) = &mut agent {
        fields.insert("tools".into(), Value::Array(tools));
    }
    let mut state = State {
        width: 80,
        height: 24,
        collaboration: vec![agent],
        ..State::default()
    };
    state.agents_panel = Some(Panel {
        detail: true,
        detail_id: Some("a1".into()),
        ..Panel::default()
    });
    let following = view::composer_frame(&state, "fixture/model", "fixture").live;
    assert!(following.iter().any(|row| row.plain().contains("tool-29")));
    state.agents_panel.as_mut().unwrap().follow = false;
    let paused = view::composer_frame(&state, "fixture/model", "fixture").live;
    assert!(paused.iter().any(|row| row.plain().contains("tool-00")));
    assert!(!paused.iter().any(|row| row.plain().contains("tool-29")));
}

#[test]
fn reviewed_open_records_remain_inspectable_without_a_pending_footer() {
    let mut reviewed = record(1, "returned");
    if let Value::Object(fields) = &mut reviewed {
        fields.insert("reviewed".into(), Value::Bool(true));
        fields.insert(
            "checks".into(),
            Value::Array(vec![Value::string("search check failed")]),
        );
    }
    let mut state = State {
        width: 80,
        height: 24,
        collaboration: vec![reviewed],
        ..State::default()
    };
    let normal = view::composer_frame(&state, "fixture/model", "fixture").live;
    assert!(!normal.last().unwrap().plain().contains("AGENTS"));
    state.agents_panel = Some(Panel::default());
    let panel = view::composer_frame(&state, "fixture/model", "fixture").live;
    assert!(panel.iter().any(|row| row.plain().contains("AGENT 1")));
    assert!(panel.iter().any(|row| row.plain().contains("Reviewed")));
}

#[path = "integration_tests.rs"]
mod integration_tests;

#[path = "replay_tests.rs"]
mod replay_tests;

#[test]
fn only_receipted_agent_actions_replace_tool_cards() {
    for action in ["create", "update", "review", "stop", "close"] {
        assert!(receipt_replaces_tool(&Value::object([(
            "action",
            Value::string(action),
        )])));
    }
    for action in ["list", "inspect", "wait", "diff", "integrate", "invalid"] {
        assert!(!receipt_replaces_tool(&Value::object([(
            "action",
            Value::string(action),
        )])));
    }
}
