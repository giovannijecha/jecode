use super::*;

#[test]
fn a_worker_viewer_uses_credentials_updated_before_the_turn() {
    let directory = Directory::new();
    let key = "isolated-fixture-key";
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "wait-1",
                    "bash",
                    Value::object([(
                        "command",
                        Value::string("until [ -e finish-work ]; do sleep 0.02; done"),
                    )]),
                )],
            ),
        ),
        (200, completion("Finished.", vec![])),
    ]);
    let mut tools = Tools::new(directory.path()).unwrap();
    tools.configure_output(
        directory.path().join("logs"),
        crate::redact::Redactor::new("previous-fixture-key".into()),
    );
    let started = tools.execute(
        "bash",
        &Value::object([
            (
                "command",
                Value::string(format!(
                    "printf '%s' '{key}'; touch emitted-key; until [ -e stop-server ]; do sleep 0.02; done"
                )),
            ),
            ("wait_ms", Value::number(0)),
        ])
        .encode(),
    );
    let id = started.get("process_id").and_then(Value::as_str).unwrap();
    let mut app = App::new(
        Agent::new(
            OpenRouter::new("previous-fixture-key".into(), "fixture/model".into()).unwrap(),
            tools,
        ),
        tests::config(&directory),
        None,
    );
    app.state.width = 80;
    app.state.height = 30;
    // This is the same replacement used after a settings key change. Only
    // the loopback fixture client is ever asked to make a model request.
    app.agent
        .as_mut()
        .unwrap()
        .replace_client(OpenRouter::fixture(fixture.endpoint.clone()));
    submit(&mut app, "Continue");
    assert!(app.agent.is_none());
    wait_until(&mut app, |_| directory.path().join("emitted-key").exists());
    app.open_processes(id);
    let process = &panel(&app).processes[0];
    assert!(process.command.contains("[redacted]"));
    assert!(!process.command.contains(key));
    assert_eq!(logs(&app).views[0].text(), "[redacted]");
    std::fs::write(directory.path().join("finish-work"), "").unwrap();
    std::fs::write(directory.path().join("stop-server"), "").unwrap();
    wait_until(&mut app, |app| app.worker.is_none());
    assert_eq!(fixture.finish().len(), 2);
}
