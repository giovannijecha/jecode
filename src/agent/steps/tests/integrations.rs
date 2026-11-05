use super::*;

#[test]
fn json_text_arguments_keep_the_same_cards_after_resume() {
    let project = Directory::new();
    let home = Directory::new();
    fs::write(project.path().join("a.txt"), "old").unwrap();
    let files = r#"[{"path":"a.txt","edits":[{"old_text":"old","new_text":"new"}]}]"#;
    let edit = Value::object([("files", Value::string(files))]).encode();
    let write = r#"{"path":"b.txt","content":"text"}"#;
    let fixture = fixture(vec![
        Value::object([
            ("tool", Value::string("edit")),
            ("arguments", Value::string(edit)),
        ]),
        Value::object([
            ("tool", Value::string("write")),
            ("arguments", Value::string(write)),
        ]),
    ]);
    let mut running = agent(&project, &fixture);
    running.enable_sessions(home.path()).unwrap();
    let mut live = Vec::new();
    running
        .run_turn("Do it", &mut |event| {
            if let crate::events::Event::ToolStarted {
                id,
                name,
                arguments,
            } = event
            {
                live.push((id, name, arguments));
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(
        fs::read_to_string(project.path().join("a.txt")).unwrap(),
        "new"
    );
    assert_eq!(
        fs::read_to_string(project.path().join("b.txt")).unwrap(),
        "text"
    );
    let id = running.sessions().unwrap().id();
    drop(running);

    let mut resumed = agent(&project, &fixture);
    resumed.enable_sessions(home.path()).unwrap();
    resumed.resume(&id).unwrap();
    let restored = resumed
        .sessions()
        .unwrap()
        .snapshot()
        .records()
        .into_iter()
        .filter_map(|record| match record {
            crate::sessions::Record::Tool {
                id,
                name,
                arguments,
                ..
            } => Some((id, name, arguments)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        restored, live,
        "resume must show the calls that actually ran"
    );
}

#[test]
fn an_mcp_success_continues_and_a_server_error_stops_the_rest() {
    for is_error in [false, true] {
        let project = Directory::new();
        let home = Directory::new();
        let rpc = |id, result| {
            (
                200,
                Value::object([
                    ("jsonrpc", Value::string("2.0")),
                    ("id", Value::number(id)),
                    ("result", result),
                ]),
            )
        };
        let remote = HttpFixture::new(vec![
            rpc(
                1,
                Value::object([(
                    "supportedVersions",
                    Value::Array(vec![Value::string("2026-07-28")]),
                )]),
            ),
            rpc(
                2,
                Value::object([(
                    "tools",
                    Value::Array(vec![Value::object([
                        ("name", Value::string("fixture")),
                        (
                            "inputSchema",
                            Value::object([("type", Value::string("object"))]),
                        ),
                    ])]),
                )]),
            ),
            rpc(
                3,
                Value::object([
                    ("content", Value::Array(vec![])),
                    ("isError", Value::Bool(is_error)),
                ]),
            ),
        ]);
        let connection = crate::mcp::Store::new(home.path().to_path_buf())
            .save(&crate::mcp::Connection::draft("Fixture", &remote.endpoint))
            .unwrap();
        let fixture = fixture(vec![
            step(
                "mcp",
                Value::object([
                    ("action", Value::string("call")),
                    ("connection", Value::string(&connection.id)),
                    ("tool", Value::string("fixture")),
                ]),
            ),
            write("after.txt", "finished"),
        ]);
        let mut agent = agent(&project, &fixture);
        agent.configure_mcp(home.path()).unwrap();
        agent.run_turn("Do it", &mut |_| Ok(())).unwrap();
        let results = results(&agent);
        let steps = entries(&results[0].1);
        assert_eq!(
            steps[0]
                .get("result")
                .unwrap()
                .get("outcome")
                .and_then(Value::as_str),
            Some(if is_error { "failed" } else { "completed" })
        );
        assert_eq!(
            project.path().join("after.txt").exists(),
            !is_error,
            "{steps:?}"
        );
        assert_eq!(remote.finish().len(), 3);
    }
}

#[test]
fn an_unavailable_lsp_query_stops_the_rest() {
    let project = Directory::new();
    let home = Directory::new();
    fs::write(project.path().join("sample.rs"), "fn sample() {}\n").unwrap();
    let fixture = fixture(vec![
        step(
            "lsp",
            Value::object([
                ("action", Value::string("definition")),
                ("path", Value::string("sample.rs")),
                ("line", Value::number(1)),
                ("column", Value::number(1)),
            ]),
        ),
        write("after.txt", "no"),
    ]);
    let mut agent = agent(&project, &fixture);
    agent.configure_lsp(home.path()).unwrap();
    agent.run_turn("Do it", &mut |_| Ok(())).unwrap();
    let results = results(&agent);
    let steps = entries(&results[0].1);
    assert_eq!(
        steps[0]
            .get("result")
            .unwrap()
            .get("status")
            .and_then(Value::as_str),
        Some("unavailable")
    );
    assert_eq!(
        steps[1].get("outcome").and_then(Value::as_str),
        Some("not_run")
    );
    assert!(!project.path().join("after.txt").exists());
}

#[test]
fn a_background_command_timeout_stops_the_rest() {
    let project = Directory::new();
    let fixture = fixture(vec![
        step(
            "bash",
            Value::object([
                ("command", Value::string("while :; do sleep 0.02; done")),
                ("wait_ms", Value::number(2000)),
                ("timeout_seconds", Value::number(1)),
            ]),
        ),
        write("after.txt", "no"),
    ]);
    let mut agent = agent(&project, &fixture);
    agent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    agent.run_turn("Do it", &mut |_| Ok(())).unwrap();
    let results = results(&agent);
    let steps = entries(&results[0].1);
    assert_eq!(
        steps[0]
            .get("result")
            .unwrap()
            .get("status")
            .and_then(Value::as_str),
        Some("timed_out")
    );
    assert_eq!(
        steps[1].get("outcome").and_then(Value::as_str),
        Some("not_run")
    );
    assert!(!project.path().join("after.txt").exists());
}
