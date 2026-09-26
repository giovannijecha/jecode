//! Recalled prompts and withdrawn queued messages retain separate draft owners.
use super::*;

#[test]
fn a_withdrawn_message_restores_the_previous_draft_after_queueing() {
    let (mut model, mut session, entered, release) = running(false);
    queue(&mut model, &mut session, "withdraw me");
    model.editor.insert("previous\n  draft");
    model.editor.left();
    let original_cursor = model.editor.cursor;
    account::input(&mut model, Key::RetrieveQueued, &mut session);
    account::input(&mut model, Key::Text(" edited".into()), &mut session);
    account::input(&mut model, Key::Enter, &mut session);
    assert_eq!(model.editor.text, "previous\n  draft");
    assert_eq!(model.editor.cursor, original_cursor);
    assert_eq!(
        model.account.as_ref().unwrap().pending_messages(),
        ["withdraw me edited"]
    );
    release.send(()).unwrap();
    finish(&mut model, &mut session);
    assert_eq!(entered.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
    finish(&mut model, &mut session);
    assert!(
        model
            .account
            .as_ref()
            .unwrap()
            .pending_messages()
            .is_empty()
    );
}

#[test]
fn editing_a_recalled_prompt_ends_history_browsing() {
    let (mut model, mut session, _, release) = running(false);
    model.editor.insert("unsent draft");
    account::input(&mut model, Key::HistoryPrevious, &mut session);
    assert_eq!(model.editor.text, "original task");
    account::input(&mut model, Key::Text(" revised".into()), &mut session);
    assert!(!model.prompt_history.browsing());
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert_eq!(model.editor.text, "original task revised");
    release.send(()).unwrap();
    finish(&mut model, &mut session);
}

#[test]
fn slash_command_in_history_stays_literal_during_arrow_navigation() {
    let (mut model, mut session, _, release) = running(false);
    model.prompt_history.record("/help");
    account::input(&mut model, Key::HistoryPrevious, &mut session);
    assert_eq!(model.editor.text, "/help");
    assert!(model.menu.pasted_literal);
    account::input(&mut model, Key::HistoryNext, &mut session);
    assert!(model.editor.text.is_empty());
    release.send(()).unwrap();
    finish(&mut model, &mut session);
}
