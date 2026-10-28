use super::*;

#[test]
fn receipt_during_stream_keeps_an_open_code_block_whole() {
    let mut state = State::default();
    state.begin_tool_turn();
    let opened = "Before\n```rust\nfn first() {}\n";
    let closed = "Before\n```rust\nfn first() {}\nfn second() {}\n```\nAfter";
    state.event(Event::Streaming {
        text: opened.into(),
    });
    state.collaboration_event(receipt(vec![record(1, "running")]), None);
    state.event(Event::Streaming {
        text: closed.into(),
    });
    state.event(Event::Message {
        text: closed.into(),
    });
    assert_eq!(state.items.len(), 2);
    assert!(matches!(&state.items[0], Item::Tool { name, .. } if name == "collaboration"));
    assert!(matches!(&state.items[1], Item::Text { text, .. } if text == closed));
}

#[test]
fn agent_events_join_the_chapter_they_follow_and_stand_alone_after_prose() {
    let mut state = State {
        width: 100,
        ..State::default()
    };
    state.begin_tool_turn();
    let sent = event(1, "sent", 1, vec![record(1, "running")]);
    state.collaboration_event(sent, Some("Update agents"));
    state.event(Event::ToolStarted {
        id: "r1".into(),
        name: "read".into(),
        arguments: Value::object([
            ("path", Value::string("docs/CLI.md")),
            ("work", Value::string("Update agents")),
        ]),
    });
    state.event(Event::ToolFinished {
        id: "r1".into(),
        name: "read".into(),
        summary: "64 lines".into(),
        result: Value::object([("content", Value::string("text"))]),
    });
    let received = event(2, "received", 1, vec![record(1, "running")]);
    state.collaboration_event(received, None);
    let chapter = all(&state);
    assert!(
        chapter.contains("WORK  Update agents  3 calls"),
        "{chapter}"
    );
    assert!(
        chapter.contains("1 file read · 2 agent events"),
        "{chapter}"
    );
    assert!(chapter.contains("AGENT 1 Assignment v2 sent"), "{chapter}");
    assert!(
        chapter.contains("AGENT 1 Assignment v2 received"),
        "{chapter}"
    );
    state.message(crate::tui::state::Kind::Assistant, "Both run.");
    state.collaboration_event(event(3, "returned", 1, vec![record(1, "returned")]), None);
    let Some(Item::Tool { arguments, .. }) = state.items.last() else {
        panic!("missing event");
    };
    assert_eq!(arguments.get("lone"), Some(&Value::Bool(true)));
    state.finish_tool_turn();
    state.collaboration_event(event(4, "closed", 1, vec![record(1, "stopped")]), None);
    assert_eq!(state.toggle_tools(), Some((true, 1)));
    assert!(all(&state).contains("Search part 1 · isolated workspace"));
}

#[test]
fn ctrl_o_opens_every_agent_event_in_the_chapter_as_its_receipt() {
    let mut state = State {
        width: 100,
        ..State::default()
    };
    state.begin_tool_turn();
    state.collaboration_event(
        event(1, "sent", 1, vec![record(1, "running")]),
        Some("Work"),
    );
    state.collaboration_event(
        event(2, "returned", 1, vec![record(1, "returned")]),
        Some("Work"),
    );
    state.finish_tool_turn();
    let collapsed = all(&state);
    assert!(collapsed.contains("AGENT 1 Returned · 1 file changed  pending review"));
    assert_eq!(state.toggle_tools(), Some((true, 2)));
    let open = all(&state);
    for expected in [
        "Preserve order",
        "Search part 1 · isolated workspace",
        "src/search.rs  +3 −1",
        "Search implemented",
        "Pending review by main agent",
    ] {
        assert!(open.contains(expected), "{expected}: {open}");
    }
}

#[test]
fn failed_checks_are_bad_and_unknown_outcomes_are_warnings() {
    let mut failed = record(2, "returned");
    set(
        &mut failed,
        "checks",
        Value::Array(vec![Value::string("bash: history_groups_by_day failed")]),
    );
    let mut state = State {
        width: 100,
        ..State::default()
    };
    state.begin_tool_turn();
    state.collaboration_event(event(1, "returned", 2, vec![failed]), Some("Review"));
    state.collaboration_event(
        event(2, "unknown", 1, vec![record(1, "unknown")]),
        Some("Review"),
    );
    let rows = state.rows().concat();
    let all = text(&rows);
    assert!(all.contains("2 calls · 1 failed"), "{all}");
    let styled = |needle: &str| {
        rows.iter()
            .flat_map(|row| &row.spans)
            .find(|span| span.text.contains(needle))
            .map(|span| span.style)
    };
    assert_eq!(
        styled("check failed · history_groups_by_day failed"),
        Some(BAD)
    );
    assert_eq!(styled("inspect in /agents"), Some(WARNING));
}
