use super::super::*;
use crate::{
    attachments::{MARKER, Prompt},
    backend::openrouter::OpenRouter,
    events::Event,
    json::Value,
    test_support::{Directory, HttpFixture, Response, completion, tool_call},
    tools::Tools,
};

fn app(home: &Directory, directory: &Directory, fixture: &HttpFixture) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    App::new(agent, tests::config(directory), None)
}

fn key(app: &mut App, code: u16) {
    app.input(Decoded::Key(terminal::Key {
        code,
        modifiers: 0,
        character: 0,
    }))
    .unwrap();
}

fn held(head: &str) -> (Response, std::sync::mpsc::Sender<()>) {
    let (release, wait) = std::sync::mpsc::channel();
    (
        Response::GatedStream {
            head: head.into(),
            tail: "data: [DONE]\n\n".into(),
            release: wait,
        },
        release,
    )
}

fn rejection() -> (u16, Value) {
    (
        404,
        Value::object([(
            "error",
            Value::object([("message", Value::string("No image endpoint"))]),
        )]),
    )
}

#[test]
fn failed_image_prompt_returns_to_composer_and_resumed_retry_sends_one_image() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![rejection(), (200, completion("Done.", vec![]))]);
    let mut app = app(&home, &directory, &fixture);
    let attachment = app
        .persistence
        .as_ref()
        .unwrap()
        .store()
        .attachments()
        .import_bytes("clipboard.png", &crate::attachments::tests::png(2, 2))
        .unwrap();
    let prompt = Prompt::new(format!("{MARKER} Build this mock"), vec![attachment]);
    app.state.editor.set(prompt.clone());
    app.submit().unwrap();
    tests::finish(&mut app);
    assert_eq!(app.state.editor.prompt(), prompt);
    assert!(tests::rows(&app).is_empty());
    let saved = app.persistence.as_ref().unwrap().snapshot();
    assert_eq!(saved.input.draft.prompt(), prompt);
    assert!(saved.records().iter().all(
        |record| !matches!(record, crate::sessions::Record::Text { role, .. } if role == "user")
    ));
    let id = app.persistence.as_ref().unwrap().id();
    drop(app);
    let mut app = self::app(&home, &directory, &fixture);
    app.resume_session(&id);
    assert_eq!(app.state.editor.prompt(), prompt);
    assert!(tests::rows(&app).is_empty());
    app.submit().unwrap();
    tests::finish(&mut app);
    let requests = fixture.finish();
    let messages = requests[1]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    let images = messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("image_url"))
        .count();
    assert_eq!(images, 1);
    assert_eq!(
        tests::rows(&app),
        [
            format!("user: {}", prompt.display()),
            "assistant: Done.".into()
        ]
    );
}

#[test]
fn failure_keeps_meaningful_drafts_and_ignores_whitespace() {
    for (text, attach, keep) in [
        ("New unsent text", false, true),
        (" \t", false, false),
        ("\n", false, false),
        ("", true, true),
    ] {
        let home = Directory::new();
        let directory = Directory::new();
        let fixture = HttpFixture::new(vec![rejection()]);
        let mut app = app(&home, &directory, &fixture);
        app.state.editor.insert("Failed request");
        app.submit().unwrap();
        app.state.editor.insert(text);
        if attach {
            let attachment = app
                .persistence
                .as_ref()
                .unwrap()
                .store()
                .attachments()
                .import_bytes("clipboard.png", &crate::attachments::tests::png(2, 2))
                .unwrap();
            app.state.editor.attach(attachment);
        }
        app.state.editor.cursor = 0;
        let draft = app.state.editor.clone();
        tests::finish(&mut app);
        assert_eq!(app.state.editor.text, "Failed request");
        assert_eq!(app.state.queue.paused.is_some(), keep);
        let input = app.persistence.as_ref().unwrap().snapshot().input;
        assert_eq!(input.paused.is_some(), keep);
        if let Some(saved) = input.paused {
            assert_eq!(app.state.queue.paused.as_ref(), Some(&draft));
            assert_eq!(saved.prompt(), draft.prompt());
            assert_eq!(saved.cursor, draft.cursor);
        }
        fixture.finish();
    }
}

#[test]
fn escape_before_output_restores_prompt_accepted_followups_and_saved_composer() {
    let home = Directory::new();
    let directory = Directory::new();
    let (response, release) = held(
        "data: {\"choices\":[{\"delta\":{\"reasoning\":\"Thinking\"},\"finish_reason\":null}]}\n\n",
    );
    let fixture = HttpFixture::streaming(vec![response]);
    let mut app = app(&home, &directory, &fixture);
    app.state.queue.push("Joined request").unwrap();
    app.capture_input();
    app.state.editor.insert("Original request");
    app.submit().unwrap();
    tests::wait_until(&mut app, |app| app.state.status == "Thinking…");
    app.state.editor.insert("Unsent draft");
    key(&mut app, 27);
    // Let the socket finish after cancellation, as the worker is being stopped.
    release.send(()).unwrap();
    tests::finish(&mut app);
    // The unanswered requests return as one draft; newer text is paused.
    assert_eq!(app.state.editor.text, "Original request\n\nJoined request");
    assert_eq!(
        app.state.queue.pending(),
        Some(("Unsent draft".into(), true))
    );
    assert!(tests::rows(&app).is_empty());
    let id = app.persistence.as_ref().unwrap().id();
    let store = app.persistence.as_ref().unwrap().store();
    drop(app);
    let (handle, _) =
        crate::sessions::Handle::open(store, &id, crate::redact::Redactor::empty()).unwrap();
    let saved = handle.snapshot();
    assert_eq!(saved.input.draft.text, "Original request\n\nJoined request");
    assert!(saved.records().iter().all(
        |record| !matches!(record, crate::sessions::Record::Text { role, .. } if role == "user")
    ));
    assert_eq!(saved.context.returned.len(), 2);
    fixture.finish();
}

#[test]
fn escape_after_partial_text_keeps_the_transcript_and_new_draft() {
    let home = Directory::new();
    let directory = Directory::new();
    let (response, release) = held(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Partial answer\"},\"finish_reason\":null}]}\n\n",
    );
    let fixture = HttpFixture::streaming(vec![response]);
    let mut app = app(&home, &directory, &fixture);
    app.state.editor.insert("Original request");
    app.submit().unwrap();
    tests::wait_until(&mut app, |app| {
        app.state
            .items
            .iter()
            .any(|item| matches!(item, state::Item::Streaming { .. }))
    });
    app.state.editor.insert("Continue later");
    key(&mut app, 27);
    release.send(()).unwrap();
    tests::finish(&mut app);
    assert_eq!(app.state.editor.text, "Continue later");
    assert_eq!(
        tests::rows(&app),
        ["user: Original request", "assistant: Partial answer"]
    );
    assert!(
        app.persistence
            .as_ref()
            .unwrap()
            .snapshot()
            .context
            .returned
            .is_empty()
    );
    fixture.finish();
}

#[test]
fn error_after_a_tool_preserves_completed_work_for_continue() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "write",
                    "write",
                    Value::object([
                        ("path", Value::string("result.txt")),
                        ("content", Value::string("saved")),
                    ]),
                )],
            ),
        ),
        rejection(),
    ]);
    let mut app = app(&home, &directory, &fixture);
    app.state.editor.insert("Original request");
    app.submit().unwrap();
    tests::finish(&mut app);
    assert!(app.state.editor.prompt().is_empty());
    assert_eq!(tests::rows(&app), ["user: Original request"]);
    assert!(app.state.items.iter().any(|item| matches!(
        item,
        state::Item::Tool {
            result: Some(_),
            ..
        }
    )));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("result.txt")).unwrap(),
        "saved"
    );
    assert!(
        app.persistence
            .as_ref()
            .unwrap()
            .snapshot()
            .context
            .returned
            .is_empty()
    );
    fixture.finish();
}

#[test]
fn error_after_partial_text_keeps_the_prompt_and_response_in_the_transcript() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::streaming(vec![Response::Stream(vec![
        (Duration::ZERO, "data: {\"choices\":[{\"delta\":{\"content\":\"Visible response\"},\"finish_reason\":null}]}\n\n".into()),
        (Duration::ZERO, "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"error\"}]}\n\ndata: [DONE]\n\n".into()),
    ])]);
    let mut app = app(&home, &directory, &fixture);
    app.state.editor.insert("Original request");
    app.submit().unwrap();
    tests::finish(&mut app);
    assert!(app.state.editor.prompt().is_empty());
    assert_eq!(
        tests::rows(&app),
        ["user: Original request", "assistant: Visible response"]
    );
    assert!(
        app.persistence
            .as_ref()
            .unwrap()
            .snapshot()
            .context
            .returned
            .is_empty()
    );
    fixture.finish();
}

#[test]
fn failed_queued_preparation_while_recalling_history_keeps_the_failed_prompt_after_escape() {
    for edited in [false, true] {
        let home = Directory::new();
        let root = Directory::new();
        let fixture = HttpFixture::new(vec![]);
        let mut app = app(&home, &root, &fixture);
        app.state.history.record("Older prompt");
        app.state.editor.insert("My unsent draft");
        app.state.editor.cursor = 3;
        let draft = app.state.editor.clone();
        app.state.history.navigate(&mut app.state.editor, true);
        if edited {
            app.input(Decoded::Text(" revised".into())).unwrap();
        }
        std::fs::write(root.path().join("AGENTS.md"), "x".repeat(64 * 1024 + 1)).unwrap();
        app.state.queue.push("Queued task").unwrap();
        app.capture_input();
        app.poll().unwrap();
        assert!(app.worker.is_none());
        assert_eq!(app.state.editor.text, "Queued task");
        app.input(Decoded::Key(terminal::Key {
            code: 27,
            modifiers: 0,
            character: 0,
        }))
        .unwrap();
        assert_eq!(
            app.state.editor.text, "Queued task",
            "Escape discarded the recovered prompt"
        );
        // The edited recall and the unsent draft join in one paused draft.
        let paused = app.state.queue.paused.clone().unwrap();
        if edited {
            assert_eq!(paused.text, "Older prompt revised\n\nMy unsent draft");
        } else {
            assert_eq!(paused, draft);
        }
        let input = app.persistence.as_ref().unwrap().snapshot().input;
        assert_eq!(input.draft.text, "Queued task");
        assert_eq!(input.paused.unwrap().text, paused.text);
        fixture.finish();
    }
}

#[test]
fn retry_that_discards_its_partial_response_restores_unanswered_input() {
    let home = Directory::new();
    let root = Directory::new();
    let (release, gate) = std::sync::mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        Response::GatedStream {
            head: "data: {\"choices\":[{\"delta\":{\"content\":\"Discarded partial\"},\"finish_reason\":null}]}\n\n".into(),
            tail: "data: {\"error\":{\"code\":503,\"message\":\"Temporary fixture failure\"}}\n\n".into(),
            release: gate,
        },
        Response::Json(404, Value::object([("error", Value::object([("message", Value::string("No endpoint"))]))])),
    ]);
    let mut app = app(&home, &root, &fixture);
    app.state.editor.insert("Unanswered request");
    app.submit().unwrap();
    tests::wait_until(&mut app, |app| {
        app.state
            .items
            .iter()
            .any(|item| matches!(item, state::Item::Streaming { .. }))
    });
    release.send(()).unwrap();
    tests::finish(&mut app);
    assert_eq!(app.state.editor.text, "Unanswered request");
    assert!(tests::rows(&app).is_empty());
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn a_discarded_preview_does_not_count_as_visible_output() {
    let root = Directory::new();
    let agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(root.path()).unwrap(),
    );
    let mut submission = super::Submission::new(agent.input_checkpoint());
    submission.observe(&Event::Streaming {
        text: "Preview".into(),
    });
    assert!(submission.has_output());
    submission.observe(&Event::RequestDiscarded);
    assert!(!submission.has_output());
    submission.observe(&Event::Message {
        text: "Confirmed response".into(),
    });
    submission.observe(&Event::RequestDiscarded);
    assert!(submission.has_output());
}
