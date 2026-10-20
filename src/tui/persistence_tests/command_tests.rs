use super::*;

#[test]
fn init_keeps_its_command_label_after_resume_without_rewriting_the_model_request() {
    let home = Directory::new();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![(200, completion("Instructions inspected.", vec![]))]);
    let mut first = app(&home, &directory, &fixture.endpoint);
    submit(&mut first, "/init");
    finish(&mut first);
    assert!(first.state.items.iter().any(|item| matches!(
        item,
        state::Item::Text { kind: Kind::User, text } if text == "/init"
    )));
    let id = first.persistence.as_ref().unwrap().id();
    drop(first);
    assert_eq!(fixture.finish().len(), 1);

    let mut resumed = app(&home, &directory, "http://127.0.0.1:1/chat/completions");
    resumed.resume_session(&id);
    assert!(resumed.state.items.iter().any(|item| matches!(
        item,
        state::Item::Text { kind: Kind::User, text } if text == "/init"
    )));
    assert!(
        resumed
            .archive
            .messages
            .lock()
            .unwrap()
            .iter()
            .any(|message| {
                message.get("role").and_then(Value::as_str) == Some("user")
                    && message.get("content").and_then(Value::as_str)
                        == Some(crate::session::commands::INIT_PROMPT)
            })
    );
    assert!(resumed.state.history.snapshot().is_empty());
}
