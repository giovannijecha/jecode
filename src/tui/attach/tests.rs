use super::super::{Decoded, terminal, tests};
use super::*;
use crate::{
    agent::Agent,
    attachments::{MARKER, Prompt},
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, completion},
    tools::Tools,
};
use std::time::{Duration, Instant};

fn app(home: &Directory, directory: &Directory, endpoint: &str) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture(endpoint.into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    App::new(agent, tests::config(directory), None)
}

fn key(app: &mut App, code: u16, modifiers: u8) {
    assert!(
        !app.input(Decoded::Key(terminal::Key {
            code,
            modifiers,
            character: 0,
        }))
        .unwrap()
    );
}

fn settle(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !app.imports.is_empty() || app.worker.is_some() {
        app.poll().unwrap();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
}

fn notice(app: &App) -> String {
    app.state.notice.as_ref().unwrap().text.clone()
}

fn png() -> Vec<u8> {
    crate::attachments::tests::png(2, 2)
}

const OFFLINE: &str = "http://127.0.0.1:1/chat/completions";

#[test]
fn native_paste_overflow_keeps_the_draft_and_accepts_the_next_key() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut app = app(&home, &directory, OFFLINE);
    let mut decoder = super::super::Decoder::default();
    app.input(Decoded::Text("keep".into())).unwrap();
    app.terminal_event(
        terminal::Input::Paste("\x1b[200~incomplete".encode_utf16().collect()),
        &mut decoder,
    )
    .unwrap();
    app.terminal_event(terminal::Input::PasteOverflow, &mut decoder)
        .unwrap();
    assert_eq!(app.state.editor.display(), "keep");
    assert!(notice(&app).contains("1 MiB"));
    app.terminal_event(
        terminal::Input::Key(terminal::Key {
            code: 69,
            modifiers: 0,
            character: 101,
        }),
        &mut decoder,
    )
    .unwrap();
    assert_eq!(app.state.editor.display(), "keepe");
    assert!(app.imports.is_empty());
    assert!(app.worker.is_none());
}

#[test]
fn submit_keeps_the_draft_until_its_attachment_import_finishes() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Seen together.", vec![]))]);
    let mut app = app(&home, &directory, &fixture.endpoint);
    app.input(Decoded::Text("explain".into())).unwrap();
    let pool = app.attachment_pool().unwrap();
    let (release, wait) = std::sync::mpsc::channel();
    app.imports.push(Import {
        cancellation: Cancellation::default(),
        session: app.persistence.as_ref().map(|handle| handle.id()),
        task: Some(thread::spawn(move || {
            vec![
                wait.recv_timeout(Duration::from_secs(4))
                    .map_err(|error| error.to_string())
                    .and_then(|()| pool.import_bytes("shot.png", &png())),
            ]
        })),
    });
    key(&mut app, 13, 0);
    assert!(app.worker.is_none());
    assert_eq!(app.state.editor.text, "explain");
    assert!(app.state.queue.messages.is_empty());
    assert!(notice(&app).contains("still importing"));
    release.send(()).unwrap();
    settle(&mut app);
    assert_eq!(app.state.editor.display(), "explain[1# Image] ");
    key(&mut app, 13, 0);
    settle(&mut app);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    let user = requests[0]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap()
        .last()
        .unwrap();
    assert!(user.encode().contains("explain"));
    assert!(user.encode().contains("image_url"));
}

#[test]
fn a_dropped_path_attaches_and_backspace_removes_the_whole_element() {
    let home = Directory::new();
    let directory = Directory::new();
    let outside = Directory::new();
    let source = outside.path().join("report final.pdf");
    std::fs::write(&source, b"%PDF-1.4 /Type /Page").unwrap();
    let mut app = app(&home, &directory, OFFLINE);
    app.input(Decoded::Text("see ".into())).unwrap();
    app.input(Decoded::Paste(format!("\"{}\"", source.display())))
        .unwrap();
    settle(&mut app);
    assert_eq!(
        app.state.editor.display(),
        "see [1# File: report final.pdf] "
    );
    assert_eq!(app.state.editor.attachments[0].name, "report final.pdf");
    assert_eq!(notice(&app), "Attached 1 item");
    // The stored copy no longer depends on the dropped file.
    std::fs::remove_file(&source).unwrap();
    let pool = app.attachment_pool().unwrap();
    assert!(pool.load(&app.state.editor.attachments[0].id).is_ok());
    key(&mut app, 8, 0);
    key(&mut app, 8, 0);
    assert_eq!(app.state.editor.display(), "see ");
    assert!(app.state.editor.attachments.is_empty());
    // A lookalike label, typed or pasted, stays text.
    app.input(Decoded::Paste("[1# Image]".into())).unwrap();
    app.input(Decoded::Text(format!("{MARKER}[2# File: x.pdf]")))
        .unwrap();
    assert!(app.imports.is_empty());
    assert_eq!(app.state.editor.text, "see [1# Image][2# File: x.pdf]");
    assert!(app.state.editor.attachments.is_empty());
}

#[test]
fn alt_v_reads_the_clipboard_only_on_request_and_failures_keep_the_draft() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut app = app(&home, &directory, OFFLINE);
    app.input(Decoded::Text("look".into())).unwrap();
    app.clipboard_image = Some(Ok(png()));
    // Ctrl+Alt+V is not the attachment shortcut.
    key(&mut app, 86, 5);
    assert!(app.imports.is_empty());
    key(&mut app, 86, 1);
    settle(&mut app);
    assert_eq!(app.state.editor.display(), "look[1# Image] ");
    assert_eq!(app.state.editor.attachments[0].media, "image/png");
    key(&mut app, 86, 1);
    settle(&mut app);
    assert_eq!(
        notice(&app),
        "The clipboard has no image Your draft was kept."
    );
    assert_eq!(app.state.editor.display(), "look[1# Image] ");
}

#[test]
fn attach_command_and_attachment_only_messages_reach_the_provider_as_parts() {
    let home = Directory::new();
    let directory = Directory::new();
    std::fs::write(directory.path().join("shot.png"), png()).unwrap();
    let fixture = HttpFixture::new(vec![(200, completion("Seen", vec![]))]);
    let mut app = app(&home, &directory, &fixture.endpoint);
    app.dispatch(Prompt::plain("/attach missing.png")).unwrap();
    settle(&mut app);
    assert!(notice(&app).contains("Your draft was kept."));
    assert!(app.state.editor.attachments.is_empty());
    app.dispatch(Prompt::plain("/attach shot.png")).unwrap();
    settle(&mut app);
    assert_eq!(app.state.editor.display(), "[1# Image] ");
    assert!(!app.submit().unwrap());
    settle(&mut app);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    let messages = requests[0]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    let user = messages.last().unwrap();
    let parts = user.get("content").and_then(Value::as_array).unwrap();
    assert_eq!(
        parts[1].get("type").and_then(Value::as_str),
        Some("image_url")
    );
    assert!(user.get("attachments").is_none());
    // The saved message keeps a reference, never the bytes.
    let saved = app.archive.messages.lock().unwrap().clone();
    let stored = saved
        .iter()
        .find(|message| message.get("attachments").is_some())
        .unwrap();
    assert_eq!(
        stored.get("content").and_then(Value::as_str),
        Some("[1# Image] ")
    );
    assert!(!stored.encode().contains("base64"));
    // Recalling the prompt restores the attachment itself.
    key(&mut app, 80, 4);
    assert_eq!(app.state.editor.attachments.len(), 1);
    assert_eq!(app.state.editor.display(), "[1# Image] ");
}

#[test]
fn draft_attachments_survive_a_restart_and_resume() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut current = app(&home, &directory, OFFLINE);
    for (role, content) in [("user", "Earlier request"), ("assistant", "Earlier answer")] {
        current
            .archive
            .messages
            .lock()
            .unwrap()
            .push(Value::object([
                ("role", Value::string(role)),
                ("content", Value::string(content)),
            ]));
    }
    current.agent.as_ref().unwrap().save_session().unwrap();
    current.clipboard_image = Some(Ok(png()));
    current.input(Decoded::Text("keep".into())).unwrap();
    key(&mut current, 86, 1);
    settle(&mut current);
    let draft = current.state.editor.prompt();
    let id = current.persistence.as_ref().unwrap().id();
    // Dropping the app collects unreferenced assets, not this draft's.
    drop(current);
    let mut resumed = app(&home, &directory, OFFLINE);
    resumed.resume_session(&id);
    assert_eq!(resumed.state.editor.prompt(), draft);
    let pool = resumed.attachment_pool().unwrap();
    assert!(pool.view(&draft.attachments[0]).unwrap().is_some());
}

#[test]
fn plain_staged_attachments_become_one_visible_tui_element() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut first = Agent::new(
        OpenRouter::fixture(OFFLINE.into()),
        Tools::new(directory.path()).unwrap(),
    );
    first.enable_sessions(home.path()).unwrap();
    let handle = first.sessions().unwrap();
    let pool = handle.store().attachments();
    let attachment = pool.import_bytes("pending.txt", b"pending").unwrap();
    let mut input = handle.snapshot().input;
    input.draft = crate::sessions::Draft::from_prompt("existing draft".into());
    input.staged.push(attachment.clone());
    handle.input(input);
    handle.flush().unwrap();
    let id = handle.id();
    drop(handle);
    drop(first);

    let mut resumed = app(&home, &directory, OFFLINE);
    resumed.resume_session(&id);
    assert_eq!(
        resumed.state.editor.display(),
        "existing draft[1# File: pending.txt] "
    );
    let saved = resumed.persistence.as_ref().unwrap().snapshot().input;
    assert!(saved.staged.is_empty());
    assert_eq!(saved.draft.attachments, vec![attachment.clone()]);
    drop(resumed);

    let mut reopened = app(&home, &directory, OFFLINE);
    reopened.resume_session(&id);
    assert_eq!(reopened.state.editor.attachments, vec![attachment]);
}

#[test]
fn an_import_that_finishes_after_a_session_change_is_not_inserted() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut app = app(&home, &directory, OFFLINE);
    app.attach(Source::Image(Ok(png())));
    app.dispatch(Prompt::plain("/new")).unwrap();
    settle(&mut app);
    assert!(app.state.editor.attachments.is_empty());
    assert_eq!(
        notice(&app),
        "An attachment finished after the session changed; attach it again."
    );
}

#[test]
fn a_message_queued_during_a_turn_keeps_its_attachment() {
    let home = Directory::new();
    let directory = Directory::new();
    let slow = crate::test_support::Response::Stream(vec![
        (Duration::from_millis(400), "data: {\"choices\":[{\"delta\":{\"content\":\"First.\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into()),
    ]);
    let fixture = HttpFixture::streaming(vec![
        slow,
        crate::test_support::Response::Json(200, completion("Second.", vec![])),
    ]);
    let mut app = app(&home, &directory, &fixture.endpoint);
    app.input(Decoded::Text("first".into())).unwrap();
    key(&mut app, 13, 0);
    assert!(app.worker.is_some());
    app.attach(Source::Image(Ok(png())));
    while !app.imports.is_empty() {
        app.poll().unwrap();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.worker.is_some(),
        "the import finished while the turn ran"
    );
    key(&mut app, 13, 0);
    assert_eq!(app.state.queue.messages.len(), 1);
    assert_eq!(app.state.queue.messages[0].attachments.len(), 1);
    settle(&mut app);
    while !app.state.queue.messages.is_empty() || app.worker.is_some() {
        settle(&mut app);
        app.poll().unwrap();
    }
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    let last = requests[1]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    assert!(last.last().unwrap().encode().contains("image_url"));
}
