use crate::{
    agent::Agent,
    effort::Effort,
    events::Event,
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::fs;

fn agent(home: &Directory, project: &Directory, endpoint: &str) -> Agent {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
}
fn temporary_call(name: &str, id: &str) -> Value {
    tool_call(
        id,
        name,
        Value::object([
            ("path", Value::string("tmp:probes/check.txt")),
            ("content", Value::string("before\n")),
        ]),
    )
}

fn saved_exchange(agent: &Agent) {
    for (role, content) in [("user", "Earlier request"), ("assistant", "Earlier answer")] {
        agent.messages.lock().unwrap().push(Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(content)),
        ]));
    }
    agent.save_session().unwrap();
}

#[test]
fn resumed_and_new_sessions_keep_their_own_working_files_and_complete_outputs() {
    let home = Directory::new();
    let project = Directory::new();
    let other_folder = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![
                    temporary_call("write", "create"),
                    tool_call(
                        "run",
                        "bash",
                        Value::object([(
                            "command",
                            Value::string("cat \"$JECODE_TMP/probes/check.txt\""),
                        )]),
                    ),
                ],
            ),
        ),
        (200, completion("Files ready", vec![])),
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "read",
                    "read",
                    Value::object([("path", Value::string("tmp:probes/check.txt"))]),
                )],
            ),
        ),
        (200, completion("Working file reused", vec![])),
    ]);
    let mut first = agent(&home, &project, &fixture.endpoint);
    first
        .run_turn("Make a temporary working file", &mut |_| Ok(()))
        .unwrap();
    let id = first.sessions().unwrap().id();
    let path = first.temporary_info().unwrap().path;
    let saved_history = first.archive().document();
    drop(first);
    let mut foreign = agent(&home, &other_folder, &fixture.endpoint);
    assert!(foreign.resume(&id).is_err());
    let mut resumed = agent(&home, &project, &fixture.endpoint);
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.temporary_info().unwrap().path, path);
    assert_eq!(resumed.temporary_info().unwrap().files, 1);
    assert_eq!(
        resumed.archive().document().get("messages"),
        saved_history.get("messages")
    );
    resumed
        .run_turn("Reuse that file", &mut |_| Ok(()))
        .unwrap();
    let before = resumed.archive().messages.lock().unwrap().len();
    let removed = resumed.clean_temporary().unwrap();
    assert_eq!(removed.files, 1);
    assert!(
        !std::path::Path::new(&path)
            .join("probes/check.txt")
            .exists()
    );
    assert_eq!(resumed.archive().messages.lock().unwrap().len(), before);
    assert!(resumed.transport_messages().iter().any(|message| {
        message
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("Earlier tmp: file references no longer exist"))
    }));
    let original_stdout = saved_history
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .map(|message| {
            crate::json::parse(message.get("content").unwrap().as_str().unwrap()).unwrap()
        })
        .find_map(|result| {
            result
                .get("stdout_file")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap();
    let output = resumed.tools.execute(
        "read",
        &Value::object([("path", Value::string(original_stdout))]).encode(),
    );
    assert_eq!(
        output.get("content").and_then(Value::as_str),
        Some("1: before\n")
    );
    resumed
        .start_new("fixture/model".into(), Effort::Default)
        .unwrap();
    assert_ne!(resumed.temporary_info().unwrap().path, path);
    assert_eq!(resumed.temporary_info().unwrap().files, 0);
    assert!(
        resumed
            .tools
            .execute(
                "read",
                &Value::object([("path", Value::string("tmp:probes/check.txt"))]).encode()
            )
            .get("error")
            .is_some()
    );
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.temporary_info().unwrap().path, path);
    assert_eq!(fs::read_dir(project.path()).unwrap().count(), 0);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    for request in requests {
        let system = request.body.get("messages").unwrap().as_array().unwrap()[0]
            .get("content")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(system.contains("tmp:relative/path"));
        assert!(system.contains("JECODE_TMP"));
        assert!(system.contains(&path));
    }
}

#[test]
fn interruption_retains_temporary_files_without_automatically_repeating_tools() {
    let home = Directory::new();
    let project = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        completion("", vec![temporary_call("write", "write")]),
    )]);
    let mut first = agent(&home, &project, &fixture.endpoint);
    first
        .run_turn("Write then interrupt", &mut |event| {
            if matches!(event, Event::ToolFinished { .. }) {
                Err("fixture output interrupted".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    let id = first.sessions().unwrap().id();
    let path = first.temporary_info().unwrap().path;
    drop(first);
    let mut resumed = agent(&home, &project, &fixture.endpoint);
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.temporary_info().unwrap().path, path);
    assert_eq!(resumed.temporary_info().unwrap().files, 1);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn legacy_system_messages_get_current_tool_paths_without_rewriting_saved_evidence() {
    let home = Directory::new();
    let project = Directory::new();
    let mut first = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    let original = Value::object([
        ("role", Value::string("system")),
        (
            "content",
            Value::string("Legacy project-only file instructions"),
        ),
    ]);
    first.messages.lock().unwrap()[0] = original.clone();
    first.enable_sessions(home.path()).unwrap();
    saved_exchange(&first);
    first.record_local("/help", "Legacy conversation");
    first.save_session().unwrap();
    let id = first.sessions().unwrap().id();
    drop(first);
    let mut resumed = agent(&home, &project, "http://127.0.0.1:1/chat/completions");
    resumed.resume(&id).unwrap();
    resumed.context.summary = "Remember the original task and tmp:probe.txt".into();
    resumed.context.from = resumed.messages.lock().unwrap().len();
    let request = resumed.transport_messages();
    assert!(
        request[0]
            .get("content")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("tmp:relative/path")
    );
    assert!(
        request[1]
            .get("content")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("Remember the original task")
    );
    assert_eq!(resumed.messages.lock().unwrap()[0], original);
    assert!(
        resumed.context_estimate(&resumed.messages.lock().unwrap())
            >= resumed.tools.temporary_instructions().len()
    );
}

#[test]
fn cleanup_waits_for_ready_state_and_durable_save_before_removing_any_file() {
    let home = Directory::new();
    let project = Directory::new();
    let mut current = agent(&home, &project, "http://127.0.0.1:1/chat/completions");
    let result = current.tools.execute(
        "write",
        &Value::object([
            ("path", Value::string("tmp:keep.txt")),
            ("content", Value::string("keep")),
        ])
        .encode(),
    );
    assert!(result.get("error").is_none());
    current.prepare_turn("Prepared but not run").unwrap();
    assert!(current.clean_temporary().unwrap_err().contains("ready"));
    assert_eq!(current.temporary_info().unwrap().files, 1);
    current.prepared = None;
    current.checkpoint(crate::sessions::Stage::Ready).unwrap();
    let id = current.sessions().unwrap().id();
    let bucket = fs::read_dir(home.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let journal = bucket.join(format!("{id}.jsonl"));
    let original = fs::metadata(&journal).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&journal, readonly).unwrap();
    current.record_local("fixture", "Unsaved state");
    let result = current.clean_temporary();
    fs::set_permissions(journal, original).unwrap();
    assert!(result.is_err());
    assert_eq!(current.temporary_info().unwrap().files, 1);
}

#[test]
fn an_unfinished_cleanup_record_survives_restart_and_warns_the_next_model_request() {
    let home = Directory::new();
    let project = Directory::new();
    let mut current = agent(&home, &project, "http://127.0.0.1:1/chat/completions");
    saved_exchange(&current);
    let path = current.temporary_info().unwrap().path;
    let file = std::path::Path::new(&path).join("probe.txt");
    fs::write(&file, "scratch").unwrap();
    current.record_temporary_cleanup("Cleanup started and may be incomplete; inspect tmp: files before relying on earlier references.");
    current.save_session().unwrap();
    let id = current.sessions().unwrap().id();
    // Simulate shutdown after removal and before recording completion.
    fs::remove_file(file).unwrap();
    drop(current);
    let mut resumed = agent(&home, &project, "http://127.0.0.1:1/chat/completions");
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.temporary_info().unwrap().files, 0);
    assert!(
        resumed.transport_messages()[0]
            .get("content")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("Cleanup started and may be incomplete")
    );
    assert_eq!(resumed.sessions().unwrap().snapshot().records().len(), 2);
}
