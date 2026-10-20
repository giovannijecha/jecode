use super::*;
use crate::{
    events::Event,
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        super::tests::config(directory),
        None,
    )
}

fn key(app: &mut App, code: u16, modifiers: u8) {
    assert!(
        !app.terminal_event(
            Input::Key(terminal::Key {
                code,
                modifiers,
                character: if code == 32 { 32 } else { 0 }
            }),
            &mut Decoder::default()
        )
        .unwrap()
    );
}

fn finish(app: &mut App) {
    let start = Instant::now();
    while app.worker.is_some() {
        app.poll().unwrap();
        assert!(start.elapsed() < Duration::from_secs(6));
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn flags(state: &State) -> Vec<Option<bool>> {
    state
        .items
        .iter()
        .filter_map(|item| match item {
            state::Item::Tool { presentation, .. } => Some(presentation.expanded),
            _ => None,
        })
        .collect()
}

fn tool(state: &mut State, id: &str, output: &str) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: "bash".into(),
        arguments: Value::object([("command", Value::string(id))]),
    });
    state.event(Event::ToolFinished {
        id: id.into(),
        name: "bash".into(),
        summary: "exit 0".into(),
        result: Value::object([
            ("stdout", Value::string(output)),
            ("exit_code", Value::number(0)),
        ]),
    });
}

fn complete(state: &mut State) {
    state.close_tools();
    state.finish_tool_turn();
}

#[test]
fn native_ctrl_o_waits_for_the_whole_turn_preserves_drafts_and_resets_on_accepted_send() {
    let directory = Directory::new();
    std::fs::write(directory.path().join("source.txt"), "before\n").unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "First request.",
                vec![tool_call(
                    "read",
                    "read",
                    Value::object([("path", Value::string("source.txt"))]),
                )],
            ),
        ),
        (
            200,
            completion(
                "Second request.",
                vec![
                    tool_call(
                        "write",
                        "write",
                        Value::object([
                            ("path", Value::string("new.txt")),
                            ("content", Value::string("new\n")),
                        ]),
                    ),
                    tool_call(
                        "edit",
                        "edit",
                        Value::object([
                            ("path", Value::string("source.txt")),
                            ("old_text", Value::string("before")),
                            ("new_text", Value::string("after")),
                        ]),
                    ),
                ],
            ),
        ),
        (200, completion("Finished all requests.", vec![])),
        (200, completion("No tools in this turn.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("Perform the fixture work.");
    key(&mut app, 13, 0);
    assert!(app.worker.is_some());
    key(&mut app, 79, 4);
    assert!(!app.state.tool_details.expanded);
    finish(&mut app);
    assert_eq!(flags(&app.state), [None, None, None]);
    assert!(app.state.tool_details_available());
    app.state.suggestions.panel = true;
    key(&mut app, 79, 4);
    assert!(!app.state.tool_details.expanded && app.state.suggestions.panel);
    app.state.suggestions.panel = false;
    key(&mut app, 79, 4);
    assert_eq!(flags(&app.state), [Some(true); 3]);
    app.input(Decoded::Text("next draft".into())).unwrap();
    assert_eq!(flags(&app.state), [Some(true); 3]);
    key(&mut app, 32, 0);
    assert_eq!(app.state.editor.text, "next draft ");
    assert_eq!(flags(&app.state), [Some(true); 3]);
    app.state.editor.replace(String::new());
    key(&mut app, 13, 0);
    assert_eq!(flags(&app.state), [Some(true); 3]);
    app.state.editor.replace("Next request.".into());
    std::fs::write(directory.path().join("JECODE.md"), [255]).unwrap();
    key(&mut app, 13, 0);
    assert!(app.worker.is_none());
    assert_eq!(flags(&app.state), [Some(true); 3]);
    std::fs::remove_file(directory.path().join("JECODE.md")).unwrap();
    assert_eq!(app.state.queue.paused.len(), 1);
    app.state.editor.replace("Retry next request.".into());
    key(&mut app, 13, 0);
    assert!(app.worker.is_some());
    assert_eq!(flags(&app.state), [None; 3]);
    finish(&mut app);
    assert!(!app.state.tool_details_available());
    key(&mut app, 79, 4);
    assert_eq!(flags(&app.state), [None; 3]);
    assert_eq!(fixture.finish().len(), 4);
}

#[test]
fn an_errored_turn_can_expand_its_completed_tools() {
    let directory = Directory::new();
    std::fs::write(directory.path().join("source.txt"), "before\n").unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "read",
                    "read",
                    Value::object([("path", Value::string("source.txt"))]),
                )],
            ),
        ),
        (
            401,
            Value::object([(
                "error",
                Value::object([("message", Value::string("Fixture rejected"))]),
            )]),
        ),
    ]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("Read the fixture.");
    key(&mut app, 13, 0);
    finish(&mut app);
    assert_eq!(app.state.status, "Turn interrupted");
    assert!(app.state.tool_details_available());
    key(&mut app, 79, 4);
    assert_eq!(flags(&app.state), [Some(true)]);
    key(&mut app, 79, 4);
    assert_eq!(flags(&app.state), [None]);
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn resume_keeps_one_execution_turn_across_accepted_follow_up_messages() {
    let directory = Directory::new();
    let home = Directory::new();
    let write = |id: &str| {
        tool_call(
            id,
            "write",
            Value::object([
                ("path", Value::string(format!("{id}.txt"))),
                ("content", Value::string(id)),
            ]),
        )
    };
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![write("first")])),
        (200, completion("", vec![write("second")])),
        (200, completion("Finished both requests.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let inbox = agent.inbox();
    agent
        .run_turn("Start the work.", &mut |event| {
            if matches!(&event, Event::ToolStarted { id, .. } if id == "first") {
                inbox.update(|entries| {
                    entries.push("Follow up within this turn.".into()).unwrap();
                    entries.record();
                });
            }
            Ok(())
        })
        .unwrap();
    let id = agent.sessions().unwrap().id();
    let saved = agent.sessions().unwrap().snapshot();
    assert_eq!(
        saved
            .messages
            .iter()
            .filter(|message| { message.get("role").and_then(Value::as_str) == Some("user") })
            .count(),
        2
    );
    assert_eq!(
        saved
            .events
            .iter()
            .filter(|event| { event.get("type").and_then(Value::as_str) == Some("turn_start") })
            .count(),
        1
    );
    drop(agent);
    let mut resumed = app(&directory, &fixture);
    resumed
        .agent
        .as_mut()
        .unwrap()
        .enable_sessions(home.path())
        .unwrap();
    resumed.persistence = resumed.agent.as_ref().unwrap().sessions();
    resumed.resume_session(&id);
    assert_eq!(flags(&resumed.state), [None, None]);
    key(&mut resumed, 79, 4);
    assert_eq!(flags(&resumed.state), [Some(true), Some(true)]);
    key(&mut resumed, 79, 4);
    assert_eq!(flags(&resumed.state), [None, None]);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|request| !request.body.encode().contains("turn_start"))
    );
}

#[test]
fn all_cards_in_a_turn_toggle_together_and_previous_turns_keep_their_previews() {
    let mut state = State::default();
    state.begin_tool_turn();
    tool(&mut state, "old", "one\ntwo\nthree\nfour\n");
    complete(&mut state);
    let previous = state.rows()[0].clone();
    state.begin_tool_turn();
    tool(&mut state, "first", "a\nb\nc\nd\n");
    state.message(
        Kind::User,
        "Additional input accepted during the same turn.",
    );
    state.message(Kind::Assistant, "Another model request.");
    tool(&mut state, "last", "e\nf\ng\nh\n");
    assert!(state.toggle_tools().is_none());
    complete(&mut state);
    let compact = state.rows();
    assert_eq!(state.toggle_tools(), Some((true, 1)));
    assert_eq!(flags(&state), [None, Some(true), Some(true)]);
    assert_eq!(state.rows()[0], previous);
    assert_eq!(state.toggle_tools(), Some((false, 1)));
    assert_eq!(state.rows(), compact);
}

#[test]
fn bulk_reflow_is_bounded_and_reset_cannot_leave_stale_expanded_cache_rows() {
    let mut state = State::default();
    state.begin_tool_turn();
    tool(&mut state, "earlier", "old head\n1\n2\nold tail\n");
    complete(&mut state);
    state.begin_tool_turn();
    for index in 0..80 {
        let output = (0..60)
            .map(|line| format!("Tool {index:02} line {line:02}\n"))
            .collect::<String>();
        tool(&mut state, &format!("current-{index}"), &output);
    }
    complete(&mut state);
    let mut layout = transcript::Layout::default();
    while !layout.ready(&state) {
        layout.prepare(&state, state.width, 8);
    }
    let old_block = layout.blocks[1].clone();
    state.toggle_tools();
    layout.prepare(&state, state.width, 1);
    assert!(!layout.ready(&state));
    assert_eq!(layout.blocks[1], old_block);
    while !layout.ready(&state) {
        layout.prepare(&state, state.width, 8);
    }
    let expanded = layout
        .blocks
        .iter()
        .flatten()
        .map(|row| row.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(expanded.contains("Tool 00 line 00") && expanded.contains("Tool 79 line 00"));
    // Two mutations before another draw must also invalidate the earlier range.
    state.begin_tool_turn();
    tool(&mut state, "newest", "new head\na\nb\nnew tail\n");
    complete(&mut state);
    state.toggle_tools();
    while !layout.ready(&state) {
        layout.prepare(&state, state.width, 8);
    }
    let text = layout
        .blocks
        .iter()
        .flatten()
        .map(|row| row.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("Tool 00 line 00") && !text.contains("Tool 79 line 00"));
    assert!(text.contains("new head"));
}
