use super::*;
use crate::{
    effort::Effort,
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::{thread, time::Instant};

fn app(home: &Directory, directory: &Directory, endpoint: &str) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    App::new(agent, tests::config(directory), None)
}

fn submit(app: &mut App, text: &str) {
    app.state.editor.replace(text.into());
    app.edited();
    assert!(!app.submit().unwrap());
}
fn finish(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while app.worker.is_some() {
        app.poll().unwrap();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
}

fn journal(home: &Directory, current: &App) -> std::path::PathBuf {
    let bucket = std::fs::read_dir(home.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    bucket.join(format!(
        "{}.jsonl",
        current.persistence.as_ref().unwrap().id()
    ))
}

fn saved_exchange(app: &App) {
    for (role, content) in [("user", "Earlier request"), ("assistant", "Earlier answer")] {
        app.archive.messages.lock().unwrap().push(Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(content)),
        ]));
    }
    app.agent.as_ref().unwrap().save_session().unwrap();
}

#[test]
fn failed_submit_keeps_the_turn_ready_and_retry_sends_the_request_once() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Retried", vec![]))]);
    let mut current = app(&home, &directory, &fixture.endpoint);
    saved_exchange(&current);
    current.state.editor.replace("Retry this request".into());
    current.persist_input(true);
    let path = journal(&home, &current);
    let original = std::fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&path, readonly).unwrap();
    let result = current.submit();
    std::fs::set_permissions(&path, original).unwrap();
    assert!(!result.unwrap());
    assert!(current.worker.is_none());
    assert!(!current.persistence.as_ref().unwrap().active());
    assert!(current.state.editor.text.is_empty());
    assert_eq!(current.state.queue.paused[0].text, "Retry this request");
    assert_eq!(current.archive.messages.lock().unwrap().len(), 3);
    current.agent.as_mut().unwrap().clean_temporary().unwrap();
    current.open_drafts();
    current.send_paused_draft().unwrap();
    finish(&mut current);
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
            .filter(|message| {
                message.get("content").and_then(Value::as_str) == Some("Retry this request")
            })
            .count(),
        1
    );
}

#[test]
fn failed_queued_preparation_returns_all_unsent_work_in_order() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut current = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    saved_exchange(&current);
    current.state.queue.push("first queued".into()).unwrap();
    current.state.queue.push("second queued".into()).unwrap();
    current.state.editor.replace("current draft".into());
    current.persist_input(true);
    let path = journal(&home, &current);
    let original = std::fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&path, readonly).unwrap();
    let result = current.poll();
    std::fs::set_permissions(&path, original).unwrap();
    assert!(result.unwrap());
    assert!(current.worker.is_none());
    assert!(current.state.queue.messages.is_empty());
    assert_eq!(current.archive.messages.lock().unwrap().len(), 3);
    assert_eq!(current.state.editor.text, "current draft");
    assert_eq!(
        current
            .state
            .queue
            .paused
            .iter()
            .map(|draft| draft.text.as_str())
            .collect::<Vec<_>>(),
        ["first queued", "second queued"]
    );
    current.persist_input(true);
    let saved = current.persistence.as_ref().unwrap().snapshot();
    assert_eq!(saved.input.draft.text, current.state.editor.text);
    assert!(!saved.pending.active);
}

#[test]
fn fresh_launch_is_empty_and_numbered_resume_restores_tools_local_details_and_unsent_work() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "Inspecting",
                vec![tool_call(
                    "write",
                    "write",
                    Value::object([
                        ("path", Value::string("file.txt")),
                        ("content", Value::string("original")),
                    ]),
                )],
            ),
        ),
        (200, completion("Answer preserved", vec![])),
    ]);
    let mut first = app(&home, &directory, &fixture.endpoint);
    submit(&mut first, "First saved request");
    finish(&mut first);
    submit(&mut first, "/help");
    submit(&mut first, "/export");
    first.apply_selection("fixture/other".into(), Effort::High, false);
    first
        .state
        .editor
        .replace("original β draft\nsecond line".into());
    first.state.editor.cursor = 5;
    first.state.queue.push("first queued".into()).unwrap();
    first.state.queue.push("last queued".into()).unwrap();
    first
        .state
        .queue
        .begin_edit(1, &mut first.state.editor)
        .unwrap();
    first.state.editor.insert(" revised");
    first.persist_input(true);
    let id = first.persistence.as_ref().unwrap().id();
    drop(first);
    assert_eq!(fixture.finish().len(), 2);
    let mut resumed = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    assert!(resumed.state.items.is_empty());
    assert_eq!(resumed.archive.messages.lock().unwrap().len(), 1);
    assert!(
        resumed
            .state
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("/resume")
    );
    submit(&mut resumed, "/resume");
    assert!(matches!(
        resumed.state.selector.as_ref().unwrap().purpose,
        selector::Purpose::Sessions { .. }
    ));
    assert!(!resumed.state.selector.as_ref().unwrap().searchable);
    resumed.input(Decoded::Text("1".into())).unwrap();
    assert_eq!(resumed.persistence.as_ref().unwrap().id(), id);
    assert_eq!(resumed.archive.model, "fixture/other");
    assert_eq!(resumed.state.effort, "high");
    assert_eq!(resumed.config.settings.model, "fixture/model");
    assert_eq!(resumed.state.editor.text, "original β draft\nsecond line");
    assert_eq!(resumed.state.editor.cursor, 5);
    assert_eq!(
        resumed
            .state
            .queue
            .paused
            .iter()
            .map(|draft| draft.text.as_str())
            .collect::<Vec<_>>(),
        ["first queued", "last queued revised"]
    );
    assert!(resumed.state.queue.messages.is_empty());
    assert!(resumed.worker.is_none());
    resumed.poll().unwrap();
    assert!(resumed.worker.is_none());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("file.txt")).unwrap(),
        "original"
    );
    assert!(resumed.state.items.iter().any(|item| matches!(item, state::Item::Tool { name, result: Some(_), last: Some(true), .. } if name == "write")));
    assert!(resumed.state.items.iter().any(|item| matches!(item, state::Item::Local { command, details, .. } if command == "/help" && details.iter().any(|(key, _)| key == "/resume"))));
    let rows = resumed
        .state
        .rows()
        .into_iter()
        .flatten()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rows.contains("Answer preserved"));
    assert!(rows.contains("write  "));
    assert!(
        resumed
            .state
            .items
            .iter()
            .filter_map(|item| match item {
                state::Item::Tool { presentation, .. } => Some(presentation),
                _ => None,
            })
            .all(|presentation| presentation.duration().is_none())
    );
    assert!(!rows.contains("[restored tool"));
    assert!(!rows.contains("Saved JECODE-SESSION-"));
    assert!(!rows.contains("Resumed "));
    assert!(
        !rows
            .lines()
            .any(|line| line.trim() == format!("/resume {id}"))
    );
    assert!(
        resumed
            .archive
            .document()
            .encode()
            .contains("Saved JECODE-SESSION-")
    );
    assert!(
        !resumed
            .archive
            .document()
            .encode()
            .contains("original β draft")
    );
}

#[test]
fn session_search_uses_existing_selector_and_escape_preserves_draft_and_cursor() {
    let home = Directory::new();
    let directory = Directory::new();
    for index in 0..10 {
        let mut seed = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
        seed.agent
            .as_mut()
            .unwrap()
            .prepare_turn(&format!("Saved task {index}"))
            .unwrap();
        drop(seed);
    }
    let mut current = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    current.state.editor.replace("unfinished è draft".into());
    current.state.editor.cursor = 3;
    current.open_sessions();
    assert_eq!(current.saved_sessions.len(), 10);
    assert!(current.state.selector.as_ref().unwrap().searchable);
    current.input(Decoded::Text("TASK 9".into())).unwrap();
    assert_eq!(current.state.selector.as_ref().unwrap().filtered.len(), 1);
    assert_eq!(current.state.editor.text, "unfinished è draft");
    current.state.width = 40;
    current.state.height = 5;
    let frame = view::frame(&current.state, &current.archive.model, "project");
    let lines = frame
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>();
    assert!(lines.iter().any(|line| line.contains("Saved task 9")));
    current
        .input(Decoded::Key(terminal::Key {
            code: 27,
            modifiers: 0,
            character: 0,
        }))
        .unwrap();
    assert!(current.state.selector.is_none());
    assert_eq!(current.state.editor.text, "unfinished è draft");
    assert_eq!(current.state.editor.cursor, 3);
    assert_eq!(current.archive.messages.lock().unwrap().len(), 1);
}

#[test]
fn resume_rejects_another_folder_and_new_transfers_only_unsent_input() {
    let home = Directory::new();
    let a = Directory::new();
    let b = Directory::new();
    let mut first = app(&home, &a, "http://127.0.0.1:1/chat/completions");
    first
        .agent
        .as_mut()
        .unwrap()
        .prepare_turn("Session in folder A")
        .unwrap();
    let id = first.persistence.as_ref().unwrap().id();
    drop(first);
    let mut other = app(&home, &b, "http://127.0.0.1:1/chat/completions");
    other.open_sessions();
    assert!(other.saved_sessions.is_empty());
    other.resume_session(&id);
    assert_eq!(other.archive.messages.lock().unwrap().len(), 1);
    assert!(other.state.items.iter().any(|item| matches!(
        item,
        state::Item::Local {
            kind: Kind::Error,
            ..
        }
    )));
    other.state.editor.replace("saved draft".into());
    other.state.queue.push("next request".into()).unwrap();
    other.capture_input();
    let old = other.persistence.as_ref().unwrap().clone();
    other.dispatch("/new".into()).unwrap();
    assert_ne!(other.persistence.as_ref().unwrap().id(), old.id());
    assert!(!old.snapshot().input.meaningful());
    let input = other.persistence.as_ref().unwrap().snapshot().input;
    assert_eq!(input.draft.text, "saved draft");
    assert_eq!(input.queued, ["next request"]);
    assert!(
        other
            .archive
            .document()
            .encode()
            .contains("New conversation")
    );
    assert!(
        !other
            .archive
            .document()
            .encode()
            .contains("Session in folder A")
    );
}

#[test]
fn browsing_prompt_history_persists_the_unsent_draft_instead_of_the_recalled_prompt() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut current = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    current.state.history.record("earlier prompt");
    current.state.editor.replace("unsent draft".into());
    current.state.editor.cursor = 2;
    current
        .state
        .history
        .navigate(&mut current.state.editor, true);
    assert_eq!(current.state.editor.text, "earlier prompt");
    current.persist_input(true);
    let saved = current.persistence.as_ref().unwrap().snapshot().input;
    assert_eq!(saved.draft.text, "unsent draft");
    assert_eq!(saved.draft.cursor, 2);
    assert_eq!(saved.history, ["earlier prompt"]);
    assert!(saved.paused.is_empty());
}

#[test]
fn an_edited_history_recall_is_recovered_as_one_separate_paused_draft() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut current = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    saved_exchange(&current);
    current.state.history.record("earlier prompt");
    current.state.editor.replace("unsent β draft".into());
    current.state.editor.cursor = 2;
    let main = current.state.editor.clone();
    current
        .input(Decoded::Key(terminal::Key {
            code: 80,
            modifiers: 4,
            character: 0,
        }))
        .unwrap();
    current.input(Decoded::Text(" revised".into())).unwrap();
    current.state.editor.cursor = 4;
    let recall = current.state.editor.clone();
    for _ in 0..3 {
        current.persist_input(true);
        let input = current.persistence.as_ref().unwrap().snapshot().input;
        assert_eq!(input.draft.text, main.text);
        assert_eq!(input.draft.cursor, main.cursor);
        assert_eq!(
            input.paused,
            [crate::sessions::Draft {
                text: recall.text.clone(),
                cursor: recall.cursor,
            }]
        );
    }
    let id = current.persistence.as_ref().unwrap().id();
    drop(current);
    let mut resumed = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    resumed.resume_session(&id);
    assert_eq!(resumed.state.editor, main);
    assert_eq!(resumed.state.queue.paused, [recall]);
    assert!(resumed.state.queue.messages.is_empty());
    resumed.poll().unwrap();
    assert!(resumed.worker.is_none());
    assert_eq!(resumed.state.history.snapshot(), ["earlier prompt"]);
}

#[test]
fn a_failed_new_checkpoint_keeps_the_current_model_context_and_input() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut current = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    saved_exchange(&current);
    current.apply_selection("fixture/current".into(), Effort::High, false);
    current.state.editor.replace("keep this draft".into());
    current.persist_input(true);
    let id = current.persistence.as_ref().unwrap().id();
    let bucket = std::fs::read_dir(home.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let path = bucket.join(format!("{id}.jsonl"));
    let original = std::fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&path, readonly).unwrap();
    assert!(!current.dispatch("/new".into()).unwrap());
    assert_eq!(current.persistence.as_ref().unwrap().id(), id);
    assert_eq!(current.archive.model, "fixture/current");
    assert_eq!(current.state.effort, "high");
    assert_eq!(current.state.editor.text, "keep this draft");
    assert!(current.state.items.iter().any(|item| matches!(item, state::Item::Local { command, kind: Kind::Error, .. } if command == "/new")));
    std::fs::set_permissions(&path, original).unwrap();
    current.persist_input(true);
    assert!(current.save_error.is_none());
}
