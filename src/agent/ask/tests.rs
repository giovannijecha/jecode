use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    sessions::{Draft, Input},
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};

fn arguments() -> Value {
    Value::object([
        ("title", Value::string("Export")),
        ("question", Value::string("Which format?")),
        (
            "options",
            Value::Array(vec![Value::object([
                ("id", Value::string("csv")),
                ("label", Value::string("CSV")),
                ("recommended", Value::Bool(true)),
            ])]),
        ),
    ])
}

fn stage_question(agent: &mut Agent, other_call: bool) {
    agent.prepare_turn("Export the report").unwrap();
    let mut calls = vec![
        ToolCall {
            id: "ask-1".into(),
            name: "ask_user".into(),
            arguments: arguments().encode(),
        }
        .value(),
    ];
    if other_call {
        calls.push(
            ToolCall {
                id: "write-after".into(),
                name: "write".into(),
                arguments: Value::object([
                    ("path", Value::string("must-not-exist.txt")),
                    ("content", Value::string("side effect")),
                ])
                .encode(),
            }
            .value(),
        );
    }
    agent.messages.lock().unwrap().push(Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::Null),
        ("tool_calls", Value::Array(calls)),
    ]));
    agent.checkpoint(Stage::Completion).unwrap();
    agent.checkpoint(Stage::Tool("ask-1".into())).unwrap();
    agent.questions.set_available(true);
    assert!(
        agent
            .questions
            .begin(Request {
                id: "ask-1".into(),
                question: Question::parse(&arguments()).unwrap()
            })
            .unwrap()
    );
}

#[test]
fn resume_question_keeps_drafts_and_delivers_saved_answer_once_without_replaying_tools() {
    for saved_answer in [false, true] {
        let project = Directory::new();
        let home = Directory::new();
        let fixture = HttpFixture::new(vec![(200, completion("Export format recorded", vec![]))]);
        let make_agent = || {
            Agent::new(
                OpenRouter::fixture(fixture.endpoint.clone()),
                Tools::new(project.path()).unwrap(),
            )
        };
        let mut original = make_agent();
        original.enable_sessions(home.path()).unwrap();
        stage_question(&mut original, true);
        let id = original.sessions().unwrap().id();
        original.sessions().unwrap().input(Input {
            draft: Draft {
                text: "Unsent follow-up".into(),
                cursor: 4,
                ..Default::default()
            },
            question: Some(crate::ask::Draft {
                id: "ask-1".into(),
                text: "Keep dates".into(),
                cursor: 5,
                choice: Some("csv".into()),
            }),
            ..Default::default()
        });
        original.sessions().unwrap().flush().unwrap();
        if saved_answer {
            original
                .questions
                .answer("ask-1", Some("csv".into()), "Keep dates".into())
                .unwrap();
        }
        drop(original);
        let mut resumed = make_agent();
        resumed.enable_sessions(home.path()).unwrap();
        let status = resumed.resume(&id).unwrap();
        assert!(status.contains("Question"), "{status}");
        assert!(resumed.has_pending_question());
        assert_eq!(resumed.questions.pending().is_some(), !saved_answer);
        let doc = resumed.sessions().unwrap().snapshot();
        assert_eq!(doc.input.draft.text, "Unsent follow-up");
        assert_eq!(doc.input.draft.cursor, 4);
        assert_eq!(doc.input.question.as_ref().unwrap().cursor, 5);
        assert!(
            !doc.messages.iter().any(
                |message| message.get("tool_call_id").and_then(Value::as_str) == Some("ask-1")
            )
        );
        assert!(resumed.prepare_turn("unrelated prompt").is_err());
        resumed.questions.set_available(true);
        let observer = resumed.questions();
        let mut asked = 0;
        assert!(
            resumed
                .continue_question(&mut |event| {
                    if let Event::UserQuestion { request } = event {
                        asked += 1;
                        observer.answer(&request.id, Some("csv".into()), "Keep dates".into())?;
                    }
                    Ok(())
                })
                .unwrap()
        );
        assert_eq!(asked, usize::from(!saved_answer));
        assert!(!resumed.continue_question(&mut |_| Ok(())).unwrap());
        assert!(!project.path().join("must-not-exist.txt").exists());
        let doc = resumed.sessions().unwrap().snapshot();
        let results: Vec<_> = doc
            .messages
            .iter()
            .filter(|message| message.get("tool_call_id").and_then(Value::as_str) == Some("ask-1"))
            .collect();
        assert_eq!(results.len(), 1);
        let answer = json::parse(results[0].get("content").unwrap().as_str().unwrap()).unwrap();
        assert_eq!(
            answer.get("outcome").and_then(Value::as_str),
            Some("answered")
        );
        assert_eq!(answer.get("choice").and_then(Value::as_str), Some("csv"));
        assert_eq!(
            answer.get("text").and_then(Value::as_str),
            Some("Keep dates")
        );
        assert!(!doc.pending.active);
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[test]
fn ask_headless_is_unavailable_and_child_has_no_question_channel() {
    for child in [false, true] {
        let project = Directory::new();
        let fixture = HttpFixture::new(vec![
            (
                200,
                completion("", vec![tool_call("ask-1", "ask_user", arguments())]),
            ),
            (200, completion("Input is unavailable", vec![])),
        ]);
        let mut agent = Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(project.path()).unwrap(),
        );
        agent.child = child;
        agent.run_turn("Inspect", &mut |_| Ok(())).unwrap();
        assert!(!agent.has_pending_question());
        let archive = agent.archive();
        let messages = archive.messages.lock().unwrap();
        let answer = messages
            .iter()
            .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some("ask-1"))
            .unwrap();
        assert!(
            answer
                .get("content")
                .unwrap()
                .as_str()
                .unwrap()
                .contains("unavailable")
        );
        assert_eq!(fixture.finish().len(), 2);
    }
}

#[test]
fn answer_save_failure_does_not_release_question_or_invent_answer() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    stage_question(&mut agent, false);
    assert!(agent.questions.answer("ask-1", None, " ".into()).is_err());
    assert!(
        agent
            .questions
            .answer("wrong", Some("csv".into()), "".into())
            .is_err()
    );
    let mut broken = agent.archive();
    broken.messages = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    agent.questions.bind(broken, agent.sessions());
    assert!(agent.questions.answer("ask-1", None, "CSV".into()).is_err());
    assert!(agent.questions.pending().is_some());
    assert!(agent.questions.saved_reply().is_none());
    assert!(
        !agent
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.get("type").and_then(Value::as_str) == Some("user_answer"))
    );
    assert!(fixture.finish().is_empty());
}

#[test]
fn stopping_a_waiting_question_releases_the_worker_without_fabricating_an_answer() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        completion("", vec![tool_call("ask-1", "ask_user", arguments())]),
    )]);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.questions.set_available(true);
    let cancellation = agent.cancellation();
    let started = std::time::Instant::now();
    let result = agent.run_turn("Export", &mut |event| {
        if matches!(event, Event::UserQuestion { .. }) {
            cancellation.cancel();
        }
        Ok(())
    });
    assert!(result.is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(!agent.has_pending_question());
    let saved = agent.sessions().unwrap().snapshot();
    assert!(!saved.pending.active);
    let content = saved
        .messages
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some("ask-1"))
        .unwrap()
        .get("content")
        .and_then(Value::as_str)
        .unwrap();
    assert_eq!(
        json::parse(content)
            .unwrap()
            .get("outcome")
            .and_then(Value::as_str),
        Some("cancelled")
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn deleting_the_conversation_rebinds_questions_to_the_replacement_session() {
    let project = Directory::new();
    let home = Directory::new();
    let mut replies = Vec::new();
    for _ in 0..2 {
        replies.push((
            200,
            completion("", vec![tool_call("ask-1", "ask_user", arguments())]),
        ));
        replies.push((200, completion("Answer recorded", vec![])));
    }
    let fixture = HttpFixture::new(replies);
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    agent.questions.set_available(true);
    let observer = agent.questions();
    let mut respond = |event| {
        if let Event::UserQuestion { request } = event {
            observer.answer(&request.id, None, "CSV".into())?;
        }
        Ok(())
    };
    agent.run_turn("First question", &mut respond).unwrap();
    let old = agent.sessions().unwrap().id();
    let model = agent.model().to_owned();
    let effort = agent.effort();
    agent.delete_session(&old, model, effort).unwrap();
    assert_ne!(agent.sessions().unwrap().id(), old);
    agent.run_turn("Second question", &mut respond).unwrap();
    let saved = agent.sessions().unwrap().snapshot();
    assert_eq!(
        saved
            .events
            .iter()
            .filter(|event| event.get("type").and_then(Value::as_str) == Some("user_answer"))
            .count(),
        1
    );
    assert!(saved.input.question.is_none());
    assert_eq!(fixture.finish().len(), 4);
}
