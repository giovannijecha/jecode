use super::*;
use crate::{
    sessions::{Draft, Input},
    test_support::{Directory, HttpFixture, completion, tool_call},
};

fn agent(home: &Directory, directory: &Directory, endpoint: &str) -> Agent {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent
}

#[test]
fn resuming_retains_model_boundaries_raw_evidence_and_never_reexecutes_saved_tools() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "Inspecting",
                vec![tool_call(
                    "command",
                    "bash",
                    Value::object([(
                        "command",
                        Value::string("printf 'once\\n' >> executions.txt"),
                    )]),
                )],
            ),
        ),
        (200, completion("Before restart", vec![])),
        (200, completion("After restart", vec![])),
    ]);
    let mut first = agent(&home, &directory, &fixture.endpoint);
    first.run_turn("First task", &mut |_| Ok(())).unwrap();
    first.set_model("fixture/other".into()).unwrap();
    first.set_effort(Effort::Low);
    first.record_local("/model fixture/other", "Model changed");
    first.save_session().unwrap();
    let id = first.sessions().unwrap().id();
    assert!(
        first
            .sessions()
            .unwrap()
            .snapshot()
            .value()
            .encode()
            .contains("fixture-reasoning")
    );
    drop(first);
    let mut resumed = agent(&home, &directory, &fixture.endpoint);
    assert_eq!(resumed.archive().messages.lock().unwrap().len(), 1);
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.model(), "fixture/other");
    assert_eq!(resumed.effort(), Effort::Low);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("executions.txt")).unwrap(),
        "once\n"
    );
    resumed.run_turn("Continue", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    let body = requests[2].body.encode();
    assert!(body.contains("Before restart"));
    assert!(body.contains("First task"));
    assert!(body.contains("tool_call_id"));
    assert!(!body.contains("fixture-reasoning"));
    assert!(!body.contains("Model changed"));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("executions.txt")).unwrap(),
        "once\n"
    );
}

#[test]
fn unchanged_model_keeps_provider_fields_and_new_is_a_separate_saved_session() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("Previous answer", vec![])),
        (200, completion("Continued answer", vec![])),
    ]);
    let mut first = agent(&home, &directory, &fixture.endpoint);
    first.run_turn("Previous task", &mut |_| Ok(())).unwrap();
    let id = first.sessions().unwrap().id();
    drop(first);
    let mut resumed = agent(&home, &directory, &fixture.endpoint);
    resumed.resume(&id).unwrap();
    resumed.run_turn("Continue", &mut |_| Ok(())).unwrap();
    let requests = fixture.finish();
    assert!(requests[1].body.encode().contains("fixture-reasoning"));
    resumed
        .start_new("fixture/default".into(), Effort::Default)
        .unwrap();
    assert_ne!(resumed.sessions().unwrap().id(), id);
    assert_eq!(resumed.archive().messages.lock().unwrap().len(), 1);
    assert_eq!(resumed.model(), "fixture/default");
    let store = resumed.sessions().unwrap().store();
    assert!(
        store
            .list()
            .unwrap()
            .sessions
            .iter()
            .any(|session| session.id == id)
    );
    assert!(
        !resumed
            .archive()
            .document()
            .encode()
            .contains("Previous answer")
    );
}

#[test]
fn ready_recovery_does_not_submit_queued_commands_or_an_unfinished_user_request() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut first = agent(&home, &directory, "http://127.0.0.1:1/chat/completions");
    first.prepare_turn("An unfinished request").unwrap();
    let handle = first.sessions().unwrap();
    handle.input(Input {
        queued: vec!["/model fixture/queued".into(), "next task".into()],
        draft: Draft {
            text: "è draft".into(),
            cursor: 2,
            ..Default::default()
        },
        ..Input::default()
    });
    handle.flush().unwrap();
    let id = handle.id();
    drop(handle);
    drop(first);
    let mut resumed = agent(&home, &directory, "http://127.0.0.1:1/chat/completions");
    resumed.resume(&id).unwrap();
    assert_eq!(resumed.model(), "fixture/model");
    let document = resumed.sessions().unwrap().snapshot();
    assert_eq!(document.messages.len(), 2);
    assert_eq!(document.input.draft.text, "è draft");
    assert_eq!(document.input.draft.cursor, 2);
    assert_eq!(
        document
            .input
            .paused
            .iter()
            .map(|draft| draft.text.as_str())
            .collect::<Vec<_>>(),
        ["/model fixture/queued", "next task"]
    );
    assert!(document.input.queued.is_empty());
    assert!(!document.pending.active);
    assert!(resumed.prepared.is_none());
}

#[test]
fn known_results_survive_crash_between_tool_result_and_turn_completion() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        completion(
            "",
            vec![tool_call(
                "write",
                "write",
                Value::object([
                    ("path", Value::string("saved.txt")),
                    ("content", Value::string("written")),
                ]),
            )],
        ),
    )]);
    let mut first = agent(&home, &directory, &fixture.endpoint);
    first
        .run_turn("Write", &mut |event| {
            if matches!(event, Event::ToolFinished { .. }) {
                Err("isolated output closed".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    let id = first.sessions().unwrap().id();
    drop(first);
    let mut resumed = agent(&home, &directory, &fixture.endpoint);
    resumed.resume(&id).unwrap();
    let document = resumed.sessions().unwrap().snapshot();
    let content = document
        .messages
        .last()
        .unwrap()
        .get("content")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(content.contains("bytes_written"));
    assert!(!content.contains("unknown"));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("saved.txt")).unwrap(),
        "written"
    );
    assert!(!document.value().encode().contains("isolated-fixture-key"));
    fixture.finish();
}

#[test]
fn storage_failure_before_tool_start_prevents_the_side_effect_and_keeps_a_valid_context() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        completion(
            "",
            vec![tool_call(
                "write",
                "write",
                Value::object([
                    ("path", Value::string("must-not-exist")),
                    ("content", Value::string("no")),
                ]),
            )],
        ),
    )]);
    let mut first = agent(&home, &directory, &fixture.endpoint);
    first.prepare_turn("Write").unwrap();
    let handle = first.sessions().unwrap();
    let id = handle.id();
    let bucket = std::fs::read_dir(home.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let path = bucket.join(format!("{id}.jsonl"));
    let original_permissions = std::fs::metadata(&path).unwrap().permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    assert!(first.run_turn("Write", &mut |_| Ok(())).is_err());
    assert!(!directory.path().join("must-not-exist").exists());
    // Restore fixture permissions portably before cleanup and retry the save.
    std::fs::set_permissions(&path, original_permissions).unwrap();
    first.save_session().unwrap();
    assert!(crate::sessions::Document::parse(&handle.snapshot().value()).is_ok());
    fixture.finish();
}

#[test]
fn the_first_streamed_text_is_on_disk_before_the_display_event() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::streaming(vec![crate::test_support::Response::Stream(vec![(
        std::time::Duration::ZERO,
        "data: {\"choices\":[{\"delta\":{\"content\":\"Saved partial prefix\"},\"finish_reason\":null}]}\n\n".into(),
    )])]);
    let mut current = agent(&home, &directory, &fixture.endpoint);
    let id = current.sessions().unwrap().id();
    let mut saw_text = false;
    current
        .run_turn("Stream the fixture", &mut |event| {
            if matches!(event, Event::Streaming { .. }) {
                let bucket = std::fs::read_dir(home.path().join("sessions"))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                assert!(bucket.join(format!("{id}.jsonl")).exists());
                let doc = crate::sessions::Store::new(home.path().to_path_buf(), directory.path())
                    .unwrap()
                    .fixture_load(&id)
                    .unwrap();
                assert_eq!(doc.pending.partial, "Saved partial prefix");
                assert_eq!(doc.messages.len(), 2);
                saw_text = true;
            }
            if matches!(event, Event::Recovering { .. }) {
                return Err("fixture stops before reconnecting".into());
            }
            Ok(())
        })
        .unwrap_err();
    assert!(saw_text);
    let doc = current.sessions().unwrap().snapshot();
    assert!(!doc.pending.active);
    assert!(
        doc.events
            .iter()
            .any(|event| event.get("partial_text").and_then(Value::as_str)
                == Some("Saved partial prefix"))
    );
    fixture.finish();
}

#[test]
fn failed_preparation_restores_ready_state_and_retry_submits_the_prompt_once() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Retried successfully", vec![]))]);
    let mut current = agent(&home, &directory, &fixture.endpoint);
    for (role, content) in [("user", "Earlier request"), ("assistant", "Earlier answer")] {
        current
            .archive()
            .messages
            .lock()
            .unwrap()
            .push(Value::object([
                ("role", Value::string(role)),
                ("content", Value::string(content)),
            ]));
    }
    current.save_session().unwrap();
    let handle = current.sessions().unwrap();
    let bucket = std::fs::read_dir(home.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let path = bucket.join(format!("{}.jsonl", handle.id()));
    let before = std::fs::read(&path).unwrap();
    let original = std::fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&path, readonly).unwrap();
    let failed = current.prepare_turn("Retry this request");
    std::fs::set_permissions(&path, original).unwrap();
    assert!(failed.is_err());
    assert!(current.prepared.is_none());
    assert!(!handle.active());
    assert_eq!(current.archive().messages.lock().unwrap().len(), 3);
    assert_eq!(handle.snapshot().messages.len(), 3);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    current.save_session().unwrap();
    current.clean_temporary().unwrap();
    current
        .run_turn("Retry this request", &mut |_| Ok(()))
        .unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    let messages = requests[0]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.get("content").and_then(Value::as_str)
                == Some("Retry this request"))
            .count(),
        1
    );
    assert!(!handle.active());
}
