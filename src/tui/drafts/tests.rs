use super::*;

#[test]
fn one_draft_is_pending_and_a_second_waits_in_the_composer() {
    let mut queue = Queue::default();
    assert!(queue.is_empty());
    queue.push("first").unwrap();
    assert_eq!(queue.pending(), Some(("first".into(), false)));
    assert_eq!(queue.push("second").unwrap_err(), PENDING);
    assert_eq!(queue.messages(), ["first"]);

    queue.pause();
    assert!(queue.messages().is_empty());
    assert_eq!(queue.pending(), Some(("first".into(), true)));
    assert_eq!(queue.push("second").unwrap_err(), PENDING);
}

#[test]
fn taking_the_draft_back_returns_it_until_the_agent_accepts_it() {
    let mut queue = Queue::default();
    queue.push("queued").unwrap();
    assert_eq!(queue.take().unwrap().text, "queued");
    assert!(queue.is_empty());
    assert!(queue.take().is_none());

    queue.push("accepted").unwrap();
    queue.capture(|queued, paused| {
        assert_eq!(queued, ["accepted"]);
        assert!(paused.is_none());
    });
    assert_eq!(queue.inbox().accept(|_, _| Ok(())).unwrap(), ["accepted"]);
    assert!(queue.take().is_none());
    assert!(queue.is_empty());
}

#[test]
fn a_paused_draft_is_taken_back_with_its_caret() {
    let mut queue = Queue::default();
    let mut editor = Editor::default();
    editor.insert("paused text");
    editor.cursor = 3;
    queue.paused = Some(editor);
    let taken = queue.take().unwrap();
    assert_eq!((taken.text.as_str(), taken.cursor), ("paused text", 3));
    assert!(queue.is_empty());
}

#[test]
fn parking_joins_text_into_the_one_paused_draft() {
    let mut queue = Queue::default();
    let draft = |text: &str| {
        let mut editor = Editor::default();
        editor.insert(text);
        editor.cursor = 1;
        editor
    };
    queue.park(draft("   "));
    assert!(queue.paused.is_none());
    queue.park(draft("older"));
    assert_eq!(queue.paused.as_ref().unwrap().cursor, 1);
    queue.park(draft("newer"));
    assert_eq!(queue.paused.as_ref().unwrap().text, "older\n\nnewer");
    queue.capture(|queued, paused| {
        assert!(queued.is_empty());
        assert_eq!(paused.unwrap().text, "older\n\nnewer");
    });
}

#[test]
fn only_captured_messages_are_deliverable() {
    let mut queue = Queue::default();
    queue.push("uncaptured").unwrap();
    assert!(queue.inbox().accept(|_, _| Ok(())).unwrap().is_empty());
    queue.capture(|_, _| ());
    assert_eq!(queue.inbox().accept(|_, _| Ok(())).unwrap(), ["uncaptured"]);
}

#[test]
fn reset_replaces_the_pending_draft() {
    let mut queue = Queue::default();
    queue.push("queued").unwrap();
    let mut editor = Editor::default();
    editor.insert("restored");
    queue.reset(Some(editor));
    assert!(queue.messages().is_empty());
    assert_eq!(queue.pending(), Some(("restored".into(), true)));
}

#[test]
fn history_filters_commands_and_restores_original_after_recall_edits() {
    let mut history = History::default();
    history.restore(vec![
        "legacy prompt".into(),
        "/help".into(),
        "  /resume".into(),
        "last prompt".into(),
    ]);
    history.record(" /status");
    assert_eq!(history.snapshot(), ["legacy prompt", "last prompt"]);

    let mut editor = Editor::default();
    editor.insert("unsent draft");
    editor.cursor = 3;
    history.navigate(&mut editor, true);
    assert!(history.is_browsing());
    assert!(!history.has_edited_recall());
    assert_eq!(editor.text, "last prompt");
    editor.replace("edited recalled prompt".into());
    history.edited();
    assert!(history.has_edited_recall());
    history.navigate(&mut editor, false);
    assert_eq!(editor.text, "unsent draft");
    assert_eq!(editor.cursor, 3);
    assert!(!history.is_browsing());

    history.navigate(&mut editor, true);
    editor.replace("sent recalled prompt".into());
    history.edited();
    history.record(&editor.text);
    history.submitted(&mut editor);
    assert_eq!(editor.text, "unsent draft");
    assert_eq!(editor.cursor, 3);
    assert_eq!(history.snapshot().last().unwrap(), "sent recalled prompt");
    assert!(!history.is_browsing());
}

#[test]
fn history_is_bounded_and_cancel_restores_caret() {
    let mut history = History::default();
    for i in 0..60 {
        history.record(i.to_string());
    }
    assert_eq!(history.snapshot().len(), 50);
    let mut editor = Editor::default();
    editor.insert("unfinished");
    editor.cursor = 3;
    history.navigate(&mut editor, true);
    for _ in 0..60 {
        history.navigate(&mut editor, true);
    }
    assert_eq!(editor.text, "10");
    assert!(history.cancel(&mut editor));
    assert_eq!(editor.text, "unfinished");
    assert_eq!(editor.cursor, 3);
}

#[test]
fn repeated_dispatched_prompts_remain_distinct_history_entries() {
    let mut history = History::default();
    history.record("same");
    history.record("same");
    assert_eq!(history.snapshot(), ["same", "same"]);
}

#[test]
fn automatic_sends_preserve_history_navigation_without_creating_an_edited_recall() {
    let mut history = History::default();
    for i in 0..50 {
        history.record(i.to_string());
    }
    let mut editor = Editor::default();
    editor.replace("main draft".into());
    editor.cursor = 3;
    history.navigate(&mut editor, true);
    history.record("automatic prompt");
    assert_eq!(editor.text, "49");
    assert!(!history.has_edited_recall());
    history.navigate(&mut editor, false);
    assert_eq!(editor.text, "automatic prompt");
    history.navigate(&mut editor, false);
    assert_eq!(editor.text, "main draft");
    assert_eq!(editor.cursor, 3);

    for _ in 0..50 {
        history.navigate(&mut editor, true);
    }
    assert_eq!(editor.text, "1");
    history.record("another automatic prompt");
    assert!(!history.has_edited_recall());
    history.navigate(&mut editor, true);
    assert_eq!(editor.text, "1");
    history.navigate(&mut editor, false);
    assert_eq!(editor.text, "2");
}
