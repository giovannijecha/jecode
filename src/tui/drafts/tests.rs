use super::*;

#[test]
fn five_identical_prompts_remain_five_distinct_paused_drafts() {
    let mut queue = Queue::default();
    for _ in 0..5 {
        queue.push("same prompt".into()).unwrap();
    }
    queue.pause();
    assert!(queue.messages.is_empty());
    assert_eq!(queue.len(), 5);
    assert_eq!(queue.paused.len(), 5);
    assert!(queue.paused.iter().all(|draft| draft.text == "same prompt"));
    let (queued, paused) = queue.snapshot(&Editor::default());
    assert!(queued.is_empty());
    assert_eq!(paused.len(), 5);
    assert_eq!(queue.take(2).as_deref(), Some("same prompt"));
    assert_eq!(queue.len(), 4);
}

#[test]
fn queue_edit_save_cancel_and_discard_affect_only_selected_slot() {
    let mut queue = Queue::default();
    queue.push("first".into()).unwrap();
    queue.push("second".into()).unwrap();
    queue.push("third".into()).unwrap();
    let mut editor = Editor::default();
    editor.insert("unfinished composer");
    editor.cursor = 4;

    queue.begin_edit(1, &mut editor).unwrap();
    assert_eq!(queue.edit_index(), Some(1));
    editor.replace("second revised".into());
    assert_eq!(queue.draft(&editor).text, "unfinished composer");
    assert_eq!(queue.draft(&editor).cursor, 4);
    let (queued, _) = queue.snapshot(&editor);
    assert_eq!(queued, ["first", "second revised", "third"]);
    assert!(queue.save_edit(&mut editor));
    assert_eq!(editor.text, "unfinished composer");
    assert_eq!(editor.cursor, 4);
    assert_eq!(queue.messages[1], "second revised");

    queue.begin_edit(0, &mut editor).unwrap();
    editor.replace("unsaved".into());
    assert!(queue.cancel_edit(&mut editor));
    assert_eq!(queue.messages[0], "first");
    assert_eq!(editor.cursor, 4);
    assert!(queue.discard(1));
    assert_eq!(
        queue.messages,
        VecDeque::from(["first".into(), "third".into()])
    );
    assert_eq!(editor.text, "unfinished composer");
}

#[test]
fn pause_during_edit_preserves_slot_and_live_snapshot() {
    let mut queue = Queue::default();
    queue.push("automatic one".into()).unwrap();
    queue.push("automatic two".into()).unwrap();
    let mut already_paused = Editor::default();
    already_paused.replace("already paused".into());
    already_paused.cursor = 2;
    queue.paused.push(already_paused);
    let mut editor = Editor::default();
    editor.insert("main draft");
    editor.cursor = 3;
    queue.begin_edit(1, &mut editor).unwrap();
    editor.replace("live revision".into());
    editor.cursor = 5;

    queue.pause();
    assert_eq!(queue.edit_index(), Some(1));
    assert_eq!(queue.len(), 3);
    assert!(queue.messages.is_empty());
    let (queued, paused) = queue.snapshot(&editor);
    assert!(queued.is_empty());
    assert_eq!(paused[0].text, "automatic one");
    assert_eq!(paused[1].text, "live revision");
    assert_eq!(paused[1].cursor, 5);
    assert_eq!(paused[2].text, "already paused");
    assert!(queue.save_edit(&mut editor));
    assert_eq!(queue.paused[1].text, "live revision");
    assert_eq!(queue.paused[1].cursor, 5);
    assert_eq!(editor.text, "main draft");
    assert_eq!(editor.cursor, 3);
}

#[test]
fn appending_automatic_message_keeps_paused_edit_in_its_original_slot() {
    let mut queue = Queue::default();
    queue.push("automatic".into()).unwrap();
    let mut paused = Editor::default();
    paused.replace("paused original".into());
    paused.cursor = 4;
    queue.paused.push(paused);
    let mut editor = Editor::default();
    editor.insert("main");
    queue.begin_edit(1, &mut editor).unwrap();
    editor.replace("paused revision".into());
    editor.cursor = 6;

    queue.push("new automatic".into()).unwrap();
    assert_eq!(queue.edit_index(), Some(2));
    let (queued, paused) = queue.snapshot(&editor);
    assert_eq!(queued, ["automatic", "new automatic"]);
    assert_eq!(paused[0].text, "paused revision");
    assert_eq!(paused[0].cursor, 6);
    queue.pause();
    assert_eq!(queue.edit_index(), Some(2));
    assert!(queue.save_edit(&mut editor));
    assert_eq!(queue.paused[2].text, "paused revision");
    assert_eq!(editor.text, "main");
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
        history.record(&i.to_string());
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
        history.record(&i.to_string());
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
