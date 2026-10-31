use super::*;

#[test]
fn tui_submission_keeps_a_failed_draft_and_loads_rules_when_retried() {
    let directory = Directory::new();
    let path = directory.path().join("AGENTS.md");
    fs::write(&path, [0xff, 0xfe]).unwrap();
    let fixture = HttpFixture::new(vec![(200, completion("Ready", vec![]))]);
    let mut app = app(&directory, &fixture);
    submit(&mut app, "Inspect the project");
    assert!(app.worker.is_none());
    assert_eq!(app.archive.messages.lock().unwrap().len(), 1);
    assert!(app.state.notice.as_ref().unwrap().text.contains("UTF-8"));
    assert_eq!(app.state.editor.text, "Inspect the project");
    assert!(app.state.queue.paused.is_none());
    fs::write(path, "TUI project rules").unwrap();
    assert!(!app.submit().unwrap());
    finish(&mut app);
    assert_eq!(app.state.status, "Ready");
    assert!(app.state.queue.paused.is_none());
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    let messages = requests[0]
        .body
        .get("messages")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(
        messages[0]
            .get("content")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("TUI project rules")
    );
    assert_eq!(
        messages
            .iter()
            .rev()
            .find(|message| {
                message.get("role").and_then(Value::as_str) == Some("user")
                    && message.get("content").and_then(Value::as_str) == Some("Inspect the project")
            })
            .unwrap()
            .get("content")
            .unwrap()
            .as_str(),
        Some("Inspect the project")
    );
    assert!(runtime_state(&requests[0]).contains("Permissions:"));
}
