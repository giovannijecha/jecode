use super::*;

#[test]
fn own_history_requires_a_settled_document() {
    let project = Directory::new();
    let mut document = crate::sessions::Document::new(
        project.path().to_path_buf(),
        "fixture/model".into(),
        crate::effort::Effort::Default,
    )
    .unwrap();
    assert!(require_settled_document(&document).is_ok());

    document.pending.active = true;
    assert!(require_settled_document(&document).is_err());
    document.pending.active = false;

    document.input.question = Some(crate::ask::Draft {
        id: "ask-1".into(),
        ..Default::default()
    });
    assert!(require_settled_document(&document).is_err());
    document.input.question = None;

    document
        .input
        .queued
        .push(crate::attachments::Prompt::plain("unsent request"));
    assert!(require_settled_document(&document).is_err());
}

#[test]
fn own_history_poll_does_not_continue_a_returned_collaborator() {
    let project = Directory::new();
    let home = Directory::new();
    let agent = || {
        let mut agent = Agent::new(
            OpenRouter::fixture("http://127.0.0.1:1".into()),
            Tools::new(project.path()).unwrap(),
        );
        agent.enable_sessions(home.path()).unwrap();
        agent
    };
    let saved = agent();
    let workspace = crate::collaboration::workspace::Workspace::prepare(
        project.path(),
        home.path(),
        "a1",
        crate::collaboration::workspace::Mode::Shared,
    )
    .unwrap();
    let record = Value::object([
        ("id", Value::string("a1")),
        ("number", Value::number(1)),
        ("state", Value::string("returned")),
        ("workspace", Value::string("shared")),
        ("workspace_info", workspace.value()),
        ("sent_version", Value::number(1)),
        (
            "report",
            Value::object([
                ("version", Value::number(1)),
                ("text", Value::string("Synthetic report")),
            ]),
        ),
    ]);
    saved.archive().events.lock().unwrap().push(Value::object([
        ("type", Value::string("collaboration")),
        ("kind", Value::string("returned")),
        ("sequence", Value::number(1)),
        ("id", Value::string("a1")),
        ("records", Value::Array(vec![record])),
        ("after_message", Value::number(0)),
    ]));
    saved.save_session().unwrap();
    let id = saved.sessions().unwrap().id();
    drop(saved);

    let store = crate::sessions::Store::new(home.path().to_path_buf(), project.path()).unwrap();
    assert!(require_settled_journal(&store, &id).is_ok());
    let mut app = App::new(agent(), super::super::tests::config(&home), None);
    resume_for_measurement(&mut app, &id).unwrap();
    assert!(app.collaboration_return_pending);
    assert!(app.agent.as_ref().unwrap().can_continue_collaboration());
    app.poll().unwrap();
    assert!(app.worker.is_none());
    assert!(app.agent.is_some());
}
