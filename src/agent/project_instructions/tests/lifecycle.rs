use super::*;

#[test]
fn resumed_sessions_reload_current_rules_without_rewriting_original_history() {
    let directory = Directory::new();
    let home = Directory::new();
    fs::create_dir(directory.path().join("src")).unwrap();
    let path = directory.path().join("src").join(FILE_NAME);
    fs::write(&path, "Earlier project rules").unwrap();
    let fixture = HttpFixture::new(vec![
        (200, completion("First answer", vec![])),
        (200, completion("Resumed answer", vec![])),
    ]);
    let mut first = agent(&directory, &fixture);
    first.enable_sessions(home.path()).unwrap();
    first.run_turn("Original request", &mut |_| Ok(())).unwrap();
    let original = first.messages.lock().unwrap().clone();
    let id = first.sessions().unwrap().id();
    drop(first);
    fs::write(&path, "Current project rules").unwrap();
    let mut resumed = agent(&directory, &fixture);
    resumed.enable_sessions(home.path()).unwrap();
    resumed.project_instructions = "Stale cached rules".into();
    resumed.resume(&id).unwrap();
    assert!(resumed.project_instructions.is_empty());
    assert_eq!(*resumed.messages.lock().unwrap(), original);
    resumed.run_turn("Continue", &mut |_| Ok(())).unwrap();
    assert_eq!(resumed.messages.lock().unwrap()[..original.len()], original);
    assert_eq!(
        resumed.sessions().unwrap().snapshot().messages[0],
        original[0]
    );
    let requests = fixture.finish();
    assert!(current(&requests[0]).contains("Earlier project rules"));
    assert!(current(&requests[1]).contains("Current project rules"));
    assert!(!current(&requests[1]).contains("Earlier project rules"));
    assert!(!requests[1].body.encode().contains("Stale cached rules"));
    // The saved snapshot keeps its position; the resumed request extends the
    // earlier one and adds only the changed snapshot.
    let before = requests[0]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    let after = requests[1]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(after[..before.len()], *before);
    assert_eq!(crate::test_support::snapshot_count(&requests[1].body), 2);
    resumed.clear();
    assert!(resumed.project_instructions.is_empty());
    assert!(
        !resumed.messages.lock().unwrap()[0]
            .encode()
            .contains("Current project rules")
    );
}

#[test]
fn live_requests_keep_the_rules_after_context_compaction() {
    let directory = Directory::new();
    let home = Directory::new();
    fs::create_dir(directory.path().join("src")).unwrap();
    let instruction = directory.path().join("src").join(FILE_NAME);
    fs::write(&instruction, "Compaction project rules").unwrap();
    let fixture = HttpFixture::new(vec![
        (
            400,
            Value::object([(
                "error",
                Value::object([
                    ("code", Value::string("context_length_exceeded")),
                    ("message", Value::string("Maximum context length exceeded")),
                ]),
            )]),
        ),
        (
            200,
            completion(&String::from("Continue inspection"), vec![]),
        ),
        (200, completion("Continue inspection", vec![])),
        (200, completion("Finished", vec![])),
    ]);
    let mut agent = agent(&directory, &fixture);
    agent.enable_sessions(home.path()).unwrap();
    agent.set_auto_compact(90);
    let temporary = agent.tools.temporary_instructions();
    let prompt = "original objective ".repeat(2000);
    let original = agent.messages.lock().unwrap().clone();
    let mut notices = Vec::new();
    let result = agent.run_turn(&prompt, &mut |event| {
        if let Event::Maintenance { ref text } = event {
            notices.push(text.clone());
            fs::write(&instruction, "Changed during compaction").unwrap();
        }
        if matches!(event, Event::Recovering { .. }) {
            return Err("Unexpected fixture request".into());
        }
        Ok(())
    });
    let requests = fixture.finish();
    assert!(
        result.is_ok(),
        "{result:?}; tools: {:?}; notices: {notices:?}",
        requests
            .iter()
            .map(|request| request.body.get("tools").is_some())
            .collect::<Vec<_>>()
    );
    assert!(agent.context.from > 1);
    assert!(!notices.is_empty());
    assert!(!agent.context.summary.is_empty());
    assert_eq!(agent.messages.lock().unwrap()[..original.len()], original);
    assert_eq!(requests.len(), 4);
    for request in [&requests[0], &requests[3]] {
        assert!(current(request).contains("Compaction project rules"));
        assert!(!current(request).contains("Changed during compaction"));
        assert!(current(request).contains("Source: src/AGENTS.md"));
        assert!(
            request
                .body
                .get("messages")
                .unwrap()
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .get("content")
                .and_then(Value::as_str)
                .unwrap()
                .contains(&temporary)
        );
        assert!(system(request).contains("useful durable tests"));
        assert!(system(request).contains("one-off verification"));
        assert_eq!(
            system(request)
                .matches("Working files and verification:")
                .count(),
            1
        );
        assert_eq!(crate::test_support::snapshot_count(&request.body), 1);
    }
}

#[test]
fn new_sessions_do_not_save_the_previous_turns_rules_as_original_system_history() {
    let directory = Directory::new();
    let home = Directory::new();
    fs::write(directory.path().join(FILE_NAME), "Local project rules").unwrap();
    let fixture = HttpFixture::new(vec![(200, completion("Done", vec![]))]);
    let mut agent = agent(&directory, &fixture);
    agent.enable_sessions(home.path()).unwrap();
    let baseline = agent.messages.lock().unwrap()[0].clone();
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    assert!(!agent.project_instructions.is_empty());
    let previous = agent.sessions().unwrap().id();
    agent
        .start_new("fixture/model".into(), agent.effort())
        .unwrap();
    assert_ne!(agent.sessions().unwrap().id(), previous);
    assert!(agent.project_instructions.is_empty());
    assert_eq!(*agent.messages.lock().unwrap(), vec![baseline.clone()]);
    assert_eq!(
        agent.sessions().unwrap().snapshot().messages,
        vec![baseline.clone()]
    );
    agent.prepare_turn("New request").unwrap();
    assert_eq!(agent.sessions().unwrap().snapshot().messages[0], baseline);
    let transport = agent.transport_messages();
    assert!(!transport[0].encode().contains("Local project rules"));
    assert!(
        transport
            .iter()
            .any(|message| message.encode().contains("Local project rules"))
    );
    assert_eq!(fixture.finish().len(), 1);
}
