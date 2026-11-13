use super::*;

#[test]
fn quitting_with_a_question_restores_both_drafts_and_continues_the_same_tool_call() {
    let directory = Directory::new();
    let home = Directory::new();
    let first_fixture = HttpFixture::new(vec![(
        200,
        completion("", vec![tool_call("ask-1", "ask_user", question_args())]),
    )]);
    let mut first_agent = Agent::new(
        OpenRouter::fixture(first_fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    first_agent.enable_sessions(home.path()).unwrap();
    let mut first = App::new(first_agent, config(&directory), None);
    submit(&mut first, "Export the report");
    wait_until(&mut first, |app| app.state.question.is_some());
    first.state.editor.insert("Keep existing shortcuts");
    first.state.editor.cursor = 5;
    first
        .input(Decoded::Text("Keep the date filter".into()))
        .unwrap();
    first.state.ask_panel.as_mut().unwrap().entries[0]
        .editor
        .cursor = 4;
    first.close_ask();
    first.open_ask();
    let id = first.persistence.as_ref().unwrap().id();
    assert!(!first.input(Decoded::Key(key(81, 4))).unwrap());
    assert!(first.state.quitting.is_some());
    assert!(first.input(Decoded::Key(key(13, 0))).unwrap());
    assert!(!first.state.activity.as_ref().unwrap().stopping);
    drop(first);
    assert_eq!(first_fixture.finish().len(), 1);

    let next_fixture = HttpFixture::new(vec![(
        200,
        completion("I will export visible rows.", vec![]),
    )]);
    let mut next_agent = Agent::new(
        OpenRouter::fixture(next_fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    next_agent.enable_sessions(home.path()).unwrap();
    let mut resumed = App::new(next_agent, config(&directory), None);
    resumed.resume_session(&id);
    assert!(resumed.state.question.is_some());
    assert_eq!(resumed.state.editor.text, "Keep existing shortcuts");
    assert_eq!(resumed.state.editor.cursor, 5);
    let entry = resumed.state.ask_panel.as_ref().unwrap().entry();
    assert_eq!(entry.editor.text, "Keep the date filter");
    assert_eq!((entry.editor.cursor, entry.row, entry.other), (4, 2, true));
    resumed.poll().unwrap(); // The resumed worker may re-emit UserQuestion.
    let entry = resumed.state.ask_panel.as_ref().unwrap().entry();
    assert_eq!(entry.editor.cursor, 4);
    resumed.input(Decoded::Key(key(13, 0))).unwrap();
    finish(&mut resumed);
    assert!(resumed.state.question.is_none());
    assert_eq!(resumed.state.editor.cursor, 5);
    assert_eq!(
        ask_result(&resumed)
            .unwrap()
            .get("answers")
            .map(Value::encode),
        Some(r#"[{"choices":[],"id":"scope","text":"Keep the date filter"}]"#.into())
    );
    assert_eq!(next_fixture.finish().len(), 1);
}

#[test]
fn explicit_stop_before_quit_cancels_the_question_instead_of_suspending_it() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        completion("", vec![tool_call("ask-1", "ask_user", question_args())]),
    )]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, config(&directory), None);
    submit(&mut app, "Export the report");
    wait_until(&mut app, |app| app.state.question.is_some());
    let id = app.persistence.as_ref().unwrap().id();
    app.input(Decoded::Key(key(67, 4))).unwrap();
    assert!(app.state.activity.as_ref().unwrap().stopping);
    drop(app);
    assert_eq!(fixture.finish().len(), 1);

    let resumed_fixture = HttpFixture::new(vec![]);
    let mut resumed_agent = Agent::new(
        OpenRouter::fixture(resumed_fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed_agent.enable_sessions(home.path()).unwrap();
    let mut resumed = App::new(resumed_agent, config(&directory), None);
    resumed.resume_session(&id);
    assert!(resumed.state.question.is_none());
    assert_eq!(
        ask_result(&resumed)
            .and_then(|result| result.get("outcome"))
            .and_then(Value::as_str),
        Some("cancelled")
    );
    drop(resumed);
    assert!(resumed_fixture.finish().is_empty());
}
