use super::super::*;
use crate::{
    json::Value,
    openrouter::OpenRouter,
    test_support::{Directory, HttpFixture, Response, completion},
    tools::Tools,
};
use std::{sync::mpsc, thread, time::Instant};

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        tests::config(directory),
        None,
    )
}

fn key(app: &mut App, code: u16, modifiers: u8) {
    assert!(
        !app.input(Decoded::Key(terminal::Key {
            code,
            modifiers,
            character: 0
        }))
        .unwrap()
    );
}

fn text(app: &mut App, value: &str) {
    app.input(Decoded::Text(value.into())).unwrap();
}

fn submit(app: &mut App, value: &str) {
    text(app, value);
    key(app, 13, 0);
}

fn finish(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while app.worker.is_some() {
        app.poll().unwrap();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for_response(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !app
        .state
        .items
        .iter()
        .any(|item| matches!(item, state::Item::Streaming { .. }))
    {
        app.poll().unwrap();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
}

fn frame_text(app: &App) -> String {
    view::frame(&app.state, "fixture/model", "fixture directory")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn editing_and_discarding_one_of_five_equal_drafts_keeps_the_composer_separate() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    for _ in 0..5 {
        app.state.queue.push("same draft".into()).unwrap();
    }
    text(&mut app, "original composer");
    app.state.editor.cursor = 3;
    let original = app.state.editor.clone();
    key(&mut app, 38, 1);
    assert!(app.draft_menu_open());
    assert_eq!(frame_text(&app).matches("same draft").count(), 5);
    key(&mut app, 13, 0);
    assert!(app.state.queue.is_editing());
    assert!(frame_text(&app).contains("Editing draft 1/5"));
    text(&mut app, " revised");
    key(&mut app, 13, 0);
    assert!(app.draft_menu_open());
    assert_eq!(app.state.queue.messages.len(), 5);
    assert_eq!(app.state.queue.messages[0], "same draft revised");
    assert_eq!(app.state.editor, original);
    key(&mut app, 68, 4);
    assert!(frame_text(&app).contains("Discard?"));
    key(&mut app, 13, 0);
    assert_eq!(app.state.queue.messages.len(), 4);
    assert!(
        app.state
            .queue
            .messages
            .iter()
            .all(|draft| draft == "same draft")
    );
    key(&mut app, 13, 0);
    text(&mut app, " discarded edit");
    key(&mut app, 27, 0);
    assert!(app.draft_menu_open());
    assert_eq!(app.state.queue.messages.len(), 4);
    assert_eq!(app.state.editor, original);
    key(&mut app, 27, 0);
    assert_eq!(app.state.editor, original);
    assert!(!app.state.editor.text.contains("same draft"));
    assert!(fixture.finish().is_empty());
}

#[test]
fn numbered_choices_cancel_discard_marks_and_empty_drafts_panel_stays_open() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.queue.push("one".into()).unwrap();
    app.open_drafts();
    key(&mut app, 68, 4);
    text(&mut app, "1");
    assert_eq!(app.state.queue.len(), 1);
    assert!(
        app.state
            .selector
            .as_ref()
            .unwrap()
            .delete_target()
            .is_none()
    );
    key(&mut app, 68, 4);
    key(&mut app, 13, 0);
    assert!(app.draft_menu_open());
    assert!(frame_text(&app).contains("No pending drafts"));
    key(&mut app, 27, 0);
    assert!(app.state.selector.is_none());
    fixture.finish();
}

#[test]
fn fifo_automatic_sending_waits_for_the_draft_editor_and_keeps_prompt_only_history() {
    let directory = Directory::new();
    let (release, wait) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        Response::GatedStream {
            head: "data: {\"choices\":[{\"delta\":{\"content\":\"First.\"},\"finish_reason\":null}]}\n\n".into(),
            tail: "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(),
            release: wait,
        },
        Response::Json(200, completion("Second.", vec![])),
        Response::Json(200, completion("Third.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "active");
    submit(&mut app, "queued one");
    submit(&mut app, "queued two");
    text(&mut app, "main draft");
    app.state.editor.cursor = 2;
    let main = app.state.editor.clone();
    assert_eq!(app.state.history.snapshot(), ["active"]);
    key(&mut app, 38, 1);
    key(&mut app, 40, 0);
    key(&mut app, 13, 0);
    text(&mut app, " revised");
    release.send(()).unwrap();
    finish(&mut app);
    assert!(app.worker.is_none());
    assert_eq!(app.state.queue.messages.len(), 2);
    assert!(app.state.queue.is_editing());
    key(&mut app, 13, 0);
    app.poll().unwrap();
    assert!(app.worker.is_none());
    assert!(app.draft_menu_open());
    key(&mut app, 27, 0);
    app.poll().unwrap();
    finish(&mut app);
    assert_eq!(app.state.editor, main);
    assert!(app.state.queue.is_empty());
    assert_eq!(
        app.state.history.snapshot(),
        ["active", "queued one", "queued two revised"]
    );
    app.dispatch("/help".into()).unwrap();
    key(&mut app, 27, 0);
    assert_eq!(app.state.history.snapshot().len(), 3);
    key(&mut app, 80, 4);
    assert_eq!(app.state.editor.text, "queued two revised");
    key(&mut app, 27, 0);
    assert_eq!(app.state.editor, main);
    let requests = fixture.finish();
    let prompts: Vec<_> = requests
        .iter()
        .map(|request| {
            request
                .body
                .get("messages")
                .unwrap()
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .get("content")
                .and_then(Value::as_str)
                .unwrap()
        })
        .collect();
    assert_eq!(prompts, ["active", "queued one", "queued two revised"]);
}

#[test]
fn a_paused_draft_can_be_edited_and_sent_explicitly_without_sending_the_main_draft() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Done", vec![]))]);
    let mut app = app(&directory, &fixture);
    let mut paused = editor::Editor::default();
    paused.replace("paused request".into());
    app.state.queue.paused.push(paused);
    text(&mut app, "main draft");
    app.state.editor.cursor = 2;
    let main = app.state.editor.clone();
    app.open_drafts();
    assert!(frame_text(&app).contains("Ctrl+S send"));
    key(&mut app, 13, 0);
    text(&mut app, " revised");
    key(&mut app, 83, 4);
    finish(&mut app);
    assert!(app.state.queue.is_empty());
    assert_eq!(app.state.editor, main);
    assert_eq!(app.state.history.snapshot(), ["paused request revised"]);
    let requests = fixture.finish();
    assert_eq!(
        requests[0]
            .body
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .get("content")
            .and_then(Value::as_str),
        Some("paused request revised")
    );
}

#[test]
fn sending_a_paused_edit_during_work_appends_it_once_to_the_automatic_queue() {
    let directory = Directory::new();
    let (release, wait) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![
        Response::GatedStream {
            head: "data: {\"choices\":[{\"delta\":{\"content\":\"Active.\"},\"finish_reason\":null}]}\n\n".into(),
            tail: "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(),
            release: wait,
        },
        Response::Json(200, completion("Queued.", vec![])),
        Response::Json(200, completion("Paused.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "active");
    submit(&mut app, "already queued");
    let mut paused = editor::Editor::default();
    paused.replace("paused".into());
    app.state.queue.paused.push(paused);
    text(&mut app, "main draft");
    app.state.editor.cursor = 3;
    let main = app.state.editor.clone();
    app.open_drafts();
    key(&mut app, 40, 0);
    key(&mut app, 13, 0);
    text(&mut app, " revised");
    key(&mut app, 83, 4);
    assert!(!app.state.queue.is_editing());
    assert!(app.state.queue.paused.is_empty());
    assert_eq!(
        app.state.queue.messages,
        std::collections::VecDeque::from(["already queued".into(), "paused revised".into(),])
    );
    assert_eq!(app.state.editor, main);
    assert_eq!(app.state.history.snapshot(), ["active"]);
    release.send(()).unwrap();
    finish(&mut app);
    assert!(app.state.queue.is_empty());
    assert_eq!(
        app.state.history.snapshot(),
        ["active", "already queued", "paused revised"]
    );
    assert_eq!(app.state.editor, main);
    assert_eq!(fixture.finish().len(), 3);
}

#[test]
fn sending_a_paused_edit_to_a_full_queue_keeps_the_live_edit_and_original_slot() {
    let directory = Directory::new();
    let (release, wait) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![Response::GatedStream {
        head:
            "data: {\"choices\":[{\"delta\":{\"content\":\"Active.\"},\"finish_reason\":null}]}\n\n"
                .into(),
        tail: "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
            .into(),
        release: wait,
    }]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "active");
    for i in 0..8 {
        app.state.queue.push(format!("queued {i}")).unwrap();
    }
    let mut paused = editor::Editor::default();
    paused.replace("paused".into());
    app.state.queue.paused.push(paused);
    text(&mut app, "main draft");
    app.state.editor.cursor = 2;
    let main = app.state.editor.clone();
    app.open_drafts();
    for _ in 0..8 {
        key(&mut app, 40, 0);
    }
    key(&mut app, 13, 0);
    text(&mut app, " revision");
    let edited = app.state.editor.clone();
    key(&mut app, 83, 4);
    assert_eq!(app.state.queue.messages.len(), 8);
    assert_eq!(app.state.queue.paused[0].text, "paused");
    assert_eq!(app.state.queue.edit_index(), Some(8));
    assert_eq!(app.state.editor, edited);
    assert!(
        app.state
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("Queue is full")
    );
    key(&mut app, 27, 0);
    key(&mut app, 27, 0);
    assert_eq!(app.state.editor, main);
    wait_for_response(&mut app);
    key(&mut app, 67, 4);
    release.send(()).unwrap();
    finish(&mut app);
    assert!(app.state.queue.messages.is_empty());
    assert_eq!(app.state.queue.paused.len(), 9);
    assert_eq!(app.state.editor, main);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn interruption_while_editing_changes_the_slot_to_paused_without_replacing_the_composer() {
    let directory = Directory::new();
    let (release, wait) = mpsc::channel();
    let fixture = HttpFixture::streaming(vec![Response::GatedStream {
        head:
            "data: {\"choices\":[{\"delta\":{\"content\":\"Active.\"},\"finish_reason\":null}]}\n\n"
                .into(),
        tail:
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"error\"}]}\n\ndata: [DONE]\n\n"
                .into(),
        release: wait,
    }]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "active");
    submit(&mut app, "queued one");
    submit(&mut app, "queued two");
    text(&mut app, "main draft");
    app.state.editor.cursor = 2;
    let main = app.state.editor.clone();
    app.open_drafts();
    key(&mut app, 40, 0);
    key(&mut app, 13, 0);
    text(&mut app, " revised");
    app.state.editor.cursor = 4;
    let edited = app.state.editor.clone();
    release.send(()).unwrap();
    finish(&mut app);
    assert!(app.state.queue.messages.is_empty());
    assert_eq!(app.state.queue.edit_index(), Some(1));
    assert_eq!(app.state.editor, edited);
    assert_eq!(app.state.queue.paused.len(), 2);
    assert!(frame_text(&app).contains("Ctrl+S send"));
    key(&mut app, 13, 0);
    assert!(app.draft_menu_open());
    assert_eq!(app.state.editor, main);
    assert_eq!(app.state.queue.paused[0].text, "queued one");
    assert_eq!(app.state.queue.paused[1], edited);
    key(&mut app, 27, 0);
    app.poll().unwrap();
    assert!(app.worker.is_none());
    assert_eq!(app.state.queue.paused.len(), 2);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn arrows_move_only_in_the_draft_and_history_edits_preserve_the_original_caret() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.history.restore(vec![
        "/help".into(),
        "sent text".into(),
        " /model fixture".into(),
    ]);
    text(&mut app, "main draft");
    app.state.editor.cursor = 4;
    let main = app.state.editor.clone();
    key(&mut app, 38, 0);
    assert_eq!(app.state.editor, main);
    key(&mut app, 80, 4);
    assert_eq!(app.state.editor.text, "sent text");
    text(&mut app, " revised");
    key(&mut app, 78, 4);
    assert_eq!(app.state.editor, main);
    key(&mut app, 80, 4);
    app.open_drafts();
    assert_eq!(app.state.editor, main);
    assert_eq!(app.state.history.snapshot(), ["sent text"]);
    fixture.finish();
}

#[test]
fn draft_panels_and_edit_controls_fit_small_viewports_without_losing_text() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state
        .queue
        .push("long queued draft β🙂\nsecond line".into())
        .unwrap();
    app.state.editor.replace("kept main draft".into());
    let main = app.state.editor.clone();
    for (width, height) in [(80, 24), (40, 12), (18, 6), (7, 3), (2, 2)] {
        app.state.width = width;
        app.state.height = height;
        app.open_drafts();
        key(&mut app, 68, 4);
        let frame = view::frame(&app.state, "fixture/model", "fixture directory");
        assert!(frame.live.len() <= height);
        assert!(
            frame
                .live
                .iter()
                .all(|line| text::cells(&line.plain()) <= width)
        );
        key(&mut app, 27, 0);
        key(&mut app, 13, 0);
        let frame = view::frame(&app.state, "fixture/model", "fixture directory");
        assert!(frame.live.len() <= height);
        assert!(frame.cursor.is_some());
        assert!(
            frame
                .live
                .iter()
                .all(|line| text::cells(&line.plain()) <= width)
        );
        key(&mut app, 27, 0);
        key(&mut app, 27, 0);
        assert_eq!(app.state.editor, main);
        assert_eq!(app.state.queue.len(), 1);
    }
    fixture.finish();
}
