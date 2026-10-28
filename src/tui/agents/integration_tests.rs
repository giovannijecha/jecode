use super::*;

#[test]
fn delayed_child_return_restarts_idle_main_without_a_user_message() {
    let project = Directory::new();
    let home = Directory::new();
    let main_fixture = HttpFixture::new(vec![
        (200, completion("Main finished while child works.", vec![])),
        (200, completion("Reviewed the child report.", vec![])),
    ]);
    let child_fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "Checking.",
                vec![tool_call(
                    "delay",
                    "bash",
                    Value::object([("command", Value::string("sleep 0.3"))]),
                )],
            ),
        ),
        (200, completion("Child report after check.", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(main_fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(home.path()).unwrap();
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let mut app = App::new(
        parent,
        SessionConfig {
            store: Store::new(home.path().join("config")),
            settings,
            bash: crate::tools::find_bash().unwrap(),
        },
        None,
    );
    app.state.editor.insert("Begin the main task");
    app.submit().unwrap();
    let started = Instant::now();
    while app.worker.is_some() {
        app.poll().unwrap();
        assert!(started.elapsed() < Duration::from_secs(6));
        thread::sleep(Duration::from_millis(5));
    }
    app.state.editor.insert("Unsent follow-up draft");
    app.state.editor.cursor = 7;
    let draft = app.state.editor.clone();
    let observer = app.agent.as_ref().unwrap().collaboration();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    observer
        .create(
            project.path(),
            crate::collaboration::Spec {
                title: "Delayed child".into(),
                instructions: "Check and report".into(),
                context: String::new(),
                mode: crate::collaboration::workspace::Mode::Shared,
            },
            |root| {
                Ok(Agent::new(
                    OpenRouter::fixture(child_fixture.endpoint.clone()),
                    Tools::new(root)?,
                ))
            },
            gate.clone(),
        )
        .unwrap();
    observer
        .receipt("created", None, 0, "Agents", vec![])
        .unwrap();
    {
        let (lock, ready) = &*gate;
        *lock.lock().unwrap() = true;
        ready.notify_all();
    }
    let mut saw_return = false;
    let mut restarted = false;
    while started.elapsed() < Duration::from_secs(8) {
        app.poll().unwrap();
        saw_return |= app.state.items.iter().any(|item| {
            matches!(item, Item::Collaboration { event, .. }
            if value(event, "kind") == "returned")
        });
        restarted |= saw_return && app.worker.is_some();
        if restarted && app.worker.is_none() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        saw_return && restarted,
        "idle main did not resume after child return"
    );
    assert_eq!(app.state.editor, draft);
    assert!(app.state.items.iter().any(
        |item| matches!(item, Item::Text { text, .. } if text == "Reviewed the child report.")
    ));
    assert_eq!(
        app.state
            .items
            .iter()
            .filter(|item| matches!(
                item,
                Item::Text {
                    kind: crate::tui::state::Kind::User,
                    ..
                }
            ))
            .count(),
        1
    );
    observer.stop_all();
    let requests = main_fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1]
            .body
            .encode()
            .contains("Child report after check.")
    );
    assert_eq!(child_fixture.finish().len(), 2);
}

#[test]
fn explicit_main_stop_disables_automatic_collaborator_continuation() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Stopped turn", vec![]))]);
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
    app.worker = Some(crate::tui::Worker::start(
        app.agent.take().unwrap(),
        crate::attachments::Prompt::plain("Stop me"),
    ));
    app.stop();
    app.show_collaboration(receipt(vec![record(1, "returned")]));
    assert!(!app.collaboration_auto_resume);
    assert!(!app.start_collaboration_continuation());
}

#[test]
fn visible_updates_and_failed_tools_remain_in_activity() {
    let mut agent = record(1, "running");
    if let Value::Object(fields) = &mut agent {
        fields.insert(
            "updates".into(),
            Value::Array(vec![
                Value::string("Found the first issue"),
                Value::string("Rechecked after the assignment changed"),
            ]),
        );
        fields.insert("updates_truncated".into(), Value::Bool(true));
        fields.insert(
            "tools".into(),
            Value::Array(vec![Value::object([
                ("name", Value::string("bash")),
                ("state", Value::string("failed")),
                ("text", Value::string("cargo test failed")),
            ])]),
        );
    }
    let rows = crate::tui::view::agents::detail_rows(&agent, 0, 80);
    let all = rows
        .iter()
        .map(crate::tui::line::Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    let first = all.find("Found the first issue").unwrap();
    let second = all.find("Rechecked after the assignment changed").unwrap();
    assert!(first < second);
    assert!(all.contains("Earlier updates omitted"));
    assert!(rows.iter().any(|row| {
        row.plain().contains("cargo test failed")
            && row
                .spans
                .iter()
                .any(|span| span.style == crate::tui::theme::BAD)
    }));
}

#[test]
fn failed_workspace_action_is_visible_without_a_generic_agent_card() {
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
    app.worker_event(crate::events::Event::ToolStarted {
        id: "agent-1".into(),
        name: "agent".into(),
        arguments: Value::object([("action", Value::string("review"))]),
    });
    app.worker_event(crate::events::Event::ToolFinished {
        id: "agent-1".into(),
        name: "agent".into(),
        summary: "Workspace review failed".into(),
        result: Value::object([("error", Value::string("Workspace review failed"))]),
    });
    assert!(
        !app.state
            .items
            .iter()
            .any(|item| matches!(item, Item::Tool { name, .. } if name == "agent"))
    );
    assert!(
        app.state
            .items
            .iter()
            .any(|item| matches!(item, Item::Text {
        kind: crate::tui::state::Kind::Error, text,
    } if text.contains("Workspace review failed")))
    );
}

#[test]
fn inspection_and_integration_actions_keep_their_tool_cards() {
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
    for (id, action) in [("diff-1", "diff"), ("integrate-1", "integrate")] {
        app.worker_event(crate::events::Event::ToolStarted {
            id: id.into(),
            name: "agent".into(),
            arguments: Value::object([
                ("action", Value::string(action)),
                ("id", Value::string("a1")),
            ]),
        });
        app.worker_event(crate::events::Event::ToolFinished {
            id: id.into(),
            name: "agent".into(),
            summary: format!("{action} complete"),
            result: Value::object([("checkpoint", Value::string("abc123"))]),
        });
    }
    let cards = app
        .state
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Tool {
                arguments, result, ..
            } => Some((
                arguments.get("action").and_then(Value::as_str).unwrap(),
                result.as_ref(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0].0, "diff");
    assert_eq!(cards[1].0, "integrate");
    assert_eq!(
        cards[1]
            .1
            .unwrap()
            .get("checkpoint")
            .and_then(Value::as_str),
        Some("abc123")
    );
    let rendered = app
        .state
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Tool {
                name,
                arguments,
                summary,
                result,
                change,
                presentation,
                ..
            } => Some(
                crate::tui::cards::card(
                    crate::tui::cards::Tool {
                        name,
                        arguments,
                        summary,
                        result: result.as_ref(),
                        change: change.as_deref(),
                        presentation,
                    },
                    80,
                    false,
                )
                .rows,
            ),
            _ => None,
        })
        .flatten()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("AGENTS"));
    assert!(rendered.contains("diff AGENT 1"));
    assert!(rendered.contains("integrate AGENT 1"));
}

#[test]
fn footer_distinguishes_runtime_failure_from_failed_check() {
    let mut state = State {
        width: 100,
        height: 24,
        collaboration: vec![record(1, "failed")],
        ..State::default()
    };
    let runtime = view::composer_frame(&state, "fixture/model", "fixture")
        .live
        .last()
        .unwrap()
        .plain();
    assert!(runtime.contains("1 failed") && !runtime.contains("failed check"));
    let mut check = record(1, "returned");
    if let Value::Object(fields) = &mut check {
        fields.insert(
            "checks".into(),
            Value::Array(vec![Value::string("cargo test failed")]),
        );
    }
    state.collaboration = vec![check];
    let result = view::composer_frame(&state, "fixture/model", "fixture")
        .live
        .last()
        .unwrap()
        .plain();
    assert!(result.contains("1 failed check"));
}
