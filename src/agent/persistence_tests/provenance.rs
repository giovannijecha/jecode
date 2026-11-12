use super::*;

fn runtime_events(agent: &Agent) -> Vec<Value> {
    agent
        .sessions()
        .unwrap()
        .snapshot()
        .events
        .into_iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("runtime_identity"))
        .collect()
}

#[test]
fn new_and_resumed_sessions_record_one_runtime_per_activation() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Done", vec![]))]);
    let mut first = agent(&home, &directory, &fixture.endpoint);
    assert!(
        first
            .sessions()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .is_empty()
    );
    let first_runtime = runtime_events(&first).pop().unwrap();
    assert_eq!(
        first_runtime.get("jecode_version").and_then(Value::as_str),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert!(
        first_runtime
            .get("at_unix_ms")
            .and_then(Value::as_usize)
            .is_some()
    );
    assert_eq!(first_runtime.get("build"), Some(&crate::identity::value()));
    first.run_turn("First request", &mut |_| Ok(())).unwrap();
    first.save_session().unwrap();
    first.save_session().unwrap();
    assert_eq!(runtime_events(&first), vec![first_runtime.clone()]);
    let id = first.sessions().unwrap().id();
    drop(first);

    let mut second = agent(&home, &directory, &fixture.endpoint);
    second.resume(&id).unwrap();
    let activations = runtime_events(&second);
    assert_eq!(activations.len(), 2);
    assert_eq!(activations[0], first_runtime);
    assert_eq!(activations[1].get("build"), Some(&crate::identity::value()));
    second.save_session().unwrap();
    assert_eq!(runtime_events(&second), activations);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn runtime_receipts_never_enter_provider_projection_or_cache_prefix() {
    let home = Directory::new();
    let directory = Directory::new();
    let agent = agent(&home, &directory, "http://127.0.0.1:1");
    let with_provenance = agent.transport_messages();
    assert_eq!(runtime_events(&agent).len(), 1);
    agent.events.lock().unwrap().clear();
    let without_provenance = agent.transport_messages();
    assert_eq!(with_provenance, without_provenance);
    assert_eq!(
        with_provenance[0].get("jecode_cache_prefix"),
        without_provenance[0].get("jecode_cache_prefix")
    );
}

#[test]
fn new_selection_and_post_delete_replacement_get_their_own_runtime() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("First answer", vec![])),
        (200, completion("Second answer", vec![])),
    ]);
    let mut agent = agent(&home, &directory, &fixture.endpoint);
    agent.run_turn("First request", &mut |_| Ok(())).unwrap();
    agent
        .start_new("fixture/model".into(), Effort::High)
        .unwrap();
    assert_eq!(runtime_events(&agent).len(), 1);
    agent.run_turn("Second request", &mut |_| Ok(())).unwrap();
    let id = agent.sessions().unwrap().id();
    let (_, replaced) = agent
        .delete_session(&id, "fixture/model".into(), Effort::High)
        .unwrap();
    assert!(replaced);
    assert_eq!(runtime_events(&agent).len(), 1);
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn failed_resume_runtime_receipt_keeps_the_active_conversation() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut current = agent(&home, &directory, "http://127.0.0.1:1");
    for (role, content) in [("user", "Target request"), ("assistant", "Target answer")] {
        current.messages.lock().unwrap().push(Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(content)),
        ]));
    }
    current.save_session().unwrap();
    let target_id = current.sessions().unwrap().id();
    current
        .start_new("fixture/model".into(), Effort::High)
        .unwrap();
    for (role, content) in [("user", "Active request"), ("assistant", "Active answer")] {
        current.messages.lock().unwrap().push(Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(content)),
        ]));
    }
    current.save_session().unwrap();
    let active_id = current.sessions().unwrap().id();
    let active_messages = current.messages.lock().unwrap().clone();
    let store = current.sessions().unwrap().store();
    let journal = store.fixture_journal(&target_id);
    let original_permissions = std::fs::metadata(&journal).unwrap().permissions();

    let error = current
        .resume_with_before_activation(&target_id, || {
            let mut readonly = original_permissions.clone();
            readonly.set_readonly(true);
            std::fs::set_permissions(&journal, readonly).unwrap();
        })
        .unwrap_err();
    std::fs::set_permissions(&journal, original_permissions).unwrap();
    assert!(error.contains("read-only"));
    assert_eq!(current.sessions().unwrap().id(), active_id);
    assert_eq!(*current.messages.lock().unwrap(), active_messages);
    assert_eq!(runtime_events(&current).len(), 1);
    let target = store.fixture_load(&target_id).unwrap();
    assert_eq!(
        target
            .events
            .iter()
            .filter(|event| event.get("type").and_then(Value::as_str) == Some("runtime_identity"))
            .count(),
        1
    );
}
