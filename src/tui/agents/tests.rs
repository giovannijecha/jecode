use super::*;
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    config::{Settings, Store},
    events::Event,
    session::SessionConfig,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
    tui::{
        keys::Decoded,
        line::Line,
        state::{Item, State},
        theme::{BAD, WARNING},
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
                ("id", Value::string("c1")),
                ("name", Value::string("edit")),
                ("state", Value::string("done")),
                ("text", Value::string("src/search.rs")),
                (
                    "arguments",
                    Value::object([("path", Value::string("src/search.rs"))]),
                ),
                ("result", Value::object([])),
                (
                    "change",
                    Value::object([
                        ("added", Value::number(3)),
                        ("removed", Value::number(1)),
                        ("created", Value::Bool(false)),
                    ]),
                ),
            ])]),
        ),
        ("files", Value::Array(vec![Value::string("src/search.rs")])),
        (
            "checks",
            Value::Array(vec![Value::string("bash: search check passed")]),
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

fn event(sequence: usize, kind: &str, number: usize, records: Vec<Value>) -> Value {
    let id = if number == 0 {
        Value::Null
    } else {
        Value::string(format!("a{number}"))
    };
    let details = if kind == "created" {
        records
            .iter()
            .map(|record| record.get("id").unwrap().clone())
            .collect()
    } else {
        Vec::new()
    };
    Value::object([
        ("type", Value::string("collaboration")),
        ("sequence", Value::number(sequence)),
        ("kind", Value::string(kind)),
        ("id", id),
        ("number", Value::number(number)),
        ("title", Value::string(format!("Search part {number}"))),
        (
            "version",
            Value::number(if kind == "created" { 0 } else { 2 }),
        ),
        ("summary", Value::string("Agent summary")),
        ("details", Value::Array(details)),
        ("records", Value::Array(records)),
    ])
}

fn receipt(records: Vec<Value>) -> Value {
    event(1, "created", 0, records)
}

fn set(record: &mut Value, key: &str, value: Value) {
    if let Value::Object(fields) = record {
        fields.insert(key.into(), value);
    }
}

fn text(rows: &[Line]) -> String {
    rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n")
}

fn all(state: &State) -> String {
    text(&state.rows().concat())
}

#[test]
fn creation_event_is_a_lone_row_that_opens_every_assignment() {
    let original = receipt(vec![record(1, "running"), record(2, "running")]);
    let mut state = State {
        width: 40,
        ..State::default()
    };
    state.collaboration_event(original.clone(), None);
    let compact = all(&state);
    assert!(
        compact.contains("AGENTS  2 created · isolated workspaces"),
        "{compact}"
    );
    assert!(!compact.contains("AGENT 1"));
    assert_eq!(state.toggle_tools(), Some((true, 1)));
    let expanded = all(&state);
    assert!(expanded.contains("AGENT 1") && expanded.contains("AGENT 2"));
    assert!(expanded.contains("Implement search"), "{expanded}");
    assert!(matches!(&state.items[0],
        Item::Tool { name, result: Some(event), .. } if name == "collaboration" && *event == original));
}

#[test]
fn expanded_receipt_wraps_a_long_assignment_name_at_narrow_width() {
    let mut agent = record(1, "running");
    set(
        &mut agent,
        "title",
        Value::string(
            "A long assignment name that has to remain readable through the terminal-marker",
        ),
    );
    let mut state = State {
        width: 40,
        ..State::default()
    };
    state.collaboration_event(receipt(vec![agent]), None);
    state.toggle_tools();
    let rows = state.rows().concat();
    assert!(text(&rows).contains("terminal-marker"));
    assert!(
        rows.iter()
            .all(|row| crate::tui::text::cells(&row.plain()) <= 40)
    );
}

#[test]
fn model_written_titles_reports_and_names_are_sanitized() {
    let mut agent = record(1, "returned");
    set(&mut agent, "title", Value::string("Title \u{1b}[31mred"));
    set(
        &mut agent,
        "report",
        Value::object([
            ("version", Value::number(1)),
            ("text", Value::string("Report \u{1b}]0;owned\u{7}")),
        ]),
    );
    let mut returned = event(2, "returned", 1, vec![agent.clone()]);
    set(&mut returned, "title", Value::string("Title \u{1b}[31mred"));
    let mut state = State {
        width: 80,
        height: 24,
        collaboration: vec![agent],
        ..State::default()
    };
    state.collaboration_event(returned, None);
    state.toggle_tools();
    let rows = all(&state);
    assert!(
        rows.contains("Title [ESC][31mred") && rows.contains("Report [ESC]"),
        "{rows}"
    );
    state.agents_panel = Some(Panel::default());
    let list = text(&view::composer_frame(&state, "fixture/model", "fixture").live);
    assert!(list.contains("Title [ESC][31mred"), "{list}");
    assert!(!rows.contains('\u{1b}') && !list.contains('\u{1b}'));
}

#[test]
fn panel_list_inside_and_nested_escape_keep_the_exact_main_draft() {
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
    let list = text(&view::composer_frame(&app.state, "fixture/model", "fixture").live);
    assert!(list.contains("Agents · 2 open"), "{list}");
    assert!(list.contains("returned · main review"), "{list}");
    assert!(!list.contains("v1") && !list.contains("v2"), "{list}");
    app.input(Decoded::Text("agent 2".into())).unwrap();
    let panel = app.state.agents_panel.as_ref().unwrap();
    assert_eq!(panel.filtered(&app.state.collaboration), vec![1]);
    let key = |app: &mut App, code| {
        app.input(Decoded::Key(Key {
            code,
            modifiers: 0,
            character: 0,
        }))
        .unwrap();
    };
    key(&mut app, 13);
    assert_eq!(
        app.state.agents_panel.as_ref().unwrap().inside.as_deref(),
        Some("a2")
    );
    key(&mut app, 38);
    assert!(!app.state.agents_panel.as_ref().unwrap().follow);
    key(&mut app, 35);
    assert!(app.state.agents_panel.as_ref().unwrap().follow);
    let inside = text(&view::frame(&app.state, "fixture/model", "fixture").live);
    assert!(inside.contains("Implement search"), "{inside}");
    assert!(inside.contains("UPDATE  src/search.rs"), "{inside}");
    assert!(
        inside.contains("AGENT 2  Returned · Search part 2"),
        "{inside}"
    );
    assert_eq!(inside.matches("Esc").count(), 1, "{inside}");
    assert!(inside.contains("Esc  back to agents"), "{inside}");
    assert!(!inside.contains("Filter agents") && !inside.contains("Esc interrupt"));
    key(&mut app, 27);
    assert!(app.state.agents_panel.as_ref().unwrap().inside.is_none());
    assert!(!app.state.activity.as_ref().unwrap().stopping);
    let list = text(&view::composer_frame(&app.state, "fixture/model", "fixture").live);
    assert_eq!(list.matches("Esc").count(), 1, "{list}");
    assert!(
        list.contains("Esc  close") && list.contains("Enter  open"),
        "{list}"
    );
    key(&mut app, 27);
    assert!(app.state.agents_panel.is_none());
    assert!(!app.state.activity.as_ref().unwrap().stopping);
    let main = text(&view::composer_frame(&app.state, "fixture/model", "fixture").live);
    assert!(main.contains("Esc interrupt"), "{main}");
    assert_eq!(app.state.editor, draft);
    let created = receipt(vec![record(1, "running")]);
    assert!(app.show_collaboration(created.clone()));
    assert!(!app.show_collaboration(created));
    assert_eq!(
        app.state
            .items
            .iter()
            .filter(|item| matches!(item, Item::Tool { name, .. } if name == "collaboration"))
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
        inside: Some("a1".into()),
        ..Panel::default()
    });
    let rows = text(&view::frame(&state, "fixture/model", "fixture").live);
    assert!(rows.contains("src/search.rs  +3 −1"), "{rows}");
    assert!(rows.contains("pending review by main agent"), "{rows}");
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
fn inspected_identity_stays_fixed_when_live_state_changes_the_filter() {
    let mut records = vec![record(1, "running"), record(2, "running")];
    let mut panel = Panel::default();
    panel.filter.insert("running");
    panel.inside = Some("a1".into());
    assert_eq!(
        value(panel.current(&records).unwrap(), "title"),
        "Search part 1"
    );
    set(&mut records[0], "state", Value::string("returned"));
    assert_eq!(panel.filtered(&records), vec![1]);
    assert_eq!(
        value(panel.current(&records).unwrap(), "title"),
        "Search part 1"
    );
}

#[test]
fn closed_inspected_agent_returns_to_the_open_list() {
    let mut records = vec![record(1, "returned"), record(2, "running")];
    let mut panel = Panel {
        inside: Some("a1".into()),
        selected: 1,
        ..Panel::default()
    };
    records.remove(0);
    panel.reconcile(&records);
    assert!(panel.inside.is_none());
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
    let shown = text(&view::composer_frame(&state, "fixture/model", "fixture").live);
    assert!(shown.contains("Search part 2"), "{shown}");
    assert!(!shown.contains("Search part 1"), "{shown}");
    assert_eq!(state.editor.text, "unsent main instruction");
}

#[test]
fn inside_follows_new_rows_until_the_reader_pauses() {
    let mut agent = record(1, "running");
    let tools = (0..30)
        .map(|index| {
            Value::object([
                ("name", Value::string(format!("tool-{index:02}"))),
                ("state", Value::string("done")),
                ("text", Value::string("checked")),
                ("result", Value::object([])),
            ])
        })
        .collect();
    set(&mut agent, "tools", Value::Array(tools));
    let mut state = State {
        width: 80,
        height: 24,
        collaboration: vec![agent],
        ..State::default()
    };
    state.agents_panel = Some(Panel {
        inside: Some("a1".into()),
        ..Panel::default()
    });
    let following = text(&view::frame(&state, "fixture/model", "fixture").live);
    assert!(following.contains("TOOL-29") && following.contains("End  follow"));
    state.agents_panel.as_mut().unwrap().follow = false;
    let paused = text(&view::frame(&state, "fixture/model", "fixture").live);
    assert!(paused.contains("TOOL-00") && !paused.contains("TOOL-29"));
}

#[test]
fn reviewed_open_records_remain_inspectable_without_a_pending_footer() {
    let mut reviewed = record(1, "returned");
    set(&mut reviewed, "reviewed", Value::Bool(true));
    set(
        &mut reviewed,
        "checks",
        Value::Array(vec![Value::string("search check failed")]),
    );
    let mut state = State {
        width: 80,
        height: 24,
        collaboration: vec![reviewed],
        ..State::default()
    };
    let normal = view::composer_frame(&state, "fixture/model", "fixture").live;
    assert!(!normal.last().unwrap().plain().contains("AGENTS"));
    state.agents_panel = Some(Panel::default());
    let panel = text(&view::composer_frame(&state, "fixture/model", "fixture").live);
    assert!(
        panel.contains("AGENT 1") && panel.contains("reviewed"),
        "{panel}"
    );
}

#[path = "integration_tests.rs"]
mod integration_tests;

#[path = "replay_tests.rs"]
mod replay_tests;

#[path = "chapter_tests.rs"]
mod chapter_tests;

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
