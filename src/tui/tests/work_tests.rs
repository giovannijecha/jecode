use super::*;
use crate::{
    ask::{Choice, Question, Request},
    tui::{keys::Decoded, terminal::Key},
};

fn key(code: u16, modifiers: u8) -> Key {
    Key {
        code,
        modifiers,
        character: 0,
    }
}

fn question() -> Question {
    Question {
        title: "Export scope".into(),
        question: "Which rows should the CSV include?".into(),
        context: "The report currently has active filters.".into(),
        options: vec![
            Choice {
                id: "visible".into(),
                label: "Visible rows".into(),
                description: "Keep filters".into(),
                recommended: true,
            },
            Choice {
                id: "all".into(),
                label: "All rows".into(),
                description: "Ignore filters".into(),
                recommended: false,
            },
        ],
    }
}

fn question_args() -> Value {
    Value::object([
        ("title", Value::string("Export scope")),
        (
            "question",
            Value::string("Which rows should the CSV include?"),
        ),
        (
            "context",
            Value::string("The report currently has active filters."),
        ),
        (
            "options",
            Value::Array(vec![
                Value::object([
                    ("id", Value::string("visible")),
                    ("label", Value::string("Visible rows")),
                    ("description", Value::string("Keep filters")),
                    ("recommended", Value::Bool(true)),
                ]),
                Value::object([
                    ("id", Value::string("all")),
                    ("label", Value::string("All rows")),
                    ("description", Value::string("Ignore filters")),
                ]),
            ]),
        ),
    ])
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
    assert!(app.state.ask_panel.as_ref().unwrap().choice.is_none());
    assert!(app.state.items.iter().any(|item| matches!(item,
        state::Item::Tool { name, result: None, .. } if name == "ask_user")));

    app.state.editor.insert("Keep keyboard shortcuts");
    app.state.editor.cursor = 4;
    let chat = app.state.editor.clone();
    app.input(Decoded::Key(key(13, 0))).unwrap();
    assert!(
        app.state.question.is_some(),
        "empty Enter must not submit a recommendation"
    );
    app.input(Decoded::Text("Keep the date filter\n ".into()))
        .unwrap();
    app.input(Decoded::Key(key(27, 0))).unwrap();
    assert!(app.state.ask_panel.is_none());
    assert_eq!(app.state.editor, chat);
    app.persist_input(true);
    let saved = app.persistence.as_ref().unwrap().snapshot().input;
    assert_eq!(
        saved.question.as_ref().unwrap().text,
        "Keep the date filter\n "
    );
    assert_eq!(saved.draft.cursor, chat.cursor);
    app.open_todo();
    app.input(Decoded::Key(key(27, 0))).unwrap();
    app.open_agents();
    app.input(Decoded::Key(key(27, 0))).unwrap();
    app.open_ask();
    assert_eq!(
        app.state.ask_panel.as_ref().unwrap().editor.text,
        "Keep the date filter\n "
    );
    app.input(Decoded::Key(key(9, 0))).unwrap();
    app.input(Decoded::Key(key(40, 0))).unwrap();
    assert_eq!(
        app.state.ask_panel.as_ref().unwrap().choice.as_deref(),
        Some("all")
    );
    assert!(
        app.state.question.is_some(),
        "selecting a row must not submit it"
    );
    app.input(Decoded::Key(key(13, 0))).unwrap();
    finish(&mut app);
    assert!(app.state.question.is_none());
    assert_eq!(app.state.editor, chat);
    let receipt = app
        .state
        .items
        .iter()
        .find_map(|item| match item {
            state::Item::Tool {
                name,
                result: Some(result),
                ..
            } if name == "ask_user" => Some(result),
            _ => None,
        })
        .unwrap();
    assert_eq!(receipt.get("choice").and_then(Value::as_str), Some("all"));
    assert_eq!(
        receipt.get("text").and_then(Value::as_str),
        Some("Keep the date filter\n ")
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
    let cards: Vec<_> = resumed
        .state
        .items
        .iter()
        .filter_map(|item| match item {
            state::Item::Tool {
                name,
                result: Some(result),
                ..
            } if name == "ask_user" => Some(result),
            _ => None,
        })
        .collect();
    assert_eq!(cards.len(), 1);
    assert_eq!(
        cards[0].get("outcome").and_then(Value::as_str),
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
fn ask_dismissal_is_distinct_from_escape_and_answer() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let request = Request {
        id: "q".into(),
        question: question(),
    };
    app.state.question = Some(request.clone());
    app.state.ask_panel = Some(super::super::ask::Panel::new(&request, None));
    app.input(Decoded::Key(key(27, 0))).unwrap();
    assert!(app.state.question.is_some());
    assert!(app.state.ask_panel.is_none());
    assert!(
        view::composer_frame(&app.state, "fixture", "folder")
            .live
            .iter()
            .any(|line| line.plain().contains("/ask answer"))
    );
    app.open_ask();
    let frame = view::composer_frame(&app.state, "fixture", "folder");
    let lines = frame
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        lines.contains("Ctrl+D")
            && lines.contains("dismiss")
            && lines.contains("Esc")
            && lines.contains("chat")
    );
    assert!(lines.contains("Waiting for you"));
    assert!(fixture.finish().is_empty());
}

#[test]
fn replayed_question_event_keeps_restored_answer_and_closed_panel() {
    let request = Request {
        id: "ask-1".into(),
        question: question(),
    };
    let mut panel = super::super::ask::Panel::new(&request, None);
    panel.editor.insert("keep the filters");
    panel.editor.cursor = 4;
    panel.choice = Some("visible".into());
    let draft = panel.draft(&request.id);
    let mut state = State {
        question: Some(request.clone()),
        ask_panel: Some(panel),
        ask_draft: Some(draft.clone()),
        ..State::default()
    };
    state.event(crate::events::Event::UserQuestion {
        request: request.clone(),
    });
    let answer = state.ask_panel.as_ref().unwrap();
    assert_eq!(answer.editor.text, draft.text);
    assert_eq!(answer.editor.cursor, draft.cursor);
    assert_eq!(answer.choice, draft.choice);
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
    let text = state.rows()[0]
        .iter()
        .map(|line| line.plain())
        .collect::<String>();
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
    let result = app
        .state
        .items
        .iter()
        .find_map(|item| match item {
            state::Item::Tool {
                name,
                result: Some(result),
                ..
            } if name == "ask_user" => Some(result),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        result.get("outcome").and_then(Value::as_str),
        Some("dismissed")
    );
    assert!(result.get("choice").is_none());
    assert!(result.get("text").is_none());
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
    first.state.ask_panel.as_mut().unwrap().editor.cursor = 4;
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
    let answer = resumed.state.ask_panel.as_ref().unwrap();
    assert_eq!(answer.editor.text, "Keep the date filter");
    assert_eq!(answer.editor.cursor, 4);
    resumed.poll().unwrap(); // The resumed worker may re-emit UserQuestion.
    assert_eq!(resumed.state.ask_panel.as_ref().unwrap().editor.cursor, 4);
    resumed.input(Decoded::Key(key(13, 0))).unwrap();
    finish(&mut resumed);
    assert!(resumed.state.question.is_none());
    assert_eq!(resumed.state.editor.cursor, 5);
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
    let result = resumed
        .state
        .items
        .iter()
        .find_map(|item| match item {
            state::Item::Tool {
                name,
                result: Some(result),
                ..
            } if name == "ask_user" => Some(result),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        result.get("outcome").and_then(Value::as_str),
        Some("cancelled")
    );
    drop(resumed);
    assert!(resumed_fixture.finish().is_empty());
}

#[test]
fn long_question_keeps_the_selected_last_option_visible_in_the_native_sheet() {
    let mut request = Request {
        id: "q".into(),
        question: question(),
    };
    request.question.context = "A long explanation about report filters and ordering. ".repeat(15);
    request.question.options = (1..=6)
        .map(|index| Choice {
            id: format!("option-{index}"),
            label: format!("Scope option {index} with a longer description"),
            description: "This choice changes which rows appear in the export.".into(),
            recommended: index == 1,
        })
        .collect();
    let mut panel = super::super::ask::Panel::new(&request, None);
    panel.choices_focused = true;
    panel.selected = 5;
    panel.choice = Some("option-6".into());
    let state = State {
        width: 32,
        height: 15,
        question: Some(request),
        ask_panel: Some(panel),
        ..State::default()
    };
    let frame = view::composer_frame(&state, "fixture", "folder");
    assert!(
        frame.live.iter().any(|line| line.plain().contains("6.")),
        "last option must be reachable"
    );
    assert!(frame.live.len() <= 10);
}
