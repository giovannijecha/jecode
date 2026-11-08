//! Completion visibility, saved row time and the observer of a resumed question.

use super::*;

#[test]
fn a_resumed_question_leaves_the_process_panel_on_its_own_conversation() {
    let home = Directory::new();
    let directory = Directory::new();
    let question = crate::json::parse(
        r#"{"questions":[{"id":"scope","title":"Scope","question":"Which rows?"}]}"#,
    )
    .unwrap();
    let fixture = HttpFixture::new(vec![(
        200,
        completion(
            "",
            vec![crate::test_support::tool_call(
                "ask-1", "ask_user", question,
            )],
        ),
    )]);
    let mut app = session_app(&home, &directory, &fixture);
    submit(&mut app, "Export.");
    wait_until(&mut app, |app| app.state.question.is_some());
    let id = app.persistence.as_ref().unwrap().id();
    key(&mut app, 81, 4);
    let confirm = terminal::Key {
        code: 13,
        modifiers: 0,
        character: 0,
    };
    assert!(app.input(Decoded::Key(confirm)).unwrap());
    drop(app);

    let mut resumed = session_app(&home, &directory, &fixture);
    resumed.resume_session(&id);
    assert!(
        resumed.agent.is_none(),
        "the question's worker holds the agent"
    );
    resumed.open_processes("");
    resumed.poll().unwrap();
    assert!(resumed.state.processes.is_some());
    drop(resumed);
    fixture.finish();
}

#[test]
fn a_failed_process_keeps_its_result_without_hiding_the_turn_status() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![bash(
                    "start",
                    "printf 'broken\\n' >&2; until [ -e go ]; do sleep 0.02; done; exit 3",
                    true,
                )],
            ),
        ),
        (200, completion("Started.", vec![])),
        (200, completion("It failed.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Build.");
    wait_until(&mut app, |app| app.worker.is_none());
    create(&directory, "go");
    wait_until(&mut app, |app| completion_rows(app) == 1);
    let transcript = rows(&app);
    assert!(transcript.contains("Exit 3"), "{transcript}");
    assert!(
        app.state.notice.is_none(),
        "completion must not create a notice"
    );
    wait_until(&mut app, |app| {
        app.worker.is_none() && rows(app).contains("It failed.")
    });
    let status = frame(&app).join("\n");
    assert!(status.contains("Done"), "{status}");
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert!(delivered(&requests[2].body));
}

#[test]
fn a_successful_process_keeps_the_turn_ending_visible() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![bash("start", GATED, true)])),
        (200, completion("Started.", vec![])),
        (200, completion("It passed.", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Build.");
    wait_until(&mut app, |app| app.worker.is_none());
    create(&directory, "go");
    wait_until(&mut app, |app| completion_rows(app) == 1);
    assert!(
        app.state.notice.is_none(),
        "completion must not create a notice"
    );
    wait_until(&mut app, |app| app.worker.is_none());
    let status = frame(&app).join("\n");
    assert!(status.contains("Done"), "{status}");
    let requests = fixture.finish();
    assert!(delivered(&requests[2].body));
}

#[test]
fn a_saved_process_row_keeps_its_time_on_resume() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![bash("start", GATED, true)])),
        (200, completion("Started.", vec![])),
    ]);
    let mut app = session_app(&home, &directory, &fixture);
    submit(&mut app, "Build.");
    wait_until(&mut app, |app| app.worker.is_none());
    submit(&mut app, "/settings");
    create(&directory, "go");
    wait_until(&mut app, |app| completion_rows(app) == 1);
    let id = app.persistence.as_ref().unwrap().id();
    drop(app);

    let mut resumed = session_app(&home, &directory, &fixture);
    resumed.resume_session(&id);
    let elapsed = resumed.state.items.iter().find_map(|item| match item {
        Item::Tool {
            id, presentation, ..
        } if id.starts_with("process:") => Some(presentation.elapsed),
        _ => None,
    });
    assert!(elapsed.unwrap().is_some());
    fixture.finish();
}
