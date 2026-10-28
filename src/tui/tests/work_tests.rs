use super::*;
use crate::{
    ask::Request,
    tui::{keys::Decoded, terminal::Key},
};

fn key(code: u16, modifiers: u8) -> Key {
    Key {
        code,
        modifiers,
        character: 0,
    }
}

fn scope() -> Value {
    json::parse(
        r#"{"id":"scope","title":"Export scope","question":"Which rows should the CSV include?",
        "context":"The report currently has active filters.","options":[
        {"id":"visible","label":"Visible rows","description":"Keep filters","recommended":true},
        {"id":"all","label":"All rows","description":"Ignore filters"}]}"#,
    )
    .unwrap()
}

fn question_args() -> Value {
    Value::object([("questions", Value::Array(vec![scope()]))])
}

fn several_args() -> Value {
    let formats = json::parse(
        r#"{"id":"formats","title":"Formats","question":"Which formats?","multiple":true,
        "options":[{"id":"csv","label":"CSV"},{"id":"xlsx","label":"XLSX"},{"id":"json","label":"JSON"}]}"#,
    )
    .unwrap();
    let delivery = json::parse(
        r#"{"id":"delivery","title":"Delivery","question":"Where should exports go?"}"#,
    )
    .unwrap();
    Value::object([("questions", Value::Array(vec![scope(), formats, delivery]))])
}

fn request(id: &str) -> Request {
    Request::parse(id, &question_args()).unwrap()
}

fn ask_result(app: &App) -> Option<&Value> {
    app.state.items.iter().find_map(|item| match item {
        state::Item::Tool {
            name,
            result: Some(result),
            ..
        } if name == "ask_user" => Some(result),
        _ => None,
    })
}

fn conversation(state: &State) -> String {
    state
        .rows()
        .iter()
        .flatten()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n")
}

fn dock(state: &State) -> String {
    view::composer_frame(state, "fixture", "folder")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn ask_waits_for_explicit_input_and_keeps_chat_draft_through_escape() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion("", vec![tool_call("ask-1", "ask_user", question_args())]),
        ),
        (200, completion("I will export visible rows.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, config(&directory), None);
    submit(&mut app, "Export the report");
    wait_until(&mut app, |app| app.state.question.is_some());
    let entry = app.state.ask_panel.as_ref().unwrap().entry().clone();
    assert!(
        entry.picked.is_empty() && !entry.other,
        "nothing preselected"
    );
    assert!(
        !conversation(&app.state).contains("ASK USER"),
        "nothing is added to the conversation while waiting"
    );

    app.state.editor.insert("Keep keyboard shortcuts");
    app.state.editor.cursor = 4;
    let chat = app.state.editor.clone();
    app.input(Decoded::Text("Keep the date filter\n ".into()))
        .unwrap();
    let entry = app.state.ask_panel.as_ref().unwrap().entry().clone();
    assert!(
        entry.other && entry.row == 2,
        "typing fills and selects Other"
    );
    app.input(Decoded::Key(key(27, 0))).unwrap();
    assert!(app.state.ask_panel.is_none());
    assert_eq!(app.state.editor, chat);
    app.persist_input(true);
    let saved = app.persistence.as_ref().unwrap().snapshot().input;
    assert_eq!(
        saved.question.as_ref().unwrap().answers[0].text,
        "Keep the date filter\n "
    );
    assert_eq!(saved.draft.cursor, chat.cursor);
    app.open_todo();
    app.input(Decoded::Key(key(27, 0))).unwrap();
    app.open_agents();
    app.input(Decoded::Key(key(27, 0))).unwrap();
    app.open_ask();
    let panel = app.state.ask_panel.as_ref().unwrap();
    assert_eq!(panel.entry().editor.text, "Keep the date filter\n ");

    // Arrows and the wheel move the cursor only; Space selects without sending.
    app.input(Decoded::Key(key(38, 0))).unwrap();
    app.input(Decoded::Scroll(3)).unwrap();
    let entry = app.state.ask_panel.as_ref().unwrap().entry().clone();
    assert_eq!((entry.row, entry.picked.len(), entry.other), (1, 0, true));
    app.input(Decoded::Text(" ".into())).unwrap();
    let entry = app.state.ask_panel.as_ref().unwrap().entry().clone();
    assert_eq!((entry.picked.clone(), entry.other), (vec![1], false));
    assert!(app.state.question.is_some(), "Space must not send");
    // Enter sends the highlighted row, not the earlier selection.
    app.input(Decoded::Key(key(9, 0))).unwrap();
    app.input(Decoded::Key(key(38, 0))).unwrap();
    app.input(Decoded::Key(key(13, 0))).unwrap();
    finish(&mut app);
    assert!(app.state.question.is_none());
    assert_eq!(app.state.editor, chat);
    assert_eq!(
        ask_result(&app).unwrap().get("answers").map(Value::encode),
        Some(r#"[{"choices":["visible"],"id":"scope","text":""}]"#.into())
    );
    let receipt = conversation(&app.state);
    assert!(
        receipt.contains(" ASK USER  Export scope\n  └ Visible rows"),
        "{receipt}"
    );
    let id = app.persistence.as_ref().unwrap().id();
    drop(app);
    let next_fixture = HttpFixture::new(vec![]);
    let mut next_agent = Agent::new(
        OpenRouter::fixture(next_fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    next_agent.enable_sessions(home.path()).unwrap();
    let mut resumed = App::new(next_agent, config(&directory), None);
    resumed.resume_session(&id);
    assert!(resumed.state.question.is_none());
    assert_eq!(
        ask_result(&resumed)
            .and_then(|result| result.get("outcome"))
            .and_then(Value::as_str),
        Some("answered")
    );
    drop(resumed);
    assert!(
        next_fixture.finish().is_empty(),
        "resume must not send a synthetic prompt"
    );
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn several_questions_are_answered_one_at_a_time_and_sent_together() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion("", vec![tool_call("ask-1", "ask_user", several_args())]),
        ),
        (200, completion("Export settings recorded.", vec![])),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, config(&directory), None);
    app.state.width = 100;
    submit(&mut app, "Export the report");
    wait_until(&mut app, |app| app.state.question.is_some());
    app.input(Decoded::Key(key(13, 0))).unwrap();
    assert_eq!(app.state.ask_panel.as_ref().unwrap().step, 1);
    let block = dock(&app.state);
    assert!(
        block.contains("☑ Export scope   Formats   Delivery"),
        "{block}"
    );
    assert!(block.contains("2 of 3"), "{block}");
    assert!(
        block.contains(" Space  select    Enter  next    ←→  questions    Esc  back to chat"),
        "{block}"
    );
    app.input(Decoded::Text(" ".into())).unwrap();
    app.input(Decoded::Key(key(40, 0))).unwrap();
    app.input(Decoded::Key(key(40, 0))).unwrap();
    app.input(Decoded::Text(" ".into())).unwrap();
    app.input(Decoded::Key(key(37, 0))).unwrap();
    assert_eq!(app.state.ask_panel.as_ref().unwrap().step, 0);
    app.input(Decoded::Key(key(39, 0))).unwrap();
    app.input(Decoded::Key(key(13, 0))).unwrap();
    assert_eq!(app.state.ask_panel.as_ref().unwrap().step, 2);
    let block = dock(&app.state);
    assert!(
        block.contains("› Type your answer") && block.contains("Enter  send    Ctrl+J  new line"),
        "{block}"
    );
    app.input(Decoded::Key(key(13, 0))).unwrap();
    assert!(app.state.question.is_some(), "an empty answer is not sent");
    app.input(Decoded::Text("Email the team lead".into()))
        .unwrap();
    app.input(Decoded::Key(key(13, 0))).unwrap();
    finish(&mut app);
    assert_eq!(
        ask_result(&app).unwrap().get("answers").map(Value::encode),
        Some(
            r#"[{"choices":["visible"],"id":"scope","text":""},{"choices":["csv","json"],"id":"formats","text":""},{"choices":[],"id":"delivery","text":"Email the team lead"}]"#
                .into()
        )
    );
    let receipt = conversation(&app.state);
    assert!(
        receipt.contains(
            " ASK USER  3 questions\n  │ Export scope  Visible rows\n  │ Formats  CSV, JSON\n  └ Delivery  Email the team lead"
        ),
        "{receipt}"
    );
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn ask_dismissal_is_distinct_from_escape_and_answer() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let request = request("q");
    app.state.question = Some(request.clone());
    app.state.ask_panel = Some(super::super::ask::Panel::new(&request, None));
    app.input(Decoded::Key(key(27, 0))).unwrap();
    assert!(app.state.question.is_some());
    assert!(app.state.ask_panel.is_none());
    assert!(dock(&app.state).contains("/ask answer"));
    app.open_ask();
    let lines = dock(&app.state);
    assert!(
        lines.contains("Ctrl+D  dismiss") && lines.contains("Esc  back to chat"),
        "{lines}"
    );
    assert!(lines.contains("Waiting for you") && !lines.contains("/ask answer"));
    assert!(fixture.finish().is_empty());
}

#[test]
fn the_wheel_reaches_the_question_in_front_of_todo() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.open_todo();
    let Value::Object(mut question) = scope() else {
        unreachable!()
    };
    question.insert(
        "context".into(),
        Value::string("Filters decide which rows appear. ".repeat(20)),
    );
    let arguments = Value::object([("questions", Value::Array(vec![Value::Object(question)]))]);
    let request = Request::parse("q", &arguments).unwrap();
    app.state.question = Some(request.clone());
    app.state.ask_panel = Some(super::super::ask::Panel::new(&request, None));
    app.state.width = 40;
    app.state.height = 18;
    assert!(app.state.todo_panel.is_some());
    app.input(Decoded::Scroll(-2)).unwrap();
    app.input(Decoded::Scroll(1)).unwrap();
    let panel = app.state.ask_panel.as_ref().unwrap();
    assert!(panel.top.is_some(), "the overflowing question scrolls");
    assert!(panel.entry().picked.is_empty());
    assert!(fixture.finish().is_empty());
}

#[test]
fn replayed_question_event_keeps_restored_answer_and_closed_panel() {
    let request = request("ask-1");
    let mut panel = super::super::ask::Panel::new(&request, None);
    assert!(panel.type_text(&request.questions[0], "keep the filters"));
    panel.entries[0].editor.cursor = 4;
    let draft = panel.draft(&request);
    let mut state = State {
        question: Some(request.clone()),
        ask_panel: Some(panel.clone()),
        ask_draft: Some(draft),
        ..State::default()
    };
    state.event(crate::events::Event::UserQuestion {
        request: request.clone(),
    });
    assert_eq!(state.ask_panel.as_ref(), Some(&panel));
    state.ask_panel = None;
    state.event(crate::events::Event::UserQuestion { request });
    assert!(
        state.ask_panel.is_none(),
        "a historical event must not reopen an Esc-closed panel"
    );
}

#[test]
fn cancelled_question_card_says_no_answer_was_supplied() {
    let mut state = State::default();
    state.event(crate::events::Event::ToolStarted {
        id: "ask-1".into(),
        name: "ask_user".into(),
        arguments: question_args(),
    });
    state.event(crate::events::Event::ToolFinished {
        id: "ask-1".into(),
        name: "ask_user".into(),
        summary: "complete".into(),
        result: Value::object([("outcome", Value::string("cancelled"))]),
    });
    let text = conversation(&state);
    assert!(text.contains(" ASK USER  Export scope"), "{text}");
    assert!(text.contains("Question cancelled") && text.contains("no answer supplied"));
}

#[test]
fn explicit_dismissal_returns_no_answer_to_the_model() {
    let directory = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion("", vec![tool_call("ask-1", "ask_user", question_args())]),
        ),
        (
            200,
            completion("I will continue without an answer.", vec![]),
        ),
    ]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let mut app = App::new(agent, config(&directory), None);
    submit(&mut app, "Export the report");
    wait_until(&mut app, |app| app.state.question.is_some());
    app.input(Decoded::Key(key(27, 0))).unwrap();
    assert!(app.state.question.is_some());
    app.open_ask();
    app.input(Decoded::Key(key(68, 4))).unwrap();
    finish(&mut app);
    let result = ask_result(&app).unwrap();
    assert_eq!(
        result.get("outcome").and_then(Value::as_str),
        Some("dismissed")
    );
    assert!(result.get("answers").is_none());
    assert_eq!(fixture.finish().len(), 2);
}

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
