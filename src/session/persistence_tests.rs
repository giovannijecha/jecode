use super::persistence::resume;
use super::*;
use crate::{
    config::{Settings, Store},
    openrouter::OpenRouter,
    sessions::{Draft, Input},
    test_support::{Directory, HttpFixture, completion},
    tools::Tools,
};
use std::io::Cursor;

fn agent(directory: &Directory, endpoint: &str) -> Agent {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent
        .enable_sessions(&directory.path().join(".jecode"))
        .unwrap();
    agent
}
fn config(directory: &Directory) -> SessionConfig {
    SessionConfig {
        store: Store::new(directory.path().join(".jecode")),
        settings: Settings::new("isolated-fixture-key".into(), "fixture/default".into()).unwrap(),
        bash: crate::tools::find_bash().unwrap(),
    }
}

#[test]
fn plain_resume_restores_history_and_displays_unsent_text_without_calling_the_provider() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Retained plain answer", vec![]))]);
    let mut first = agent(&directory, &fixture.endpoint);
    first
        .run_turn("Plain original request", &mut |_| Ok(()))
        .unwrap();
    let handle = first.sessions().unwrap();
    handle.input(Input {
        draft: Draft {
            text: "unsent plain draft".into(),
            cursor: 2,
        },
        queued: vec!["queued plain request".into()],
        ..Input::default()
    });
    handle.flush().unwrap();
    let id = handle.id();
    drop(handle);
    drop(first);
    assert_eq!(fixture.finish().len(), 1);
    let mut current = agent(&directory, "http://127.0.0.1:1/chat/completions");
    let mut output = vec![];
    let mut status = vec![];
    chat_with_resume(
        &mut current,
        &mut config(&directory),
        &mut Cursor::new("/help\n/export\n/new\n/exit\n"),
        &mut output,
        &mut status,
        Some(id.clone()),
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Retained plain answer"));
    assert!(output.contains("Recovered draft (not sent"));
    assert!(
        output
            .contains("Recovered draft (not sent; copy/edit before sending):\nunsent plain draft")
    );
    assert!(
        output.contains(
            "Paused draft 1/1 (not sent; copy/edit before sending):\nqueued plain request"
        )
    );
    assert!(!output.contains("queued plain request\n\nunsent plain draft"));
    assert!(output.contains("/resume"));
    assert!(!output.contains("isolated-fixture-key"));
    assert_eq!(current.archive().messages.lock().unwrap().len(), 1);
    assert_eq!(current.model(), "fixture/default");
    assert_ne!(current.sessions().unwrap().id(), id);
    let input = current.sessions().unwrap().snapshot().input;
    assert_eq!(input.draft.text, "unsent plain draft");
    assert_eq!(input.paused[0].text, "queued plain request");
    assert!(input.queued.is_empty());
    let export = std::fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("JECODE-SESSION-")
        })
        .unwrap();
    assert!(
        std::fs::read_to_string(export.path())
            .unwrap()
            .contains("Retained plain answer")
    );
}

#[test]
fn cancelling_plain_selection_keeps_the_new_context() {
    let directory = Directory::new();
    let mut first = agent(&directory, "http://127.0.0.1:1/chat/completions");
    first.prepare_turn("A saved conversation").unwrap();
    drop(first);
    let mut current = agent(&directory, "http://127.0.0.1:1/chat/completions");
    let mut output = vec![];
    chat(
        &mut current,
        &mut config(&directory),
        &mut Cursor::new("/resume\n/cancel\n/exit\n"),
        &mut output,
        &mut vec![],
    )
    .unwrap();
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("Resume · current folder")
    );
    assert_eq!(current.archive().messages.lock().unwrap().len(), 1);
}

#[test]
fn plain_resume_does_not_list_an_unsent_current_conversation() {
    let directory = Directory::new();
    let mut current = agent(&directory, "http://127.0.0.1:1/chat/completions");
    let id = current.sessions().unwrap().id();
    let mut output = vec![];
    resume(
        &mut current,
        &config(&directory),
        None,
        &mut Cursor::new("/cancel\n"),
        &mut output,
        &mut vec![],
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(!output.contains(&id));
    assert!(output.contains("No saved conversations"));
    assert_eq!(current.sessions().unwrap().id(), id);
    assert!(
        current
            .sessions()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .is_empty()
    );
}

#[test]
fn plain_resume_can_delete_then_refresh_and_resume_another_session() {
    let directory = Directory::new();
    let endpoint = "http://127.0.0.1:1/chat/completions";
    let mut first = agent(&directory, endpoint);
    first.prepare_turn("Delete this conversation").unwrap();
    let first_id = first.sessions().unwrap().id();
    drop(first);
    let mut second = agent(&directory, endpoint);
    second.prepare_turn("Resume this conversation").unwrap();
    let second_id = second.sessions().unwrap().id();
    drop(second);
    let mut current = agent(&directory, endpoint);
    let mut output = vec![];
    let choices = format!("d {first_id}\ndelete\n{second_id}\n");
    resume(
        &mut current,
        &config(&directory),
        None,
        &mut Cursor::new(choices),
        &mut output,
        &mut vec![],
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert_eq!(output.matches("Resume · current folder").count(), 2);
    assert!(output.contains("Deleted Delete this conversation"));
    assert!(output.contains("Resume this conversation"));
    assert_eq!(current.sessions().unwrap().id(), second_id);
    assert!(
        current
            .sessions()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .iter()
            .all(|entry| entry.id != first_id)
    );
}

#[test]
fn cancelled_plain_delete_keeps_session_available_in_the_same_list() {
    let directory = Directory::new();
    let endpoint = "http://127.0.0.1:1/chat/completions";
    let mut saved = agent(&directory, endpoint);
    saved.prepare_turn("Keep this conversation").unwrap();
    let id = saved.sessions().unwrap().id();
    drop(saved);
    let mut current = agent(&directory, endpoint);
    let mut output = vec![];
    let choices = format!("d {id}\n\n{id}\n");
    resume(
        &mut current,
        &config(&directory),
        None,
        &mut Cursor::new(choices),
        &mut output,
        &mut vec![],
    )
    .unwrap();
    assert_eq!(current.sessions().unwrap().id(), id);
    let output = String::from_utf8(output).unwrap();
    assert_eq!(output.matches("Resume · current folder").count(), 2);
    assert!(!output.contains("Deleted Keep this conversation"));
}

#[test]
fn plain_resume_supports_repeated_deletions_before_returning_to_chat() {
    let directory = Directory::new();
    let endpoint = "http://127.0.0.1:1/chat/completions";
    let mut first = agent(&directory, endpoint);
    first.prepare_turn("First saved conversation").unwrap();
    let first_id = first.sessions().unwrap().id();
    drop(first);
    let mut second = agent(&directory, endpoint);
    second.prepare_turn("Second saved conversation").unwrap();
    let second_id = second.sessions().unwrap().id();
    drop(second);
    let mut current = agent(&directory, endpoint);
    let current_id = current.sessions().unwrap().id();
    let choices = format!("d {first_id}\ndelete\nd {second_id}\ndelete\n/cancel\n");
    let mut output = vec![];
    resume(
        &mut current,
        &config(&directory),
        None,
        &mut Cursor::new(choices),
        &mut output,
        &mut vec![],
    )
    .unwrap();
    assert_eq!(current.sessions().unwrap().id(), current_id);
    assert!(
        current
            .sessions()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .is_empty()
    );
    assert_eq!(
        String::from_utf8(output)
            .unwrap()
            .matches("Resume · current folder")
            .count(),
        3
    );
}

#[test]
fn plain_resume_deletes_saved_current_by_number_keeps_the_list_and_preserves_unsent_input() {
    let directory = Directory::new();
    let mut current = agent(&directory, "http://127.0.0.1:1/chat/completions");
    current
        .archive()
        .messages
        .lock()
        .unwrap()
        .push(crate::json::Value::object([
            ("role", crate::json::Value::string("user")),
            ("content", crate::json::Value::string("Sent request")),
        ]));
    current.save_session().unwrap();
    let old = current.sessions().unwrap();
    let id = old.id();
    old.input(Input {
        draft: Draft {
            text: "unsent draft".into(),
            cursor: 3,
        },
        queued: vec!["queued request".into()],
        ..Input::default()
    });
    let mut output = vec![];
    resume(
        &mut current,
        &config(&directory),
        None,
        &mut Cursor::new("d 1\ndelete\n"),
        &mut output,
        &mut vec![],
    )
    .unwrap();
    assert_ne!(current.sessions().unwrap().id(), id);
    assert_eq!(
        current.sessions().unwrap().snapshot().input.draft.text,
        "unsent draft"
    );
    assert_eq!(
        current.sessions().unwrap().snapshot().input.queued,
        vec!["queued request".to_string()]
    );
    current.save_session().unwrap();
    old.flush().unwrap();
    assert!(
        current
            .sessions()
            .unwrap()
            .store()
            .list()
            .unwrap()
            .sessions
            .is_empty()
    );
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("new conversation opened"));
    assert!(output.contains("No saved conversations"));
    assert_eq!(output.matches("Resume · current folder").count(), 2);
}
